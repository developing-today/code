//! The verbs over the laptop-agent daemon, mirroring `laptop-agent.py`:
//! the same requests, the same output and the same exit codes.
//!
//! The local deadline is a watchdog: the ladder runs on a helper thread
//! and the caller waits for it with a timeout. A leg that hangs past the
//! deadline is abandoned (the process exits soon after), and the
//! deadline path asks the daemon to cancel a running `/exec` by trip.

use std::io::Write;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use sha2::{Digest, Sha256};

use crate::ladder::{Answer, CallError, Failure, Ladder, Request};
use crate::{
    CANCEL_BUDGET_S, FILE_CHUNK_MAX, TIMEOUT_EXIT, TRANSPORT_GRACE_S, log_trip, offset_dir,
};

/// The wait for a reply when the verb has no deadline of its own: the
/// Python's 660 s socket timeout, which covers daemon-side exec time.
pub const CALL_DEADLINE_S: u64 = 660;
/// The `run` submit deadline (`_set_deadline(30)` in the Python).
pub const SUBMIT_DEADLINE_S: u64 = 30;

#[derive(Debug)]
pub enum CallFailure {
    /// No reply before the local deadline.
    Deadline,
    /// Every leg failed, or a non-idempotent request failed mid-flight.
    Transport(CallError),
    /// The daemon answered with an error status.
    Http { status: u16, message: String },
}

impl std::fmt::Display for CallFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CallFailure::Deadline => write!(f, "no reply before the deadline"),
            CallFailure::Transport(e) => write!(f, "transport error: {e}"),
            CallFailure::Http { status, message } => write!(f, "HTTP {status}: {message}"),
        }
    }
}

pub struct Reply {
    pub body: Value,
    pub leg: &'static str,
    pub fallbacks: Vec<Failure>,
}

fn dispatch(
    ladder: &Arc<Ladder>,
    req: Request,
    deadline: Option<Duration>,
) -> Result<Answer, CallFailure> {
    let (tx, rx) = mpsc::channel();
    let shared = Arc::clone(ladder);
    thread::spawn(move || {
        let _ = tx.send(shared.call(&req));
    });
    let got = match deadline {
        Some(d) => rx.recv_timeout(d).map_err(|_| CallFailure::Deadline)?,
        None => rx.recv().map_err(|_| CallFailure::Deadline)?,
    };
    got.map_err(CallFailure::Transport)
}

/// One request, logged as one trip. The trip id rides in the header and,
/// for `POST /exec` and `POST /jobs`, in the body (the session proxy
/// forwards bodies verbatim, so the body is the carrier that reaches the
/// daemon).
fn send(
    ladder: &Arc<Ladder>,
    trip: &str,
    method: &'static str,
    path: &str,
    body: Option<Value>,
    idempotent: bool,
    deadline: Duration,
) -> Result<Answer, CallFailure> {
    send_ex(
        ladder, trip, method, path, body, idempotent, deadline, false,
    )
}

#[allow(clippy::too_many_arguments)]
fn send_ex(
    ladder: &Arc<Ladder>,
    trip: &str,
    method: &'static str,
    path: &str,
    body: Option<Value>,
    idempotent: bool,
    deadline: Duration,
    direct_only: bool,
) -> Result<Answer, CallFailure> {
    let body = match body {
        Some(Value::Object(mut map))
            if method == "POST" && (path == "/exec" || path == "/jobs") =>
        {
            map.insert("trip".to_string(), json!(trip));
            Some(Value::Object(map))
        }
        other => other,
    };
    let req = Request {
        method,
        path: path.to_string(),
        body,
        trip: trip.to_string(),
        idempotent,
        direct_only,
    };
    let started = Instant::now();
    let t_sent = crate::now_ms();
    let result = dispatch(ladder, req, Some(deadline));
    let status = result.as_ref().ok().map(|a| a.reply.status);
    let transport = result.as_ref().ok().map(|a| crate::transport_name(a.leg));
    let t_received = result.as_ref().ok().and_then(|a| {
        serde_json::from_slice::<Value>(&a.reply.body)
            .ok()
            .and_then(|v| v.get("_t_received").and_then(Value::as_u64))
    });
    log_trip(
        trip,
        path,
        started.elapsed().as_millis(),
        status,
        transport,
        Some(t_sent),
        Some(crate::now_ms()),
        t_received,
    );
    result
}

