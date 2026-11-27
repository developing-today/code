//! The verbs over the laptop-agent daemon, mirroring `laptop-agent.py`:
//! the same requests, the same output and the same exit codes.
//!
//! The local deadline is a watchdog: the ladder runs on a helper thread
//! and the caller waits for it with a timeout. A leg that hangs past the
//! deadline is abandoned (the process exits soon after), and the
//! deadline path asks the daemon to cancel a running `/exec` by trip.

use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
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

/// The line a call served by the SSH leg prints on stderr
/// (developing-today/code#15419, design #10356 section 2): the leg
/// ladder can fall from the daemon's HTTP API to SSH, and under the
/// #13717 route-form matrix that is tier T3, so it says so in the same
/// words the fleet route forms announce themselves with.
pub(crate) const SSH_FORM_LINE: &str = "fleet: form T3 ssh/fleet";

/// Whether this process has already printed [`SSH_FORM_LINE`]; a poll
/// loop over the SSH leg says it once, not once per page.
static SSH_FORM_ANNOUNCED: AtomicBool = AtomicBool::new(false);

/// Print [`SSH_FORM_LINE`] to `w` the first time a call is answered
/// by the SSH leg. Calls answered by an HTTP leg print nothing.
pub(crate) fn note_form(
    result: &Result<Answer, CallFailure>,
    announced: &AtomicBool,
    w: &mut dyn Write,
) {
    if let Ok(answer) = result
        && answer.leg == "ssh"
        && !announced.swap(true, Ordering::SeqCst)
    {
        let _ = writeln!(w, "{SSH_FORM_LINE}");
    }
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
    note_form(&result, &SSH_FORM_ANNOUNCED, &mut std::io::stderr());
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

/// One row of `fleet http sessions` (developing-today/code#15420): a
/// daemon job seen as a connect session. The owning context id is
/// the job's recorded `FLEET_CTX_ID`, shown only when the caller
/// exported one and never guessed. `bytes_dropped` is what a spool
/// cap discarded (0 until the daemon reports it).
pub fn session_row(job: &Value) -> Value {
    let ctx_id = job
        .get("ctx")
        .and_then(|c| c.get("FLEET_CTX_ID"))
        .and_then(Value::as_str)
        .map(str::to_string);
    let bytes = |k: &str| job.get(k).and_then(Value::as_u64).unwrap_or(0);
    json!({
        "id": job.get("job_id").cloned().unwrap_or(Value::Null),
        "origin": "daemon",
        "ctx_id": ctx_id,
        "cmd": job.get("cmd").cloned().unwrap_or(Value::Null),
        "started_at": job.get("started_at").cloned().unwrap_or(Value::Null),
        "state": job.get("state").cloned().unwrap_or(Value::Null),
        "exit": job.get("exit").cloned().unwrap_or(Value::Null),
        "out_bytes": bytes("out_bytes"),
        "err_bytes": bytes("err_bytes"),
        "bytes_dropped": bytes("bytes_dropped"),
    })
}

/// Render session rows as an aligned table, one session per line.
pub fn render_sessions(rows: &[Value]) -> String {
    let mut lines = vec![[
        "ID".to_string(),
        "ORIGIN".to_string(),
        "STATE".to_string(),
        "STARTED".to_string(),
        "OUT".to_string(),
        "ERR".to_string(),
        "DROPPED".to_string(),
        "CTX".to_string(),
        "CMD".to_string(),
    ]];
    for r in rows {
        let text = |k: &str| match r.get(k) {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Null) | None => "-".to_string(),
            Some(other) => other.to_string(),
        };
        let started = r
            .get("started_at")
            .and_then(Value::as_f64)
            .map(|t| crate::iso_utc(t as i64))
            .unwrap_or_else(|| "-".to_string());
        lines.push([
            text("id"),
            text("origin"),
            text("state"),
            started,
            text("out_bytes"),
            text("err_bytes"),
            text("bytes_dropped"),
            text("ctx_id"),
            text("cmd"),
        ]);
    }
    let mut widths = [0usize; 9];
    for l in &lines {
        for (w, cell) in widths.iter_mut().zip(l.iter()) {
            *w = (*w).max(cell.chars().count());
        }
    }
    let mut out = String::new();
    for l in &lines {
        let mut row = String::new();
        for (i, cell) in l.iter().enumerate() {
            if i + 1 == l.len() {
                row.push_str(cell);
            } else {
                row.push_str(&format!("{cell:<w$}  ", w = widths[i]));
            }
        }
        out.push_str(row.trim_end());
        out.push('\n');
    }
    out
}

/// `sessions`: the daemon's jobs as connect sessions (#15420).
pub fn sessions(ladder: &Arc<Ladder>, as_json: bool) -> i32 {
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
            let rows: Vec<Value> = reply
                .body
                .get("jobs")
                .and_then(Value::as_array)
                .map(|jobs| jobs.iter().map(session_row).collect())
                .unwrap_or_default();
            if as_json {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&json!({ "sessions": rows })).unwrap_or_default()
                );
            } else {
                print!("{}", render_sessions(&rows));
            }
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

/// Read an `exec-batch` file: the POST body plus the client-side
/// deadline. Mirrors laptop-agent-legacy.py: the body is posted
/// exactly as read, `cmds` defaults to [] and `timeout_s` defaults
/// to 120. Anything else malformed is refused before any transport.
pub(crate) fn read_batch_file(path: &std::path::Path) -> Result<(Value, u64), String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read batch file {}: {e}", path.display()))?;
    let body: Value = serde_json::from_str(&text)
        .map_err(|e| format!("invalid JSON in batch file {}: {e}", path.display()))?;
    if !body.is_object() {
        return Err(format!(
            "batch file {} must contain a JSON object",
            path.display()
        ));
    }
    let timeout_s = match body.get("timeout_s") {
        Some(v) => v.as_u64().ok_or_else(|| {
            format!(
                "batch file {}: timeout_s must be a whole number",
                path.display()
            )
        })?,
        None => 120,
    };
    if let Some(cmds) = body.get("cmds")
        && !cmds.is_array()
    {
        return Err(format!(
            "batch file {}: cmds must be an array",
            path.display()
        ));
    }
    Ok((body, timeout_s))
}

