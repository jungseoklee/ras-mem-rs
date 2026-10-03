#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LongSize {
    L4,
    L8,
    SameAsHost,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Endian {
    Little,
    Big,
    SameAsHost,
}

impl Endian {
    #[inline]
    pub fn is_big(&self) -> bool {
        match self {
            Endian::Big => true,
            Endian::Little => false,
            Endian::SameAsHost => cfg!(target_endian = "big"),
        }
    }

    #[inline]
    pub fn is_little(&self) -> bool {
        !self.is_big()
    }

    #[inline]
    pub fn from_host() -> Self {
        Self::SameAsHost
    }
}

impl Default for Endian {
    fn default() -> Self {
        if cfg!(target_endian = "little") {
            Endian::Little
        } else {
            Endian::Big
        }
    }
}

impl Default for LongSize {
    fn default() -> Self {
        Self::SameAsHost
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_longsize_default() {
        assert_eq!(LongSize::default(), LongSize::SameAsHost);
    }

    #[test]
    fn test_endian_is_big_is_little() {
        for e in [Endian::Little, Endian::Big, Endian::SameAsHost] {
            assert_ne!(e.is_big(), e.is_little());
        }
    }

    #[test]
    fn test_endian_same_as_host_matches_cfg() {
        assert_eq!(Endian::SameAsHost.is_big(), cfg!(target_endian = "big"));
        assert_eq!(Endian::SameAsHost.is_little(), !cfg!(target_endian = "big"));
    }

    #[test]
    fn test_endian_from_host_constructor() {
        assert_eq!(Endian::from_host(), Endian::SameAsHost);
        // from_host should be equivalent to SameAsHost in behavior
        assert_eq!(Endian::from_host().is_big(), Endian::SameAsHost.is_big());
        assert_eq!(
            Endian::from_host().is_little(),
            Endian::SameAsHost.is_little()
        );
    }

    #[test]
    fn test_endian_default_matches_target() {
        let def = Endian::default();
        if cfg!(target_endian = "little") {
            assert_eq!(def, Endian::Little);
            assert!(def.is_little());
            assert!(!def.is_big());
        } else {
            assert_eq!(def, Endian::Big);
            assert!(def.is_big());
            assert!(!def.is_little());
        }
    }

    #[test]
    fn test_endian_default_matches_same_as_host() {
        let default = Endian::default();
        let same = Endian::SameAsHost;

        assert_eq!(default.is_big(), same.is_big());
        assert_eq!(default.is_little(), same.is_little());
    }
}
