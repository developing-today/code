//! The four transport legs, in ladder order: the warm broker socket,
//! the Cloudflare tunnel, Tailscale CONNECT, and SSH.
//!
//! Each leg maps its failures onto the ladder's three classes: absent
//! (not configured here), unsent (failed before the request was written)
//! and sent (failed after). See [`crate::ladder`] for why that split
//! decides whether a request may fall through.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fleet_proxy::http::{HttpConn, HttpError};
use fleet_proxy::tunnel::{
    Egress, connect_cf_tunnel, connect_tunnel, egress_from_env, egress_from_env_url_port,
};

use crate::ladder::{Ladder, Leg, LegError, Reply, Request};
use crate::{CF_ACCESS_SECRET, TARGET_HOST, TARGET_PORT, broker_socket, read_token, token_file};

/// The proxy port the Python tailnet CONNECT dials (`PROXY_PORT`).
pub const TAILNET_PROXY_PORT: u16 = 3130;

/// The broker's framed protocol: an 8-byte big-endian length, then the
/// JSON message; the reply is framed the same way.
pub struct BrokerLeg {
    pub socket: PathBuf,
}

impl Leg for BrokerLeg {
    fn name(&self) -> &'static str {
        "broker"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        if !self.socket.exists() {
            return Err(LegError::Absent(format!(
                "no broker socket at {}",
                self.socket.display()
            )));
        }
        let mut s = UnixStream::connect(&self.socket)
            .map_err(|e| LegError::Unsent(format!("broker connect: {}", e.kind())))?;
        let msg = serde_json::json!({
            "method": req.method,
            "path": req.path,
            "body": req.body,
            "trip": req.trip,
        });
        let payload = serde_json::to_vec(&msg).unwrap_or_default();
        let mut frame = (payload.len() as u64).to_be_bytes().to_vec();
        frame.extend_from_slice(&payload);
        s.write_all(&frame)
            .map_err(|e| LegError::Sent(format!("broker write: {}", e.kind())))?;
        let mut header = [0u8; 8];
        s.read_exact(&mut header)
            .map_err(|e| LegError::Sent(format!("broker read: {}", e.kind())))?;
        let n = u64::from_be_bytes(header);
        let mut data = vec![0u8; n as usize];
        s.read_exact(&mut data)
            .map_err(|e| LegError::Sent(format!("broker read mid-reply: {}", e.kind())))?;
        let obj: serde_json::Value = serde_json::from_slice(&data)
            .map_err(|e| LegError::Sent(format!("broker reply not JSON: {e}")))?;
        let status = obj
            .get("status")
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
            .ok_or_else(|| LegError::Sent("broker reply has no status".into()))?;
        let body = serde_json::to_vec(obj.get("body").unwrap_or(&serde_json::Value::Null))
            .unwrap_or_default();
        Ok(Reply { status, body })
    }
}

/// One HTTP exchange over an established connection. Failures after
/// the connection exists are `Sent`: the request may have reached the
/// daemon, so the ladder decides replay by idempotence.
fn exchange(
    conn: Result<HttpConn, std::io::Error>,
    req: &Request,
    token: &str,
    host_header: &str,
    extra: &[(String, String)],
) -> Result<Reply, LegError> {
    let mut conn = conn.map_err(|e| LegError::Unsent(e.to_string()))?;
    let body = req
        .body
        .as_ref()
        .map(|b| serde_json::to_vec(b).unwrap_or_default());
    let mut headers = extra.to_vec();
    headers.push(("X-Trip-Id".to_string(), req.trip.clone()));
    match conn.request_with_extra_headers(
        req.method,
        &req.path,
        host_header,
        token,
        body.as_deref(),
        &headers,
    ) {
        Ok((status, body)) => Ok(Reply { status, body }),
        Err(HttpError::Io(e)) => Err(LegError::Sent(e.to_string())),
        Err(e) => Err(LegError::Sent(e.to_string())),
    }
}

