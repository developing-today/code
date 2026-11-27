//! The fleet laptop-agent domain (epic developing-today/code#5412,
//! issue developing-today/code#10796): the client for the laptop-agent
//! daemon, in the fleet binary.
//!
//! The behaviour mirrors the Python client it replaces
//! (`~/tools/laptop-agent.py`): the same verbs, the same daemon
//! protocol, the same trip-log record shape. What the binary adds is
//! the transport ladder: broker socket, then the Cloudflare tunnel,
//! then Tailscale CONNECT, then SSH. A dead leg degrades to the next;
//! only a total failure errors, naming every leg that failed.

pub mod cli;
pub mod client;
pub mod guard;
pub mod ladder;
pub mod legs;
pub mod tls;

use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// The laptop's tailnet address and the daemon port (`TARGET_HOST` /
/// `TARGET_PORT` in the Python client).
pub const TARGET_HOST: &str = "100.99.113.103";
pub const TARGET_PORT: u16 = 4799;
/// Exit code for a caller deadline (GNU timeout's code, as the Python).
pub const TIMEOUT_EXIT: i32 = 124;
/// Local deadline = remote timeout + this grace (`TRANSPORT_GRACE_S`).
pub const TRANSPORT_GRACE_S: u64 = 15;
/// Bound on one server-side long-poll wait for `GET /jobs/<job>`
/// (developing-today/code#5757): the client asks for at most this many
/// seconds per trip, and both the daemon and the SSH fallback leg hold
/// the request up to this long, returning sooner on output or
/// completion. A quiet ~60 s job then costs 2 trips (50 s + remainder),
/// >=5x fewer than the 10 pre-long-poll backoff trips, while completion
/// is still reported within about a second.
pub const LONG_POLL_WAIT_S: u64 = 50;
/// Time the deadline path may spend asking the daemon to cancel.
pub const CANCEL_BUDGET_S: u64 = 3;
/// Per-stream capture cap the daemon applies to /exec output.
pub const FILE_CHUNK_MAX: u64 = 1 << 20;
/// The fleet secret carrying the Cloudflare Access service token.
pub const CF_ACCESS_SECRET: &str = "cloudflare.access-laptop-agent";
/// Seconds from now by which an `exec` deadline is clamped (`LAPTOP_AGENT_DEADLINE_S`).
pub const EXTERNAL_DEADLINE_ENV: &str = "LAPTOP_AGENT_DEADLINE_S";

pub fn home() -> PathBuf {
    fleet_config::Config::home_dir()
}

pub fn token_file() -> PathBuf {
    home().join("tools/laptop-agent-token")
}

pub fn offset_dir() -> PathBuf {
    home().join(".cache/laptop-agent")
}

pub fn broker_socket() -> PathBuf {
    offset_dir().join("broker.sock")
}

pub fn trip_log() -> PathBuf {
    home().join(".local/state/laptop-agent/trips.jsonl")
}

/// The agent token, trimmed. It is sent as a header only; it never
/// reaches argv, a log line or an error message.
pub fn read_token(path: &Path) -> Result<String, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("token file {} unreadable: {}", path.display(), e.kind()))?;
    let token = text.trim().to_string();
    if token.is_empty() {
        return Err(format!("token file {} is empty", path.display()));
    }
    Ok(token)
}

/// Twelve hex characters from the kernel's entropy source: one trip id
/// per request, carried in the X-Trip-Id header and the body.
pub fn new_trip() -> String {
    let mut bytes = [0u8; 6];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom")
        && f.read_exact(&mut bytes).is_ok()
    {
        return bytes.iter().map(|b| format!("{b:02x}")).collect();
    }
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!("{:012x}", (nanos as u64) & 0xffff_ffff_ffff)
}

