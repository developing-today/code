//! Best-effort local record of fleet CLI invocations for quota analytics.

use fleet_config::Config;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA: &str = "fleet-invocation/v1";
const FILE: &str = "invocations.jsonl";
const MAX_BYTES: u64 = 4 * 1024 * 1024;
const KEEP: usize = 4;

pub fn path() -> PathBuf {
    std::env::var_os("FLEET_INVOCATION_LEDGER")
        .map(PathBuf::from)
        .unwrap_or_else(|| Config::state_dir().join(FILE))
}

pub fn record(
    invoked_as: &str,
    args: &[String],
    code: i32,
    duration_ms: u128,
) -> serde_json::Value {
    let host = std::env::var("HOSTNAME").ok().filter(|s| !s.is_empty());
    let caller = [
        "FLEET_CALLER",
        "OPENCODE_SESSION_ID",
        "MCPX_SESSION_ID",
        "FLEET_SESSION_ID",
        "SYSTEMD_UNIT",
        "CRON_JOB",
    ]
    .iter()
    .find_map(|key| std::env::var(key).ok().filter(|s| !s.is_empty()));
    json!({
        "schema": SCHEMA,
        "at_unix_ms": SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis()).unwrap_or(0),
        "invoked_as": invoked_as,
        "command": if matches!(invoked_as, "fleet" | "fleet-serve" | "fleet-connect") { args.first().map(String::as_str) } else if invoked_as == "repo-queue" { Some("queue") } else { Some(invoked_as) },
        "subcommand": if invoked_as == "repo-queue" { args.first().map(String::as_str) } else { args.get(1).map(String::as_str) },
        "sub_subcommand": if invoked_as == "repo-queue" { args.get(1).map(String::as_str) } else { args.get(2).map(String::as_str) },
        "exit_code": code,
        "duration_ms": duration_ms,
        "host": host,
        "caller": caller,
    })
}

pub fn append(path: &Path, value: &serde_json::Value) -> std::io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    if path
        .metadata()
        .map(|m| m.len() >= MAX_BYTES)
        .unwrap_or(false)
    {
        for i in (1..KEEP).rev() {
            let from = rotated(path, i);
            let to = rotated(path, i + 1);
            if from.exists() {
                let _ = std::fs::remove_file(&to);
                if let Err(error) = std::fs::rename(from, to) {
                    if error.kind() != std::io::ErrorKind::NotFound {
                        return Err(error);
                    }
                }
            }
        }
        let _ = std::fs::remove_file(rotated(path, 1));
        std::fs::rename(path, rotated(path, 1))?;
    }
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{value}")
}

fn rotated(path: &Path, index: usize) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{index}"));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn appends_versioned_records_and_rotates() {
        let dir = std::env::temp_dir().join(format!("fleet-ledger-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join(FILE);
        append(
            &path,
            &record("fleet", &["queue".into(), "list".into()], 0, 4),
        )
        .unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&std::fs::read_to_string(&path).unwrap())
                .unwrap()["schema"],
            SCHEMA
        );
        std::fs::write(&path, vec![b'x'; MAX_BYTES as usize]).unwrap();
        append(&path, &record("fleet", &[], 2, 1)).unwrap();
        assert!(rotated(&path, 1).exists());
        assert_eq!(std::fs::read_to_string(&path).unwrap().lines().count(), 1);
        let _ = std::fs::remove_dir_all(dir);
    }
}