/// The Cloudflare leg. Its Access service token is a fleet secret,
/// resolved when the leg is tried (so a healthy broker never pays for
/// it); the value never leaves this process except in the headers.
///
/// Resolution mirrors the Python client (code#5702 lineage): a success
/// is cached for the life of the process, a failure is cached for
/// 60 s — the resolver chain can take ~2 minutes to fail (broker
/// retries, then the Bitwarden subprocess), and no caller should pay
/// that more than once a minute. The chain's own error is surfaced in
/// the Absent message: SecretError is value-free by design, and "the
/// secret is unavailable" without its reason is undiagnosable (the
/// 2026-11-17 parity pass on #10796 burned its CF budget on exactly
/// that opacity).
pub struct CfLeg {
    pub token: String,
    pub config: fleet_config::Config,
    pub cf_host: String,
    pub cache: Mutex<CfCache>,
}

#[derive(Default)]
pub struct CfCache {
    ready: Option<Vec<(String, String)>>,
    failed_at: Option<Instant>,
}

impl CfLeg {
    fn headers(&self) -> Result<Vec<(String, String)>, LegError> {
        let mut cache = self.cache.lock().unwrap();
        if let Some(headers) = &cache.ready {
            return Ok(headers.clone());
        }
        if let Some(at) = cache.failed_at {
            if at.elapsed() < Duration::from_secs(60) {
                return Err(LegError::Absent(format!(
                    "secret {CF_ACCESS_SECRET} unavailable (failure cached <60s)"
                )));
            }
        }
        let resolver = fleet_secrets::Resolver::default_chain(&self.config);
        let resolved = resolver
            .resolve(CF_ACCESS_SECRET)
            .map_err(|e| LegError::Absent(format!("secret {CF_ACCESS_SECRET} unavailable: {e}")));
        let secret = match resolved {
            Ok(secret) => secret,
            Err(e) => {
                cache.failed_at = Some(Instant::now());
                return Err(e);
            }
        };
        match cf_access_headers(secret.expose()) {
            Some(headers) => {
                cache.ready = Some(headers.clone());
                cache.failed_at = None;
                Ok(headers)
            }
            None => {
                cache.failed_at = Some(Instant::now());
                Err(LegError::Absent(format!(
                    "secret {CF_ACCESS_SECRET} is not a service token"
                )))
            }
        }
    }
}

impl Leg for CfLeg {
    fn name(&self) -> &'static str {
        "cloudflare"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        let egress: Egress = egress_from_env_url_port().map_err(|e| LegError::Absent(e))?;
        let headers = self.headers()?;
        let conn = connect_cf_tunnel(&egress, &self.cf_host).and_then(HttpConn::from_tls);
        exchange(conn, req, &self.token, &self.cf_host, &headers)
    }
}

/// `CF-Access-*` headers from the secret's JSON (`client_id`,
/// `client_secret`). None when the value is not a service token.
pub fn cf_access_headers(json_text: &str) -> Option<Vec<(String, String)>> {
    let value: serde_json::Value = serde_json::from_str(json_text).ok()?;
    let id = value.get("client_id")?.as_str()?;
    let secret = value.get("client_secret")?.as_str()?;
    if id.is_empty() || secret.is_empty() {
        return None;
    }
    Some(vec![
        ("CF-Access-Client-Id".to_string(), id.to_string()),
        ("CF-Access-Client-Secret".to_string(), secret.to_string()),
    ])
}

/// Tailscale: a CONNECT through the tailnet egress proxy to the
/// laptop's daemon port.
pub struct TailscaleLeg {
    pub token: String,
}

impl Leg for TailscaleLeg {
    fn name(&self) -> &'static str {
        "tailscale"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        let egress = egress_from_env(TAILNET_PROXY_PORT).map_err(LegError::Absent)?;
        let conn = connect_tunnel(Some(&egress), TARGET_HOST, TARGET_PORT).and_then(HttpConn::new);
        let host = format!("{TARGET_HOST}:{TARGET_PORT}");
        exchange(conn, req, &self.token, &host, &[])
    }
}

/// SSH: the daemon's request surface re-implemented over
/// `~/tools/laptop-ssh-real.sh`, mirroring the Python client's SSH
/// fallback (code#2879 lineage). All agent transports converge on the
/// single laptop daemon; when it dies, plain SSH still works (separate
/// sshd, separate path). This leg therefore does not tunnel HTTP: it
/// runs shell commands over SSH that produce the daemon-shaped
/// replies for the surfaces it can carry (`/health`, `/exec`,
/// `/jobs` including poll/kill/list), and owns `sshfb-` jobs — spooled
/// under the laptop's `~/.local/state/laptop-agent/sshfb-jobs/` —
/// for their whole life. Anything else is Absent here.
pub struct SshLeg {
    pub script: PathBuf,
}

