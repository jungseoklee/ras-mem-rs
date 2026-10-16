use std::time::SystemTime;

#[derive(Debug, Clone)]
pub struct SchedSwitchEvent {
    pub timestamp: SystemTime,
    pub prev_comm: String,
    pub prev_pid: i32,
    pub prev_prio: i32,
    pub prev_state: i64,
    pub next_comm: String,
    pub next_pid: i32,
    pub next_prio: i32,
}
