use std::fs::{File, OpenOptions};
use std::os::unix::fs::OpenOptionsExt;
use std::{collections::HashMap, sync::Arc};

use tokio::io::unix::AsyncFd;
use tokio::sync::mpsc;
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

use crate::traceevent::common::{Endian, LongSize};
use crate::traceevent::kbuffer::KBuffer;
use crate::traceevent::tep::TepRecord;
use crate::traceevent::trace_seq::TraceSeq;
use crate::{
    core::{context::Context, handler::EventHandler},
    event::Event,
    fs::tracefs::TraceFs,
    traceevent::tep::TepHandle,
};

pub struct TraceReader<C: Context + 'static> {
    fs: TraceFs,
    tep: Arc<TepHandle<C>>,
    handlers: HashMap<u32, Arc<dyn EventHandler<C>>>,
    ctx: Arc<C>,
}

impl<C: Context + Send + Sync + 'static> TraceReader<C> {
    pub fn new(
        fs: TraceFs,
        tep: Arc<TepHandle<C>>,
        handlers: HashMap<(String, String), Arc<dyn EventHandler<C>>>,
        ctx: Arc<C>,
    ) -> Self {
        Self {
            fs,
            handlers: handlers
                .into_iter()
                .filter_map(|((g, n), h)| Some((tep.find_event_by_name(&g, &n)?.id, h)))
                .collect(),
            tep,
            ctx,
        }
    }

    pub async fn run(
        self,
        tx: mpsc::Sender<Event>,
        cancel: CancellationToken,
    ) -> anyhow::Result<()> {
        let Self {
            fs,
            tep,
            handlers,
            ctx,
        } = self;
        let handlers = Arc::new(handlers);
        let pipes = fs.per_cpu_trace_pipe_raw()?;
        let subbuf_size = fs.buffer_subbuf_size()?;
        let mut set = JoinSet::new();

        anyhow::ensure!(!pipes.is_empty(), "no online cpus");

        for (cpu, path) in pipes {
            let file = match OpenOptions::new()
                .read(true)
                .custom_flags(libc::O_NONBLOCK)
                .open(&path)
            {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("skipping cpu={cpu} open {path:?} failed: {e:#}");
                    continue;
                }
            };
            let fd = match AsyncFd::new(file) {
                Ok(fd) => fd,
                Err(e) => {
                    eprintln!("skipping cpu={cpu} AsyncFd failed: {e}");
                    continue;
                }
            };
            set.spawn(run_cpu_reader(
                cpu,
                fd,
                tep.clone(),
                handlers.clone(),
                ctx.clone(),
                tx.clone(),
                cancel.clone(),
                subbuf_size,
            ));
        }
        if set.is_empty() {
            anyhow::bail!("all per-cpu readers failed to open");
        }

        drop(tx); // drop original sender — channel closes when all readers exit
        let mut failed = false;
        while let Some(r) = set.join_next().await {
            match r {
                Ok(Ok(())) => {}
                Ok(Err(e)) => {
                    failed = true;
                    eprintln!("cpu reader failed: {e:#}");
                }
                Err(e) => {
                    failed = true;
                    eprintln!("cpu reader panicked: {e:#}");
                }
            }
        }
        if failed {
            anyhow::bail!("one or more cpu readers failed");
        }
        anyhow::Ok(())
    }
}

async fn run_cpu_reader<C: Context + Send + Sync + 'static>(
    cpu: u32,
    fd: AsyncFd<File>,
    tep: Arc<TepHandle<C>>,
    handlers: Arc<HashMap<u32, Arc<dyn EventHandler<C>>>>,
    ctx: Arc<C>,
    tx: mpsc::Sender<Event>,
    cancel: CancellationToken,
    subbuf_size: usize,
) -> anyhow::Result<()> {
    let mut kbuf = KBuffer::new(LongSize::SameAsHost, Endian::SameAsHost);
    let mut buf = vec![0u8; subbuf_size];
    let mut seq = TraceSeq::new();

    loop {
        if cancel.is_cancelled() || tx.is_closed() {
            break;
        }

        let mut filled: usize = 0;
        while filled < subbuf_size {
            if cancel.is_cancelled() || tx.is_closed() {
                break;
            }
            let mut guard = tokio::select! {
                r = fd.readable() => match r {
                    Ok(g) => g,
                    Err(_) => continue,
                },
                _ = cancel.cancelled() => break,
            };
            match guard.try_io(|inner| {
                use std::io::Read;
                inner.get_ref().read(&mut buf[filled..])
            }) {
                Ok(Ok(0)) => {
                    // pipe closed / EOF
                    guard.clear_ready();
                    break;
                }
                Ok(Ok(n)) => filled += n,
                Ok(Err(e)) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    guard.clear_ready();
                    continue;
                }
                Ok(Err(e)) => anyhow::bail!(e),
                Err(_would_block) => continue,
            }
        }
        if filled == 0 {
            continue;
        }
        if filled < subbuf_size {
            // partial should not happen on real tracefs, treat as bad page
            eprintln!("cpu={cpu} short read {filled} < {subbuf_size}");
            continue;
        }
        let page = std::mem::replace(&mut buf, Vec::new());
        let old = match kbuf.load_subbuffer(page) {
            Ok(old) => old,
            Err(e) => {
                eprintln!("cpu={cpu} bad subbuffer: {e}");
                buf = vec![0u8; subbuf_size];
                continue;
            }
        };
        buf = old;
        if buf.len() != subbuf_size {
            buf.resize(subbuf_size, 0);
        }
        while let Some((data, ts)) = kbuf.read_event() {
            if let Some(event) = parse_event(data, ts, cpu, &kbuf, &tep, &handlers, &ctx, &mut seq)
            {
                if tx.send(event).await.is_err() {
                    return Ok(());
                }
            }
        }
    }
    anyhow::Ok(())
}

