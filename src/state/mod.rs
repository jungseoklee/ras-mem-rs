use crate::event::{mc::McState, sched_switch::state::SchedSwitchState};
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct State {
    pub mc_state: Mutex<McState>,
    pub sched_switch_state: Mutex<SchedSwitchState>,
}
