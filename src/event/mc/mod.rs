pub mod config;
pub mod event;
pub mod handler;
pub mod state;

pub use config::McConfig;
pub use event::McEvent;
pub use handler::McEventHandler;
pub use state::McState;

use crate::core::context::Context;
use std::sync::MutexGuard;

pub trait McContext: Context {
    fn mc_config(&self) -> &McConfig {
        &self.config().mc_config
    }

    fn mc_state(&self) -> MutexGuard<'_, McState> {
        self.state()
            .mc_state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }
}

impl<C: Context> McContext for C {}