pub fn call(
    ladder: &Arc<Ladder>,
    trip: &str,
    method: &'static str,
    path: &str,
    body: Option<Value>,
    idempotent: bool,
    deadline: Duration,
) -> Result<Reply, CallFailure> {
    let answer = send(ladder, trip, method, path, body, idempotent, deadline)?;
    let mut value: Value = if answer.reply.body.is_empty() {
        json!({})
    } else {
        serde_json::from_slice(&answer.reply.body).map_err(|e| CallFailure::Http {
            status: answer.reply.status,
            message: format!("unparseable reply: {e}"),
        })?
    };
    // Trip-log v2 internal echo (code#5702): never surface it to
    // callers; send_ex already recorded it in the trip line.
    if let Some(obj) = value.as_object_mut() {
        obj.remove("_t_received");
    }
    if answer.reply.status >= 400 {
        let message = match value.get("error") {
            Some(Value::String(s)) => s.clone(),
            Some(other) => other.to_string(),
            None => value.to_string(),
        };
        return Err(CallFailure::Http {
            status: answer.reply.status,
            message,
        });
    }
    Ok(Reply {
        body: value,
        leg: answer.leg,
        fallbacks: answer.fallbacks,
    })
}

/// Best-effort cancel of an in-flight `/exec` by trip (the Python's
/// `_cancel_in_flight`): a tight budget, and not logged as a trip.
pub fn cancel(ladder: &Arc<Ladder>, trip: &str) -> bool {
    let req = Request {
        method: "POST",
        path: format!("/exec/{trip}/cancel"),
        body: Some(json!({ "trip": trip })),
        trip: trip.to_string(),
        idempotent: true,
        direct_only: false,
    };
    dispatch(ladder, req, Some(Duration::from_secs(CANCEL_BUDGET_S)))
        .map(|a| a.reply.status == 200)
        .unwrap_or(false)
}

/// The exit code and stderr line for a failed call.
fn fail(err: CallFailure) -> i32 {
    match err {
        CallFailure::Deadline => {
            eprintln!("laptop-agent: command timed out");
            TIMEOUT_EXIT
        }
        other => {
            eprintln!("laptop-agent: {other}");
            1
        }
    }
}

/// Say which leg answered when earlier legs failed. The Python prints
/// nothing here, so this is the one line the ladder adds.
fn note_fallbacks(reply: &Reply) {
    if reply.fallbacks.is_empty() {
        return;
    }
    let tried: Vec<&str> = reply.fallbacks.iter().map(|f| f.leg).collect();
    eprintln!(
        "laptop-agent: note: answered by {} after {} failed",
        reply.leg,
        tried.join(", ")
    );
}

fn default_deadline() -> Duration {
    Duration::from_secs(CALL_DEADLINE_S)
}

pub fn health(ladder: &Arc<Ladder>) -> i32 {
    let trip = crate::new_trip();
    match call(
        ladder,
        &trip,
        "GET",
        "/health",
        None,
        true,
        default_deadline(),
    ) {
        Ok(reply) => {
            note_fallbacks(&reply);
            println!("{}", reply.body);
            0
        }
        Err(e) => fail(e),
    }
}

pub fn jobs(ladder: &Arc<Ladder>) -> i32 {
    let trip = crate::new_trip();
    match call(
        ladder,
        &trip,
        "GET",
        "/jobs",
        None,
        true,
        default_deadline(),
    ) {
        Ok(reply) => {
            note_fallbacks(&reply);
            println!(
                "{}",
                serde_json::to_string_pretty(&reply.body).unwrap_or_default()
            );
            0
        }
        Err(e) => fail(e),
    }
}

pub fn kill(ladder: &Arc<Ladder>, job: &str) -> i32 {
    let trip = crate::new_trip();
    let path = format!("/jobs/{job}/kill");
    match call(
        ladder,
        &trip,
        "POST",
        &path,
        Some(json!({})),
        true,
        default_deadline(),
    ) {
        Ok(reply) => {
            note_fallbacks(&reply);
            println!("{}", reply.body);
            0
        }
        Err(e) => fail(e),
    }
}

/// What a command runs: an argv, or a shell string (`--shell`).
pub fn command_spec(cwd: Option<&str>, shell: bool, cmd: &[String]) -> Value {
    if shell {
        json!({ "cwd": cwd, "shell": cmd.join(" ") })
    } else {
        json!({ "cwd": cwd, "argv": cmd })
    }
}

