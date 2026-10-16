use std::{sync::Arc, thread};

use tokio_util::sync::CancellationToken;
use tracing::{error, info};

use crate::{
    config::Config,
    core::{runner::run_instance_pipeline, runtime::build_runtime},
    state::State,
};

mod config;
mod core;
mod event;
mod fs;
mod state;
mod traceevent;

pub use core::context::Context;
pub use core::instance::Instance;

#[derive(Debug, Default)]
pub struct RasMemContext {
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

pub async fn main_with_instances(
    instances: Vec<Instance>,
    ct: CancellationToken,
) -> anyhow::Result<Arc<RasMemContext>> {
    let _ = tracing_subscriber::fmt::try_init();

    let ctx = Arc::new(RasMemContext::default());
    let mut handles = Vec::new();

    for instance in instances {
        let (cxt_clone, ct_clone) = (ctx.clone(), ct.clone());
        let ct_for_thread = ct.clone();
        let handle = thread::Builder::new()
            .name(format!("su-{}", instance.thread_name()))
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
            Ok(Ok(())) => info!("{instance:?} ok"),
            Ok(Err(e)) => error!("{instance:?} failed: {e:#}"),
            Err(_) => error!("{instance:?} panicked"),
        }
    }

    anyhow::Ok(ctx)
}
