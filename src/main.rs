use ras_mem_rs::{Instance, main_with_instances};
use tokio_util::sync::CancellationToken;

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let _ = main_with_instances(
        vec![Instance::Dram, Instance::Hbm],
        CancellationToken::new(),
    )
    .await;
    Ok(())
}
