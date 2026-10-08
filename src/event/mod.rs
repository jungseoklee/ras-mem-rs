pub mod mc;
pub mod registry;

use crate::event::mc::McEvent;

#[derive(Debug, Clone)]
pub enum Event {
    Mc(McEvent),
}
