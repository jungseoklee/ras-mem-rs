pub mod mc;
pub mod registry;
pub mod sched_switch;

use crate::event::{mc::McEvent, sched_switch::event::SchedSwitchEvent};

#[derive(Debug, Clone)]
pub enum Event {
    Mc(McEvent),
    SchedSwitch(SchedSwitchEvent),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub event: Event,
    pub id: u32,
    pub cpu: u32,
    pub ts: u64,
}
