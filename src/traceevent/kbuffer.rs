use crate::traceevent::common::{Endian, LongSize};

const MISSING_EVENTS: u64 = 1 << 31;
const MISSING_STORED: u64 = 1 << 30;
const COMMIT_MASK: u64 = (1 << 27) - 1;
const TS_MSB: u64 = 0xf8u64 << 56;
const TS_SHIFT: u64 = 27;

pub const TYPE_PADDING: u32 = 29;
pub const TYPE_TIME_EXTEND: u32 = 30;
pub const TYPE_TIME_STAMP: u32 = 31;

#[derive(Clone, Debug)]
pub struct KBuffer {
    long_8: bool,
    big_endian: bool,
    needs_swap: bool,
    subbuffer: Vec<u8>,
    data_start: usize,
    size: usize,
    curr: usize,
    next: usize,
    index: usize,
    timestamp: u64,
    lost_events: i64,
    first: usize,
}

impl KBuffer {
    pub fn new(long: LongSize, endian: Endian) -> Self {
        let host_big = cfg!(target_endian = "big");
        let long_8 = match long {
            LongSize::L8 => true,
            LongSize::L4 => false,
            LongSize::SameAsHost => std::mem::size_of::<usize>() == 8,
        };
        let big_endian = match endian {
            Endian::Big => true,
            Endian::Little => false,
            Endian::SameAsHost => host_big,
        };
        Self {
            long_8,
            big_endian,
            needs_swap: host_big ^ big_endian,
            subbuffer: Vec::new(),
            data_start: if long_8 { 16 } else { 12 },
            size: 0,
            curr: 0,
            next: 0,
            index: 0,
            timestamp: 0,
            lost_events: 0,
            first: 0,
        }
    }

    #[inline]
    fn read_u32_at(&self, off: usize) -> Option<u32> {
        let b = self.subbuffer.get(off..off + 4)?;
        let v = u32::from_ne_bytes([b[0], b[1], b[2], b[3]]);
        Some(if self.needs_swap { v.swap_bytes() } else { v })
    }

