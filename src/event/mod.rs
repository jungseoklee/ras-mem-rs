pub mod mc;
pub mod registry;

use crate::event::mc::McEvent;

#[derive(Debug, Clone)]
pub enum Event {
    Mc(McEvent),
}

#[derive(Debug, Clone)]
pub struct Message {
    pub event: Event,
    pub id: u32,
    pub cpu: u32,
    pub ts: u64,
}