/// The remote shell proves it started by echoing this line first;
/// its absence means the shell never started (proxy down, sshd down,
/// auth failure), not that the command failed.
const SSH_MAGIC: &str = "__LAPTOP_AGENT_SSHFB_V1__";
const SSHFB_PREFIX: &str = "sshfb-";
const SSHFB_REMOTE_DIR: &str = ".local/state/laptop-agent/sshfb-jobs";
/// The Python client's kill switch for this leg.
pub const NO_SSH_FALLBACK_ENV: &str = "LAPTOP_AGENT_NO_SSH_FALLBACK";

impl SshLeg {
    pub fn new() -> SshLeg {
        SshLeg {
            script: crate::home().join("tools/laptop-ssh-real.sh"),
        }
    }

    fn gate(&self) -> Result<(), LegError> {
        if std::env::var_os(NO_SSH_FALLBACK_ENV).is_some_and(|v| !v.is_empty()) {
            return Err(LegError::Absent(format!(
                "ssh fallback disabled by {NO_SSH_FALLBACK_ENV}"
            )));
        }
        if !self.script.exists() {
            return Err(LegError::Absent(format!(
                "no laptop-ssh-real.sh at {}",
                self.script.display()
            )));
        }
        Ok(())
    }

    /// Run one command string on the laptop over real SSH. Returns
    /// (exit, stdout after the magic line, stderr); the exit is None
    /// when the local wait timed out (the ssh process group was
    /// killed; the remote command may outlive it — plain SSH has no
    /// remote cancel, the documented limitation of this path). A
    /// missing magic line is Unsent: the request never reached a
    /// remote shell.
    fn run(
        &self,
        remote: &str,
        timeout: Duration,
    ) -> Result<(Option<i32>, Vec<u8>, Vec<u8>), LegError> {
        use std::os::unix::process::CommandExt;
        use std::process::{Command, Stdio};
        let mut child = Command::new(&self.script)
            .arg(format!("echo {SSH_MAGIC}; {remote}"))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .process_group(0)
            .spawn()
            .map_err(|e| LegError::Unsent(format!("cannot launch ssh: {e}")))?;
        let mut out_pipe = child.stdout.take();
        let mut err_pipe = child.stderr.take();
        let out_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = out_pipe.take() {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });
        let err_thread = std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut pipe) = err_pipe.take() {
                let _ = pipe.read_to_end(&mut buf);
            }
            buf
        });
        let started = Instant::now();
        let exit = loop {
            match child.try_wait() {
                Ok(Some(status)) => break status.code(),
                Ok(None) => {
                    if started.elapsed() >= timeout {
                        let pid = child.id();
                        let _ = Command::new("kill")
                            .args(["-KILL", &format!("-{pid}")])
                            .status();
                        let _ = child.kill();
                        let _ = child.wait();
                        break None;
                    }
                    std::thread::sleep(Duration::from_millis(25));
                }
                Err(e) => return Err(LegError::Unsent(format!("ssh wait failed: {e}"))),
            }
        };
        let out = out_thread.join().unwrap_or_default();
        let err = err_thread.join().unwrap_or_default();
        let marker = format!("{SSH_MAGIC}\n").into_bytes();
        let Some(idx) = find_subslice(&out, &marker) else {
            return Err(LegError::Unsent(format!(
                "remote shell never started (exit={exit:?}): {}",
                String::from_utf8_lossy(&err)
                    .trim()
                    .chars()
                    .take(200)
                    .collect::<String>()
            )));
        };
        Ok((exit, out[idx + marker.len()..].to_vec(), err))
    }

    fn health(&self) -> Result<Reply, LegError> {
        let remote = format!(
            "head -1 /proc/loadavg; head -1 /proc/uptime; \
             ls $HOME/.local/state/laptop-agent/jobs \
             $HOME/{SSHFB_REMOTE_DIR} 2>/dev/null | wc -l"
        );
        let (_, out, _) = self.run(&remote, Duration::from_secs(30))?;
        let text = String::from_utf8_lossy(&out);
        let mut lines = text.lines();
        let parsed = (|| {
            let load1: f64 = lines.next()?.split_whitespace().next()?.parse().ok()?;
            let uptime_s: u64 = lines
                .next()?
                .split_whitespace()
                .next()?
                .parse::<f64>()
                .ok()? as u64;
            let jobs: u64 = lines.next()?.split_whitespace().next()?.parse().ok()?;
            Some((load1, uptime_s, jobs))
        })();
        let Some((load1, uptime_s, jobs)) = parsed else {
            return Err(LegError::Unsent("unparsable ssh health reply".into()));
        };
        Ok(json_reply(serde_json::json!({
            "ok": true,
            "uptime_s": uptime_s,
            "jobs": jobs,
            "load1": load1,
            "transport": "ssh-fallback",
        })))
    }

    fn exec(&self, req: &Request) -> Result<Reply, LegError> {
        let empty = serde_json::json!({});
        let body = req.body.as_ref().unwrap_or(&empty);
        let timeout_s = body
            .get("timeout_s")
            .and_then(|v| v.as_u64())
            .unwrap_or(120);
        let cmds = body.get("cmds").and_then(|v| v.as_array());
        let mut results = Vec::new();
        if let Some(cmds) = cmds {
            for spec in cmds {
                results.push(self.exec_one(spec, timeout_s)?);
            }
        }
        Ok(json_reply(serde_json::json!({ "results": results })))
    }

    fn exec_one(
        &self,
        spec: &serde_json::Value,
        timeout_s: u64,
    ) -> Result<serde_json::Value, LegError> {
        let remote = format!("exec bash -lc {}", sh_quote(&ssh_inner(spec)));
        let started = Instant::now();
        let (exit, out, err) = self.run(&remote, Duration::from_secs(timeout_s + 15))?;
        let duration_ms = started.elapsed().as_millis() as u64;
        let (out, out_trunc) = cap_tail(&out);
        let (err, err_trunc) = cap_tail(&err);
        let mut result = serde_json::json!({
            "stdout": String::from_utf8_lossy(&out),
            "stderr": String::from_utf8_lossy(&err),
            "exit": exit,
            "duration_ms": duration_ms,
            "truncated": out_trunc || err_trunc,
        });
        if exit.is_none() {
            result["timeout"] = serde_json::json!(true);
        }
        Ok(result)
    }

    fn launch_job(&self, req: &Request) -> Result<Reply, LegError> {
        let empty = serde_json::json!({});
        let spec = req.body.as_ref().unwrap_or(&empty);
        let job_id = format!("{SSHFB_PREFIX}{}", crate::new_trip());
        let d = format!("$HOME/{SSHFB_REMOTE_DIR}/{job_id}");
        let script = format!(
            "echo $$ > {d}/pid; date +%s > {d}/start; \
             bash -lc {} > {d}/out 2> {d}/err; echo $? > {d}/exit",
            sh_quote(&ssh_inner(spec))
        );
        let remote = format!(
            "d={d}; mkdir -p \"$d\" && setsid bash -c {} </dev/null >/dev/null 2>&1 & echo launched",
            sh_quote(&script)
        );
        self.run(&remote, Duration::from_secs(30))?;
        Ok(json_reply(serde_json::json!({ "job_id": job_id })))
    }

    fn poll_job(&self, job_id: &str, query: &str) -> Result<Reply, LegError> {
        let out_off = query_u64(query, "out_off");
        let err_off = query_u64(query, "err_off");
        let d = format!("$HOME/{SSHFB_REMOTE_DIR}/{job_id}");
        let remote = format!(
            "d={d}; \
             os=$(stat -c%s \"$d/out\" 2>/dev/null || echo 0); \
             es=$(stat -c%s \"$d/err\" 2>/dev/null || echo 0); \
             start=$(cat \"$d/start\" 2>/dev/null || date +%s); \
             if [ -f \"$d/exit\" ]; then st=done; ex=$(cat \"$d/exit\"); \
             fin=$(stat -c%Y \"$d/exit\"); else st=running; ex=; \
             fin=$(date +%s); fi; \
             echo \"STATE $st\"; echo \"EXIT $ex\"; \
             echo \"DUR $(( (fin - start) * 1000 ))\"; \
             echo \"OUTNEXT $os\"; echo \"ERRNEXT $es\"; \
             printf 'OUTB64 '; tail -c +$(( {} + 1 )) \"$d/out\" \
             2>/dev/null | base64 -w0; echo; \
             printf 'ERRB64 '; tail -c +$(( {} + 1 )) \"$d/err\" \
             2>/dev/null | base64 -w0; echo",
            out_off, err_off
        );
        let (_, out, _) = self.run(&remote, Duration::from_secs(60))?;
        let text = String::from_utf8_lossy(&out);
        let mut fields: std::collections::HashMap<String, String> = Default::default();
        for line in text.lines() {
            let (key, rest) = line.split_once(' ').unwrap_or((line, ""));
            fields.insert(key.to_string(), rest.to_string());
        }
        let Some(state) = fields.get("STATE") else {
            return Err(LegError::Unsent(format!(
                "unparsable poll reply for {job_id}"
            )));
        };
        let exit = fields.get("EXIT").and_then(|v| v.parse::<i64>().ok());
        let parse_u64 = |key: &str, default: u64| {
            fields
                .get(key)
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or(default)
        };
        Ok(json_reply(serde_json::json!({
            "job_id": job_id,
            "state": state,
            "exit": exit,
            "stdout": String::from_utf8_lossy(&b64_decode(fields.get("OUTB64").map(String::as_str).unwrap_or(""))),
            "stderr": String::from_utf8_lossy(&b64_decode(fields.get("ERRB64").map(String::as_str).unwrap_or(""))),
            "out_next": parse_u64("OUTNEXT", out_off),
            "err_next": parse_u64("ERRNEXT", err_off),
            "duration_ms": parse_u64("DUR", 0),
        })))
    }

    fn kill_job(&self, job_id: &str) -> Result<Reply, LegError> {
        let d = format!("$HOME/{SSHFB_REMOTE_DIR}/{job_id}");
        let remote = format!(
            "d={d}; pg=$(cat \"$d/pid\" 2>/dev/null); \
             if [ -n \"$pg\" ]; then \
             kill -TERM -$pg 2>/dev/null; sleep 2; \
             kill -KILL -$pg 2>/dev/null; fi; \
             ( set -C; echo -15 > \"$d/exit\" ) 2>/dev/null; echo ok"
        );
        self.run(&remote, Duration::from_secs(30))?;
        Ok(json_reply(serde_json::json!({
            "ok": true,
            "job_id": job_id,
        })))
    }

    fn list_jobs(&self) -> Result<Reply, LegError> {
        let remote = format!(
            "for d in $HOME/{SSHFB_REMOTE_DIR}/*/; do \
             [ -d \"$d\" ] || continue; id=$(basename \"$d\"); \
             if [ -f \"$d/exit\" ]; then echo \"$id done \
             $(cat \"$d/exit\")\"; else echo \"$id running\"; fi; \
             done"
        );
        let (_, out, _) = self.run(&remote, Duration::from_secs(30))?;
        let text = String::from_utf8_lossy(&out);
        let mut jobs = Vec::new();
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 2 {
                jobs.push(serde_json::json!({
                    "job_id": parts[0],
                    "state": parts[1],
                    "exit": parts.get(2).and_then(|v| v.parse::<i64>().ok()),
                }));
            }
        }
        Ok(json_reply(serde_json::json!({ "jobs": jobs })))
    }
}

