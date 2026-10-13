use std::collections::HashMap;

use crate::traceevent::common::Endian;

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

pub struct TepHandle {
    pub endian: Endian,
    pub long_size: usize,
    pub events_by_id: HashMap<u32, TepEvent>,
    pub ids_by_name: HashMap<(String, String), u32>,
}

impl TepHandle {
    pub fn new() -> Self {
        Self {
            endian: Endian::default(),
            long_size: std::mem::size_of::<usize>(),
            events_by_id: HashMap::new(),
            ids_by_name: HashMap::new(),
        }
    }

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
            if let Some(f) = Self::parse_field_line(line) {
                if f.name == "commit" && f.size != 0 {
                    if long_size == 0 {
                        self.long_size = f.size;
                    }
                }
            }
        }
        let _ = size;
        Ok(())
    }

    // C: tep_parse_event(tep, buf, size, sys)
    pub fn parse_event(&mut self, buf: &str, size: usize, sys: &str) -> Result<u32, String> {
        let mut name = String::new();
        let mut id: u32 = 0;
        let mut fields = Vec::new();
        let mut print_fmt = String::new();
        let mut in_format = false;
        let mut in_print_fmt = false;
        let group = sys.to_string();

        for line in buf.lines() {
            let l = line.trim();
            if in_print_fmt {
                if !l.is_empty() {
                    if !print_fmt.is_empty() {
                        print_fmt.push(' ');
                    }
                    print_fmt.push_str(l);
                }
                continue;
            }
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
                in_print_fmt = true;
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

    // C: tep_get_field_val(s, event, name, record, val, err)
    pub fn get_field_val<'a>(
        &self,
        event: &TepEvent,
        name: &str,
        record: &TepRecord<'a>,
    ) -> Option<i64> {
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

    // C: tep_get_field_raw(s, event, name, record, len, err)
    pub fn get_field_raw<'a>(
        &self,
        event: &TepEvent,
        name: &str,
        record: &'a TepRecord,
    ) -> Option<(&'a [u8], usize)> {
        let f = event.fields.iter().find(|x| x.name == name)?;
        if f.offset + f.size > record.data.len() {
            return None;
        }
        let raw = &record.data[f.offset..f.offset + f.size];
        let trimmed = if f.is_string {
            let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
            &raw[..end]
        } else {
            raw
        };
        Some((trimmed, trimmed.len()))
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

impl Default for TepHandle {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn test_parse_header_page() {
        let mut h: TepHandle = TepHandle::new();
        h.long_size = 4;
        let buf = r#"
field: unsigned short common_type; offset:0; size:2; signed:0;
field: unsigned char common_flags; offset:2; size:1; signed:0;
field: unsigned long commit; offset:0; size:8; signed:0;
"#;
        h.parse_header_page(buf, buf.len(), 0).unwrap();
        assert_eq!(h.long_size, 8);
    }

    #[test]
    fn test_parse_header_page_with_long_size() {
        let mut h: TepHandle = TepHandle::new();
        let buf = "";
        h.parse_header_page(buf, buf.len(), 42).unwrap();
        assert_eq!(h.long_size, 42);
    }

    #[test]
    fn test_parse_header_page_with_commit_and_substring() {
        let mut h: TepHandle = TepHandle::new();
        h.long_size = 4;
        let buf = r#"
field: unsigned int committed; offset:0; size:4; signed:0;
field: unsigned int commit_id; offset:4; size:4; signed:0;
"#;
        h.parse_header_page(buf, buf.len(), 0).unwrap();
        assert_eq!(h.long_size, 4);

        let buf = r#"field: unsigned long commit; offset:0; size:8; signed:0;"#;
        h.parse_header_page(buf, buf.len(), 0).unwrap();
        assert_eq!(h.long_size, 8);
    }

    #[test]
    fn test_parse_event() {
        let mut h: TepHandle = TepHandle::new();
        let buf = sample_event_format();
        let id = h.parse_event(buf, buf.len(), "sched").unwrap();
        assert_eq!(id, 62);
    }

    #[test]
    fn test_parse_event_with_multiline_print_fmt() {
        let mut h: TepHandle = TepHandle::new();
        let buf = r#"
name: mc_event
ID: 100
format:
    field:unsigned short common_type; offset:0; size:2; signed:0;
    field:char msg[32]; offset:4; size:32; signed:0;
print fmt: "msg=%s count=%d", REC->msg,
    REC->count,
    REC->extra
"#;
        h.parse_event(buf, buf.len(), "test").unwrap();
        let ev = h.find_event(100).unwrap();
        assert!(ev.print_fmt.contains("REC->msg"));
        assert!(ev.print_fmt.contains("REC->count"));
        assert!(ev.print_fmt.contains("REC->extra"));
    }

    #[test]
    fn test_find_event() {
        let mut h: TepHandle = TepHandle::new();
        let buf = sample_event_format();
        h.parse_event(buf, buf.len(), "sched").unwrap();
        assert!(h.find_event(62).is_some());
        assert!(h.find_event_by_name("sched", "sched_switch").is_some());
    }

    #[test]
    fn test_get_field_val() {
        let mut h: TepHandle = TepHandle::new();
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
        assert_eq!(h.get_field_val(&ev, "prev_pid", &rec).unwrap(), 1234);
    }

    #[test]
    fn test_get_field_raw_with_nul_trim() {
        let mut h: TepHandle = TepHandle::new();
        let fmt = r#"
name: mc_event
ID: 20
format:
    field:char label[32]; offset:0; size:32; signed:0;
print fmt: "%s"
"#;
        h.parse_event(fmt, fmt.len(), "ras").unwrap();
        let ev = h.find_event(20).unwrap().clone();
        let mut data = vec![0u8; 32];
        data[0..5].copy_from_slice(b"dimm0");
        data[5] = 0;
        let rec = TepRecord {
            size: 32,
            offset: 0,
            missed_events: 0,
            record_size: 32,
            data: &data,
            ts: 0,
            common_type: 20,
            cpu: 0,
        };
        let (raw, len) = h.get_field_raw(&ev, "label", &rec).unwrap();
        // old returned 32 bytes with NULs, fixed returns 5
        assert_eq!(len, 5);
        assert_eq!(raw, b"dimm0");
        // emulate what caller would do in tep_ext.rs: String::from_utf8_lossy
        let s = String::from_utf8_lossy(raw).to_string();
        assert_eq!(s, "dimm0");
        assert!(!raw.contains(&0u8));
    }
}
