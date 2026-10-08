use crate::event::mc::McConfig;

#[derive(Debug, Default)]
pub struct Config {
    // global configuration
    pub user_hz: i64,
    pub use_uptime: bool,
    pub uptime_diff: i64,

    // hw specific configuration
    pub mc_config: McConfig,
}