impl Leg for SshLeg {
    fn name(&self) -> &'static str {
        "ssh"
    }

    fn call(&self, req: &Request) -> Result<Reply, LegError> {
        self.gate()?;
        let (path, query) = match req.path.split_once('?') {
            Some((p, q)) => (p, q),
            None => (req.path.as_str(), ""),
        };
        match (req.method, path) {
            ("GET", "/health") => self.health(),
            ("POST", "/exec") => self.exec(req),
            ("POST", "/jobs") => self.launch_job(req),
            ("GET", "/jobs") => self.list_jobs(),
            ("GET", p) if p.starts_with("/jobs/") => self.poll_job(job_segment(p), query),
            ("POST", p)
                if p.starts_with("/jobs/") && p.trim_end_matches('/').ends_with("/kill") =>
            {
                self.kill_job(job_segment(p))
            }
            _ => Err(LegError::Absent(format!(
                "no ssh fallback for {} {}",
                req.method, req.path
            ))),
        }
    }
}

/// The job id segment of a `/jobs/<id>...` path.
fn job_segment(path: &str) -> &str {
    path.strip_prefix("/jobs/")
        .unwrap_or(path)
        .split('/')
        .next()
        .unwrap_or("")
}

fn json_reply(value: serde_json::Value) -> Reply {
    Reply {
        status: 200,
        body: serde_json::to_vec(&value).unwrap_or_default(),
    }
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// One query parameter as u64; absent or unparsable means 0, as the
/// Python poll handler's `int(... or 0)` on a missing key.
fn query_u64(query: &str, key: &str) -> u64 {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=')
            && k == key
        {
            return v.parse().unwrap_or(0);
        }
    }
    0
}

