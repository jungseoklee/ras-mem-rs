use std::time::{Duration, SystemTime, UNIX_EPOCH};

use tracing::warn;

use crate::core::instance::Instance;
use crate::core::{context::Context, handler::EventHandler};
use crate::event::Event;
use crate::traceevent::tep::{TepEvent, TepHandle, TepRecord};
use crate::traceevent::tep_ext::TepHandleExt;

use super::McContext;
use super::event::{HwEventErrType, McEvent};

pub struct McEventHandler;

impl<C: Context> EventHandler<C> for McEventHandler {
    fn group(&self) -> &'static str {
        "ras"
    }

    fn name(&self) -> &'static str {
        "mc_event"
    }

    fn instance(&self) -> Instance {
        Instance::Dram
    }

    fn parse(
        &self,
        tep: &TepHandle,
        event: &TepEvent,
        record: &TepRecord,
        ctx: &C,
    ) -> Result<Event, String> {
        let raw_ty = tep.parse_i64(event, record, "error_type")?;
        let error_type = HwEventErrType::from(raw_ty);

        let now_raw = if ctx.config().use_uptime {
            let hz = ctx.config().user_hz;
            let secs = (record.ts as i64 / hz) + ctx.config().uptime_diff;
            UNIX_EPOCH + Duration::from_secs(secs.max(0) as u64)
        } else {
            SystemTime::now()
        };

        let mc_event = McEvent {
            timestamp: now_raw,
            error_count: tep.parse_i64(event, record, "error_count")?,
            error_type,
            msg: tep.parse_raw(event, record, "msg")?,
            label: tep.parse_raw(event, record, "label")?,
            mc_index: tep.parse_i64(event, record, "mc_index")?,
            top_layer: tep.parse_i64(event, record, "top_layer")? as i8,
            middle_layer: tep.parse_i64(event, record, "middle_layer")? as i8,
            lower_layer: tep.parse_i64(event, record, "lower_layer")? as i8,
            address: tep.parse_i64(event, record, "address")? as u64,
            grain: tep.parse_i64(event, record, "grain_bits")?,
            syndrome: tep.parse_i64(event, record, "syndrome")? as u64,
            driver_detail: tep.parse_raw(event, record, "driver_detail")?,
        };
        Ok(Event::Mc(mc_event))
    }

    fn process(&self, event: &Event, ctx: &C) {
        let Event::Mc(ev) = event else { panic!() };
        if ev.error_type != HwEventErrType::Corrected {
            return;
        }

        let Some(threshold) = ctx.mc_config().ce_threshold else {
            return;
        };
        if threshold == 0 {
            return;
        }

        let now_sec = ev
            .timestamp
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs() as i64)
            .unwrap_or(0);

        // Don't do I/O while holding mutex
        let exceeded = {
            let mut state = ctx.mc_state();
            if state.cur_sec == now_sec {
                state.per_sec += ev.error_count as u64;
            } else {
                state.cur_sec = now_sec;
                state.per_sec = ev.error_count as u64;
            }
            state.per_sec > threshold
        };

        if exceeded {
            warn!("ce threshold exceeded");
        }
    }
}

impl McEventHandler {
    pub fn format_event(ev: &McEvent, level: &str) -> String {
        use std::fmt::Write;
        let mut s = String::with_capacity(256);
        let _ = write!(
            s,
            "{} {} {} {} error{}",
            level,
            format_timestamp(&ev.timestamp),
            ev.error_count,
            ev.error_type.as_str(),
            if ev.error_count > 1 { "s:" } else { ":" }
        );
        if !ev.msg.is_empty() {
            let _ = write!(s, " {}", ev.msg);
        }
        if !ev.label.is_empty() {
            let _ = write!(s, " on {}", ev.label);
        }
        let _ = write!(s, " (mc: {}", ev.mc_index);

        if ev.top_layer >= 0 || ev.middle_layer >= 0 || ev.lower_layer >= 0 {
            if ev.lower_layer >= 0 {
                let _ = write!(
                    s,
                    " location: {}:{}:{}",
                    ev.top_layer, ev.middle_layer, ev.lower_layer
                );
            } else if ev.middle_layer >= 0 {
                let _ = write!(s, " location: {}:{}", ev.top_layer, ev.middle_layer);
            } else {
                let _ = write!(s, " location: {}", ev.top_layer);
            }
        }
        if ev.address != 0 {
            let _ = write!(s, " address: 0x{:08x}", ev.address);
        }
        let _ = write!(s, " grain: {}", ev.grain);
        if ev.syndrome != 0 {
            let _ = write!(s, " syndrome: 0x{:08x}", ev.syndrome);
        }
        if !ev.driver_detail.is_empty() {
            let _ = write!(s, " {}", ev.driver_detail);
        }
        s.push(')');
        s
    }
}

