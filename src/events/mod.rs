pub mod mc;
pub mod registry;

use crate::events::mc::McEvent;

#[derive(Debug, Clone)]
pub enum Event {
    Mc(McEvent),
}