/// The per-stream capture cap, as the daemon's: past it, keep the
/// tail and say so.
fn cap_tail(data: &[u8]) -> (&[u8], bool) {
    let cap = crate::FILE_CHUNK_MAX as usize;
    if data.len() > cap {
        (&data[data.len() - cap..], true)
    } else {
        (data, false)
    }
}

/// POSIX shell quoting, mirroring Python's `shlex.quote`: safe
/// strings pass through, everything else is single-quoted with
/// embedded quotes escaped as `'\''`.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// Flatten a {cwd, argv|shell, env?} spec into one command string,
/// mirroring the daemon's shell mode (the `bash -lc` happens one
/// level up, in the remote string the caller builds).
pub fn ssh_inner(spec: &serde_json::Value) -> String {
    let mut inner = String::new();
    if let Some(env) = spec.get("env").and_then(|v| v.as_object())
        && !env.is_empty()
    {
        let exports: Vec<String> = env
            .iter()
            .map(|(k, v)| {
                let text = match v {
                    serde_json::Value::String(s) => s.clone(),
                    other => other.to_string(),
                };
                format!("export {k}={}", sh_quote(&text))
            })
            .collect();
        inner += &exports.join("; ");
        inner += "; ";
    }
    if let Some(cwd) = spec.get("cwd").and_then(|v| v.as_str()) {
        inner += &format!("cd {} || exit 126; ", sh_quote(cwd));
    }
    if let Some(shell) = spec.get("shell").and_then(|v| v.as_str()) {
        inner += shell;
    } else if let Some(argv) = spec.get("argv").and_then(|v| v.as_array()) {
        let words: Vec<String> = argv
            .iter()
            .map(|a| match a {
                serde_json::Value::String(s) => sh_quote(s),
                other => sh_quote(&other.to_string()),
            })
            .collect();
        inner += &words.join(" ");
    }
    inner
}