pub fn exec(ladder: &Arc<Ladder>, spec: Value, timeout_s: u64) -> i32 {
    let trip = crate::new_trip();
    let deadline = Duration::from_secs(timeout_s + TRANSPORT_GRACE_S);
    let body = json!({ "cmds": [spec], "timeout_s": timeout_s });
    match call(ladder, &trip, "POST", "/exec", Some(body), false, deadline) {
        Ok(reply) => {
            note_fallbacks(&reply);
            print_exec(&reply.body)
        }
        Err(CallFailure::Deadline) => {
            cancel(ladder, &trip);
            fail(CallFailure::Deadline)
        }
        Err(e) => fail(e),
    }
}

fn print_exec(body: &Value) -> i32 {
    let Some(r) = body.get("results").and_then(|v| v.get(0)) else {
        eprintln!("laptop-agent: reply has no result");
        return 1;
    };
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    let _ = out.write_all(
        r.get("stdout")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes(),
    );
    let _ = err.write_all(
        r.get("stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .as_bytes(),
    );
    if r.get("truncated").and_then(Value::as_bool).unwrap_or(false) {
        eprintln!(
            "laptop-agent: warning: output truncated at the 1 MiB/stream capture cap (tail kept)"
        );
    }
    if let Some(e) = r.get("error").filter(|v| !v.is_null()) {
        eprintln!(
            "laptop-agent: {}",
            e.as_str()
                .map(str::to_string)
                .unwrap_or_else(|| e.to_string())
        );
    }
    if r.get("timeout").and_then(Value::as_bool).unwrap_or(false) {
        eprintln!("laptop-agent: command timed out");
        return TIMEOUT_EXIT;
    }
    r.get("exit")
        .and_then(Value::as_i64)
        .map(|v| v as i32)
        .unwrap_or(1)
}

pub fn run(ladder: &Arc<Ladder>, spec: Value) -> i32 {
    let trip = crate::new_trip();
    let deadline = Duration::from_secs(SUBMIT_DEADLINE_S + TRANSPORT_GRACE_S);
    match call(ladder, &trip, "POST", "/jobs", Some(spec), false, deadline) {
        Ok(reply) => {
            note_fallbacks(&reply);
            match reply.body.get("job_id").and_then(Value::as_str) {
                Some(id) => {
                    println!("{id}");
                    0
                }
                None => {
                    eprintln!("laptop-agent: reply has no job_id");
                    1
                }
            }
        }
        Err(e) => fail(e),
    }
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
struct Offsets {
    out: u64,
    err: u64,
}

fn offsets_file(job: &str) -> std::path::PathBuf {
    offset_dir().join(format!("{job}.json"))
}

fn load_offsets(job: &str) -> Offsets {
    std::fs::read_to_string(offsets_file(job))
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default()
}

fn save_offsets(job: &str, offs: &Offsets) {
    let _ = std::fs::create_dir_all(offset_dir());
    let _ = std::fs::write(
        offsets_file(job),
        serde_json::to_string(offs).unwrap_or_default(),
    );
}

/// Python's rendering of an optional value in the done line (`None`).
fn py_opt(v: &Value) -> String {
    match v {
        Value::Null => "None".to_string(),
        other => other.to_string(),
    }
}

pub fn poll(ladder: &Arc<Ladder>, job: &str, wait_s: u64) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(wait_s);
    let mut offs = load_offsets(job);
    loop {
        let trip = crate::new_trip();
        let path = format!("/jobs/{job}?out_off={}&err_off={}", offs.out, offs.err);
        let reply = match call(ladder, &trip, "GET", &path, None, true, default_deadline()) {
            Ok(reply) => reply,
            Err(e) => return fail(e),
        };
        let body = &reply.body;
        let mut out = std::io::stdout();
        let mut err = std::io::stderr();
        let _ = out.write_all(
            body.get("stdout")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes(),
        );
        let _ = out.flush();
        let _ = err.write_all(
            body.get("stderr")
                .and_then(Value::as_str)
                .unwrap_or("")
                .as_bytes(),
        );
        let _ = err.flush();
        let (Some(out_next), Some(err_next)) = (
            body.get("out_next").and_then(Value::as_u64),
            body.get("err_next").and_then(Value::as_u64),
        ) else {
            eprintln!("laptop-agent: reply has no out_next/err_next");
            return 1;
        };
        offs = Offsets {
            out: out_next,
            err: err_next,
        };
        save_offsets(job, &offs);
        if body.get("state").and_then(Value::as_str) == Some("done") {
            let exit = body.get("exit").cloned().unwrap_or(Value::Null);
            let dur = body.get("duration_ms").cloned().unwrap_or(Value::Null);
            eprintln!(
                "[laptop-agent] job {job} done exit={} duration_ms={}",
                py_opt(&exit),
                py_opt(&dur)
            );
            return exit.as_i64().map(|v| v as i32).unwrap_or(1);
        }
        let now = Instant::now();
        if now >= deadline {
            eprintln!("[laptop-agent] job {job} still running");
            return 0;
        }
        thread::sleep((deadline - now).min(Duration::from_secs(3)));
    }
}

/// The sha256 of the remote file, computed on the laptop by `shasum`
/// (present on macOS, where the daemon runs) through `/exec`.
fn remote_sha256(ladder: &Arc<Ladder>, remote: &str) -> Result<String, String> {
    let cmd = vec![
        "shasum".to_string(),
        "-a".to_string(),
        "256".to_string(),
        remote.to_string(),
    ];
    let spec = command_spec(None, false, &cmd);
    let trip = crate::new_trip();
    let body = json!({ "cmds": [spec], "timeout_s": 60 });
    let reply = call(
        ladder,
        &trip,
        "POST",
        "/exec",
        Some(body),
        false,
        Duration::from_secs(60 + TRANSPORT_GRACE_S),
    )
    .map_err(|e| e.to_string())?;
    let r = reply
        .body
        .get("results")
        .and_then(|v| v.get(0))
        .ok_or("reply has no result")?;
    if r.get("exit").and_then(Value::as_i64) != Some(0) {
        let err = r
            .get("stderr")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        return Err(format!("cannot hash {remote}: {err}"));
    }
    let out = r.get("stdout").and_then(Value::as_str).unwrap_or("");
    out.split_whitespace()
        .next()
        .filter(|h| h.len() == 64 && h.chars().all(|c| c.is_ascii_hexdigit()))
        .map(|h| h.to_ascii_lowercase())
        .ok_or_else(|| format!("unexpected shasum output for {remote}"))
}

/// One `GET /file` range. Raw bytes, so the reply is not parsed as JSON.
fn fetch_chunk(ladder: &Arc<Ladder>, remote: &str, off: u64) -> Result<Vec<u8>, CallFailure> {
    let trip = crate::new_trip();
    let path = format!(
        "/file?path={}&off={off}&len={FILE_CHUNK_MAX}",
        percent_encode(remote)
    );
    let answer = send_ex(
        ladder,
        &trip,
        "GET",
        &path,
        None,
        true,
        default_deadline(),
        true,
    )?;
    if answer.reply.status >= 400 {
        let message = String::from_utf8_lossy(&answer.reply.body).into_owned();
        return Err(CallFailure::Http {
            status: answer.reply.status,
            message,
        });
    }
    Ok(answer.reply.body)
}

fn percent_encode(s: &str) -> String {
    s.bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (b as char).to_string()
            }
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// Copy a remote file to `dest`, hash-checked: the bytes are written only
/// when their sha256 matches the remote's own.
pub fn fetch(ladder: &Arc<Ladder>, remote: &str, dest: &std::path::Path) -> i32 {
    let want = match remote_sha256(ladder, remote) {
        Ok(h) => h,
        Err(message) => {
            eprintln!("laptop-agent: {message}");
            return 1;
        }
    };
    let mut data = Vec::new();
    loop {
        match fetch_chunk(ladder, remote, data.len() as u64) {
            Ok(chunk) if chunk.is_empty() => break,
            Ok(chunk) => data.extend_from_slice(&chunk),
            Err(e) => return fail(e),
        }
    }
    let got = format!("{:x}", Sha256::digest(&data));
    if got != want {
        eprintln!(
            "laptop-agent: sha256 mismatch for {remote}: remote {want}, received {got}; nothing written"
        );
        return 1;
    }
    if let Err(e) = std::fs::write(dest, &data) {
        eprintln!("laptop-agent: cannot write {}: {e}", dest.display());
        return 1;
    }
    eprintln!(
        "[laptop-agent] fetched {remote} -> {} ({} bytes, sha256 {got})",
        dest.display(),
        data.len()
    );
    0
}
