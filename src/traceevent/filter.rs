#[derive(Clone, Debug, Default)]
pub struct TepFilter {
    exprs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FilterError {
    Empty,
}

impl std::fmt::Display for FilterError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FilterError::Empty => write!(f, "empty filter expression"),
        }
    }
}

impl std::error::Error for FilterError {}

impl TepFilter {
    pub fn new() -> Self {
        Self { exprs: Vec::new() }
    }

    pub fn add_filter_str(&mut self, s: &str) -> Result<(), FilterError> {
        let s = s.trim();
        if s.is_empty() {
            return Err(FilterError::Empty);
        }
        self.exprs.push(s.to_string());
        Ok(())
    }

    pub fn is_empty(&self) -> bool {
        self.exprs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.exprs.len()
    }

    pub fn as_slice(&self) -> &[String] {
        &self.exprs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_new() {
        let f = TepFilter::new();
        assert!(f.is_empty());
    }

    #[test]
    fn test_add_filter_str() {
        let mut f = TepFilter::new();
        f.add_filter_str("type==1").unwrap();
        f.add_filter_str("mc_index==0").unwrap();
        assert_eq!(2, f.len());
        assert_eq!(["type==1", "mc_index==0"], f.as_slice());
    }

    #[test]
    fn test_add_filter_str_with_whitespaces() {
        let mut f = TepFilter::new();
        f.add_filter_str(" type==1 ").unwrap();
        assert_eq!(["type==1"], f.as_slice());
    }

    #[test]
    fn test_filter_error_empty() {
        let mut f = TepFilter::new();
        assert_eq!(Err(FilterError::Empty), f.add_filter_str(" "));
    }
}