/// Standard-alphabet base64 decode, tolerant of whitespace and
/// missing padding (the poll reply's payloads arrive wrapped by
/// shell echo). Invalid bytes are skipped, as Python's
/// `b64decode(validate=False)` does.
pub fn b64_decode(text: &str) -> Vec<u8> {
    fn value(c: u8) -> Option<u32> {
        match c {
            b'A'..=b'Z' => Some(u32::from(c - b'A')),
            b'a'..=b'z' => Some(u32::from(c - b'a') + 26),
            b'0'..=b'9' => Some(u32::from(c - b'0') + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }
    let mut out = Vec::new();
    let mut acc: u32 = 0;
    let mut bits = 0;
    for byte in text.bytes() {
        if byte == b'=' {
            break;
        }
        let Some(v) = value(byte) else { continue };
        acc = (acc << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// The ladder for this process: broker, Cloudflare, Tailscale, SSH.
pub fn ladder(config: &fleet_config::Config) -> Result<Ladder, String> {
    let token = read_token(&token_file())?;
    let legs: Vec<Box<dyn Leg>> = vec![
        Box::new(BrokerLeg {
            socket: broker_socket(),
        }),
        Box::new(CfLeg {
            token: token.clone(),
            config: config.clone(),
            cf_host: config.cf_host().to_string(),
            cache: Mutex::new(CfCache::default()),
        }),
        Box::new(TailscaleLeg {
            token: token.clone(),
        }),
        Box::new(SshLeg::new()),
    ];
    Ok(Ladder::new(legs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cf_headers_come_from_a_service_token_only() {
        let h = cf_access_headers(r#"{"client_id":"id.access","client_secret":"s"}"#).unwrap();
        assert_eq!(
            h[0],
            ("CF-Access-Client-Id".to_string(), "id.access".to_string())
        );
        assert_eq!(h[1].0, "CF-Access-Client-Secret");
        assert!(cf_access_headers(r#"{"client_id":"id"}"#).is_none());
        assert!(cf_access_headers("not json").is_none());
        assert!(cf_access_headers(r#"{"client_id":"","client_secret":"s"}"#).is_none());
    }

    #[test]
    fn a_missing_broker_socket_is_absent_not_sent() {
        let leg = BrokerLeg {
            socket: PathBuf::from("/nonexistent/laptop-agent-test/broker.sock"),
        };
        let req = Request {
            method: "GET",
            path: "/health".into(),
            body: None,
            trip: "t".into(),
            idempotent: true,
            direct_only: false,
        };
        assert!(matches!(leg.call(&req), Err(LegError::Absent(_))));
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
    fn ssh_leg_is_absent_without_the_script() {
        let leg = SshLeg {
            script: PathBuf::from("/nonexistent/laptop-agent-test/laptop-ssh-real.sh"),
        };
        assert!(matches!(leg.call(&health_req()), Err(LegError::Absent(_))));
    }

    #[test]
    fn ssh_leg_carries_no_other_surfaces() {
        let leg = SshLeg {
            script: PathBuf::from("/nonexistent/laptop-agent-test/laptop-ssh-real.sh"),
        };
        // Gate first: with no script every surface is Absent, including
        // the ones the leg does carry.
        let mut req = health_req();
        req.path = "/file".into();
        assert!(matches!(leg.call(&req), Err(LegError::Absent(_))));
    }

    #[test]
    fn sh_quote_matches_shlex() {
        assert_eq!(sh_quote("plain-1.2/x"), "plain-1.2/x");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("a b"), "'a b'");
        assert_eq!(sh_quote("it's"), "'it'\\''s'");
        assert_eq!(sh_quote("$HOME"), "'$HOME'");
    }

    #[test]
    fn ssh_inner_flattens_env_cwd_and_argv() {
        let spec = serde_json::json!({
            "cwd": "/tmp/a b",
            "env": {"A": "1", "B": "x y"},
            "argv": ["echo", "hi there"],
        });
        assert_eq!(
            ssh_inner(&spec),
            "export A=1; export B='x y'; cd '/tmp/a b' || exit 126; echo 'hi there'"
        );
        let shell = serde_json::json!({"shell": "ls | wc -l"});
        assert_eq!(ssh_inner(&shell), "ls | wc -l");
    }

    #[test]
    fn b64_decode_round_trips_and_tolerates_noise() {
        assert_eq!(b64_decode("aGVsbG8="), b"hello");
        assert_eq!(b64_decode("aGVs\nbG8"), b"hello");
        assert_eq!(b64_decode(""), b"");
    }

    #[test]
    fn ssh_health_answers_through_a_fake_script() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("la-ssh-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("laptop-ssh-real.sh");
        std::fs::write(
            &script,
            "#!/bin/sh\nprintf '%s\\n' '__LAPTOP_AGENT_SSHFB_V1__' '0.25 0.5 0.75 1/100' '12345.0 999.0' '7'\n",
        )
        .unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let leg = SshLeg { script };
        let reply = leg.call(&health_req()).unwrap();
        assert_eq!(reply.status, 200);
        let body: serde_json::Value = serde_json::from_slice(&reply.body).unwrap();
        assert_eq!(body["transport"], "ssh-fallback");
        assert_eq!(body["uptime_s"], 12345);
        assert_eq!(body["jobs"], 7);
        assert_eq!(body["load1"], 0.25);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