/// One client-side trip record. The field names and their order are
/// the Python client's (`_log_trip`): a log reader cannot tell which
/// client wrote a line.
#[derive(serde::Serialize, Debug, PartialEq)]
pub struct TripRecord {
    /// Trip-log schema version; 2 matches the Python client's v2 lines.
    pub v: u8,
    pub ts: String,
    pub trip: String,
    pub path: String,
    pub total_ms: u128,
    pub status: Option<u16>,
    /// The leg that answered, in the Python client's spelling
    /// (broker / cf / ts / ssh); absent when no leg answered.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transport: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_sent: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_returned: Option<u64>,
    /// Daemon-echoed receive time (the reply's `_t_received`), when
    /// the reply carried one — the Python client's third v2 stamp.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub t_received: Option<u64>,
    /// The caller's `FLEET_CTX_ID` (code#14188): the join key that
    /// ties this trip to the daemon's spool and the brokers' trip
    /// lines. Absent when the caller has no context.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ctx_id: Option<String>,
}

/// Milliseconds since the Unix epoch.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}

/// The Python client's spelling for a ladder leg name.
pub fn transport_name(leg: &str) -> String {
    match leg {
        "cloudflare" => "cf".to_string(),
        "tailscale" => "ts".to_string(),
        other => other.to_string(),
    }
}

/// Append one trip record with a single O_APPEND write. Logging must
/// never break or visibly delay a call, so every error is swallowed.
pub fn log_trip(
    trip: &str,
    path: &str,
    total_ms: u128,
    status: Option<u16>,
    transport: Option<String>,
    t_sent: Option<u64>,
    t_returned: Option<u64>,
    t_received: Option<u64>,
) {
    let _ = append_trip(
        &trip_log(),
        &TripRecord {
            v: 2,
            ts: iso_utc(now_secs()),
            trip: trip.to_string(),
            path: path.to_string(),
            total_ms,
            status,
            transport,
            t_sent,
            t_returned,
            t_received,
            ctx_id: fleet_config::ctx::Ctx::from_env().id().map(str::to_string),
        },
    );
}

fn append_trip(file: &Path, rec: &TripRecord) -> std::io::Result<()> {
    use std::io::Write;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(rec).unwrap_or_default();
    line.push('\n');
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(file)?;
    f.write_all(line.as_bytes())
}

fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a Unix time, the Python's `strftime`
/// shape (civil-from-days, proleptic Gregorian).
pub fn iso_utc(secs: i64) -> String {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso_utc_matches_known_instants() {
        assert_eq!(iso_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(iso_utc(1_700_000_000), "2023-11-14T22:13:20Z");
        assert_eq!(iso_utc(951_782_400), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn trip_ids_are_twelve_hex_characters() {
        let t = new_trip();
        assert_eq!(t.len(), 12);
        assert!(t.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn trip_record_keeps_the_python_field_order() {
        let rec = TripRecord {
            v: 2,
            ts: "T".into(),
            trip: "t".into(),
            path: "/exec".into(),
            total_ms: 5,
            status: None,
            transport: Some("ts".into()),
            t_sent: Some(100),
            t_returned: Some(105),
            t_received: Some(103),
            ctx_id: None,
        };
        assert_eq!(
            serde_json::to_string(&rec).unwrap(),
            r#"{"v":2,"ts":"T","trip":"t","path":"/exec","total_ms":5,"status":null,"transport":"ts","t_sent":100,"t_returned":105,"t_received":103}"#
        );
        // #14188: a caller with context adds the join key last.
        let rec = TripRecord {
            ctx_id: Some("task-22655".into()),
            ..rec
        };
        assert!(
            serde_json::to_string(&rec)
                .unwrap()
                .ends_with(r#","t_received":103,"ctx_id":"task-22655"}"#)
        );
    }

    #[test]
    fn empty_token_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("la-token-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("token");
        std::fs::write(&p, "  \n").unwrap();
        assert!(read_token(&p).unwrap_err().contains("empty"));
        std::fs::write(&p, "abc\n").unwrap();
        assert_eq!(read_token(&p).unwrap(), "abc");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
