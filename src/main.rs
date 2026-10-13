use std::{sync::Arc, thread};

use tokio_util::sync::CancellationToken;
use tracing::error;

use crate::{
    config::Config,
    core::{
        context::Context, instance::Instance, runner::run_instance_pipeline, runtime::build_runtime,
    },
    state::State,
};

mod config;
mod core;
mod event;
mod fs;
mod state;
mod traceevent;

#[derive(Debug, Default)]
struct RasMemContext {
    config: Config,
    state: State,
}

impl Context for RasMemContext {
    fn config(&self) -> &Config {
        &self.config
    }
    fn state(&self) -> &State {
        &self.state
    }
}

fn all_instances() -> Vec<Instance> {
    vec![Instance::Dram, Instance::Hbm]
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();

    let (ctx, ct) = (Arc::new(RasMemContext::default()), CancellationToken::new());
    let mut handles = Vec::new();

    for instance in all_instances() {
        let (cxt_clone, ct_clone) = (ctx.clone(), ct.clone());
        let ct_for_thread = ct.clone();
        let handle = thread::Builder::new()
            .name(format!("{}-supervisor", instance.name()))
            .spawn(move || -> anyhow::Result<()> {
                let res = (|| {
                    let rt = build_runtime(instance)?;
                    rt.block_on(async move {
                        run_instance_pipeline(instance, cxt_clone, ct_clone, 1024).await
                    })
                })();
                if let Err(_e) = &res {
                    ct_for_thread.cancel();
                }
                res
            })?;
        handles.push((instance, handle));
    }

    ct.cancelled().await;

    for (instance, h) in handles {
        match h.join() {
            Ok(Ok(())) => error!("{instance:?} ok"),
            Ok(Err(e)) => error!("{instance:?} failed: {e:#}"),
            Err(_) => error!("{instance:?} panicked"),
        }
    }

    anyhow::Ok(())
}
