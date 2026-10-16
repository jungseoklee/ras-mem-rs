use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::warn;

use crate::core::{context::Context, handler::EventHandler, instance::Instance};
use crate::event::Event;
use crate::event::sched_switch::SchedSwitchContext;
use crate::event::sched_switch::event::SchedSwitchEvent;
use crate::traceevent::tep::{TepEvent, TepHandle, TepRecord};
use crate::traceevent::tep_ext::TepHandleExt;

pub struct SchedSwitchEventHandler;

impl<C: Context> EventHandler<C> for SchedSwitchEventHandler {
    fn group(&self) -> &'static str {
        "sched"
    }

    fn name(&self) -> &'static str {
        "sched_switch"
    }

    fn instance(&self) -> Instance {
        Instance::Test
    }

    fn parse(
        &self,
        tep: &TepHandle,
        event: &TepEvent,
        record: &TepRecord<'_>,
        ctx: &C,
    ) -> anyhow::Result<Event, String> {
        let now = if ctx.config().use_uptime {
            let hz = ctx.config().user_hz;
            let secs = (record.ts as i64 / hz) + ctx.config().uptime_diff;
            UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
        } else {
            SystemTime::now()
        };
        Ok(Event::SchedSwitch(SchedSwitchEvent {
            timestamp: now,
            prev_comm: tep.parse_raw(event, record, "prev_comm")?,
            prev_pid: tep.parse_i64(event, record, "prev_pid")? as i32,
            prev_prio: tep.parse_i64(event, record, "prev_prio")? as i32,
            prev_state: tep.parse_i64(event, record, "prev_state")?,
            next_comm: tep.parse_raw(event, record, "next_comm")?,
            next_pid: tep.parse_i64(event, record, "next_pid")? as i32,
            next_prio: tep.parse_i64(event, record, "next_prio")? as i32,
        }))
    }

    fn process(&self, event: &crate::event::Event, ctx: &C) {
        let Event::SchedSwitch(_) = event else {
            warn!("unexpected event type in sched_switch handler");
            return;
        };
        let mut state = ctx.sched_switch_state();
        state.switch_count += 1;
    }
}
