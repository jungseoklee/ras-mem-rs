use std::{collections::HashMap, sync::Arc};

use anyhow::Context as _;
use tokio::{sync::mpsc, task::JoinSet};
use tokio_util::sync::CancellationToken;

use crate::{
    core::{context::Context, instance::Instance, processor::EventProcessor, reader::TraceReader},
    event::{self, Message},
    fs::tracefs::TraceFs,
    traceevent::tep::TepHandle,
};

pub async fn run_instance_pipeline<C: Context + Send + Sync + 'static>(
    instance: Instance,
    ctx: Arc<C>,
    cancellation_token: CancellationToken,
    channel_size: usize,
) -> anyhow::Result<()> {
    let fs =
        TraceFs::from_instance(instance).with_context(|| format!("{instance:?} no tracefs"))?;
    run_instance_pipeline_with_fs(fs, instance, ctx, cancellation_token, channel_size).await
}

pub async fn run_instance_pipeline_with_fs<C: Context + Send + Sync + 'static>(
    fs: TraceFs,
    instance: Instance,
    ctx: Arc<C>,
    cancellation_token: CancellationToken,
    channel_size: usize,
) -> anyhow::Result<()> {
    anyhow::ensure!(channel_size > 0, "channel_size must be > 0");

    let header_page_path = fs.header_page_path();
    let header_page = std::fs::read_to_string(&header_page_path)
        .with_context(|| format!("read {header_page_path:?}"))?;

    anyhow::ensure!(!header_page.trim().is_empty(), "header_page: empty");

    let mut tep = TepHandle::new();
    tep.parse_header_page(&header_page, header_page.len(), 0)
        .map_err(|e| anyhow::anyhow!("parse_header_page: {e}"))?;

    // TODO: should we enforce this validation in tep.parse_header_page?
    anyhow::ensure!(
        header_page.contains("field:") && header_page.contains("timestamp"),
        "parse_header_page: invalid header_page"
    );

    let handlers_for_instance = event::registry::handlers_for::<C>(instance);
    if handlers_for_instance.is_empty() {
        eprintln!("{instance:?}: no handlers registered, skipping");
        return anyhow::Ok(());
    }

    let mut handlers = HashMap::new();
    let mut events_to_enable = Vec::new();
    for h in handlers_for_instance {
        let event_format_path = fs.event_format_path(h.group(), h.name());
        let event_format = std::fs::read_to_string(&event_format_path)
            .with_context(|| format!("read {event_format_path:?}"))?;
        let id = tep
            .parse_event(&event_format, event_format.len(), h.group())
            .map_err(|e| anyhow::anyhow!("parse_event {}:{}: {e}", h.group(), h.name()))?;
        events_to_enable.push(format!("{}:{}", h.group(), h.name()));
        handlers.insert(id, Arc::from(h));
    }
    fs.enable(events_to_enable)?;

    let local_cancellation_token = cancellation_token.child_token();

    let (tx, rx) = mpsc::channel::<Message>(channel_size);
    let tep = Arc::new(tep);

    let reader = TraceReader::new(fs, tep.clone(), handlers.clone(), ctx.clone());
    let processor = EventProcessor::new(handlers, ctx.clone());
    let mut set = JoinSet::new();

    set.spawn(reader.run(tx, local_cancellation_token.clone()));
    set.spawn(processor.run(rx, local_cancellation_token.clone()));

    tokio::select! {
        Some(res) = set.join_next() => { eprintln!("{instance:?} task done: {res:?}"); }
        _ = cancellation_token.cancelled() => { eprintln!("{instance:?} global cancel"); }
    }

    local_cancellation_token.cancel();
    set.abort_all();
    while let Some(_) = set.join_next().await {}
    anyhow::Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        core::{context::Context, instance::Instance},
        fs::tracefs::TraceFs,
    };
    use std::{fs, path::PathBuf, sync::Arc};
    use tempfile::TempDir;
    use tokio_util::sync::CancellationToken;

    struct TestContext;
    impl Context for TestContext {
        fn config(&self) -> &crate::config::Config {
            unimplemented!()
        }

        fn state(&self) -> &crate::state::State {
            unimplemented!()
        }
    }

    struct Fixture {
        _tmp: TempDir,
        root: PathBuf,
    }
    impl Fixture {
        fn new() -> Self {
            let tmp = TempDir::new().unwrap();
            let root = tmp.path().to_path_buf();
            fs::create_dir_all(root.join("instances/dram/per_cpu/cpu0")).unwrap();
            fs::create_dir_all(root.join("events/ras/mc_event")).unwrap();
            fs::create_dir_all(root.join("events/header_page").parent().unwrap()).unwrap();
            for p in [
                "instances/dram/set_event",
                "instances/dram/buffer_size_kb",
                "instances/dram/buffer_percent",
                "instances/dram/buffer_subbuf_size_kb",
                "instances/dram/buffer_subbuf_size",
                "events/header_page",
            ] {
                fs::File::create(root.join(p)).unwrap();
            }
            fs::write(root.join("instances/dram/buffer_subbuf_size_kb"), "4").unwrap();
            fs::write(root.join("instances/dram/buffer_subbuf_size"), "4096").unwrap();
            fs::write(
                root.join("instances/dram/per_cpu/cpu0/trace_pipe_raw"),
                vec![0u8; 4096],
            )
            .unwrap();
            fs::write(
                root.join("events/header_page"),
                r#"
field: u64 timestamp; offset:0; size:8; signed:0;
field: local_t commit; offset:8; size:8; signed:1;
field: int overwrite; offset:8; size:8; signed:1;
field: char data; offset:16; size:4080; signed:0;
"#,
            )
            .unwrap();
            fs::write(
                root.join("events/ras/mc_event/format"),
                r#"
name: mc_event
ID: 42
format:
	field:unsigned short common_type;	offset:0;	size:2;	signed:0;
	field:unsigned char common_flags;	offset:2;	size:1;	signed:0;
	field:unsigned char common_preempt_count;	offset:3;	size:1;	signed:0;
	field:int common_pid;	offset:4;	size:4;	signed:1;
	field:__data_loc char[] msg;	offset:8;	size:4;	signed:0;
print fmt: "%s", REC->msg
"#,
            )
            .unwrap();
            Self { _tmp: tmp, root }
        }
        fn make_tracefs(&self) -> TraceFs {
            TraceFs::from_path(self.root.clone(), "dram").unwrap()
        }
        fn remove_header_page(&self) {
            fs::remove_file(self.root.join("events/header_page")).unwrap();
        }
        fn write_header_page(&self, s: &str) {
            fs::write(self.root.join("events/header_page"), s).unwrap();
        }
        fn remove_event_format(&self) {
            fs::remove_file(self.root.join("events/ras/mc_event/format")).unwrap();
        }
        fn write_format(&self, s: &str) {
            fs::write(self.root.join("events/ras/mc_event/format"), s).unwrap();
        }
    }

    #[tokio::test]
    async fn test_run_instance_pipeline() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);

        let res =
            run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, CancellationToken::new(), 16)
                .await;

        assert!(res.is_ok());
        let set_event = fs::read_to_string(fx.root.join("instances/dram/set_event")).unwrap();
        assert!(set_event.contains("ras:mc_event"));
    }

    #[tokio::test]
    async fn test_run_instance_pipeline_with_zero_channel_size() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);

        let res =
            run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, CancellationToken::new(), 0)
                .await;

        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("channel_size must be > 0")
        );
    }

    #[tokio::test]
    async fn test_run_instance_pipeline_without_header_page() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);
        fx.remove_header_page();

        let res =
            run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, CancellationToken::new(), 10)
                .await;

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("events/header_page"));
    }

    #[tokio::test]
    async fn test_run_instance_pipeline_with_invalid_header_page() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);
        fx.write_header_page("invalid");

        let res =
            run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, CancellationToken::new(), 10)
                .await;

        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("parse_header_page: invalid header_page")
        );
    }

    #[tokio::test]
    async fn test_run_instance_pipeline_without_event_format() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);
        fx.remove_event_format();

        let res =
            run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, CancellationToken::new(), 10)
                .await;

        assert!(res.is_err());
        assert!(
            res.unwrap_err()
                .to_string()
                .contains("events/ras/mc_event/format")
        );
    }

    #[tokio::test]
    async fn test_run_instance_pipeline_with_invalid_event_format() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);
        fx.write_format("invalid");

        let res =
            run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, CancellationToken::new(), 10)
                .await;

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("parse_event"));
    }

    #[tokio::test]
    async fn test_run_instance_pipeline_with_cancellation() {
        let fx = Fixture::new();
        let fs = fx.make_tracefs();
        let ctx = Arc::new(TestContext);
        let token = CancellationToken::new();
        token.cancel();

        let res = run_instance_pipeline_with_fs(fs, Instance::Dram, ctx, token, 8).await;

        assert!(res.is_ok());
    }

    #[test]
    fn test_coverage_workaround() {
        let ctx = TestContext;
        let _ = std::panic::catch_unwind(|| {
            let _ = ctx.config();
        });
        let _ = std::panic::catch_unwind(|| {
            let _ = ctx.state();
        });
    }
}
