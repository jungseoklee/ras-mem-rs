#[derive(Clone, Debug, Default)]
pub struct TraceSeq {
    buf: String,
}

impl TraceSeq {
    pub fn new() -> Self {
        Self { buf: String::new() }
    }

    pub fn reset(&mut self) {
        self.buf.clear();
    }

    pub fn push(&mut self, c: char) {
        self.buf.push(c);
    }

    pub fn push_str(&mut self, s: &str) {
        self.buf.push_str(s);
    }

    pub fn as_str(&self) -> &str {
        &self.buf
    }
}

impl std::fmt::Write for TraceSeq {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.push_str(s);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fmt::Write;

    #[test]
    fn test_new() {
        let seq = TraceSeq::new();
        assert_eq!("", seq.as_str());
        assert!(seq.as_str().is_empty());
    }

    #[test]
    fn test_push_str() {
        let mut seq = TraceSeq::new();
        seq.push_str("hello");
        assert_eq!("hello", seq.as_str());
        seq.push_str(", world");
        assert_eq!("hello, world", seq.as_str());
    }

    #[test]
    fn test_push() {
        let mut seq = TraceSeq::new();
        seq.push('a');
        seq.push(':');
        seq.push('z');
        assert_eq!("a:z", seq.as_str());
    }

    #[test]
    fn test_fmt_write_trait() {
        let mut seq = TraceSeq::new();
        write!(&mut seq, "tsc={} walltime={}", 1234, 5678).unwrap();
        assert_eq!("tsc=1234 walltime=5678", seq.as_str());
    }

    #[test]
    fn test_reset() {
        let mut seq = TraceSeq::new();
        seq.push_str("first");
        assert_eq!("first", seq.as_str());
        seq.reset();
        write!(&mut seq, "reused").unwrap();
        assert_eq!("reused", seq.as_str());
    }

    #[test]
    fn test_default() {
        let mut seq: TraceSeq = Default::default();
        assert_eq!("", seq.as_str());
        seq.push_str("from default");
        assert_eq!("from default", seq.as_str());
    }
}
