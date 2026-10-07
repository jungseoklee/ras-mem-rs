#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Instance {
    Dram,
    Hbm,
}

impl Instance {
    pub fn name(&self) -> &'static str {
        match self {
            Self::Dram => "ras_mem_d",
            Self::Hbm => "ras_mem_h",
        }
    }

    pub fn buffer_size_kb(&self) -> u64 {
        match self {
            Self::Dram => 1024,
            Self::Hbm => 8192,
        }
    }

    pub fn buffer_percent(&self) -> u8 {
        match self {
            Self::Dram => 50,
            Self::Hbm => 0,
        }
    }

    pub fn thread_name(&self) -> &'static str {
        self.name()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dram() {
        let inst = Instance::Dram;

        assert_eq!(inst.name(), "ras_mem_d");
        assert_eq!(inst.buffer_size_kb(), 1024);
        assert_eq!(inst.buffer_percent(), 50);
        assert_eq!(inst.thread_name(), "ras_mem_d");
    }

    #[test]
    fn test_hbm() {
        let inst = Instance::Hbm;

        assert_eq!(inst.name(), "ras_mem_h");
        assert_eq!(inst.buffer_size_kb(), 8192);
        assert_eq!(inst.buffer_percent(), 0);
        assert_eq!(inst.thread_name(), "ras_mem_h");
    }
}
