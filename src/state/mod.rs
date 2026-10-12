use crate::event::mc::McState;
use std::sync::Mutex;

#[derive(Debug, Default)]
pub struct State {
    pub mc_state: Mutex<McState>,
}
