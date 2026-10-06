use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::PathBuf,
};

#[derive(Debug)]
pub struct TraceFs {
    root: PathBuf,
}

impl TraceFs {
    pub fn new() -> anyhow::Result<Self> {
        let path = super::mount::find_tracing_dir()
            .ok_or_else(|| anyhow::anyhow!("tracing fs not mounted"))?;
        Self::from_path(path)
    }

    fn from_path(tracing_root: PathBuf) -> anyhow::Result<Self> {
        if !tracing_root.join("instances").exists() {
            anyhow::bail!("3.15+ kernel required");
        }

        let root = tracing_root.join("instances/ferras");
        fs::create_dir_all(&root)?;

        if !root.join("set_event").exists() {
            anyhow::bail!("set_event not found");
        }
        Ok(Self { root })
    }

    pub fn enable<I, S>(&self, events: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let payload: String = events
            .into_iter()
            .map(|e| format!("{}\n", e.as_ref()))
            .collect();
        self.append(&payload)
    }

    pub fn disable<I, S>(&self, events: I) -> anyhow::Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let payload: String = events
            .into_iter()
            .map(|e| format!("!{}\n", e.as_ref()))
            .collect();
        self.append(&payload)
    }

    pub fn clear(&self) -> anyhow::Result<()> {
        fs::write(self.set_event_path(), "")?;
        Ok(())
    }

    fn append(&self, payload: &str) -> anyhow::Result<()> {
        if payload.is_empty() {
            return Ok(());
        }
        let mut f = OpenOptions::new()
            .write(true)
            .append(true)
            .open(self.set_event_path())?;
        f.write_all(payload.as_bytes())?;
        Ok(())
    }

    fn set_event_path(&self) -> PathBuf {
        self.root.join("set_event")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fs() -> (TempDir, TraceFs) {
        let tmp = TempDir::new().unwrap();
        let tracing_root = tmp.path().to_path_buf(); // owned, borrow ends

        let instance = tracing_root.join("instances/ferras");
        fs::create_dir_all(&instance).unwrap();
        fs::File::create(instance.join("set_event")).unwrap();

        let fs = TraceFs::from_path(tracing_root).unwrap();
        (tmp, fs)
    }

    fn read(fs: &TraceFs) -> String {
        fs::read_to_string(fs.set_event_path()).unwrap_or_default()
    }

    #[test]
    fn test_from_path_without_instances() {
        let tmp = TempDir::new().unwrap();

        let res = TraceFs::from_path(tmp.path().to_path_buf());

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("3.15"));
    }

    #[test]
    fn test_from_path_without_set_event() {
        let tmp = TempDir::new().unwrap();
        let tracing_root = tmp.path().to_path_buf(); // owned, borrow ends
        let instance = tracing_root.join("instances/ferras");
        fs::create_dir_all(&instance).unwrap();

        let res = TraceFs::from_path(tracing_root);

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("set_event not found"));
    }

    #[test]
    fn test_enable() {
        let (_tmp, fs) = fs();

        fs.enable(["ras:mc_event", "kmem:mm_page_alloc"]).unwrap();

        assert_eq!(read(&fs), "ras:mc_event\nkmem:mm_page_alloc\n");
    }

    #[test]
    fn test_enable_with_an_empty_input() {
        let (_tmp, fs) = fs();

        fs.enable(Vec::<String>::new()).unwrap();

        assert_eq!(read(&fs), "");
    }

    #[test]
    fn test_disable() {
        let (_tmp, fs) = fs();

        fs.disable(["ras:mc_event", "kmem:mm_page_alloc"]).unwrap();

        assert_eq!(read(&fs), "!ras:mc_event\n!kmem:mm_page_alloc\n");
    }

    #[test]
    fn test_enable_and_disable() {
        let (_tmp, fs) = fs();

        fs.enable(["ras:mc_event"]).unwrap();
        fs.disable(["ras:mc_event"]).unwrap();

        assert_eq!(read(&fs), "ras:mc_event\n!ras:mc_event\n");
    }

    #[test]
    fn test_clear() {
        let (_tmp, fs) = fs();

        fs.enable(["ras:mc_event"]).unwrap();
        fs.clear().unwrap();

        assert_eq!(read(&fs), "");
    }
}
