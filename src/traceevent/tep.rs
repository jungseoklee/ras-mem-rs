use std::collections::HashMap;
use std::sync::Arc;

use crate::traceevent::common::Endian;
use crate::traceevent::trace_seq::TraceSeq;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Field {
    pub name: String,
    pub offset: usize,
    pub size: usize,
    pub is_signed: bool,
    pub is_string: bool,
}

#[derive(Debug, Clone)]
pub struct TepEvent {
    pub id: u32,
    pub name: String,
    pub group: String,
    pub fields: Vec<Field>,
    pub print_fmt: String,
}

#[derive(Debug, Clone, Copy)]
pub struct TepRecord<'a> {
    pub size: usize,
    pub offset: usize,
    pub missed_events: i64,
    pub record_size: usize,
    pub data: &'a [u8],
    pub ts: u64,
    pub common_type: u16,
    pub cpu: u32,
}

pub type HandlerFn<C> =
    Box<dyn for<'a> Fn(&mut TraceSeq, &TepRecord<'a>, &TepEvent, &C) -> i32 + Send + Sync>;

struct HandlerEntry<C> {
    handler: HandlerFn<C>,
    context: Arc<C>,
}

pub struct TepHandle<C = ()> {
    pub endian: Endian,
    pub long_size: usize,
    pub events_by_id: HashMap<u32, TepEvent>,
    pub ids_by_name: HashMap<(String, String), u32>,
    handlers: HashMap<(String, String), HandlerEntry<C>>,
    pub page_size: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegisterStatus {
    Success = 0,
    SuccessOverwrite = 1,
}

impl<C> TepHandle<C>
where
    C: Send + Sync + 'static,
{
    pub fn new() -> Self {
        Self {
            endian: Endian::default(),
            long_size: std::mem::size_of::<usize>(),
            events_by_id: HashMap::new(),
            ids_by_name: HashMap::new(),
            handlers: HashMap::new(),
            page_size: 4096,
        }
    }

    // TODO Potential Bug #1 - parse_header_page is fragile on "commit" detection
    // Current: if line.contains("commit") { parse_field_line(line) }
    // Problem: Only works if the line literally contains "commit" string and
    // assumes long_size = field.size without checking field name.
    // On some kernels header_page format changes order.
    // Fix: Explicitly check f.name == "commit" after parse_field_line():
    // if let Some(f) = Self::parse_field_line(line) {
    // if f.name == "commit" { self.long_size = f.size; }
    // }
    // C: tep_parse_header_page(tep, buf, size, long_size)
    pub fn parse_header_page(
        &mut self,
        buf: &str,
        size: usize,
        long_size: usize,
    ) -> Result<(), String> {
        if long_size != 0 {
            self.long_size = long_size;
        }
        for line in buf.lines() {
            let line = line.trim();
            if line.contains("commit") {
                if let Some(f) = Self::parse_field_line(line) {
                    if long_size == 0 && f.size != 0 {
                        self.long_size = f.size;
                    }
                }
            }
        }
        let _ = size;
        Ok(())
    }

    // TODO Potential Bug #2 - parse_event truncates multi-line print fmt
    // Current: print_fmt = l["print fmt:".len()..].trim().to_string(); in_format = false;
    // Problem: Real tracefs format has print fmt spanning multiple lines, e.g.
    // print fmt: "%s %d", REC->msg, REC->error_count
    // Your code only captures first line and drops the rest.
    // Not used for mc_event now (you use custom format_event), but will
    // break generic trace-cmd report / tep_print_event compat.
    // Fix: Accumulate lines until EOF or next event section, not just first line.
    // C: tep_parse_event(tep, buf, size, sys)
    pub fn parse_event(&mut self, buf: &str, size: usize, sys: &str) -> Result<u32, String> {
        let mut name = String::new();
        let mut id: u32 = 0;
        let mut fields = Vec::new();
        let mut print_fmt = String::new();
        let mut in_format = false;
        let group = sys.to_string();

        for line in buf.lines() {
            let l = line.trim();
            if l.starts_with("name:") {
                name = l["name:".len()..].trim().to_string();
            } else if l.starts_with("ID:") {
                id = l["ID:".len()..].trim().parse().unwrap_or(0);
            } else if l.starts_with("format:") {
                in_format = true;
            } else if in_format && l.starts_with("field:") {
                if let Some(f) = Self::parse_field_line(l) {
                    fields.push(f);
                }
            } else if l.starts_with("print fmt:") {
                print_fmt = l["print fmt:".len()..].trim().to_string();
                in_format = false;
            }
        }

        if name.is_empty() {
            return Err("no name in format".into());
        }
        if group.is_empty() {
            return Err("group must not be empty (sys)".into());
        }

        let event = TepEvent {
            id,
            name: name.clone(),
            group: group.clone(),
            fields,
            print_fmt,
        };
        self.events_by_id.insert(id, event);
        self.ids_by_name.insert((group, name), id);
        let _ = size;
        Ok(id)
    }

    // C: tep_find_event(tep, id)
    pub fn find_event(&self, id: u32) -> Option<&TepEvent> {
        self.events_by_id.get(&id)
    }

    // C: tep_find_event_by_name(tep, sys, name)
    pub fn find_event_by_name(&self, sys: &str, name: &str) -> Option<&TepEvent> {
        let id = self.ids_by_name.get(&(sys.to_string(), name.to_string()))?;
        self.events_by_id.get(id)
    }

    // C: tep_find_event_by_record(tep, record)
    pub fn find_event_by_record<'a>(&self, record: &TepRecord<'a>) -> Option<&TepEvent> {
        self.events_by_id.get(&(record.common_type as u32))
    }

