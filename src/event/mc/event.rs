use std::time::SystemTime;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum HwEventErrType {
    Corrected = 0,
    Uncorrected = 1,
    Deferred = 2,
    Fatal = 3,
    Info = 4,
}

impl HwEventErrType {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Corrected => "Corrected",
            Self::Uncorrected => "Uncorrected",
            Self::Deferred => "Deferred",
            Self::Fatal => "Fatal",
            Self::Info => "Info",
        }
    }

    pub fn log_level(&self) -> &'static str {
        match self {
            Self::Uncorrected | Self::Deferred => "crit",
            Self::Fatal => "emerg",
            Self::Corrected => "err",
            Self::Info => "debug",
        }
    }
}

impl From<i64> for HwEventErrType {
    fn from(v: i64) -> Self {
        match v {
            0 => Self::Corrected,
            1 => Self::Uncorrected,
            2 => Self::Deferred,
            3 => Self::Fatal,
            _ => Self::Info,
        }
    }
}

#[derive(Debug, Clone)]
pub struct McEvent {
    pub timestamp: SystemTime,

    pub error_count: i64,
    pub error_type: HwEventErrType,

    pub msg: String,
    pub label: String,
    pub mc_index: i64,

    pub top_layer: i8,
    pub middle_layer: i8,
    pub lower_layer: i8,

    pub address: u64,
    pub grain: i64,
    pub syndrome: u64,
    pub driver_detail: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_as_str() {
        assert_eq!(HwEventErrType::Corrected.as_str(), "Corrected");
        assert_eq!(HwEventErrType::Uncorrected.as_str(), "Uncorrected");
        assert_eq!(HwEventErrType::Deferred.as_str(), "Deferred");
        assert_eq!(HwEventErrType::Fatal.as_str(), "Fatal");
        assert_eq!(HwEventErrType::Info.as_str(), "Info");
    }

    #[test]
    fn test_log_level() {
        assert_eq!(HwEventErrType::Corrected.log_level(), "err");
        assert_eq!(HwEventErrType::Uncorrected.log_level(), "crit");
        assert_eq!(HwEventErrType::Deferred.log_level(), "crit");
        assert_eq!(HwEventErrType::Fatal.log_level(), "emerg");
        assert_eq!(HwEventErrType::Info.log_level(), "debug");
    }

    #[test]
    fn test_from() {
        assert_eq!(HwEventErrType::from(0), HwEventErrType::Corrected);
        assert_eq!(HwEventErrType::from(1), HwEventErrType::Uncorrected);
        assert_eq!(HwEventErrType::from(2), HwEventErrType::Deferred);
        assert_eq!(HwEventErrType::from(3), HwEventErrType::Fatal);
        assert_eq!(HwEventErrType::from(-1), HwEventErrType::Info);
        assert_eq!(HwEventErrType::from(99), HwEventErrType::Info);
    }
}