/// Python truthiness for a JSON field, matching the legacy
/// `if r.get(...)` checks: None/missing, false, 0, "", [], and {}
/// are falsy; everything else is truthy.
pub(crate) fn json_truthy(value: Option<&Value>) -> bool {
    match value {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n
            .as_i64()
            .map(|i| i != 0)
            .unwrap_or_else(|| n.as_f64().map(|f| f != 0.0).unwrap_or(false)),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(m)) => !m.is_empty(),
    }
}

/// The worst per-command exit code. Mirrors the legacy `_batch`
/// accumulator: a result with a (truthy) error counts 1, a (truthy)
/// timeout counts TIMEOUT_EXIT, otherwise the command's own exit; a
/// reply with no results exits 0 after printing.
pub(crate) fn batch_exit_code(reply_body: &Value) -> i32 {
    let Some(results) = reply_body.get("results").and_then(Value::as_array) else {
        return 0;
    };
    let mut worst = 0;
    for r in results {
        if json_truthy(r.get("error")) {
            worst = worst.max(1);
        } else if json_truthy(r.get("timeout")) {
            worst = worst.max(TIMEOUT_EXIT);
        } else if let Some(exit) = r.get("exit").and_then(Value::as_i64) {
            worst = worst.max(exit as i32);
        }
    }
    worst
}

pub(crate) fn batch_truncated(reply_body: &Value) -> bool {
    reply_body
        .get("results")
        .and_then(Value::as_array)
        .map(|results| results.iter().any(|r| json_truthy(r.get("truncated"))))
        .unwrap_or(false)
}

/// Render a reply the way the legacy prints it:
/// `print(json.dumps(out, indent=1))` — one space per level. Key
/// order follows the JSON map (sorted here, wire order there);
/// byte-identity holds for ASCII payloads with sorted keys.
pub(crate) fn render_batch_reply(value: &Value) -> String {
    use serde::Serialize;
    let mut buf = Vec::new();
    let formatter = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, formatter);
    value
        .serialize(&mut ser)
        .expect("in-memory JSON serialization");
    buf.push(b'\n');
    String::from_utf8(buf).expect("serializer emits UTF-8")
}