    // C: tep_register_event_handler(tep, id, sys_name, event_name, func, context)
    pub fn register_event_handler<F>(
        &mut self,
        id: i32,
        sys_name: &str,
        event_name: &str,
        func: F,
        context: Arc<C>,
    ) -> RegisterStatus
    where
        F: for<'a> Fn(&mut TraceSeq, &TepRecord<'a>, &TepEvent, &C) -> i32 + Send + Sync + 'static,
    {
        let _ = id; // kept for API compatibility, libtraceevent allows -1 for wildcard
        let key = (sys_name.to_string(), event_name.to_string());
        let existed = self.handlers.contains_key(&key);
        self.handlers.insert(
            key,
            HandlerEntry {
                handler: Box::new(func),
                context,
            },
        );
        if existed {
            RegisterStatus::SuccessOverwrite
        } else {
            RegisterStatus::Success
        }
    }

    // C: tep_unregister_event_handler(tep, id, sys_name, event_name, func, context)
    pub fn unregister_event_handler(&mut self, id: i32, sys_name: &str, event_name: &str) {
        let _ = id;
        self.handlers
            .remove(&(sys_name.to_string(), event_name.to_string()));
    }

    // C: tep_print_event(tep, s, record, fmt, ...)
    pub fn print_event<'a>(&self, s: &mut TraceSeq, record: &TepRecord<'a>) -> i32 {
        let event = match self.events_by_id.get(&(record.common_type as u32)) {
            Some(e) => e,
            None => return -1,
        };
        let key = (event.group.clone(), event.name.clone());
        if let Some(entry) = self.handlers.get(&key) {
            (entry.handler)(s, record, event, &entry.context)
        } else {
            s.push_str(&format!(
                "{}:{} raw={:?}",
                event.group, event.name, record.data
            ));
            0
        }
    }

    // C: tep_get_field_val(s, event, name, record, val, err)
    pub fn get_field_val<'a>(
        &self,
        s: &mut TraceSeq,
        event: &TepEvent,
        name: &str,
        record: &TepRecord<'a>,
    ) -> Option<i64> {
        let _ = s;
        let f = event.fields.iter().find(|x| x.name == name)?;
        if f.offset + f.size > record.data.len() {
            return None;
        }
        Some(Self::read_int(
            &record.data[f.offset..f.offset + f.size],
            f.is_signed,
            self.endian,
        ))
    }

    // TODO Potential Bug #3 - get_field_raw returns NUL-padded bytes, not trimmed string
    // Current: Some((raw, raw.len())) where raw = &record.data[offset..offset+size]
    // Problem: char[32] fields like msg, label, driver_detail are NUL-terminated
    // C strings. If tep_ext::parse_raw does String::from_utf8_lossy(raw)
    // without trimming \0, you get "DIMM0\0\0\0..." leaking into Event.
    // Your handler.rs tests write_str adds explicit \0, but real kernel
    // data will have padding.
    // Fix: In tep_ext.rs parse_raw(), trim at first NUL:
    // let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
    // String::from_utf8_lossy(&raw[..end]).to_string()
    // Same for driver_detail.
    // C: tep_get_field_raw(s, event, name, record, len, err)
    pub fn get_field_raw<'a>(
        &self,
        s: &mut TraceSeq,
        event: &TepEvent,
        name: &str,
        record: &'a TepRecord,
    ) -> Option<(&'a [u8], usize)> {
        let _ = s;
        let f = event.fields.iter().find(|x| x.name == name)?;
        if f.offset + f.size > record.data.len() {
            return None;
        }
        let raw = &record.data[f.offset..f.offset + f.size];
        Some((raw, raw.len()))
    }

    fn parse_field_line(line: &str) -> Option<Field> {
        let parts: Vec<&str> = line.split(';').collect();
        let first = parts.first()?.trim();
        let name_part = first.split_whitespace().last()?;
        let name = name_part.split('[').next().unwrap_or(name_part).to_string();
        let mut offset = 0usize;
        let mut size = 0usize;
        let mut is_signed = false;
        let mut found_offset = false;
        let mut found_size = false;
        for p in &parts[1..] {
            let p = p.trim();
            if p.starts_with("offset:") {
                if let Ok(v) = p["offset:".len()..].trim().parse() {
                    offset = v;
                    found_offset = true;
                }
            } else if p.starts_with("size:") {
                if let Ok(v) = p["size:".len()..].trim().parse() {
                    size = v;
                    found_size = true;
                }
            } else if p.starts_with("signed:") {
                is_signed = p["signed:".len()..].trim() == "1";
            }
        }
        if !found_offset || !found_size {
            return None;
        }
        Some(Field {
            name,
            offset,
            size,
            is_signed,
            is_string: name_part.contains('['),
        })
    }

    fn read_int(slice: &[u8], signed: bool, endian: Endian) -> i64 {
        let len = slice.len().min(8);
        if len == 0 {
            return 0;
        }
        let mut buf = [0u8; 8];
        let is_big = endian.is_big();
        if is_big {
            buf[8 - len..].copy_from_slice(&slice[..len]);
        } else {
            buf[..len].copy_from_slice(&slice[..len]);
        }
        let unsigned = if is_big {
            u64::from_be_bytes(buf)
        } else {
            u64::from_le_bytes(buf)
        };
        if !signed {
            return unsigned as i64;
        }
        let shift = 64 - len * 8;
        ((unsigned as i64) << shift) >> shift
    }
}

