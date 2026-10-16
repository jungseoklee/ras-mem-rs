use crate::event::{mc::McConfig, sched_switch::config::SchedSwitchConfig};

#[derive(Debug, Default)]
pub struct Config {
    // global configuration
    pub user_hz: i64,
    pub use_uptime: bool,
    pub uptime_diff: i64,

    // event specific configuration
    pub mc_config: McConfig,
    pub sched_switch_config: SchedSwitchConfig,
}
