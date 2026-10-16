use std::{collections::HashSet, path::PathBuf};

pub struct ProcFs {
    root: PathBuf, // /proc
}

impl ProcFs {
    pub fn new() -> anyhow::Result<Self> {
        Self::from_path(PathBuf::from("/proc"))
    }

    pub(crate) fn from_path(root: PathBuf) -> anyhow::Result<Self> {
        Ok(Self { root })
    }

    pub fn allowed_cpu_ids(&self) -> anyhow::Result<HashSet<u32>> {
        let mut cpu_ids = HashSet::new();
        let status = std::fs::read_to_string(self.root.join("self/status"))?;
        for line in status.lines() {
            if let Some(list) = line.strip_prefix("Cpus_allowed_list:") {
                for part in list.trim().split(',') {
                    if let Some((a, b)) = part.split_once('-') {
                        let a: u32 = a.trim().parse()?;
                        let b: u32 = b.trim().parse()?;
                        cpu_ids.extend(a..=b);
                    } else if !part.trim().is_empty() {
                        cpu_ids.insert(part.trim().parse()?);
                    }
                }
            }
        }
        Ok(cpu_ids)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    fn fs_with_status(status: &str) -> (TempDir, ProcFs) {
        let tmp = TempDir::new().unwrap();
        let self_dir = tmp.path().join("self");
        fs::create_dir_all(&self_dir).unwrap();
        fs::write(self_dir.join("status"), status).unwrap();
        let fs = ProcFs::from_path(tmp.path().to_path_buf()).unwrap();
        (tmp, fs)
    }

    #[test]
    fn test_allowed_cpu_ids() {
        let (_tmp, fs) = fs_with_status("Cpus_allowed_list: 0-7\n");
        let ids = fs.allowed_cpu_ids().unwrap();
        assert_eq!(
            fs.allowed_cpu_ids().unwrap(),
            HashSet::from([0, 1, 2, 3, 4, 5, 6, 7])
        );
    }

    #[test]
    fn test_allowed_cpu_ids_with_single_cpu() {
        let (_tmp, fs) = fs_with_status("Cpus_allowed_list: 3\n");
        assert_eq!(fs.allowed_cpu_ids().unwrap(), HashSet::from([3]));
    }

    #[test]
    fn test_allowed_cpu_ids_with_spaces_and_commas() {
        let (_tmp, fs) = fs_with_status("Cpus_allowed_list: 0-1, 3, 5-6\n");
        assert_eq!(
            fs.allowed_cpu_ids().unwrap(),
            HashSet::from([0, 1, 3, 5, 6])
        );
    }
}
