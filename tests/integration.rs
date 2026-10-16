use std::time::Duration;

use ras_mem_rs::{Context, Instance, main_with_instances};
use tokio_util::sync::CancellationToken;

#[tokio::test]
#[cfg(target_os = "linux")]
#[ignore = r#"run this commmand: cargo test test_main_with_instances --no-run 2>&1 | sed -n 's/.*(\(.*\)).*/\1/p' | grep integration | xargs -I{} sudo {} --ignored --nocapture"#]
async fn test_main_with_instances() -> anyhow::Result<()> {
    let ct = CancellationToken::new();

    let ct_clone = ct.clone();
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_millis(50)).await;
        ct_clone.cancel();
    });

    let ctx = tokio::time::timeout(
        Duration::from_secs(5),
        main_with_instances(vec![Instance::Test], ct),
    )
    .await
    .expect("hang")?;

    let sched = ctx.as_ref().state().sched_switch_state.lock().unwrap();
    assert!(sched.switch_count > 0);

    Ok(())
}
