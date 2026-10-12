use std::{collections::HashMap, sync::Arc};

use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

use crate::{
    core::{context::Context, handler::EventHandler},
    event::Message,
};

pub struct EventProcessor<C: Context + 'static> {
    handlers: HashMap<u32, Arc<dyn EventHandler<C>>>,
    ctx: Arc<C>,
}

impl<C: Context + Send + Sync + 'static> EventProcessor<C> {
    pub fn new(handlers: HashMap<u32, Arc<dyn EventHandler<C>>>, ctx: Arc<C>) -> Self {
        Self { handlers, ctx }
    }

    pub async fn run(
        self,
        mut rx: mpsc::Receiver<Message>,
        cancellation_token: CancellationToken,
    ) -> anyhow::Result<()> {
        let Self { handlers, ctx } = self;

        loop {
            let msg = tokio::select! {
                _ = cancellation_token.cancelled() => break,
                message = rx.recv() => match message {
                    Some(e) => e,
                    None => break,
                },
            };

            if let Some(h) = handlers.get(&msg.id) {
                h.process(&msg.event, &ctx);
            }
        }

        anyhow::Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::{
        sync::Mutex,
        time::{Duration, UNIX_EPOCH},
    };

    use crate::{
        core::instance::Instance,
        event::{
            Event,
            mc::{McEvent, event::HwEventErrType},
        },
        traceevent::{
            tep::{TepEvent, TepHandle, TepRecord},
            trace_seq::TraceSeq,
        },
    };

    use super::*;

    struct TestContext;
    impl Context for TestContext {
        fn config(&self) -> &crate::config::Config {
            unimplemented!()
        }

        fn state(&self) -> &crate::state::State {
            unimplemented!()
        }
    }

    struct TestHandler {
        num_events: Arc<Mutex<usize>>,
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
            _record: &TepRecord<'_>,
            _ctx: &C,
        ) -> Result<crate::event::Event, String> {
            unimplemented!()
        }
        fn process(&self, _event: &Event, _ctx: &C) {
            let mut n = self.num_events.lock().unwrap();
            *n += 1;
        }
    }

    struct Fixture {
        handlers: HashMap<u32, Arc<dyn EventHandler<TestContext>>>,
        ctx: Arc<TestContext>,
        num_events: Arc<Mutex<usize>>,
    }

    impl Fixture {
        fn new() -> Self {
            let num_events = Arc::new(Mutex::new(0));
            let mut handlers = HashMap::new();
            handlers.insert(
                42,
                Arc::new(TestHandler {
                    num_events: num_events.clone(),
                }) as Arc<dyn EventHandler<TestContext>>,
            );
            Self {
                handlers,
                ctx: Arc::new(TestContext {}),
                num_events,
            }
        }

        fn make_message(&self, id: u32) -> Message {
            Message {
                event: Event::Mc(McEvent {
                    timestamp: UNIX_EPOCH,
                    error_count: 1,
                    error_type: HwEventErrType::Corrected,
                    msg: "msg".into(),
                    label: "dimm0".into(),
                    mc_index: 0,
                    top_layer: -1,
                    middle_layer: -1,
                    lower_layer: -1,
                    address: 0,
                    grain: 0,
                    syndrome: 0,
                    driver_detail: "".into(),
                }),
                id,
                cpu: 0,
                ts: 0,
            }
        }
    }

    #[tokio::test]
    async fn test_run() {
        let fx = Fixture::new();
        let processor = EventProcessor::new(fx.handlers.clone(), fx.ctx.clone());
        let (tx, rx) = mpsc::channel(10);
        let token = CancellationToken::new();

        let h = tokio::spawn(processor.run(rx, token.clone()));
        tx.send(fx.make_message(42)).await.unwrap();
        tx.send(fx.make_message(42)).await.unwrap();
        tx.send(fx.make_message(42)).await.unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(*fx.num_events.lock().unwrap(), 3);

        token.cancel();
        h.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_run_with_unregistered_id() {
        let fx = Fixture::new();
        let processor = EventProcessor::new(fx.handlers.clone(), fx.ctx.clone());
        let (tx, rx) = mpsc::channel(10);
        let token = CancellationToken::new();

        let h = tokio::spawn(processor.run(rx, token.clone()));
        tx.send(fx.make_message(99)).await.unwrap();

        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(*fx.num_events.lock().unwrap(), 0);

        token.cancel();
        h.await.unwrap().unwrap();
    }

    #[tokio::test]
    async fn test_run_with_cancellation() {
        let fx = Fixture::new();
        let processor = EventProcessor::new(fx.handlers.clone(), fx.ctx.clone());
        let (_tx, rx) = mpsc::channel(10);
        let token = CancellationToken::new();

        let h = tokio::spawn(processor.run(rx, token.clone()));
        tokio::time::sleep(Duration::from_millis(10)).await;
        token.cancel();

        tokio::time::timeout(Duration::from_secs(1), h)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_run_with_dropped_tx() {
        let fx = Fixture::new();
        let processor = EventProcessor::new(fx.handlers.clone(), fx.ctx.clone());
        let (tx, rx) = mpsc::channel(10);
        let token = CancellationToken::new();

        let h = tokio::spawn(processor.run(rx, token));
        drop(tx);
        tokio::time::timeout(Duration::from_secs(1), h)
            .await
            .unwrap()
            .unwrap()
            .unwrap();
    }
}
