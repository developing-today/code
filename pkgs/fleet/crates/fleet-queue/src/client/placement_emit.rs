//! Best-effort placement-record emission on the queue-claim path (#4986).
//!
//! After a successful claim (the `GuardOutcome::Serve` arm of
//! `cmd_claim`), the queue client invokes the placement-record PoC
//! binary with answers describing the handoff and logs the emitted
//! record. The claim itself is the placement for queue work (#8187):
//! every claimed task records the claiming lane's option under rule
//! `"exception"` with an E1 exception id citing #8187.
//!
//! The claim never fails because of this. When `FLEET_PLACEMENT_POLICY`
//! is unset the emitter is fully silent; a missing binary, an
//! unreadable policy, or a record error costs one stderr line and the
//! claim proceeds with its original output and exit code.

use serde_json::{Value, json};
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};

/// Resolve the placement-record binary: explicit override or PATH.
fn record_bin() -> String {
    std::env::var("FLEET_PLACEMENT_RECORD_BIN")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "placement-record".to_string())
}

/// The policy file this emission evaluates against. None means the
/// wiring is not configured here: skip silently.
fn policy_path() -> Option<String> {
    std::env::var("FLEET_PLACEMENT_POLICY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// An optional non-empty env value.
fn env_value(name: &str) -> Option<String> {
    std::env::var(name)
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

/// Map a claiming lane to the placement option it runs on.
/// `oc-*` lanes run OpenChamber sessions; `code-*` lanes run
/// Muse sessions. Lanes with no family prefix (ad-hoc
/// `agent-next-*`, fleet ops lanes) name no option and are skipped:
/// a record with a guessed option is worse than none.
pub fn lane_option(lane: &str) -> Option<&'static str> {
    if lane.starts_with("oc-") {
        Some("opencode-session")
    } else if lane.starts_with("code-") {
        Some("Muse")
    } else {
        None
    }
}

/// Today's UTC date as `YYYY-MM-DD`, for the E1 exception id.
fn today_ymd() -> String {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Howard Hinnant's civil-from-days algorithm.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as i64;
    format!("{:04}-{:02}-{:02}", if m <= 2 { y + 1 } else { y }, m, d)
}

/// Build the placement-record answers for a claimed task. The claim
/// is the placement (#8187), so every handoff records an E1
/// exception with the claiming lane's option; the task text names
/// the claimed item for the audit trail.
pub fn placement_answers(
    task: &Value,
    worker: &str,
    lane: &str,
    option: &str,
    today: &str,
) -> Value {
    let id = task.get("id").and_then(|v| v.as_i64()).unwrap_or(0);
    let kind = task.get("kind").and_then(|v| v.as_str()).unwrap_or("task");
    let title = task.get("title").and_then(|v| v.as_str()).unwrap_or("");
    json!({
        "task": format!("claimed {kind} task {id} ({title}) by {worker}/{lane}"),
        "exception_id": format!("E1-{today}-8187"),
        "exception_option": option,
    })
}

static ANSWERS_SEQ: AtomicU64 = AtomicU64::new(0);

/// Emit the placement record for a freshly claimed task row.
/// Best-effort: returns always, never panics, never alters the claim.
pub fn emit_placement_record(row: &Value, worker: &str, lane: &str) {
    let Some(option) = lane_option(lane) else {
        return;
    };
    let Some(policy) = policy_path() else {
        return;
    };
    let answers = placement_answers(row, worker, lane, option, &today_ymd());
    let path = std::env::temp_dir().join(format!(
        "placement-answers-{}-{}.json",
        std::process::id(),
        ANSWERS_SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let body = serde_json::to_string(&answers).unwrap_or_else(|_| "{}".to_string());
    if std::fs::write(&path, body).is_err() {
        return;
    }
    let mut cmd = std::process::Command::new(record_bin());
    cmd.arg("--policy").arg(&policy).arg("--answers").arg(&path);
    // Optional quota wiring: the dispatcher log path passes the live
    // quota-current read through so pool-drawn options evaluate
    // against the B5 measurement gate instead of erroring.
    if let (Some(qc), Some(pool)) = (
        env_value("FLEET_PLACEMENT_QUOTA_CURRENT"),
        env_value("FLEET_PLACEMENT_POOL"),
    ) {
        cmd.arg("--quota-current").arg(qc).arg("--pool").arg(pool);
    }
    if let Some(hs) = env_value("FLEET_PLACEMENT_HARD_STOPS") {
        cmd.arg("--hard-stops").arg(hs);
    }
    let result = cmd.output();
    let _ = std::fs::remove_file(&path);
    match result {
        Ok(out) if out.status.success() => {
            let record = String::from_utf8_lossy(&out.stdout).trim().to_string();
            eprintln!("placement-record: {record}");
            if let Some(log) = env_value("FLEET_PLACEMENT_LOG") {
                let _ = (|| -> std::io::Result<()> {
                    let mut f = std::fs::OpenOptions::new()
                        .create(true)
                        .append(true)
                        .open(&log)?;
                    writeln!(f, "{record}")?;
                    Ok(())
                })();
            }
        }
        Ok(out) => {
            let code = out
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".to_string());
            let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
            eprintln!("placement-record: recorder exited {code}: {detail}");
        }
        Err(e) => {
            eprintln!("placement-record: recorder unavailable: {e}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    // Env access is process-global (and unsafe on this toolchain),
    // so env-mutating tests serialize on this lock.
    static ENV_LOCK: Mutex<()> = Mutex::new(());

    fn set_env(name: &str, value: &str) {
        // SAFETY: held under ENV_LOCK; no other thread touches env.
        unsafe {
            std::env::set_var(name, value);
        }
    }

    fn remove_env(name: &str) {
        // SAFETY: held under ENV_LOCK; no other thread touches env.
        unsafe {
            std::env::remove_var(name);
        }
    }

    #[test]
    fn lane_option_names_the_claiming_lanes_option() {
        // Oracle: the mapping function itself.
        assert_eq!(lane_option("oc-code-02"), Some("opencode-session"));
        assert_eq!(lane_option("oc-anything"), Some("opencode-session"));
        assert_eq!(lane_option("code-a"), Some("Muse"));
        assert_eq!(lane_option("code-j"), Some("Muse"));
        assert_eq!(lane_option("agent-next-23-local"), None);
        assert_eq!(lane_option(""), None);
    }

    #[test]
    fn placement_answers_record_the_8187_exception() {
        // Oracle: the answers JSON handed to placement-record.
        let task = json!({"id": 25572, "kind": "small_fix", "title": "Triage #4986"});
        let a = placement_answers(
            &task,
            "repo-worker-code",
            "oc-code-02",
            "opencode-session",
            "2026-11-27",
        );
        assert_eq!(a["exception_id"], json!("E1-2026-11-27-8187"));
        assert_eq!(a["exception_option"], json!("opencode-session"));
        assert_eq!(
            a["task"],
            json!("claimed small_fix task 25572 (Triage #4986) by repo-worker-code/oc-code-02")
        );
    }

    #[test]
    fn today_ymd_is_a_calendar_date() {
        // Oracle: the formatted string itself.
        let t = today_ymd();
        assert_eq!(t.len(), 10);
        assert!(t.starts_with("2026-"), "{t}");
    }

    #[test]
    fn emit_without_policy_is_a_silent_no_op() {
        // Oracle: the log file stays absent.
        let _guard = ENV_LOCK.lock().unwrap();
        remove_env("FLEET_PLACEMENT_POLICY");
        let log = std::env::temp_dir().join(format!("placement-noop-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        set_env("FLEET_PLACEMENT_LOG", log.to_str().unwrap());
        emit_placement_record(&json!({"id": 1}), "w", "oc-x");
        assert!(!log.exists());
        remove_env("FLEET_PLACEMENT_LOG");
    }

    #[test]
    fn emit_through_a_fake_binary_logs_the_record() {
        // Oracle: the appended log file.
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = std::env::temp_dir().join(format!("placement-fake-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fake = dir.join("placement-record");
        std::fs::write(
            &fake,
            "#!/bin/sh\necho '{\"option\":\"opencode-session\",\"rule\":\"exception\"}'\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        let log = dir.join("records.log");
        set_env("FLEET_PLACEMENT_POLICY", "/nonexistent/policy.yaml");
        set_env("FLEET_PLACEMENT_RECORD_BIN", fake.to_str().unwrap());
        set_env("FLEET_PLACEMENT_LOG", log.to_str().unwrap());
        emit_placement_record(
            &json!({"id": 9, "kind": "triage", "title": "T"}),
            "repo-worker-code",
            "oc-code-02",
        );
        let logged = std::fs::read_to_string(&log).unwrap();
        assert!(logged.contains("\"rule\":\"exception\""), "{logged}");
        assert!(
            logged.contains("\"option\":\"opencode-session\""),
            "{logged}"
        );
        remove_env("FLEET_PLACEMENT_POLICY");
        remove_env("FLEET_PLACEMENT_RECORD_BIN");
        remove_env("FLEET_PLACEMENT_LOG");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn emit_with_missing_binary_never_fails_the_claim() {
        // Oracle: return (no panic) with nothing configured to observe.
        let _guard = ENV_LOCK.lock().unwrap();
        set_env("FLEET_PLACEMENT_POLICY", "/nonexistent/policy.yaml");
        set_env(
            "FLEET_PLACEMENT_RECORD_BIN",
            "/nonexistent/placement-record",
        );
        remove_env("FLEET_PLACEMENT_LOG");
        emit_placement_record(&json!({"id": 2}), "w", "code-a");
        remove_env("FLEET_PLACEMENT_POLICY");
        remove_env("FLEET_PLACEMENT_RECORD_BIN");
    }

    #[test]
    fn emit_for_an_unmapped_lane_is_a_no_op() {
        // Oracle: the log file stays absent.
        let _guard = ENV_LOCK.lock().unwrap();
        set_env("FLEET_PLACEMENT_POLICY", "/nonexistent/policy.yaml");
        let log =
            std::env::temp_dir().join(format!("placement-unmapped-{}.log", std::process::id()));
        let _ = std::fs::remove_file(&log);
        set_env("FLEET_PLACEMENT_LOG", log.to_str().unwrap());
        emit_placement_record(&json!({"id": 3}), "w", "agent-next-1");
        assert!(!log.exists());
        remove_env("FLEET_PLACEMENT_POLICY");
        remove_env("FLEET_PLACEMENT_LOG");
    }
}
