use tokio::runtime::{Builder, Runtime};

use crate::core::instance::Instance;

pub fn build_runtime(instance: Instance) -> anyhow::Result<Runtime> {
    let rt = Builder::new_multi_thread()
        .thread_name(format!("tokio-{}", instance.thread_name()))
        .worker_threads(instance.worker_threads())
        .enable_all()
        .build()?;
    Ok(rt)
}
