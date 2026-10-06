use std::{
    fs::File,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
};

pub fn find_mount_point(fs_type: &str) -> Option<PathBuf> {
    let file = File::open("/proc/mounts").ok()?;
    find_mount_point_from_reader(BufReader::new(file), fs_type)
}

fn find_mount_point_from_reader<R: BufRead>(reader: R, fs_type: &str) -> Option<PathBuf> {
    for line in reader.lines().filter_map(Result::ok) {
        let mut parts = line.split_whitespace();
        let Some(_device) = parts.next() else {
            continue;
        };
        let Some(dir) = parts.next() else { continue };
        let Some(typ) = parts.next() else { continue };

        if typ == fs_type {
            return Some(PathBuf::from(unescape_mount_path(dir)));
        }
    }
    None
}

pub fn find_tracing_dir() -> Option<PathBuf> {
    find_tracing_dir_with_finder(|fs_type| find_mount_point(fs_type), Path::exists)
}

fn find_tracing_dir_with_finder<F, E>(finder: F, exists: E) -> Option<PathBuf>
where
    F: Fn(&str) -> Option<PathBuf>,
    E: Fn(&Path) -> bool,
{
    if let Some(dir) = finder("tracefs") {
        return Some(dir);
    }
    if let Some(dir) = finder("debugfs") {
        let candidate = dir.join("tracing");
        if exists(&candidate) {
            return Some(candidate);
        }
    }
    ["/sys/kernel/tracing", "/sys/kernel/debug/tracing"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| exists(p))
}

fn unescape_mount_path(s: &str) -> String {
    // same as yours, but check 0..=7 only
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' {
            let mut oct = String::new();
            for _ in 0..3 {
                if let Some(&d) = chars.peek() {
                    if ('0'..='7').contains(&d) {
                        oct.push(d);
                        chars.next();
                    } else {
                        break;
                    }
                }
            }
            if oct.len() == 3 {
                if let Ok(v) = u8::from_str_radix(&oct, 8) {
                    out.push(v as char);
                    continue;
                }
            }
            out.push('\\');
            out.push_str(&oct);
        } else {
            out.push(c);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn test_find_mount_point_from_reader() {
        let data = "proc /proc proc rw 0 0\ntracefs /sys/kernel/tracing tracefs rw 0 0\n";
        let result = find_mount_point_from_reader(Cursor::new(data), "tracefs");
        assert_eq!(result, Some(PathBuf::from("/sys/kernel/tracing")));
    }

    #[test]
    fn test_find_mount_point_from_reader_with_escaped_space() {
        let data = "dev /media/my\\040disk ext4 rw 0 0\n";
        let result = find_mount_point_from_reader(Cursor::new(data), "ext4");
        assert_eq!(result, Some(PathBuf::from("/media/my disk")));
    }

    #[test]
    fn test_find_mount_point_from_reader_with_nonexistent_fs_type() {
        let data = "proc /proc proc rw 0 0\n";
        let result = find_mount_point_from_reader(Cursor::new(data), "tracefs");
        assert_eq!(result, None);
    }

    #[test]
    fn test_find_mount_point_from_reader_with_malformed_line() {
        let data =
            "malformedline\nproc /proc proc rw 0 0\ntracefs /sys/kernel/tracing tracefs rw 0 0\n";
        let result = find_mount_point_from_reader(Cursor::new(data), "tracefs");
        assert_eq!(result, Some(PathBuf::from("/sys/kernel/tracing")));
    }

    #[test]
    fn test_find_tracing_dir_with_finder_with_tracefs() {
        let finder = |fs: &str| match fs {
            "tracefs" => Some(PathBuf::from("/tracing")),
            _ => None,
        };
        let exists = |_: &Path| true;
        assert_eq!(
            find_tracing_dir_with_finder(finder, exists),
            Some(PathBuf::from("/tracing"))
        );
    }

    #[test]
    fn test_find_tracing_dir_with_finder_with_debugfs() {
        let finder = |fs: &str| match fs {
            "tracefs" => None,
            "debugfs" => Some(PathBuf::from("/sys/kernel/debug")),
            _ => None,
        };
        let exists = |p: &Path| p == Path::new("/sys/kernel/debug/tracing");
        assert_eq!(
            find_tracing_dir_with_finder(finder, exists),
            Some(PathBuf::from("/sys/kernel/debug/tracing"))
        );
    }

    #[test]
    fn test_find_tracing_dir_with_finder_with_default() {
        let finder = |fs: &str| match fs {
            "tracefs" => None,
            "debugfs" => Some(PathBuf::from("/sys/kernel/debug")),
            _ => None,
        };
        // debugfs/tracing doesn't exist, but /sys/kernel/tracing does
        let exists = |p: &Path| p == Path::new("/sys/kernel/tracing");
        assert_eq!(
            find_tracing_dir_with_finder(finder, exists),
            Some(PathBuf::from("/sys/kernel/tracing"))
        );
    }

    #[test]
    fn test_unescape() {
        assert_eq!(unescape_mount_path("/media/my\\040disk"), "/media/my disk");
        assert_eq!(unescape_mount_path("/tmp/foo\\011bar"), "/tmp/foo\tbar");
        assert_eq!(
            unescape_mount_path("/path/with\\134backslash"),
            "/path/with\\backslash"
        );
        assert_eq!(unescape_mount_path("/no/escape"), "/no/escape");
        assert_eq!(unescape_mount_path("\\040"), " ");
    }
}
