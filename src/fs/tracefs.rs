use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
};

use crate::core::instance::Instance;

#[derive(Debug)]
pub struct TraceFs {
    root: PathBuf,          // /sys/kernel/tracing
    instance_root: PathBuf, // /sys/kernel/tracing/instances/{name}
}

impl TraceFs {
    pub fn from_instance(instance: Instance) -> anyhow::Result<Self> {
        let root = super::mount::find_tracing_dir()
            .ok_or_else(|| anyhow::anyhow!("tracing fs not mounted"))?;

        let fs = Self::from_path(root, instance.name())?;
        fs.clear()?;
        fs.set_buffer_size_kb(instance.buffer_size_kb())?;
        fs.set_buffer_percent(instance.buffer_percent())?;
        Ok(fs)
    }

    pub(crate) fn from_path(root: PathBuf, name: &str) -> anyhow::Result<Self> {
        if !root.join("instances").exists() {
            anyhow::bail!("3.15+ kernel required");
        }

        let instance_root = root.join(format!("instances/{name}"));
        fs::create_dir_all(&instance_root)?;

        if !instance_root.join("set_event").exists() {
            anyhow::bail!("set_event not found");
        }
        anyhow::Ok(Self {
            root,
            instance_root,
        })
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn instance_root(&self) -> &Path {
        &self.instance_root
    }

    pub fn header_page_path(&self) -> PathBuf {
        self.root.join("events/header_page")
    }

    pub fn event_format_path(&self, group: &str, name: &str) -> PathBuf {
        self.root.join(format!("events/{}/{}/format", group, name))
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
        fs::write(self.set_event(), "")?;
        anyhow::Ok(())
    }

    pub fn set_buffer_size_kb(&self, kb: u64) -> anyhow::Result<()> {
        fs::write(self.instance_root.join("buffer_size_kb"), kb.to_string())?;
        anyhow::Ok(())
    }

    pub fn set_buffer_percent(&self, pct: u8) -> anyhow::Result<()> {
        if pct > 100 {
            anyhow::bail!("buffer_percent must be 0..100");
        }
        fs::write(self.instance_root.join("buffer_percent"), pct.to_string())?;
        anyhow::Ok(())
    }

    pub fn buffer_subbuf_size(&self) -> anyhow::Result<usize> {
        Ok(self.buffer_subbuf_size_kb()? * 1024)
    }

    pub fn buffer_subbuf_size_kb(&self) -> anyhow::Result<usize> {
        let size_kb = fs::read_to_string(self.instance_root.join("buffer_subbuf_size_kb"))?;
        Ok(size_kb.trim().parse()?)
    }

    pub fn per_cpu_trace_pipe_raw(&self) -> anyhow::Result<Vec<(u32, PathBuf)>> {
        let per_cpu = self.instance_root.join("per_cpu");
        anyhow::ensure!(per_cpu.exists(), "per_cpu not found at {per_cpu:?}");

        // TODO: handle offline cpus, e.g. we might need to check a list of online and offline periodically
        let mut online = Vec::new();
        for e in fs::read_dir(&per_cpu)? {
            let e = e?;
            let name = e.file_name().to_string_lossy().to_string();
            let Ok(cpu_id) = name.trim_start_matches("cpu").parse::<u32>() else {
                continue;
            };
            let p = e.path().join("trace_pipe_raw");
            if p.exists() {
                online.push((cpu_id, p));
            }
        }

        anyhow::ensure!(
            !online.is_empty(),
            "no trace_pipe_raw found under {per_cpu:?}"
        );
        online.sort_by_key(|(id, _)| *id);
        Ok(online)
    }

    fn append(&self, payload: &str) -> anyhow::Result<()> {
        if payload.is_empty() {
            return Ok(());
        }
        let mut f = OpenOptions::new()
            .write(true)
            .append(true)
            .open(self.set_event())?;
        f.write_all(payload.as_bytes())?;
        anyhow::Ok(())
    }

    fn set_event(&self) -> PathBuf {
        self.instance_root.join("set_event")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn fs_with_name(name: &str) -> (TempDir, TraceFs) {
        let tmp = TempDir::new().unwrap();
        let root = tmp.path().to_path_buf(); // owned, borrow ends

        let instance = root.join(format!("instances/{name}"));
        fs::create_dir_all(instance.join("per_cpu/cpu0")).unwrap();
        fs::create_dir_all(instance.join("per_cpu/cpu1")).unwrap();
        fs::File::create(instance.join("set_event")).unwrap();
        fs::File::create(instance.join("buffer_size_kb")).unwrap();
        fs::File::create(instance.join("buffer_percent")).unwrap();
        fs::File::create(instance.join("buffer_subbuf_size_kb")).unwrap();
        fs::File::create(instance.join("per_cpu/cpu0/trace_pipe_raw")).unwrap();
        fs::File::create(instance.join("per_cpu/cpu1/trace_pipe_raw")).unwrap();
        fs::write(instance.join("buffer_subbuf_size_kb"), "42").unwrap();

        let fs = TraceFs::from_path(root, name).unwrap();
        (tmp, fs)
    }

    fn fs() -> (TempDir, TraceFs) {
        fs_with_name("ras_mem")
    }

    fn read(fs: &TraceFs) -> String {
        fs::read_to_string(fs.set_event()).unwrap_or_default()
    }

    #[test]
    fn test_from_path_without_instances() {
        let tmp = TempDir::new().unwrap();

        let res = TraceFs::from_path(tmp.path().to_path_buf(), "ras_mem");

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("3.15"));
    }

    #[test]
    fn test_from_path_without_set_event() {
        let tmp = TempDir::new().unwrap();
        let tracing_root = tmp.path().to_path_buf(); // owned, borrow ends
        let instance = tracing_root.join("instances/ras_mem");
        fs::create_dir_all(&instance).unwrap();

        let res = TraceFs::from_path(tracing_root, "ras_mem");

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
    fn test_enable_with_two_different_names() {
        let tmp = TempDir::new().unwrap();
        let tracing_root = tmp.path().to_path_buf();

        for name in ["hbm", "dram"] {
            let instance = tracing_root.join(format!("instances/{name}"));
            fs::create_dir_all(&instance).unwrap();
            fs::File::create(instance.join("set_event")).unwrap();
            fs::File::create(instance.join("buffer_size_kb")).unwrap();
            fs::File::create(instance.join("buffer_percent")).unwrap();
        }

        let h = TraceFs::from_path(tracing_root.clone(), "hbm").unwrap();
        let d = TraceFs::from_path(tracing_root, "dram").unwrap();

        h.enable(["amdgpu:amdgpu_ras_event"]).unwrap();
        d.enable(["ras:mc_event"]).unwrap();

        assert_eq!(read(&h), "amdgpu:amdgpu_ras_event\n");
        assert_eq!(read(&d), "ras:mc_event\n");
    }

    #[test]
    fn test_clear() {
        let (_tmp, fs) = fs();

        fs.enable(["ras:mc_event"]).unwrap();
        fs.clear().unwrap();

        assert_eq!(read(&fs), "");
    }

    #[test]
    fn test_buffer_size_kb_and_buffer_percent() {
        let (_tmp, fs) = fs_with_name("ras_mem_h");
        fs.set_buffer_size_kb(8192).unwrap();
        fs.set_buffer_percent(0).unwrap();
        assert_eq!(
            fs::read_to_string(fs.instance_root.join("buffer_size_kb")).unwrap(),
            "8192"
        );
        assert_eq!(
            fs::read_to_string(fs.instance_root.join("buffer_percent")).unwrap(),
            "0"
        );

        let (_tmp2, fs) = fs_with_name("ras_mem_d");
        fs.set_buffer_size_kb(1024).unwrap();
        fs.set_buffer_percent(50).unwrap();
        assert_eq!(
            fs::read_to_string(fs.instance_root.join("buffer_size_kb")).unwrap(),
            "1024"
        );
    }

    #[test]
    fn test_buffer_percent_with_invalid_percent() {
        let (_tmp, fs) = fs();
        assert!(fs.set_buffer_percent(101).is_err());
    }

    #[test]
    fn test_buffer_subbuf_size_kb() {
        let (_tmp, fs) = fs();
        let size_kb = fs.buffer_subbuf_size_kb().unwrap();
        assert_eq!(size_kb, 42);
    }

    #[test]
    fn test_per_cpu_trace_pipe_raw() {
        let (_tmp, fs) = fs();

        let pipes = fs.per_cpu_trace_pipe_raw().unwrap();

        assert_eq!(pipes.len(), 2);
        assert_eq!(pipes[0].0, 0);
        assert_eq!(pipes[1].0, 1);
        assert!(pipes[0].1.to_string_lossy().contains("cpu0"));
        assert!(pipes[1].1.to_string_lossy().contains("cpu1"));
    }

    #[test]
    fn test_per_cpu_trace_pipe_raw_without_per_cpu() {
        let tmp = TempDir::new().unwrap();
        let tracing_root = tmp.path().to_path_buf(); // owned, borrow ends
        let instance = tracing_root.join("instances/ras_mem");
        fs::create_dir_all(&instance).unwrap();
        fs::File::create(instance.join("set_event")).unwrap();
        let fs = TraceFs::from_path(tracing_root, "ras_mem").unwrap();

        let res = fs.per_cpu_trace_pipe_raw();

        assert!(res.is_err());
        assert!(res.unwrap_err().to_string().contains("per_cpu not found"));
    }

    #[test]
    fn test_per_cpu_trace_pipe_raw_without_trace_pipe_raw() {
        let tmp = TempDir::new().unwrap();
        let tracing_root = tmp.path().to_path_buf(); // owned, borrow ends
        let instance = tracing_root.join("instances/ras_mem");
        fs::create_dir_all(&instance).unwrap();
        fs::create_dir_all(instance.join("per_cpu/cpu0")).unwrap();
        fs::create_dir_all(instance.join("per_cpu/cpu1")).unwrap();
        fs::create_dir_all(instance.join("per_cpu/malformed")).unwrap();
        fs::File::create(instance.join("set_event")).unwrap();
        // fs::File::create(instance.join("per_cpu/cpu1/trace_pipe_raw")).unwrap();
        let fs = TraceFs::from_path(tracing_root, "ras_mem").unwrap();

        let res = fs.per_cpu_trace_pipe_raw();

        assert!(res.is_err());
    }
}
