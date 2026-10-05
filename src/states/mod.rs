use crate::events::mc::McState;
use std::sync::Mutex;

#[derive(Default)]
pub struct State {
    pub mc_state: Mutex<McState>,
}
