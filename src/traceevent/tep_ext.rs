use crate::traceevent::tep::{TepEvent, TepHandle, TepRecord};
use crate::traceevent::trace_seq::TraceSeq;

pub trait TepHandleExt<C>
where
    C: Send + Sync + 'static,
{
    fn parse_i64(
        &self,
        seq: &mut TraceSeq,
        event: &TepEvent,
        record: &TepRecord,
        name: &str,
    ) -> Result<i64, String>;

    fn parse_raw(
        &self,
        seq: &mut TraceSeq,
        event: &TepEvent,
        record: &TepRecord,
        name: &str,
    ) -> Result<String, String>;
}

impl<C> TepHandleExt<C> for TepHandle<C>
where
    C: Send + Sync + 'static,
{
    fn parse_i64(
        &self,
        seq: &mut TraceSeq,
        event: &TepEvent,
        record: &TepRecord,
        name: &str,
    ) -> Result<i64, String> {
        self.get_field_val(seq, event, name, record)
            .ok_or_else(|| format!("can't parse field {name}"))
    }

    fn parse_raw(
        &self,
        seq: &mut TraceSeq,
        event: &TepEvent,
        record: &TepRecord,
        name: &str,
    ) -> Result<String, String> {
        let (raw, _len) = self
            .get_field_raw(seq, event, name, record)
            .ok_or_else(|| format!("can't parse field {name} raw"))?;

        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::traceevent::tep::{Field, TepEvent, TepHandle, TepRecord};
    use crate::traceevent::trace_seq::TraceSeq;

    struct Fixture {
        handle: TepHandle<()>,
        event: TepEvent,
        seq: TraceSeq,
        data: Vec<u8>,
    }

    impl Fixture {
        fn new() -> Self {
            Self::with_data_size(128)
        }

        fn with_data_size(size: usize) -> Self {
            Self {
                handle: TepHandle::new(),
                event: TepEvent {
                    id: 999,
                    name: "test_event".into(),
                    group: "test".into(),
                    fields: vec![
                        Field {
                            name: "u32_val".into(),
                            offset: 0,
                            size: 4,
                            is_signed: false,
                            is_string: false,
                        },
                        Field {
                            name: "i32_val".into(),
                            offset: 4,
                            size: 4,
                            is_signed: true,
                            is_string: false,
                        },
                        Field {
                            name: "msg".into(),
                            offset: 8,
                            size: 32,
                            is_string: true,
                            is_signed: false,
                        },
                        Field {
                            name: "label".into(),
                            offset: 40,
                            size: 32,
                            is_string: true,
                            is_signed: false,
                        },
                        Field {
                            name: "addr".into(),
                            offset: 72,
                            size: 8,
                            is_signed: false,
                            is_string: false,
                        },
                        Field {
                            name: "delta".into(),
                            offset: 80,
                            size: 8,
                            is_signed: true,
                            is_string: false,
                        },
                    ],
                    print_fmt: "".into(),
                },
                seq: TraceSeq::new(),
                data: vec![0u8; size],
            }
        }

        fn record(&self) -> TepRecord {
            TepRecord {
                size: self.data.len(),
                offset: 0,
                missed_events: 0,
                record_size: self.data.len(),
                data: self.data.clone(),
                ts: 0,
                common_type: 999,
                cpu: 0,
            }
        }

        fn write_i32(&mut self, offset: usize, v: i32) {
            self.data[offset..offset + 4].copy_from_slice(&v.to_le_bytes());
        }

        fn write_u64(&mut self, offset: usize, v: u64) {
            self.data[offset..offset + 8].copy_from_slice(&v.to_le_bytes());
        }

        fn write_str(&mut self, offset: usize, s: &str) {
            let bytes = s.as_bytes();
            let len = bytes.len().min(self.data.len() - offset);
            self.data[offset..offset + len].copy_from_slice(&bytes[..len]);
            if offset + len < self.data.len() {
                self.data[offset + len] = 0;
            }
        }
    }

    #[test]
    fn test_parse_i64() {
        let mut fx = Fixture::new();
        fx.write_i32(0, 1);
        fx.write_i32(4, 42);

        let rec = fx.record();

        assert_eq!(
            fx.handle
                .parse_i64(&mut fx.seq, &fx.event, &rec, "u32_val")
                .unwrap(),
            1
        );
        assert_eq!(
            fx.handle
                .parse_i64(&mut fx.seq, &fx.event, &rec, "i32_val")
                .unwrap(),
            42
        );
    }

    #[test]
    fn test_parse_i64_with_nonexistent_field() {
        let mut fx = Fixture::new();
        let rec = fx.record();

        let err = fx
            .handle
            .parse_i64(&mut fx.seq, &fx.event, &rec, "nonexistent")
            .unwrap_err();

        assert!(err.contains("can't parse field nonexistent"));
    }

    #[test]
    fn test_parse_i64_with_out_of_bounds() {
        let mut fx = Fixture::with_data_size(10); // addr at 72 won't fit
        let rec = fx.record();

        let err = fx
            .handle
            .parse_i64(&mut fx.seq, &fx.event, &rec, "addr")
            .unwrap_err();

        assert!(err.contains("addr"));
    }

    #[test]
    fn test_parse_i64_with_signed_negative() {
        let mut fx = Fixture::new();
        fx.write_u64(80, (-1i64) as u64);
        let rec = fx.record();

        assert_eq!(
            fx.handle
                .parse_i64(&mut fx.seq, &fx.event, &rec, "delta")
                .unwrap(),
            -1
        );
    }

    #[test]
    fn test_parse_raw() {
        let mut fx = Fixture::new();
        fx.write_str(8, "hello world");
        fx.write_str(40, "my-label");
        let rec = fx.record();

        assert_eq!(
            fx.handle
                .parse_raw(&mut fx.seq, &fx.event, &rec, "msg")
                .unwrap(),
            "hello world"
        );
        assert_eq!(
            fx.handle
                .parse_raw(&mut fx.seq, &fx.event, &rec, "label")
                .unwrap(),
            "my-label"
        );
    }

    #[test]
    fn test_parse_raw_with_empty_string() {
        let fx = Fixture::new();
        let rec = fx.record();

        assert_eq!(
            fx.handle
                .parse_raw(&mut fx.seq.clone(), &fx.event, &rec, "msg")
                .unwrap(),
            ""
        );
    }

    #[test]
    fn test_parse_raw_with_nonexistent_field() {
        let mut fx = Fixture::new();
        let rec = fx.record();

        let err = fx
            .handle
            .parse_raw(&mut fx.seq, &fx.event, &rec, "nonexistent")
            .unwrap_err();

        assert!(err.contains("can't parse field nonexistent raw"));
    }

    #[test]
    fn test_parse_raw_with_non_utf8_lossy() {
        let mut fx = Fixture::new();
        fx.data[8] = 0xFF;
        fx.data[9] = 0xFE;
        fx.data[10] = b'A';
        fx.data[11] = 0;
        let rec = fx.record();

        let s = fx
            .handle
            .parse_raw(&mut fx.seq, &fx.event, &rec, "msg")
            .unwrap();

        assert!(s.contains('A'));
        assert_eq!(s.chars().count(), 3);
    }
}