fn parse_event<C: Context>(
    data: &[u8],
    ts: u64,
    cpu: u32,
    kbuf: &KBuffer,
    tep: &TepHandle<C>,
    handlers: &HashMap<u32, Arc<dyn EventHandler<C>>>,
    ctx: &C,
    seq: &mut TraceSeq,
) -> Option<Event> {
    if data.len() < 2 {
        return None;
    }
    let common_type = match kbuf.endian() {
        Endian::Little => u16::from_le_bytes([data[0], data[1]]),
        Endian::Big => u16::from_be_bytes([data[0], data[1]]),
        _ => unreachable!(
            "KBuffer::endian() must be Big/Little only, got {:?}",
            kbuf.endian()
        ),
    };
    let record = TepRecord {
        size: data.len(),
        offset: kbuf.curr_offset(),
        missed_events: kbuf.missed_events(),
        record_size: kbuf.curr_size() as usize,
        data: data.to_vec(), // TODO: use reference instead, a.k.a, zero copy.
        ts,
        common_type,
        cpu,
    };
    let event = tep.find_event_by_record(&record)?;
    let h = handlers.get(&event.id)?;
    seq.reset();
    match h.parse(tep, seq, event, &record, ctx) {
        Ok(ev) => Some(ev),
        Err(e) => {
            eprintln!("cpu={cpu} id={} parse failed: {e:#}", event.id);
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::core::context::Context;
    use crate::core::instance::Instance;
    use crate::event::mc::event::HwEventErrType;
    use crate::event::{Event, mc::McEvent};
    use crate::state::State;
    use crate::traceevent::common::{Endian, LongSize};
    use crate::traceevent::kbuffer::KBuffer;
    use crate::traceevent::tep::{TepEvent, TepHandle, TepRecord};
    use crate::traceevent::trace_seq::TraceSeq;
    use std::collections::HashMap;
    use std::fs::File;
    use std::os::unix::fs::OpenOptionsExt;
    use std::sync::Arc;
    use std::time::{Duration, UNIX_EPOCH};
    use tempfile::TempDir;
    use tokio::io::unix::AsyncFd;
    use tokio::sync::mpsc;
    use tokio_util::sync::CancellationToken;

    struct TestContext {
        config: Config,
        state: State,
    }
    impl Context for TestContext {
        fn config(&self) -> &Config {
            &self.config
        }
        fn state(&self) -> &State {
            &self.state
        }
    }

    struct TestHandler {
        failure: bool,
    }
    impl<C: Context> EventHandler<C> for TestHandler {
        fn group(&self) -> &'static str {
            "group"
        }
        fn name(&self) -> &'static str {
            "name"
        }
        fn instance(&self) -> Instance {
            Instance::Dram
        }
        fn parse(
            &self,
            _tep: &TepHandle<C>,
            _seq: &mut TraceSeq,
            _event: &TepEvent,
            rec: &TepRecord,
            _ctx: &C,
        ) -> Result<Event, String> {
            if self.failure {
                return Err("failure".into());
            }
            Ok(Event::Mc(McEvent {
                timestamp: UNIX_EPOCH + Duration::from_secs(rec.ts),
                error_count: 1,
                error_type: HwEventErrType::Corrected,
                msg: "msg".into(),
                label: "dimm0".into(),
                mc_index: rec.cpu as i64,
                top_layer: -1,
                middle_layer: -1,
                lower_layer: -1,
                address: 0,
                grain: 0,
                syndrome: 0,
                driver_detail: "".into(),
            }))
        }
        fn update_state(&self, _event: &Event, _ctx: &C) {}
    }

    struct Fixture {
        tmp: TempDir,
        tep: Arc<TepHandle<TestContext>>,
        kbuf: KBuffer,
        handlers: HashMap<(String, String), Arc<dyn EventHandler<TestContext>>>,
        handlers_by_id: HashMap<u32, Arc<dyn EventHandler<TestContext>>>,
        seq: TraceSeq,
        ctx: Arc<TestContext>,
        data: Vec<u8>,
    }
    impl Fixture {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();

            let mut handlers: HashMap<(String, String), Arc<dyn EventHandler<TestContext>>> =
                HashMap::new();
            handlers.insert(
                ("group".into(), "name".into()),
                Arc::new(TestHandler { failure: false }),
            );
            handlers.insert(
                ("noop".into(), "noop".into()),
                Arc::new(TestHandler { failure: false }),
            );

            let mut tep = TepHandle::new();
            tep.events_by_id.insert(
                42,
                TepEvent {
                    id: 42,
                    name: "name".into(),
                    group: "group".into(),
                    fields: vec![],
                    print_fmt: String::new(),
                },
            );
            tep.ids_by_name.insert(("group".into(), "name".into()), 42);

            let mut ret = Self {
                tmp,
                tep: Arc::new(tep),
                kbuf: KBuffer::new(LongSize::SameAsHost, Endian::SameAsHost),
                handlers,
                handlers_by_id: HashMap::new(),
                seq: TraceSeq::new(),
                ctx: Arc::new(TestContext {
                    config: Config::default(),
                    state: State::default(),
                }),
                data: {
                    let mut d = vec![0u8; 128];
                    d[0..2].copy_from_slice(&42u16.to_le_bytes());
                    d
                },
            };
            ret.refresh_handlers_by_id();
            ret
        }

        fn refresh_handlers_by_id(&mut self) {
            self.handlers_by_id.clear();
            for ((g, n), h) in &self.handlers {
                if let Some(id) = self.tep.ids_by_name.get(&(g.clone(), n.clone())) {
                    self.handlers_by_id.insert(*id, h.clone());
                }
            }
        }

        fn with_common_type(mut self, id: u16) -> Self {
            self.data[0..2].copy_from_slice(&id.to_le_bytes());
            self
        }

        fn with_failure(mut self) -> Self {
            let h: Arc<dyn EventHandler<TestContext>> = Arc::new(TestHandler { failure: true });
            // update both maps, keep invariant
            self.handlers
                .insert(("group".into(), "name".into()), h.clone());
            self.refresh_handlers_by_id();
            self
        }

        fn make_async_fd(&self, subbuf_size: usize) -> AsyncFd<File> {
            let path = self.tmp.path().join("async_fd");
            std::fs::write(&path, vec![0u8; subbuf_size]).unwrap();
            AsyncFd::new(
                std::fs::OpenOptions::new()
                    .read(true)
                    .custom_flags(libc::O_NONBLOCK)
                    .open(&path)
                    .unwrap(),
            )
            .unwrap()
        }

        fn make_page(&self) -> Vec<u8> {
            let payload = self.data.clone();
            let ds = 12;
            let commit = 4 + 4 + payload.len();
            let ts: u64 = 100;
            let mut page = vec![0u8; ds + commit];
            page[0..8].copy_from_slice(&ts.to_le_bytes());
            page[8..12].copy_from_slice(&(commit as u32).to_le_bytes());
            w32le(&mut page, ds, enc(0, 0));
            w32le(&mut page, ds + 4, (payload.len() + 4) as u32);
            page[ds + 8..ds + 8 + payload.len()].copy_from_slice(&payload);
            page
        }

        fn make_tracefs(&self, pages: Vec<Vec<u8>>, subbuf_size: usize) -> TraceFs {
            let root = self.tmp.path().join("instances/reader");
            let _ = std::fs::remove_dir_all(self.tmp.path().join("instances"));
            std::fs::create_dir_all(&root).unwrap();
            for f in [
                "set_event",
                "buffer_size_kb",
                "buffer_percent",
                "buffer_subbuf_size_kb",
                "buffer_subbuf_size",
            ] {
                std::fs::File::create(root.join(f)).unwrap();
            }
            std::fs::write(
                root.join("buffer_subbuf_size_kb"),
                (subbuf_size / 1024).to_string(),
            )
            .unwrap();
            std::fs::write(root.join("buffer_subbuf_size"), subbuf_size.to_string()).unwrap();

            for (cpu, page) in pages.into_iter().enumerate() {
                let dir = root.join(format!("per_cpu/cpu{}", cpu));
                std::fs::create_dir_all(&dir).unwrap();
                let mut buf = vec![0u8; subbuf_size];
                buf[..page.len().min(subbuf_size)]
                    .copy_from_slice(&page[..page.len().min(subbuf_size)]);
                std::fs::write(dir.join("trace_pipe_raw"), buf).unwrap();
            }
            TraceFs::from_path(self.tmp.path().to_path_buf(), "reader").unwrap()
        }
    }

    fn enc(delta: u32, tl: u32) -> u32 {
        (delta << 5) | (tl & 0x1f)
    }

    fn w32le(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    macro_rules! parse_event {
        ($fx:expr) => {
            super::parse_event(
                &$fx.data,
                100,
                3,
                &$fx.kbuf,
                &$fx.tep,
                &$fx.handlers_by_id,
                &$fx.ctx,
                &mut $fx.seq,
            )
        };
        ($fx:expr, ts = $ts:expr, cpu = $cpu:expr) => {
            super::parse_event(
                &$fx.data,
                $ts,
                $cpu,
                &$fx.kbuf,
                &$fx.tep,
                &$fx.handlers_by_id,
                &$fx.ctx,
                &mut $fx.seq,
            )
        };
        ($fx:expr, $data:expr) => {
            super::parse_event(
                $data,
                100,
                3,
                &$fx.kbuf,
                &$fx.tep,
                &$fx.handlers_by_id,
                &$fx.ctx,
                &mut $fx.seq,
            )
        };
    }

    #[tokio::test]
    async fn test_run() {
        let fx = Fixture::new();
        let page = fx.make_page();
        let fs = fx.make_tracefs(vec![page], 4096);
        let reader = TraceReader::new(fs, fx.tep.clone(), fx.handlers.clone(), fx.ctx.clone());
        let (tx, mut rx) = mpsc::channel(10);
        let token = CancellationToken::new();
        let token_cloned = token.clone();

        let h = tokio::spawn(async move { reader.run(tx, token_cloned).await });

        let ev = tokio::time::timeout(Duration::from_millis(100), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(matches!(ev, Event::Mc(_)));
        token.cancel();
        let _ = tokio::time::timeout(Duration::from_millis(100), h).await;
    }

    #[tokio::test]
    async fn test_run_with_cancellation() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs(vec![vec![0u8; 4096], vec![0u8; 4096]], 4096);
        let reader = TraceReader::new(fs, fx.tep.clone(), fx.handlers.clone(), fx.ctx.clone());
        let (tx, mut rx) = mpsc::channel(100);
        let token = CancellationToken::new();
        let token_cloned = token.clone();

        let h = tokio::spawn(async move { reader.run(tx, token_cloned).await });

        tokio::time::sleep(Duration::from_millis(10)).await;
        token.cancel();
        let res = tokio::time::timeout(Duration::from_secs(2), h)
            .await
            .unwrap()
            .unwrap();
        assert!(res.is_ok());
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_run_cpu_reader_with_cancellation() {
        let fx = Fixture::new();
        let fd = fx.make_async_fd(4096);
        let (tx, mut rx) = mpsc::channel(10);
        let token = CancellationToken::new();
        let h = tokio::spawn(run_cpu_reader(
            0,
            fd,
            fx.tep,
            Arc::new(fx.handlers_by_id),
            fx.ctx,
            tx,
            token.clone(),
            4096,
        ));
        tokio::time::sleep(Duration::from_millis(10)).await;
        token.cancel();
        tokio::time::timeout(Duration::from_millis(100), h)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
        assert!(rx.recv().await.is_none());
    }

    #[tokio::test]
    async fn test_run_cpu_reader_with_dropped_rx() {
        let fx = Fixture::new();
        let fd = fx.make_async_fd(4096);
        let (tx, rx) = mpsc::channel(1);
        drop(rx);
        let fut = run_cpu_reader(
            0,
            fd,
            fx.tep,
            Arc::new(fx.handlers_by_id),
            fx.ctx,
            tx,
            CancellationToken::new(),
            4096,
        );
        assert!(fut.await.is_ok());
    }

    #[test]
    fn test_parse_event() {
        let mut fx = Fixture::new();

        let Some(Event::Mc(mc)) = parse_event!(fx) else {
            panic!()
        };

        assert_eq!(mc.mc_index, 3);
        assert_eq!(mc.label, "dimm0");
    }

    #[test]
    fn test_parse_event_with_one_byte_data() {
        let mut fx = Fixture::new();

        assert!(parse_event!(fx, &[0u8; 1]).is_none());
    }

    #[test]
    fn test_parse_event_with_unknown_common_type() {
        let mut fx = Fixture::new().with_common_type(999);

        assert!(parse_event!(fx).is_none());
    }

    #[test]
    fn test_parse_event_with_failure() {
        let mut fx = Fixture::new().with_failure();

        assert!(parse_event!(fx).is_none());
    }
}