/// Words for the primary-checkout guard: the argv, or the shell
/// string split on whitespace — the same split `exec --shell` uses.
fn batch_words(cmd: &Value) -> Vec<String> {
    if let Some(shell) = cmd.get("shell").and_then(Value::as_str) {
        return shell.split_whitespace().map(str::to_string).collect();
    }
    cmd.get("argv")
        .and_then(Value::as_array)
        .map(|argv| {
            argv.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// `exec-batch FILE.json`: guard every command, POST the file body
/// unchanged, print the reply, exit with the worst per-command code.
pub fn exec_batch(ladder: &Arc<Ladder>, path: &std::path::Path) -> i32 {
    let (body, timeout_s) = match read_batch_file(path) {
        Ok(parsed) => parsed,
        Err(message) => {
            eprintln!("laptop-agent: {message}");
            return 1;
        }
    };
    if let Some(cmds) = body.get("cmds").and_then(Value::as_array) {
        for cmd in cmds {
            let cwd = cmd.get("cwd").and_then(Value::as_str);
            if let Err(message) = crate::guard::check_job(cwd, &batch_words(cmd)) {
                eprintln!("laptop-agent: {message}");
                return 1;
            }
        }
    }
    let trip = crate::new_trip();
    let deadline = Duration::from_secs(timeout_s + TRANSPORT_GRACE_S);
    match call(ladder, &trip, "POST", "/exec", Some(body), false, deadline) {
        Ok(reply) => {
            note_fallbacks(&reply);
            print!("{}", render_batch_reply(&reply.body));
            // One warning per truncated result, like the legacy loop.
            if let Some(results) = reply.body.get("results").and_then(Value::as_array) {
                for r in results {
                    if json_truthy(r.get("truncated")) {
                        eprintln!(
                            "laptop-agent: warning: a command's output was truncated at the 1 MiB/stream capture cap (tail kept)"
                        );
                    }
                }
            }
            batch_exit_code(&reply.body)
        }
        Err(CallFailure::Deadline) => {
            cancel(ladder, &trip);
            fail(CallFailure::Deadline)
        }
        Err(e) => fail(e),
    }
}

/// What a command runs: an argv, or a shell string (`--shell`).
/// The caller's `FLEET_CTX_*` record rides the spec's `env`
/// (code#14188): the daemon merges it into the job's environment and
/// records it in the spool, and the SSH fallback leg exports it on
/// the remote line, so the context reaches the command on every leg.
pub fn command_spec(cwd: Option<&str>, shell: bool, cmd: &[String]) -> Value {
    command_spec_with_ctx(&fleet_config::ctx::Ctx::from_env(), cwd, shell, cmd)
}

/// [`command_spec`] with an explicit context record. An empty record
/// adds no `env` key, so pre-contract specs are byte-identical.
pub fn command_spec_with_ctx(
    ctx: &fleet_config::ctx::Ctx,
    cwd: Option<&str>,
    shell: bool,
    cmd: &[String],
) -> Value {
    let mut spec = if shell {
        json!({ "cwd": cwd, "shell": cmd.join(" ") })
    } else {
        json!({ "cwd": cwd, "argv": cmd })
    };
    if let (Some(obj), Some(env)) = (spec.as_object_mut(), ctx.to_frame()) {
        obj.insert("env".to_string(), json!(env));
    }
    spec
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

#[derive(serde::Serialize, serde::Deserialize, Default, Debug, Clone, PartialEq)]
pub(crate) struct Offsets {
    pub(crate) out: u64,
    pub(crate) err: u64,
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

/// Seconds between `poll` status requests after `polls` completed polls
/// (#5757: exponential 1 s doubling, capped at 10 s, so long jobs cost
/// ~5x fewer round trips at a worst-case detection latency of 10 s).
pub fn poll_interval(polls: u32) -> Duration {
    Duration::from_secs(1u64.saturating_mul(1 << polls.min(4)).min(10))
}

/// How one drain of a job's spool ended.
#[derive(Debug)]
pub(crate) enum Drained {
    /// The job is done and every spooled byte was written out; the
    /// final reply body (for the exit code and duration).
    Done(Value),
    /// The wait ran out while the job was still running.
    StillRunning,
    /// A request failed.
    Failed(CallFailure),
    /// A reply had no `out_next`/`err_next`.
    BadReply,
}

/// True once a `done` job's spool has been read to the end
/// (developing-today/code#15420): either the daemon reported the
/// spool sizes (`out_size`/`err_size`) and the cursors reached them,
/// or this reply carried no bytes on either stream. A done job used
/// to stop at its first reply, which carries at most 1 MiB per
/// stream, so a larger spool was cut short.
fn spool_drained(
    body: &Value,
    out_next: u64,
    err_next: u64,
    got_out: usize,
    got_err: usize,
) -> bool {
    if got_out == 0 && got_err == 0 {
        return true;
    }
    match (
        body.get("out_size").and_then(Value::as_u64),
        body.get("err_size").and_then(Value::as_u64),
    ) {
        (Some(out_size), Some(err_size)) => out_next >= out_size && err_next >= err_size,
        _ => false,
    }
}

/// Page a job's spool from `offs` until the job is done and the spool
/// is drained, or until `deadline` while it is still running. `fetch`
/// asks the daemon for one page at the given offsets; `save` records
/// the cursors after each page; `sleep` waits between polls of a
/// running job. A finished job is paged without sleeping.
pub(crate) fn drain<F, S, W>(
    mut fetch: F,
    offs: &mut Offsets,
    deadline: Instant,
    out: &mut dyn Write,
    err: &mut dyn Write,
    mut save: S,
    mut sleep: W,
) -> Drained
where
    F: FnMut(&Offsets) -> Result<Value, CallFailure>,
    S: FnMut(&Offsets),
    W: FnMut(Duration),
{
    let mut polls: u32 = 0;
    loop {
        let body = match fetch(offs) {
            Ok(body) => body,
            Err(e) => return Drained::Failed(e),
        };
        let chunk_out = body.get("stdout").and_then(Value::as_str).unwrap_or("");
        let chunk_err = body.get("stderr").and_then(Value::as_str).unwrap_or("");
        let _ = out.write_all(chunk_out.as_bytes());
        let _ = out.flush();
        let _ = err.write_all(chunk_err.as_bytes());
        let _ = err.flush();
        let (Some(out_next), Some(err_next)) = (
            body.get("out_next").and_then(Value::as_u64),
            body.get("err_next").and_then(Value::as_u64),
        ) else {
            return Drained::BadReply;
        };
        // The cursors are byte offsets into the spool; a reply that
        // did not move them carried nothing new.
        let got_out = out_next.saturating_sub(offs.out) as usize;
        let got_err = err_next.saturating_sub(offs.err) as usize;
        *offs = Offsets {
            out: out_next,
            err: err_next,
        };
        save(offs);
        if body.get("state").and_then(Value::as_str) == Some("done") {
            if spool_drained(&body, out_next, err_next, got_out, got_err) {
                return Drained::Done(body);
            }
            continue;
        }
        let now = Instant::now();
        if now >= deadline {
            return Drained::StillRunning;
        }
        sleep((deadline - now).min(poll_interval(polls)));
        polls = polls.saturating_add(1);
    }
}

/// The explicit marker `attach` prints when the daemon's spool cap
/// discarded output (developing-today/code#15420, design #10356
/// section 3.3): output is never silently truncated. `None` when the
/// reply reports no dropped bytes, including a daemon from before the
/// cap that sends no count at all.
pub(crate) fn spool_cap_marker(body: &Value) -> Option<String> {
    let dropped = body.get("bytes_dropped").and_then(Value::as_u64)?;
    (dropped > 0).then(|| format!("[spool cap reached: {dropped} bytes dropped]"))
}

pub fn poll(ladder: &Arc<Ladder>, job: &str, wait_s: u64) -> i32 {
    let deadline = Instant::now() + Duration::from_secs(wait_s);
    let mut offs = load_offsets(job);
    let fetch = |o: &Offsets| {
        let trip = crate::new_trip();
        let wait = deadline
            .saturating_duration_since(Instant::now())
            .as_secs_f64()
            .ceil() as u64;
        let wait = wait.min(10);
        let path = format!(
            "/jobs/{job}?out_off={}&err_off={}&wait_s={wait}",
            o.out, o.err
        );
        call(ladder, &trip, "GET", &path, None, true, default_deadline()).map(|r| r.body)
    };
    let mut out = std::io::stdout();
    let mut err = std::io::stderr();
    match drain(
        fetch,
        &mut offs,
        deadline,
        &mut out,
        &mut err,
        |o| save_offsets(job, o),
        thread::sleep,
    ) {
        Drained::Done(body) => {
            if let Some(marker) = spool_cap_marker(&body) {
                eprintln!("{marker}");
            }
            let exit = body.get("exit").cloned().unwrap_or(Value::Null);
            let dur = body.get("duration_ms").cloned().unwrap_or(Value::Null);
            eprintln!(
                "[laptop-agent] job {job} done exit={} duration_ms={}",
                py_opt(&exit),
                py_opt(&dur)
            );
            exit.as_i64().map(|v| v as i32).unwrap_or(1)
        }
        Drained::StillRunning => {
            eprintln!("[laptop-agent] job {job} still running");
            0
        }
        Drained::Failed(e) => fail(e),
        Drained::BadReply => {
            eprintln!("laptop-agent: reply has no out_next/err_next");
            1
        }
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

/// One `GET /file` range of `len` bytes. Raw bytes, so the reply is
/// not parsed as JSON.
fn fetch_chunk(
    ladder: &Arc<Ladder>,
    remote: &str,
    off: u64,
    len: u64,
) -> Result<Vec<u8>, CallFailure> {
    let trip = crate::new_trip();
    let path = format!("/file?path={}&off={off}&len={len}", percent_encode(remote));
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

/// The byte size of the remote file, via `stat -c%s` through `/exec`
/// (the same GNU `stat` the SSH fallback leg uses to size a range).
fn remote_size(ladder: &Arc<Ladder>, remote: &str) -> Result<u64, String> {
    let cmd = vec![
        "stat".to_string(),
        "-c%s".to_string(),
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
        return Err(format!("cannot stat {remote}: {err}"));
    }
    let out = r.get("stdout").and_then(Value::as_str).unwrap_or("");
    out.trim()
        .parse::<u64>()
        .map_err(|_| format!("unexpected stat output for {remote}"))
}

/// The legacy `fetch` overshoot refusal, byte-for-byte with
/// `laptop-agent-legacy.py` (`fetch PATH OUT`).
fn overshoot_message(local: &std::path::Path, off: u64, remote: &str, total: u64) -> String {
    format!(
        "laptop-agent: local {} is {off} bytes but remote {remote} is only {total}; refusing to append \u{2014} remove or rename the local file and retry",
        local.display()
    )
}

/// The legacy `fetch` receipt, byte-for-byte with `laptop-agent-legacy.py`:
/// `N round trip(s)`, with the resume offset when OUT already existed.
fn fetch_receipt(
    total: u64,
    remote: &str,
    dest: &std::path::Path,
    trips: u64,
    resumed_from: u64,
) -> String {
    format!(
        "fetched {total} bytes from {remote} -> {} in {trips} round trip(s){}",
        dest.display(),
        if resumed_from > 0 {
            format!(" (resumed from offset {resumed_from})")
        } else {
            String::new()
        }
    )
}

/// Copy a remote file to `dest`, hash-checked: the bytes are appended only
/// when the assembled file's sha256 matches the remote's own. `chunk` is
/// the GET /file range size (`--chunk`, default 1 MiB); it is clamped to
/// the daemon's 1 MiB per-request cap.
///
/// Parity with `laptop-agent-legacy.py` (`fetch PATH OUT`): an existing
/// OUT is resumed from its current size (the prefix is never re-fetched),
/// a local file larger than the remote is refused with the legacy
/// message, an empty range before EOF fails the fetch, and success
/// prints the legacy `N round trip(s)` receipt. The legacy client trusts
/// the transport; this transport additionally verifies the sha256 and
/// writes nothing new on mismatch.
///
/// The remote size comes from `stat` over `/exec` rather than the
/// `x-file-size` reply header the legacy reads off `GET /file`: the
/// ladder's reply carries status and body only, so the header is not
/// visible here. The refused-fetch file effect is identical either way
/// (no bytes appended); only the trip spent learning the size differs
/// (one `/exec` instead of one `GET /file`).
pub fn fetch(ladder: &Arc<Ladder>, remote: &str, dest: &std::path::Path, chunk: u64) -> i32 {
    let len = chunk.clamp(1, FILE_CHUNK_MAX);
    let want = match remote_sha256(ladder, remote) {
        Ok(h) => h,
        Err(message) => {
            eprintln!("laptop-agent: {message}");
            return 1;
        }
    };
    let total = match remote_size(ladder, remote) {
        Ok(n) => n,
        Err(message) => {
            eprintln!("laptop-agent: {message}");
            return 1;
        }
    };
    let resumed_from = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
    if resumed_from > total {
        eprintln!("{}", overshoot_message(dest, resumed_from, remote, total));
        return 1;
    }
    let mut hasher = Sha256::new();
    if resumed_from > 0 {
        let mut prefix = match std::fs::File::open(dest) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("laptop-agent: cannot read {}: {e}", dest.display());
                return 1;
            }
        };
        let mut buf = [0u8; 65536];
        loop {
            use std::io::Read as _;
            match prefix.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(e) => {
                    eprintln!("laptop-agent: cannot read {}: {e}", dest.display());
                    return 1;
                }
            }
        }
    }
    let mut data = Vec::new();
    let mut off = resumed_from;
    let mut trips: u64 = 0;
    loop {
        match fetch_chunk(ladder, remote, off, len) {
            Ok(piece) => {
                trips += 1;
                if piece.is_empty() {
                    if off < total {
                        eprintln!("laptop-agent: empty chunk before EOF; fetch incomplete");
                        return 1;
                    }
                    break;
                }
                off += piece.len() as u64;
                data.extend_from_slice(&piece);
                if off >= total {
                    break;
                }
            }
            Err(e) => return fail(e),
        }
    }
    hasher.update(&data);
    let got: String = hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();
    if got != want {
        eprintln!(
            "laptop-agent: sha256 mismatch for {remote}: remote {want}, received {got}; nothing written"
        );
        return 1;
    }
    if !data.is_empty() {
        let mut fh = match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(dest)
        {
            Ok(f) => f,
            Err(e) => {
                eprintln!("laptop-agent: cannot write {}: {e}", dest.display());
                return 1;
            }
        };
        if let Err(e) = fh.write_all(&data) {
            eprintln!("laptop-agent: cannot write {}: {e}", dest.display());
            return 1;
        }
    } else if resumed_from == 0 {
        // An empty remote still creates OUT, like the legacy `open(OUT, "wb")`.
        if let Err(e) = std::fs::write(dest, b"") {
            eprintln!("laptop-agent: cannot write {}: {e}", dest.display());
            return 1;
        }
    }
    eprintln!(
        "{}",
        fetch_receipt(total, remote, dest, trips, resumed_from)
    );
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ladder::{Leg, LegError, Reply as LegReply};

    /// A leg that always gives the same answer.
    struct FixedLeg {
        name: &'static str,
        ok: bool,
    }

    impl Leg for FixedLeg {
        fn name(&self) -> &'static str {
            self.name
        }
        fn call(&self, _req: &Request) -> Result<LegReply, LegError> {
            if self.ok {
                Ok(LegReply {
                    status: 200,
                    body: b"{\"ok\": true}".to_vec(),
                })
            } else {
                Err(LegError::Unsent("connect refused".into()))
            }
        }
    }

    fn ladder_of(legs: &[(&'static str, bool)]) -> Arc<Ladder> {
        Arc::new(Ladder::new(
            legs.iter()
                .map(|&(name, ok)| Box::new(FixedLeg { name, ok }) as Box<dyn Leg>)
                .collect(),
        ))
    }

    fn health_req() -> Request {
        Request {
            method: "GET",
            path: "/health".into(),
            body: None,
            trip: "t".into(),
            idempotent: true,
            direct_only: false,
        }
    }

    #[test]
    fn ssh_leg_serving_a_call_announces_form_t3_ssh_15419() {
        let ladder = ladder_of(&[
            ("broker", false),
            ("cloudflare", false),
            ("tailscale", false),
            ("ssh", true),
        ]);
        let result = dispatch(&ladder, health_req(), Some(Duration::from_secs(5)));
        assert_eq!(result.as_ref().map(|a| a.leg).ok(), Some("ssh"));
        let announced = AtomicBool::new(false);
        let mut err = Vec::new();
        note_form(&result, &announced, &mut err);
        let text = String::from_utf8(err).unwrap();
        assert!(text.contains("form T3 ssh"), "{text}");
        assert_eq!(text, format!("{SSH_FORM_LINE}\n"));
        // A second call in the same process (a poll loop) says nothing more.
        let mut again = Vec::new();
        note_form(&result, &announced, &mut again);
        assert!(again.is_empty());
    }

    #[test]
    fn http_leg_serving_a_call_prints_no_form_line_15419() {
        for served_by in ["broker", "cloudflare", "tailscale"] {
            let legs: Vec<(&'static str, bool)> = ["broker", "cloudflare", "tailscale", "ssh"]
                .into_iter()
                .map(|n| (n, n == served_by))
                .collect();
            let result = dispatch(
                &ladder_of(&legs),
                health_req(),
                Some(Duration::from_secs(5)),
            );
            assert_eq!(result.as_ref().map(|a| a.leg).ok(), Some(served_by));
            let mut err = Vec::new();
            note_form(&result, &AtomicBool::new(false), &mut err);
            assert!(err.is_empty(), "{served_by}");
        }
        // A failed call prints no form line either.
        let failed = dispatch(
            &ladder_of(&[("broker", false), ("ssh", false)]),
            health_req(),
            Some(Duration::from_secs(5)),
        );
        assert!(failed.is_err());
        let mut err = Vec::new();
        note_form(&failed, &AtomicBool::new(false), &mut err);
        assert!(err.is_empty());
    }

    #[test]
    fn session_row_maps_a_daemon_job_15420() {
        let job = json!({
            "job_id": "j1", "state": "done", "exit": 0, "cwd": "/",
            "cmd": "make test", "started_at": 86400.5,
            "ctx": {"FLEET_CTX_ID": "ctx-9", "FLEET_CTX_TASK_ID": "7"},
            "out_bytes": 2_000_000, "err_bytes": 12,
        });
        let row = session_row(&job);
        assert_eq!(row["id"], "j1");
        assert_eq!(row["origin"], "daemon");
        assert_eq!(row["ctx_id"], "ctx-9");
        assert_eq!(row["out_bytes"], 2_000_000);
        assert_eq!(row["bytes_dropped"], 0);
        let table = render_sessions(&[row]);
        let lines: Vec<&str> = table.lines().collect();
        assert_eq!(lines.len(), 2);
        assert!(lines[0].starts_with("ID"));
        assert!(lines[1].contains("1970-01-02T00:00:00Z"), "{table}");
        assert!(lines[1].ends_with("make test"), "{table}");
    }

    #[test]
    fn spool_cap_marker_names_the_dropped_bytes_15420() {
        let body = json!({"state": "done", "bytes_dropped": 4096});
        assert_eq!(
            spool_cap_marker(&body).as_deref(),
            Some("[spool cap reached: 4096 bytes dropped]")
        );
        assert_eq!(spool_cap_marker(&json!({"bytes_dropped": 0})), None);
        // A daemon from before the cap sends no count: no marker.
        assert_eq!(spool_cap_marker(&json!({"state": "done"})), None);
    }

    #[test]
    fn session_row_never_guesses_a_context_id() {
        let row = session_row(&json!({"job_id": "j2", "state": "running", "ctx": {}}));
        assert!(row["ctx_id"].is_null());
        assert!(
            render_sessions(&[row])
                .lines()
                .nth(1)
                .unwrap()
                .contains(" - ")
        );
    }

    /// One stream of a fake daemon spool, served the way the daemon's
    /// `job_public` serves it: at most 1 MiB from the requested offset.
    const PAGE: usize = 1 << 20;

    fn page(spool: &[u8], off: u64) -> (String, u64) {
        let start = (off as usize).min(spool.len());
        let end = (start + PAGE).min(spool.len());
        (
            String::from_utf8(spool[start..end].to_vec()).unwrap(),
            end as u64,
        )
    }

    /// A fake `GET /jobs/<id>` over in-memory spools. `sizes` adds the
    /// `out_size`/`err_size` fields newer daemons send (#15420).
    fn fake_daemon<'a>(
        out: &'a [u8],
        err: &'a [u8],
        sizes: bool,
        calls: &'a std::cell::Cell<u32>,
    ) -> impl FnMut(&Offsets) -> Result<Value, CallFailure> + 'a {
        move |o: &Offsets| {
            calls.set(calls.get() + 1);
            let (stdout, out_next) = page(out, o.out);
            let (stderr, err_next) = page(err, o.err);
            let mut body = json!({
                "state": "done",
                "exit": 0,
                "duration_ms": 5,
                "stdout": stdout,
                "stderr": stderr,
                "out_next": out_next,
                "err_next": err_next,
            });
            if sizes {
                body["out_size"] = json!(out.len());
                body["err_size"] = json!(err.len());
            }
            Ok(body)
        }
    }

    fn spool(len: usize, seed: u8) -> Vec<u8> {
        (0..len)
            .map(|i| b'a' + ((i as u8).wrapping_add(seed) % 26))
            .collect()
    }

    /// #15420: attaching to a finished job with more than 1 MiB
    /// spooled returns every byte, not just the first page.
    #[test]
    fn attach_to_finished_job_over_1mib_returns_every_byte_15420() {
        let out_spool = spool(3 * PAGE + 123, 0);
        let err_spool = spool(PAGE + 7, 5);
        for sizes in [true, false] {
            let calls = std::cell::Cell::new(0);
            let mut offs = Offsets::default();
            let (mut out, mut err) = (Vec::new(), Vec::new());
            let got = drain(
                fake_daemon(&out_spool, &err_spool, sizes, &calls),
                &mut offs,
                Instant::now(),
                &mut out,
                &mut err,
                |_| {},
                |_| panic!("a finished job is drained without sleeping"),
            );
            assert!(matches!(got, Drained::Done(_)), "sizes={sizes}: {got:?}");
            assert_eq!(out.len(), out_spool.len(), "sizes={sizes}");
            assert!(out == out_spool, "stdout differs, sizes={sizes}");
            assert!(err == err_spool, "stderr differs, sizes={sizes}");
            assert_eq!(
                offs,
                Offsets {
                    out: out_spool.len() as u64,
                    err: err_spool.len() as u64
                }
            );
            // With sizes the drain stops on the page that reaches them;
            // without, one more (empty) page confirms the end.
            let pages = out_spool.len().div_ceil(PAGE) as u32;
            assert_eq!(calls.get(), if sizes { pages } else { pages + 1 });
        }
    }

    #[test]
    fn drain_resumes_from_saved_offsets_and_saves_each_page() {
        let out_spool = spool(2 * PAGE + 10, 1);
        let calls = std::cell::Cell::new(0);
        let mut offs = Offsets {
            out: PAGE as u64,
            err: 0,
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let mut saved = Vec::new();
        let got = drain(
            fake_daemon(&out_spool, b"", true, &calls),
            &mut offs,
            Instant::now(),
            &mut out,
            &mut err,
            |o| saved.push(o.clone()),
            |_| {},
        );
        assert!(matches!(got, Drained::Done(_)));
        assert!(out[..] == out_spool[PAGE..]);
        assert_eq!(saved.len(), 2);
        assert_eq!(saved.last().unwrap().out, out_spool.len() as u64);
    }

    #[test]
    fn done_job_whose_cursor_stalls_still_finishes() {
        // Sizes claim more than the spool serves (a spool cut under
        // the daemon): an empty page ends the drain, never a loop.
        let calls = std::cell::Cell::new(0);
        let fetch = |o: &Offsets| {
            calls.set(calls.get() + 1);
            Ok(json!({
                "state": "done", "exit": 3, "stdout": "", "stderr": "",
                "out_next": o.out, "err_next": o.err,
                "out_size": 99, "err_size": 0,
            }))
        };
        let mut offs = Offsets::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let got = drain(
            fetch,
            &mut offs,
            Instant::now(),
            &mut out,
            &mut err,
            |_| {},
            |_| {},
        );
        match got {
            Drained::Done(body) => assert_eq!(body["exit"], 3),
            other => panic!("{other:?}"),
        }
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn running_job_sleeps_until_done_then_drains() {
        let calls = std::cell::Cell::new(0);
        let fetch = |o: &Offsets| {
            let n = calls.get();
            calls.set(n + 1);
            if n == 0 {
                return Ok(json!({
                    "state": "running", "stdout": "ab", "stderr": "",
                    "out_next": 2, "err_next": 0,
                }));
            }
            let rest = if o.out < 4 { "cd" } else { "" };
            Ok(json!({
                "state": "done", "exit": 0, "stdout": rest, "stderr": "",
                "out_next": o.out + rest.len() as u64, "err_next": 0,
            }))
        };
        let mut sleeps = 0;
        let mut offs = Offsets::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let got = drain(
            fetch,
            &mut offs,
            Instant::now() + Duration::from_secs(60),
            &mut out,
            &mut err,
            |_| {},
            |_| sleeps += 1,
        );
        assert!(matches!(got, Drained::Done(_)));
        assert_eq!(out, b"abcd");
        assert_eq!(sleeps, 1);
    }

    #[test]
    fn running_job_past_deadline_reports_still_running() {
        let fetch = |_: &Offsets| {
            Ok(json!({
                "state": "running", "stdout": "", "stderr": "",
                "out_next": 0, "err_next": 0,
            }))
        };
        let mut offs = Offsets::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let got = drain(
            fetch,
            &mut offs,
            Instant::now(),
            &mut out,
            &mut err,
            |_| {},
            |_| {},
        );
        assert!(matches!(got, Drained::StillRunning));
    }

    #[test]
    fn reply_without_cursors_is_a_bad_reply() {
        let fetch = |_: &Offsets| Ok(json!({ "state": "done" }));
        let mut offs = Offsets::default();
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let got = drain(
            fetch,
            &mut offs,
            Instant::now(),
            &mut out,
            &mut err,
            |_| {},
            |_| {},
        );
        assert!(matches!(got, Drained::BadReply));
    }

    /// #14188: the FLEET_CTX_* record rides the exec/jobs spec `env`,
    /// and the SSH fallback leg exports it on the remote line.
    #[test]
    fn command_spec_carries_ctx_in_env_and_ssh_leg_exports_it_14188() {
        let cmd = vec!["true".to_string()];
        let empty = fleet_config::ctx::Ctx::default();
        let bare = command_spec_with_ctx(&empty, None, false, &cmd);
        assert!(bare.get("env").is_none(), "no context, no env key: {bare}");

        let ctx = fleet_config::ctx::Ctx::from_pairs([
            ("FLEET_CTX_ID", "task-22655"),
            ("FLEET_CTX_TASK_ID", "22655"),
        ]);
        let spec = command_spec_with_ctx(&ctx, Some("/tmp"), true, &cmd);
        assert_eq!(spec["env"]["FLEET_CTX_ID"], "task-22655");
        assert_eq!(spec["env"]["FLEET_CTX_TASK_ID"], "22655");
        let inner = crate::legs::ssh_inner(&spec);
        assert!(
            inner.starts_with("export FLEET_CTX_ID=task-22655; export FLEET_CTX_TASK_ID=22655; "),
            "{inner}"
        );
    }

    /// #5757 poll-backoff oracle: 1 s doubling capped at 10 s.
    #[test]
    fn poll_interval_starts_at_1s_doubles_and_caps_at_10s_5757() {
        assert_eq!(poll_interval(0), Duration::from_secs(1));
        assert_eq!(poll_interval(1), Duration::from_secs(2));
        assert_eq!(poll_interval(2), Duration::from_secs(4));
        assert_eq!(poll_interval(3), Duration::from_secs(8));
        assert_eq!(poll_interval(4), Duration::from_secs(10));
        assert_eq!(poll_interval(5), Duration::from_secs(10));
        assert_eq!(poll_interval(100), Duration::from_secs(10));
    }

    /// Monotonic and never above the 10 s detection-latency cap.
    #[test]
    fn poll_interval_never_exceeds_the_10s_cap_5757() {
        let mut prev = Duration::ZERO;
        for n in 0..20 {
            let cur = poll_interval(n);
            assert!(cur >= prev, "non-decreasing at poll {n}");
            assert!(cur <= Duration::from_secs(10), "capped at poll {n}");
            prev = cur;
        }
    }

    /// code#16088 exec-batch parity slice: the legacy `_batch` exit
    /// oracle — worst per-command code wins (an error counts 1, a
    /// timeout counts TIMEOUT_EXIT, otherwise the command's own
    /// exit; a reply with no results exits 0 after printing, exactly
    /// like the Python's `worst` accumulator).
    #[test]
    fn batch_exit_matches_python_legacy_worst_code_oracle() {
        assert_eq!(batch_exit_code(&json!({})), 0);
        assert_eq!(batch_exit_code(&json!({"results": []})), 0);
        assert_eq!(batch_exit_code(&json!({"results": [{"exit": 0}]})), 0);
        assert_eq!(
            batch_exit_code(&json!({"results": [{"exit": 0}, {"exit": 3}, {"exit": 1}]})),
            3
        );
        assert_eq!(batch_exit_code(&json!({"results": [{"error": "boom"}]})), 1);
        assert_eq!(
            batch_exit_code(&json!({"results": [{"timeout": true}]})),
            TIMEOUT_EXIT
        );
        assert_eq!(
            batch_exit_code(&json!({"results": [{"error": "x"}, {"timeout": true}, {"exit": 5}]})),
            TIMEOUT_EXIT
        );
        assert_eq!(
            batch_exit_code(&json!({"results": [{"exit": 0}, {"exit": 7}]})),
            7
        );
        // Python truthiness: falsy error/timeout values do not count.
        assert_eq!(batch_exit_code(&json!({"results": [{"error": ""}]})), 0);
        assert_eq!(batch_exit_code(&json!({"results": [{"error": null}]})), 0);
        assert_eq!(
            batch_exit_code(&json!({"results": [{"timeout": false}]})),
            0
        );
    }

    /// code#16088: the legacy prints `json.dumps(out, indent=1)` —
    /// one space per level (bytes captured from CPython for an ASCII
    /// payload; key order follows the JSON map, so the fixture uses
    /// sorted keys).
    #[test]
    fn batch_reply_renders_python_indent_one() {
        let body = json!({"results": [{"exit": 0, "stdout": "hi"}]});
        assert_eq!(
            render_batch_reply(&body),
            "{\n \"results\": [\n  {\n   \"exit\": 0,\n   \"stdout\": \"hi\"\n  }\n ]\n}\n"
        );
    }

    #[test]
    fn batch_truncated_flags_any_truncated_result() {
        assert!(!batch_truncated(&json!({})));
        assert!(!batch_truncated(&json!({"results": [{"exit": 0}]})));
        assert!(batch_truncated(
            &json!({"results": [{"exit": 0}, {"exit": 0, "truncated": true}]})
        ));
    }

    /// code#16088: the file carries the POST body; `cmds` defaults
    /// to [], `timeout_s` defaults to 120 (the legacy `body.get`
    /// defaults). Malformed files are refused before any transport.
    #[test]
    fn batch_file_parses_body_with_legacy_defaults() {
        let dir = std::env::temp_dir().join(format!("la-batch-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("batch.json");
        std::fs::write(&file, r#"{"cmds": [{"argv": ["echo", "hi"]}]}"#).unwrap();
        let (body, timeout) = read_batch_file(&file).unwrap();
        assert_eq!(timeout, 120);
        assert_eq!(body["cmds"].as_array().unwrap().len(), 1);
        let full = dir.join("full.json");
        std::fs::write(&full, r#"{"cmds": [], "timeout_s": 45}"#).unwrap();
        let (_, timeout_full) = read_batch_file(&full).unwrap();
        assert_eq!(timeout_full, 45);
        assert!(read_batch_file(&dir.join("missing.json")).is_err());
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "not json").unwrap();
        assert!(read_batch_file(&bad).is_err());
        let bad_timeout = dir.join("bad-timeout.json");
        std::fs::write(&bad_timeout, r#"{"cmds": [], "timeout_s": "soon"}"#).unwrap();
        assert!(read_batch_file(&bad_timeout).is_err());
        let bad_cmds = dir.join("bad-cmds.json");
        std::fs::write(&bad_cmds, r#"{"cmds": {}}"#).unwrap();
        assert!(read_batch_file(&bad_cmds).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// code#16088 fetch --chunk parity slice: the requested chunk size
    /// rides the GET /file `len` parameter on every range request.
    #[test]
    fn fetch_chunk_wires_len_to_the_file_request() {
        struct Capture {
            seen: Arc<std::sync::Mutex<Vec<String>>>,
        }
        impl Leg for Capture {
            fn name(&self) -> &'static str {
                "tailscale"
            }
            fn call(&self, req: &Request) -> Result<LegReply, LegError> {
                self.seen.lock().unwrap().push(req.path.clone());
                Ok(LegReply {
                    status: 200,
                    body: Vec::new(),
                })
            }
        }
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let ladder = Arc::new(Ladder::new(vec![
            Box::new(Capture { seen: seen.clone() }) as Box<dyn Leg>
        ]));
        fetch_chunk(&ladder, "/remote/a.bin", 0, 64).unwrap();
        fetch_chunk(&ladder, "/remote/a.bin", 64, 64).unwrap();
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 2);
        assert!(seen[0].contains("len=64"), "{}", seen[0]);
        assert!(seen[0].contains("off=0"), "{}", seen[0]);
        assert!(seen[1].contains("off=64"), "{}", seen[1]);
    }

    /// code#16088 fetch parity slice (oracle:
    /// `laptop-agent-legacy.py` `fetch PATH OUT`): an existing OUT
    /// is resumed from its current size — the prefix is never
    /// re-fetched — and the completed file verifies.
    struct FetchDaemon {
        content: Vec<u8>,
        sha_hex: String,
        get_offs: std::sync::Mutex<Vec<u64>>,
    }

    impl FetchDaemon {
        fn parse_off_len(path: &str) -> (u64, u64) {
            let mut off = 0;
            let mut len = 0;
            for part in path.split(['?', '&']) {
                if let Some(v) = part.strip_prefix("off=") {
                    off = v.parse().unwrap_or(0);
                }
                if let Some(v) = part.strip_prefix("len=") {
                    len = v.parse().unwrap_or(0);
                }
            }
            (off, len)
        }
    }

    impl Leg for FetchDaemon {
        fn name(&self) -> &'static str {
            "tailscale"
        }
        fn call(&self, req: &Request) -> Result<LegReply, LegError> {
            if req.method == "POST" && req.path == "/exec" {
                let body = req.body.as_ref().map(|b| b.to_string()).unwrap_or_default();
                if body.contains("shasum") {
                    let reply = json!({"results": [{
                        "exit": 0,
                        "stdout": format!("{}  /remote/a.bin\n", self.sha_hex),
                        "stderr": "",
                    }]});
                    return Ok(LegReply {
                        status: 200,
                        body: serde_json::to_vec(&reply).unwrap(),
                    });
                }
                if body.contains("stat") {
                    let reply = json!({"results": [{
                        "exit": 0,
                        "stdout": format!("{}\n", self.content.len()),
                        "stderr": "",
                    }]});
                    return Ok(LegReply {
                        status: 200,
                        body: serde_json::to_vec(&reply).unwrap(),
                    });
                }
                return Err(LegError::Unsent("unexpected /exec call".into()));
            }
            if req.method == "GET" && req.path.starts_with("/file?") {
                let (off, len) = Self::parse_off_len(&req.path);
                self.get_offs.lock().unwrap().push(off);
                let start = (off as usize).min(self.content.len());
                let end = (start + len as usize).min(self.content.len());
                return Ok(LegReply {
                    status: 200,
                    body: self.content[start..end].to_vec(),
                });
            }
            Err(LegError::Unsent(format!("unexpected {} {}", req.method, req.path)))
        }
    }

    fn fetch_daemon(content: &[u8]) -> (Arc<Ladder>, Arc<FetchDaemon>) {
        use sha2::Digest as _;
        let hex: String = sha2::Sha256::digest(content)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        let daemon = Arc::new(FetchDaemon {
            content: content.to_vec(),
            sha_hex: hex,
            get_offs: std::sync::Mutex::new(Vec::new()),
        });
        let ladder = Arc::new(Ladder::new(vec![
            Box::new(FetchDaemonRef(Arc::clone(&daemon))) as Box<dyn Leg>
        ]));
        (ladder, daemon)
    }

    struct FetchDaemonRef(Arc<FetchDaemon>);

    impl Leg for FetchDaemonRef {
        fn name(&self) -> &'static str {
            "tailscale"
        }
        fn call(&self, req: &Request) -> Result<LegReply, LegError> {
            self.0.call(req)
        }
    }

    fn fetch_tmpdir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "la-fetch-{name}-{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn fetch_resumes_from_existing_out_without_refetching_prefix() {
        let content = b"hello fetch resume world, chunked into small ranges!".to_vec();
        let (ladder, daemon) = fetch_daemon(&content);
        let dir = fetch_tmpdir("resume");
        let dest = dir.join("out.bin");
        let prefix = 10;
        std::fs::write(&dest, &content[..prefix]).unwrap();
        let rc = fetch(&ladder, "/remote/a.bin", &dest, 8);
        assert_eq!(rc, 0, "resume completes");
        assert_eq!(std::fs::read(&dest).unwrap(), content);
        let offs = daemon.get_offs.lock().unwrap();
        assert!(!offs.is_empty(), "at least one range request");
        assert!(
            !offs.iter().any(|&o| o == 0),
            "prefix is reused, never re-fetched: {offs:?}"
        );
        assert_eq!(offs[0], prefix as u64, "{offs:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// code#16088 fetch parity slice (oracle: the `print`/`sys.exit`
    /// strings in `laptop-agent-legacy.py` `fetch PATH OUT`): the success
    /// receipt and the overshoot refusal match byte-for-byte, including
    /// the `round trip(s)` spelling and the resume suffix.
    #[test]
    fn fetch_receipt_and_overshoot_match_legacy_spelling() {
        let dest = std::path::Path::new("out.bin");
        assert_eq!(
            fetch_receipt(11, "/remote/a.bin", dest, 2, 0),
            "fetched 11 bytes from /remote/a.bin -> out.bin in 2 round trip(s)"
        );
        assert_eq!(
            fetch_receipt(11, "/remote/a.bin", dest, 1, 10),
            "fetched 11 bytes from /remote/a.bin -> out.bin in 1 round trip(s) (resumed from offset 10)"
        );
        assert_eq!(
            overshoot_message(dest, 20, "/remote/a.bin", 11),
            "laptop-agent: local out.bin is 20 bytes but remote /remote/a.bin is only 11; refusing to append \u{2014} remove or rename the local file and retry"
        );
    }

    #[test]
    fn fetch_refuses_when_local_out_is_bigger_than_remote() {
        let content = b"small remote".to_vec();
        let (ladder, _daemon) = fetch_daemon(&content);
        let dir = fetch_tmpdir("overshoot");
        let dest = dir.join("out.bin");
        let bigger = [content.clone(), b" + local extra bytes".to_vec()].concat();
        std::fs::write(&dest, &bigger).unwrap();
        let rc = fetch(&ladder, "/remote/a.bin", &dest, 8);
        assert_eq!(rc, 1, "overshoot refuses like the legacy client");
        assert_eq!(
            std::fs::read(&dest).unwrap(),
            bigger,
            "refused fetch leaves the local file alone"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