    #[inline]
    fn read_u64_at(&self, off: usize) -> Option<u64> {
        let b = self.subbuffer.get(off..off + 8)?;
        let v = u64::from_ne_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]);
        Some(if self.needs_swap { v.swap_bytes() } else { v })
    }

    #[inline]
    fn read_long_at(&self, off: usize) -> Option<u64> {
        if self.long_8 {
            self.read_u64_at(off)
        } else {
            self.read_u32_at(off).map(|v| v as u64)
        }
    }

    #[inline]
    fn type_len(v: u32, big: bool) -> u32 {
        if big { (v >> 27) & 0x1f } else { v & 0x1f }
    }

    #[inline]
    fn ts(v: u32, big: bool) -> u64 {
        if big {
            (v & ((1 << 27) - 1)) as u64
        } else {
            (v >> 5) as u64
        }
    }

    fn update_pointers(&mut self) -> Option<u32> {
        let hdr_off = self.data_start + self.curr;
        let type_len_ts = self.read_u32_at(hdr_off)?;
        let mut ptr = hdr_off + 4;
        let delta = Self::ts(type_len_ts, self.big_endian);
        let type_len = Self::type_len(type_len_ts, self.big_endian);

        let (index, next, new_ts) = match type_len {
            TYPE_PADDING => (self.size, self.size, self.timestamp.wrapping_add(delta)),
            TYPE_TIME_EXTEND | TYPE_TIME_STAMP => {
                let extend = self.read_u32_at(ptr)? as u64;
                ptr += 4;
                let full = (extend << TS_SHIFT) + delta;
                if type_len == TYPE_TIME_STAMP {
                    let abs = full | (self.timestamp & TS_MSB);
                    (ptr - self.data_start, ptr - self.data_start, abs)
                } else {
                    (
                        ptr - self.data_start,
                        ptr - self.data_start,
                        self.timestamp.wrapping_add(full),
                    )
                }
            }
            0 => {
                let len_field = self.read_u32_at(ptr)? as usize;
                if len_field < 4 {
                    return None;
                }
                ptr += 4;
                let payload = len_field - 4;
                let aligned = (payload + 3) & !3;
                let idx = ptr - self.data_start;
                (idx, idx + aligned, self.timestamp.wrapping_add(delta))
            }
            n => {
                let len = (n as usize) * 4;
                let idx = ptr - self.data_start;
                (idx, idx + len, self.timestamp.wrapping_add(delta))
            }
        };

        if type_len == TYPE_TIME_STAMP {
            self.timestamp = new_ts;
        } else {
            // padding also adds delta in C before jumping to end
            self.timestamp = new_ts;
        }
        self.index = index;
        self.next = next.min(self.size);
        Some(type_len)
    }

    pub fn load_subbuffer(&mut self, page: &[u8]) -> Result<(), &'static str> {
        let min = if self.long_8 { 16 } else { 12 };
        if page.len() < min {
            return Err("page too small");
        }
        self.subbuffer = page.to_vec(); // TODO: can we achieve zero-copy here?
        self.timestamp = self.read_u64_at(0).ok_or("page too small")?;
        let flags = self.read_long_at(8).ok_or("page too small")?;
        self.size = (flags & COMMIT_MASK) as usize;
        if self.size > self.subbuffer.len().saturating_sub(self.data_start) {
            self.size = self.subbuffer.len().saturating_sub(self.data_start);
        }
        self.lost_events = if flags & MISSING_EVENTS != 0 {
            if flags & MISSING_STORED != 0 {
                self.read_long_at(self.data_start + self.size)
                    .map(|v| v as i64)
                    .unwrap_or(-1)
            } else {
                -1
            }
        } else {
            0
        };
        self.curr = 0;
        self.next = 0;
        self.index = 0;
        if self.advance().is_none() && self.size != 0 {
            self.curr = self.size;
            self.next = self.size;
            self.index = self.size;
        }
        self.first = self.curr;
        Ok(())
    }

    fn advance(&mut self) -> Option<u32> {
        loop {
            if self.next >= self.size {
                self.curr = self.size;
                self.next = self.size;
                return None;
            }
            self.curr = self.next;
            let t = self.update_pointers()?;
            match t {
                TYPE_TIME_EXTEND | TYPE_TIME_STAMP | TYPE_PADDING => continue,
                _ => return Some(t),
            }
        }
    }

    pub fn read_event(&self) -> Option<(&[u8], u64)> {
        if self.curr >= self.size || self.index >= self.size {
            return None;
        }
        let s = self.data_start + self.index;
        let e = self.data_start + self.next;
        if e > self.subbuffer.len() || s > e {
            return None;
        }
        Some((self.subbuffer.get(s..e)?, self.timestamp))
    }

    pub fn next_event(&mut self) -> Option<(&[u8], u64)> {
        self.advance()?;
        self.read_event()
    }

    pub fn event_size(&self) -> usize {
        self.next.saturating_sub(self.index)
    }

    pub fn curr_size(&self) -> i32 {
        if self.curr >= self.size {
            -1
        } else {
            (self.next - self.curr) as i32
        }
    }

    pub fn curr_offset(&self) -> usize {
        self.curr + self.data_start
    }

    pub fn curr_index(&self) -> usize {
        self.curr
    }

    pub fn missed_events(&self) -> i64 {
        if self.curr != self.first {
            0
        } else {
            self.lost_events
        }
    }

    pub fn timestamp(&self) -> u64 {
        self.timestamp
    }

    pub fn subbuffer_size(&self) -> usize {
        self.size
    }

    pub fn start_of_data(&self) -> usize {
        self.first + self.data_start
    }

    pub fn data_start_offset(&self) -> usize {
        self.data_start
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w32le(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    fn w32be(b: &mut [u8], off: usize, v: u32) {
        b[off..off + 4].copy_from_slice(&v.to_be_bytes());
    }

    fn enc(delta: u32, tl: u32, big: bool) -> u32 {
        if big {
            (tl << 27) | (delta & ((1 << 27) - 1))
        } else {
            (delta << 5) | (tl & 0x1f)
        }
    }

    fn make_page(long_8: bool, endian: Endian, ts: u64, commit: usize, extra: usize) -> Vec<u8> {
        let ds = if long_8 { 16 } else { 12 };
        let mut page = vec![0u8; ds + commit + extra];
        match endian {
            Endian::Little | Endian::SameAsHost => {
                page[0..8].copy_from_slice(&ts.to_le_bytes());
                if long_8 {
                    page[8..16].copy_from_slice(&(commit as u64).to_le_bytes());
                } else {
                    page[8..12].copy_from_slice(&(commit as u32).to_le_bytes());
                }
            }
            Endian::Big => {
                page[0..8].copy_from_slice(&ts.to_be_bytes());
                if long_8 {
                    page[8..16].copy_from_slice(&(commit as u64).to_be_bytes());
                } else {
                    page[8..12].copy_from_slice(&(commit as u32).to_be_bytes());
                }
            }
        }
        page
    }

    fn add_fixed(
        page: &mut [u8],
        ds: usize,
        curr: usize,
        tl: u32,
        delta: u32,
        payload: &[u8],
        big: bool,
    ) -> usize {
        let w = if big {
            w32be as fn(&mut [u8], usize, u32)
        } else {
            w32le
        };
        w(page, ds + curr, enc(delta, tl, big));
        let ps = ds + curr + 4;
        let len = (tl as usize) * 4;
        page[ps..ps + payload.len().min(len)].copy_from_slice(&payload[..payload.len().min(len)]);
        curr + 4 + len
    }

    #[test]
    fn test_l4_little() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 1000, 8, 0);
        w32le(&mut page, ds, enc(0, 1, false));
        w32le(&mut page, ds + 4, 0xdeadbeef);

        k.load_subbuffer(&page).unwrap();

        assert_eq!(k.subbuffer_size(), 8);

        let (ev, ts) = k.read_event().unwrap();

        assert_eq!(ts, 1000);
        assert_eq!(ev.len(), 4);
        assert_eq!(k.event_size(), 4);
        assert_eq!(k.curr_size(), 8);
        assert_eq!(k.curr_offset(), ds);
        assert_eq!(k.curr_index(), 0);
        assert!(k.next_event().is_none());
    }

    #[test]
    fn test_l8_little() {
        let mut k = KBuffer::new(LongSize::L8, Endian::Little);
        let ds = 16;
        let mut page = make_page(true, Endian::Little, 5555, 8, 0);
        w32le(&mut page, ds, enc(10, 1, false));
        w32le(&mut page, ds + 4, 0x11223344);

        k.load_subbuffer(&page).unwrap();
        let (ev, ts) = k.read_event().unwrap();

        assert_eq!(ts, 5555 + 10);
        assert_eq!(ev.len(), 4);
    }

    #[test]
    fn test_l4_big() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Big);
        let ds = 12;
        let mut page = make_page(false, Endian::Big, 2000, 8, 0);
        w32be(&mut page, ds, enc(5, 1, true));
        w32be(&mut page, ds + 4, 0xaabbccdd);

        k.load_subbuffer(&page).unwrap();
        let (_, ts) = k.read_event().unwrap();

        assert_eq!(ts, 2005);
    }

    #[test]
    fn test_large_type() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let commit = 20;
        let mut page = make_page(false, Endian::Little, 0, commit, 0);
        w32le(&mut page, ds, enc(0, 0, false));
        w32le(&mut page, ds + 4, 14);
        page[ds + 8..ds + 18].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);

        k.load_subbuffer(&page).unwrap();
        let (ev, _) = k.read_event().unwrap();

        assert_eq!(ev.len(), 12);
        assert_eq!(&ev[..10], &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10]);
        assert_eq!(k.curr_size(), 20);
    }

    #[test]
    fn test_time_extend() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 1000, 16, 0);
        w32le(&mut page, ds, enc(2, TYPE_TIME_EXTEND, false));
        w32le(&mut page, ds + 4, 1);
        w32le(&mut page, ds + 8, enc(3, 1, false));
        w32le(&mut page, ds + 12, 0x12345678);
        k.load_subbuffer(&page).unwrap();
        let (_, ts) = k.read_event().unwrap();
        assert_eq!(ts, 1000 + (1u64 << 27) + 2 + 3);
    }

    #[test]
    fn test_time_stamp() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let ts_with_msb: u64 = 0xf8u64 << 56 | 0x11223344;
        let mut page = make_page(false, Endian::Little, ts_with_msb, 16, 0);
        w32le(&mut page, ds, enc(5, TYPE_TIME_STAMP, false));
        w32le(&mut page, ds + 4, 0x10);
        w32le(&mut page, ds + 8, enc(0, 1, false));
        w32le(&mut page, ds + 12, 0xdead);

        k.load_subbuffer(&page).unwrap();
        let (_, ts) = k.read_event().unwrap();

        let msb = ts_with_msb & TS_MSB;
        let expected_abs = ((0x10u64) << 27) + 5 | msb;
        assert_eq!(ts, expected_abs); // delta 0 for second event

        let mut page2 = make_page(false, Endian::Little, ts_with_msb, 16, 0);
        w32le(&mut page2, ds, enc(5, TYPE_TIME_STAMP, false));
        w32le(&mut page2, ds + 4, 0x10);
        w32le(&mut page2, ds + 8, enc(1, 1, false));
        w32le(&mut page2, ds + 12, 0xdead);

        k.load_subbuffer(&page2).unwrap();
        let (_, ts2) = k.read_event().unwrap();

        assert_eq!(ts2, expected_abs + 1);
    }

    #[test]
    fn test_padding() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 0, 16, 0);
        w32le(&mut page, ds, enc(0, 1, false));
        w32le(&mut page, ds + 4, 0xaaaa);
        w32le(&mut page, ds + 8, enc(0, TYPE_PADDING, false));
        w32le(&mut page, ds + 12, 0);

        k.load_subbuffer(&page).unwrap();

        assert!(k.read_event().is_some());
        assert!(k.next_event().is_none());
        assert_eq!(k.curr_size(), -1);
    }

    #[test]
    fn test_needs_swap() {
        let k = KBuffer::new(LongSize::L4, Endian::Big);
        assert_eq!(k.needs_swap, !cfg!(target_endian = "big"));
        let k2 = KBuffer::new(LongSize::L4, Endian::Little);
        assert_eq!(k2.needs_swap, cfg!(target_endian = "big"));
    }

    #[test]
    fn test_multiple_events() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 0, 24, 0);
        let mut off = 0;
        off = add_fixed(&mut page, ds, off, 1, 1, &[0x11; 4], false);
        off = add_fixed(&mut page, ds, off, 1, 2, &[0x22; 4], false);
        add_fixed(&mut page, ds, off, 1, 3, &[0x33; 4], false);

        k.load_subbuffer(&page).unwrap();

        let mut cnt = 0;
        let mut last = 0;
        loop {
            let (ev, ts) = k.read_event().unwrap();
            cnt += 1;
            assert_eq!(ev.len(), 4);
            assert!(ts >= last);
            last = ts;
            if k.next_event().is_none() {
                break;
            }
        }
        assert_eq!(cnt, 3);
    }

    #[test]
    fn test_missed_events() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let flags: u32 = MISSING_EVENTS as u32 | MISSING_STORED as u32 | 8;
        let mut page = make_page(false, Endian::Little, 0, 8, 8);
        w32le(&mut page, 8, flags);
        w32le(&mut page, ds, enc(0, 1, false));
        w32le(&mut page, ds + 4, 0);
        w32le(&mut page, ds + 8, 42);

        k.load_subbuffer(&page).unwrap();

        assert_eq!(k.missed_events(), 42);

        k.next_event();

        assert_eq!(k.missed_events(), 0);
    }

    #[test]
    fn test_missed_events_with_unknown() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let flags: u32 = MISSING_EVENTS as u32 | 8;
        let mut page = make_page(false, Endian::Little, 0, 8, 0);
        w32le(&mut page, 8, flags);
        w32le(&mut page, ds, enc(0, 1, false));

        k.load_subbuffer(&page).unwrap();

        assert_eq!(k.missed_events(), -1);
    }

    #[test]
    fn test_missed_events_with_initial_timestamp() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let flags: u32 = MISSING_EVENTS as u32 | MISSING_STORED as u32 | 16;
        let mut page = make_page(false, Endian::Little, 0, 16, 8);
        w32le(&mut page, 8, flags);
        w32le(&mut page, ds, enc(0, TYPE_TIME_EXTEND, false));
        w32le(&mut page, ds + 4, 0);
        w32le(&mut page, ds + 8, enc(0, 1, false));
        w32le(&mut page, ds + 12, 0x1234);
        w32le(&mut page, ds + 16, 7);

        k.load_subbuffer(&page).unwrap();

        assert_eq!(k.curr_index(), 8);
        assert_eq!(k.missed_events(), 7);
    }

    #[test]
    fn test_commit() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let mut page = make_page(false, Endian::Little, 0, 20, 0);
        w32le(&mut page, 8, 1000u32);
        w32le(&mut page, 12, enc(0, 1, false));

        k.load_subbuffer(&page).unwrap();

        assert!(k.subbuffer_size() <= 20);
        assert!(k.subbuffer_size() > 0);
    }

    #[test]
    fn test_start_of_data() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 0, 16, 0);
        w32le(&mut page, ds, enc(0, TYPE_TIME_EXTEND, false));
        w32le(&mut page, ds + 4, 0);
        w32le(&mut page, ds + 8, enc(0, 1, false));

        k.load_subbuffer(&page).unwrap();

        assert_eq!(k.start_of_data(), 20);
        assert_eq!(k.data_start_offset(), 12);
    }

    #[test]
    fn test_invalid_page() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);

        assert!(k.load_subbuffer(&[]).is_err());
        assert!(k.load_subbuffer(&[0u8; 5]).is_err());

        let page = make_page(false, Endian::Little, 0, 0, 0);

        k.load_subbuffer(&page).unwrap();

        assert!(k.read_event().is_none());
    }

    #[test]
    fn test_fixed_type_len() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 0, 64, 0);
        add_fixed(&mut page, ds, 0, 7, 1, &[0xAA; 28], false);

        k.load_subbuffer(&page).unwrap();
        let (ev, _) = k.read_event().unwrap();

        assert_eq!(ev.len(), 28);
        assert_eq!(k.curr_size(), 32); // 4 header + 28
    }

    #[test]
    fn test_next_event_with_next_payload() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 0, 24, 0);
        let mut off = 0;
        off = add_fixed(&mut page, ds, off, 1, 10, &[0xAA; 4], false); // delta 10
        off = add_fixed(&mut page, ds, off, 1, 20, &[0xBB; 4], false); // delta 20
        add_fixed(&mut page, ds, off, 1, 30, &[0xCC; 4], false); // delta 30

        k.load_subbuffer(&page).unwrap();
        let (ev0, ts0) = k.read_event().unwrap();

        assert_eq!(ev0, &[0xAA; 4]);
        assert_eq!(ts0, 10);
        assert_eq!(k.curr_index(), 0);
        assert_eq!(k.curr_offset(), 12);

        let (ev1, ts1) = k.next_event().unwrap();

        assert_eq!(ev1, &[0xBB; 4], "next_event should return second payload");
        assert_eq!(ts1, 10 + 20);
        assert_eq!(k.curr_index(), 8);

        let (ev2, ts2) = k.next_event().unwrap();

        assert_eq!(ev2, &[0xCC; 4]);
        assert_eq!(ts2, 10 + 20 + 30);
        assert_eq!(k.curr_index(), 16);

        assert!(k.next_event().is_none());
        assert_eq!(k.curr_size(), -1);
    }

    #[test]
    fn test_next_event_with_time_extend_and_stamp() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let ts0: u64 = 0xf0u64 << 56 | 100;
        let mut page = make_page(false, Endian::Little, ts0, 32, 0);
        let mut off = 0;
        w32le(&mut page, ds + off, enc(2, TYPE_TIME_EXTEND, false));
        w32le(&mut page, ds + off + 4, 1);
        off += 8;
        w32le(&mut page, ds + off, enc(5, 1, false));
        page[ds + off + 4..ds + off + 8].copy_from_slice(&[0x11; 4]);
        off += 8;
        w32le(&mut page, ds + off, enc(7, TYPE_TIME_STAMP, false));
        w32le(&mut page, ds + off + 4, 0x20);
        off += 8;
        w32le(&mut page, ds + off, enc(3, 1, false));
        page[ds + off + 4..ds + off + 8].copy_from_slice(&[0x22; 4]);

        k.load_subbuffer(&page).unwrap();

        let (ev_a, ts_a) = k.read_event().unwrap();
        assert_eq!(ev_a, &[0x11; 4]);
        assert_eq!(ts_a, ts0 + (1u64 << 27) + 2 + 5);

        let (ev_b, ts_b) = k.next_event().unwrap();
        assert_eq!(ev_b, &[0x22; 4]);
        let expected_abs = ((0x20u64) << 27) + 7 | (ts0 & TS_MSB);
        assert_eq!(ts_b, expected_abs + 3);

        assert!(k.next_event().is_none());
    }

    #[test]
    fn test_next_event_with_type_zero_and_padding() {
        let mut k = KBuffer::new(LongSize::L4, Endian::Little);
        let ds = 12;
        let mut page = make_page(false, Endian::Little, 0, 48, 0);
        w32le(&mut page, ds, enc(1, 0, false));
        w32le(&mut page, ds + 4, 8);
        page[ds + 8..ds + 12].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
        w32le(&mut page, ds + 12, enc(2, 2, false));
        page[ds + 16..ds + 24].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        w32le(&mut page, ds + 24, enc(0, TYPE_PADDING, false));

        k.load_subbuffer(&page).unwrap();

        let (ev0, ts0) = k.read_event().unwrap();
        assert_eq!(ev0.len(), 4);
        assert_eq!(ev0, &[0xDE, 0xAD, 0xBE, 0xEF]);
        assert_eq!(ts0, 1);

        let (ev1, ts1) = k.next_event().unwrap();
        assert_eq!(ev1.len(), 8);
        assert_eq!(ev1, &[1, 2, 3, 4, 5, 6, 7, 8]);
        assert_eq!(ts1, 3);

        assert!(k.next_event().is_none());
    }
}