impl<C> Default for TepHandle<C>
where
    C: Send + Sync + 'static,
{
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    fn sample_event_format() -> &'static str {
        r#"
name: sched_switch
ID: 62
format:
    field:unsigned short common_type; offset:0; size:2; signed:0;
    field:unsigned char common_flags; offset:2; size:1; signed:0;
    field:unsigned char common_preempt_count; offset:3; size:1; signed:0;
    field:int common_pid; offset:4; size:4; signed:1;
    field:char prev_comm[16]; offset:8; size:16; signed:0;
    field:pid_t prev_pid; offset:24; size:4; signed:1;
    field:int prev_prio; offset:28; size:4; signed:1;
    field:char next_comm[16]; offset:32; size:16; signed:0;
    field:pid_t next_pid; offset:48; size:4; signed:1;
print fmt: "prev_comm=%s prev_pid=%d"
"#
    }

    fn header_page() -> &'static str {
        r#"
    field: unsigned short common_type; offset:0; size:2; signed:0;
    field: unsigned char common_flags; offset:2; size:1; signed:0;
    field: unsigned long commit; offset:0; size:8; signed:0;
        "#
    }

    #[test]
    fn test_parse_header_page_order() {
        let mut h: TepHandle<()> = TepHandle::new();
        h.long_size = 4;
        let buf = header_page();
        h.parse_header_page(buf, buf.len(), 0).unwrap();
        assert_eq!(h.long_size, 8);
    }

    #[test]
    fn test_parse_event_order() {
        let mut h: TepHandle<()> = TepHandle::new();
        let buf = sample_event_format();
        let id = h.parse_event(buf, buf.len(), "sched").unwrap();
        assert_eq!(id, 62);
    }

    #[test]
    fn test_find_event_orders() {
        let mut h: TepHandle<()> = TepHandle::new();
        let buf = sample_event_format();
        h.parse_event(buf, buf.len(), "sched").unwrap();
        assert!(h.find_event(62).is_some());
        assert!(h.find_event_by_name("sched", "sched_switch").is_some());
    }

    #[test]
    fn test_register_event_handler_order() {
        let mut h: TepHandle<String> = TepHandle::new();
        let buf = sample_event_format();
        h.parse_event(buf, buf.len(), "sched").unwrap();
        let ctx = Arc::new("ctx".to_string());
        let status = h.register_event_handler(
            -1,
            "sched",
            "sched_switch",
            |_seq, _rec, _ev, _ctx| 0,
            ctx.clone(),
        );
        assert_eq!(status, RegisterStatus::Success);
        let status2 =
            h.register_event_handler(62, "sched", "sched_switch", |_seq, _rec, _ev, _ctx| 1, ctx);
        assert_eq!(status2, RegisterStatus::SuccessOverwrite);
    }

    #[test]
    fn test_print_event_order() {
        let mut h: TepHandle<String> = TepHandle::new();
        let buf = sample_event_format();
        h.parse_event(buf, buf.len(), "sched").unwrap();
        h.register_event_handler(
            -1,
            "sched",
            "sched_switch",
            |seq, _rec, ev, ctx| {
                seq.push_str(&format!("handled {} ctx={}", ev.name, ctx));
                42
            },
            Arc::new("myctx".to_string()),
        );
        let mut data = vec![0u8; 64];
        data[0] = 62;
        let rec = TepRecord {
            size: 64,
            offset: 0,
            missed_events: 0,
            record_size: 64,
            data: &data,
            ts: 0,
            common_type: 62,
            cpu: 0,
        };
        let mut seq = TraceSeq::new();
        let ret = h.print_event(&mut seq, &rec);
        assert_eq!(ret, 42);
    }

    #[test]
    fn test_get_field_val_order() {
        let mut h: TepHandle<()> = TepHandle::new();
        let buf = sample_event_format();
        h.parse_event(buf, buf.len(), "sched").unwrap();
        let mut data = vec![0u8; 64];
        data[0] = 62;
        data[1] = 0;
        data[24..28].copy_from_slice(&1234i32.to_le_bytes());
        let rec = TepRecord {
            size: 64,
            offset: 0,
            missed_events: 0,
            record_size: 64,
            data: &data,
            ts: 0,
            common_type: 62,
            cpu: 0,
        };
        let ev = h
            .find_event_by_name("sched", "sched_switch")
            .unwrap()
            .clone();
        let mut seq = TraceSeq::new();
        assert_eq!(
            h.get_field_val(&mut seq, &ev, "prev_pid", &rec).unwrap(),
            1234
        );
    }
}
