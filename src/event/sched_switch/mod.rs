use std::sync::MutexGuard;

use crate::{
    core::context::Context,
    event::sched_switch::{config::SchedSwitchConfig, state::SchedSwitchState},
};

pub mod config;
pub mod event;
pub mod handler;
pub mod state;

pub trait SchedSwitchContext: Context {
    fn sched_switch_config(&self) -> &SchedSwitchConfig {
        &self.config().sched_switch_config
    }

    fn sched_switch_state(&self) -> MutexGuard<'_, SchedSwitchState> {
        self.state()
            .sched_switch_state
            .lock()
            .unwrap_or_else(|p| p.into_inner())
    }
}

impl<C: Context> SchedSwitchContext for C {}
