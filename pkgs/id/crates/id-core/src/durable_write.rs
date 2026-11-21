//! Crash-safe replacement of a small state file: the bytes reach disk before the
//! rename makes them visible at `path`, so a power loss never leaves an empty or
//! torn file at the real path.
use std::fs::File;
use std::io::Write as _;
use std::path::Path;

/// Writes `bytes` to `tmp`, syncs it, renames it over `path`, then syncs the parent directory.
pub fn write_durable(path: &Path, tmp: &Path, bytes: &[u8]) -> std::io::Result<()> {
    {
        let mut file = File::create(tmp)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    std::fs::rename(tmp, path)?;
    if let Some(dir) = path.parent().filter(|dir| !dir.as_os_str().is_empty())
        && let Ok(dir) = File::open(dir)
    {
        let _ = dir.sync_all();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::write_durable;

    #[test]
    fn replaces_file_and_leaves_no_temp() {
        let dir = std::env::temp_dir().join(format!("durable-write-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("create dir");
        let path = dir.join("state.json");
        let tmp = dir.join("state.json.tmp");

        write_durable(&path, &tmp, b"first").expect("first write");
        write_durable(&path, &tmp, b"second").expect("second write");

        assert_eq!(std::fs::read(&path).expect("read"), b"second");
        assert!(!tmp.exists());
        std::fs::remove_dir_all(&dir).expect("cleanup");
    }
}