fn format_timestamp(now: &SystemTime) -> String {
    // std-only can't do local %z like C. For 1:1 use chrono.
    // Keep secs for now, testable.
    match now.duration_since(UNIX_EPOCH) {
        Ok(d) => d.as_secs().to_string(),
        Err(_) => "0".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::core::context::Context;
    use crate::event::mc::{McConfig, McState};
    use crate::state::State;
    use crate::traceevent::tep::{Field, TepEvent, TepHandle, TepRecord};
    use std::sync::Mutex;
    use std::time::Duration;

    struct TestContext {
        config: Config,
        state: State,
    }
    impl Context for TestContext {
        fn config(&self) -> &Config {
            &self.config
        }
        fn state(&self) -> &State {
            &self.state
        }
    }

    struct Fixture {
        handle: TepHandle,
        event: TepEvent,
        data: Vec<u8>,
        ctx: TestContext,
    }
    impl Fixture {
        fn new(threshold: Option<u64>) -> Self {
            let mut cfg = Config::default();
            cfg.mc_config = McConfig {
                ce_threshold: threshold,
            };
            cfg.use_uptime = false;
            cfg.user_hz = 100;
            cfg.uptime_diff = 0;
            let mut st = State::default();
            st.mc_state = Mutex::new(McState::default());

            let fields = vec![
                Field {
                    name: "error_type".into(),
                    offset: 0,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "error_count".into(),
                    offset: 8,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "msg".into(),
                    offset: 16,
                    size: 32,
                    is_string: true,
                    is_signed: false,
                },
                Field {
                    name: "label".into(),
                    offset: 48,
                    size: 32,
                    is_string: true,
                    is_signed: false,
                },
                Field {
                    name: "mc_index".into(),
                    offset: 80,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "top_layer".into(),
                    offset: 88,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "middle_layer".into(),
                    offset: 96,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "lower_layer".into(),
                    offset: 104,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "address".into(),
                    offset: 112,
                    size: 8,
                    is_signed: false,
                    is_string: false,
                },
                Field {
                    name: "grain_bits".into(),
                    offset: 120,
                    size: 8,
                    is_signed: true,
                    is_string: false,
                },
                Field {
                    name: "syndrome".into(),
                    offset: 128,
                    size: 8,
                    is_signed: false,
                    is_string: false,
                },
                Field {
                    name: "driver_detail".into(),
                    offset: 136,
                    size: 64,
                    is_string: true,
                    is_signed: false,
                },
            ];

            Self {
                handle: TepHandle::new(),
                event: TepEvent {
                    id: 1,
                    name: "mc_event".into(),
                    group: "ras".into(),
                    fields,
                    print_fmt: "".into(),
                },
                data: vec![0u8; 200],
                ctx: TestContext {
                    config: cfg,
                    state: st,
                },
            }
        }

        fn record_from_data(data: &[u8]) -> TepRecord<'_> {
            TepRecord {
                size: data.len(),
                offset: 0,
                missed_events: 0,
                record_size: data.len(),
                data,
                ts: 100000,
                common_type: 1,
                cpu: 0,
            }
        }

        fn write_i64(&mut self, off: usize, v: i64) {
            self.data[off..off + 8].copy_from_slice(&v.to_le_bytes());
        }

        fn write_u64(&mut self, off: usize, v: u64) {
            self.data[off..off + 8].copy_from_slice(&v.to_le_bytes());
        }

        fn write_str(&mut self, off: usize, s: &str) {
            let b = s.as_bytes();
            let len = b.len().min(self.data.len() - off - 1);
            self.data[off..off + len].copy_from_slice(&b[..len]);
            self.data[off + len] = 0;
        }
    }

    #[test]
    fn test_parse() {
        let mut fx = Fixture::new(Some(10));
        fx.write_i64(0, 0); // Corrected
        fx.write_i64(8, 2);
        fx.write_str(16, "ue");
        fx.write_str(48, "dimm0");
        fx.write_i64(80, 1);
        fx.write_i64(88, 0);
        fx.write_i64(96, 1);
        fx.write_i64(104, 2);
        fx.write_u64(112, 0xabcd);
        fx.write_i64(120, 8);
        fx.write_u64(128, 0x1234);
        fx.write_str(136, "detail");
        let h = McEventHandler;
        let rec = Fixture::record_from_data(&fx.data);

        let ev = h.parse(&fx.handle, &fx.event, &rec, &fx.ctx).unwrap();

        match ev {
            crate::event::Event::Mc(mc) => {
                assert_eq!(mc.error_type, HwEventErrType::Corrected);
                assert_eq!(mc.error_count, 2);
                assert_eq!(mc.label, "dimm0");
                assert_eq!(mc.address, 0xabcd);
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn test_update_state() {
        let fx = Fixture::new(Some(5));
        let h = McEventHandler;

        let ev = crate::event::Event::Mc(McEvent {
            timestamp: UNIX_EPOCH + Duration::from_secs(1000),
            error_count: 3,
            error_type: HwEventErrType::Corrected,
            msg: "".into(),
            label: "".into(),
            mc_index: 0,
            top_layer: -1,
            middle_layer: -1,
            lower_layer: -1,
            address: 0,
            grain: 0,
            syndrome: 0,
            driver_detail: "".into(),
        });

        h.process(&ev, &fx.ctx);
        assert_eq!(fx.ctx.state.mc_state.lock().unwrap().per_sec, 3);

        h.process(&ev, &fx.ctx);
        assert_eq!(fx.ctx.state.mc_state.lock().unwrap().per_sec, 6);

        let fatal = crate::event::Event::Mc(McEvent {
            timestamp: UNIX_EPOCH + Duration::from_secs(1000),
            error_count: 10,
            error_type: HwEventErrType::Fatal,
            msg: "".into(),
            label: "".into(),
            mc_index: 0,
            top_layer: -1,
            middle_layer: -1,
            lower_layer: -1,
            address: 0,
            grain: 0,
            syndrome: 0,
            driver_detail: "".into(),
        });

        h.process(&fatal, &fx.ctx);

        assert_eq!(fx.ctx.state.mc_state.lock().unwrap().per_sec, 6);
    }
}
