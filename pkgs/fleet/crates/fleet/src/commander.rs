//! Tier T1 (commander/fleet) — developing-today/code#13654, #13717.
//!
//! A client for the self-hosted Desktop Commander MCP endpoint
//! (#13404), built in the endpoint's own pattern: the bearer is
//! resolved by name from the secrets broker at call time, one MCP
//! session per invocation, `timeout_ms` is always sent (the tool
//! schema requires it), and every spawned process is polled to
//! completion or force-terminated — an exec never leaves a running
//! process behind on the laptop (the pile-up failure mode of
//! 2026-11-23). The remote exit code is read from the endpoint's own
//! completion line ("Process completed with exit code N").

use std::time::{Duration, Instant};

use fleet_config::Config;
use fleet_secrets::Resolver;
use serde_json::{Value, json};

use crate::forms::{ServeOutcome, T1, announce, shell_quote};

const ENDPOINT: &str = "https://commander.security.cab/mcp";
const BEARER_SECRET: &str = "desktop-commander.machine-bearer";
/// The edge 1010-blocks non-browser user agents (#13404).
const BROWSER_UA: &str = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36 \
    (KHTML, like Gecko) Chrome/126.0.0.0 Safari/537.36";
const EXIT_TIMEOUT: i32 = 124;
const EXIT_USAGE: i32 = 2;
const DEFAULT_TIMEOUT_S: u64 = 120;
/// How long one read_process_output poll may wait server-side.
const POLL_WAIT_MS: u64 = 5_000;

const USAGE: &str = "\
Usage: fleet commander <command> [args...]

Tier T1 (commander/fleet) of the laptop route forms (#13717): run
commands on the laptop through the Desktop Commander MCP endpoint.

Commands:
  health   Initialize a session, run `hostname`, report latency
  exec [--cwd D] [--timeout S] -- cmd...
           Run a command line on the laptop (through its shell);
           the remote exit code propagates. --timeout is in seconds
           and always applies (default 120): on expiry the remote
           process is force-terminated and the exit code is 124.";

struct Client {
    agent: ureq::Agent,
    bearer: String,
    session: String,
    next_id: u64,
}

impl Client {
    fn connect(config: &Config) -> Result<Client, String> {
        let bearer = Resolver::default_chain(config)
            .resolve(BEARER_SECRET)
            .map_err(|e| format!("secret {BEARER_SECRET} unavailable: {e}"))?
            .expose()
            .to_string();
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .timeout_connect(Some(Duration::from_secs(15)))
            .http_status_as_error(false)
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            )
            .build()
            .into();
        let mut client = Client {
            agent,
            bearer,
            session: String::new(),
            next_id: 1,
        };
        client.rpc(
            "initialize",
            Some(json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": { "name": "fleet", "version": env!("CARGO_PKG_VERSION") },
            })),
        )?;
        if client.session.is_empty() {
            return Err("commander endpoint sent no Mcp-Session-Id".into());
        }
        client.notify("notifications/initialized")?;
        Ok(client)
    }

    /// One JSON-RPC round trip. The response may be SSE (`data:`
    /// lines) or a bare JSON body; the message matching our request
    /// id wins. An `initialize` response's session id is captured.
    fn rpc(&mut self, method: &str, params: Option<Value>) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let mut body = json!({ "jsonrpc": "2.0", "id": id, "method": method });
        if let Some(p) = params {
            body["params"] = p;
        }
        let (status, session, text) = self.post(&body.to_string())?;
        if method == "initialize" {
            if let Some(s) = session {
                self.session = s;
            }
        }
        if !(200..300).contains(&status) {
            return Err(format!("commander HTTP {status}"));
        }
        let msg = pick_message(&text, id)
            .ok_or_else(|| format!("commander reply to {method} carried no result"))?;
        if let Some(err) = msg.get("error") {
            let m = err
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("rpc error");
            return Err(format!("commander {method}: {m}"));
        }
        Ok(msg.get("result").cloned().unwrap_or(Value::Null))
    }

    fn notify(&mut self, method: &str) -> Result<(), String> {
        let body = json!({ "jsonrpc": "2.0", "method": method });
        let (status, _, _) = self.post(&body.to_string())?;
        if !(200..300).contains(&status) {
            return Err(format!("commander HTTP {status} on {method}"));
        }
        Ok(())
    }

    fn post(&mut self, body: &str) -> Result<(u16, Option<String>, String), String> {
        let mut req = self
            .agent
            .post(ENDPOINT)
            .header("Authorization", &format!("Bearer {}", self.bearer))
            .header("User-Agent", BROWSER_UA)
            .header("Accept", "application/json, text/event-stream")
            .content_type("application/json");
        if !self.session.is_empty() {
            req = req.header("Mcp-Session-Id", &self.session);
        }
        let mut resp = req
            .send(body)
            .map_err(|e| format!("commander transport: {e}"))?;
        let status = resp.status().as_u16();
        let session = resp
            .headers()
            .get("mcp-session-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        let text = resp
            .body_mut()
            .read_to_string()
            .map_err(|e| format!("commander reply unreadable: {e}"))?;
        Ok((status, session, text))
    }

    fn tools_call(&mut self, name: &str, arguments: Value) -> Result<String, String> {
        let result = self.rpc(
            "tools/call",
            Some(json!({ "name": name, "arguments": arguments })),
        )?;
        let text = content_text(&result);
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            return Err(format!("commander tool {name} failed: {text}"));
        }
        Ok(text)
    }
}

fn pick_message(text: &str, want_id: u64) -> Option<Value> {
    let mut candidates: Vec<Value> = Vec::new();
    for line in text.lines() {
        if let Some(data) = line.trim().strip_prefix("data:") {
            if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                candidates.push(v);
            }
        }
    }
    if candidates.is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(text.trim()) {
            candidates.push(v);
        }
    }
    candidates
        .iter()
        .find(|v| v.get("id").and_then(Value::as_u64) == Some(want_id))
        .cloned()
        .or_else(|| candidates.last().cloned())
}

fn content_text(result: &Value) -> String {
    result
        .get("content")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|c| c.get("text").and_then(Value::as_str))
                .collect::<Vec<_>>()
                .join("\n")
        })
        .unwrap_or_default()
}

fn parse_pid(text: &str) -> Option<u64> {
    let idx = text.find("PID ")? + 4;
    let num: String = text[idx..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    num.parse().ok()
}

struct ReadChunk {
    output: String,
    total_lines: u64,
    exit_code: Option<i32>,
    completed: bool,
}

/// Parse one read_process_output reply: a leading `[...]` status
/// line (whose `(total: N lines)` is the authoritative output-line
/// count), a blank separator, the new output lines, and — once the
/// process ends — the endpoint's completion line.
fn parse_read_chunk(text: &str, prev_total: u64) -> ReadChunk {
    let mut lines: Vec<&str> = text.lines().collect();
    let mut total: Option<u64> = None;
    if let Some(first) = lines.first() {
        let first = *first;
        if first.starts_with('[') && first.ends_with(']') {
            if let Some(t) = first
                .split("(total: ")
                .nth(1)
                .and_then(|s| s.split_whitespace().next())
                .and_then(|n| n.parse::<u64>().ok())
            {
                total = Some(t);
            }
            lines.remove(0);
            if lines.first().map(|l| l.is_empty()).unwrap_or(false) {
                lines.remove(0);
            }
        }
    }
    let mut exit_code = None;
    let mut completed = false;
    let mut out_lines: Vec<&str> = Vec::new();
    for l in lines {
        if let Some(idx) = l.find("Process completed with exit code ") {
            completed = true;
            let rest = &l[idx + "Process completed with exit code ".len()..];
            let num: String = rest
                .chars()
                .take_while(|c| c.is_ascii_digit() || *c == '-')
                .collect();
            exit_code = num.parse::<i32>().ok();
            continue;
        }
        if l.contains("Process completed") || l.contains("Process terminated") {
            completed = true;
            continue;
        }
        out_lines.push(l);
    }
    let total_lines = total.unwrap_or(prev_total + out_lines.len() as u64);
    let mut output = out_lines.join("\n");
    if !output.is_empty() {
        output.push('\n');
    }
    ReadChunk {
        output,
        total_lines,
        exit_code,
        completed,
    }
}

struct Capture {
    outcome: ServeOutcome,
    output: String,
}

fn exec_capture(config: &Config, cwd: Option<&str>, timeout_s: u64, cmd: &[String]) -> Capture {
    let fail = |m: String| Capture {
        outcome: ServeOutcome::Failed(m),
        output: String::new(),
    };
    let mut client = match Client::connect(config) {
        Ok(c) => c,
        Err(m) => return fail(m),
    };
    let mut line = cmd.join(" ");
    if let Some(d) = cwd {
        line = format!("cd {} && {}", shell_quote(d), line);
    }
    let start = match client.tools_call(
        "start_process",
        json!({ "command": line, "timeout_ms": 3000 }),
    ) {
        Ok(t) => t,
        Err(m) => return fail(m),
    };
    let pid = match parse_pid(&start) {
        Some(p) => p,
        None => {
            return fail(format!(
                "commander start_process named no PID: {}",
                start.lines().next().unwrap_or("")
            ));
        }
    };
    let deadline = Instant::now() + Duration::from_secs(timeout_s.max(1));
    let mut offset: u64 = 0;
    let mut output = String::new();
    loop {
        match client.tools_call(
            "read_process_output",
            json!({ "pid": pid, "offset": offset, "timeout_ms": POLL_WAIT_MS }),
        ) {
            Ok(text) => {
                let chunk = parse_read_chunk(&text, offset);
                offset = chunk.total_lines;
                output.push_str(&chunk.output);
                if let Some(code) = chunk.exit_code {
                    return Capture {
                        outcome: ServeOutcome::Served(code),
                        output,
                    };
                }
                if chunk.completed {
                    return Capture {
                        outcome: ServeOutcome::Served(1),
                        output,
                    };
                }
            }
            Err(m) => {
                let _ = client.tools_call("force_terminate", json!({ "pid": pid }));
                return Capture {
                    outcome: ServeOutcome::Failed(m),
                    output,
                };
            }
        }
        if Instant::now() >= deadline {
            let _ = client.tools_call("force_terminate", json!({ "pid": pid }));
            return Capture {
                outcome: ServeOutcome::TimedOut,
                output,
            };
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Serve one exec through this tier, printing the remote output.
/// Used by the standalone verb and by tier 0.
pub fn serve(config: &Config, cwd: Option<&str>, timeout_s: u64, cmd: &[String]) -> ServeOutcome {
    use std::io::Write;
    let capture = exec_capture(config, cwd, timeout_s, cmd);
    print!("{}", capture.output);
    let _ = std::io::stdout().flush();
    capture.outcome
}

fn health(config: &Config) -> i32 {
    let started = Instant::now();
    let capture = exec_capture(config, None, 20, &["hostname".to_string()]);
    let ms = started.elapsed().as_millis();
    match capture.outcome {
        ServeOutcome::Served(0) if capture.output.trim() == "amd" => {
            println!(
                "{}",
                json!({ "ok": true, "form": "T1 commander/fleet", "host": "amd", "latency_ms": ms })
            );
            0
        }
        ServeOutcome::Served(code) => {
            eprintln!(
                "commander: health: unexpected result (exit {code}, output {:?})",
                capture.output.trim()
            );
            1
        }
        ServeOutcome::TimedOut => {
            eprintln!("commander: health: timed out");
            EXIT_TIMEOUT
        }
        ServeOutcome::Failed(m) => {
            eprintln!("commander: health: {m}");
            1
        }
    }
}

struct ExecArgs {
    cwd: Option<String>,
    timeout_s: u64,
    cmd: Vec<String>,
}

fn parse_exec_args(args: &[String]) -> Result<ExecArgs, String> {
    let mut cwd = None;
    let mut timeout_s = DEFAULT_TIMEOUT_S;
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a == "--" {
            let cmd: Vec<String> = args[i + 1..].to_vec();
            if cmd.is_empty() {
                return Err("exec: no command after --".into());
            }
            return Ok(ExecArgs {
                cwd,
                timeout_s,
                cmd,
            });
        } else if a == "--cwd" {
            i += 1;
            cwd = Some(
                args.get(i)
                    .ok_or("exec: --cwd needs a value")?
                    .to_string(),
            );
        } else if let Some(v) = a.strip_prefix("--cwd=") {
            cwd = Some(v.to_string());
        } else if a == "--timeout" {
            i += 1;
            timeout_s = args
                .get(i)
                .ok_or("exec: --timeout needs a value")?
                .parse()
                .map_err(|_| "exec: --timeout must be seconds".to_string())?;
        } else if let Some(v) = a.strip_prefix("--timeout=") {
            timeout_s = v
                .parse()
                .map_err(|_| "exec: --timeout must be seconds".to_string())?;
        } else {
            return Err(format!("exec: unexpected argument {a} (command goes after --)"));
        }
        i += 1;
    }
    Err("exec: missing -- before the command".into())
}

pub fn run(config: &Config, args: &[String]) -> i32 {
    match args.first().map(|s| s.as_str()) {
        Some("--help") | Some("-h") => {
            println!("{USAGE}");
            0
        }
        Some("health") => health(config),
        Some("exec") => {
            let parsed = match parse_exec_args(&args[1..]) {
                Ok(p) => p,
                Err(m) => {
                    eprintln!("commander: {m}\n{USAGE}");
                    return EXIT_USAGE;
                }
            };
            announce(T1);
            match serve(config, parsed.cwd.as_deref(), parsed.timeout_s, &parsed.cmd) {
                ServeOutcome::Served(code) => code,
                ServeOutcome::TimedOut => {
                    eprintln!(
                        "commander: timed out after {}s (remote process terminated)",
                        parsed.timeout_s
                    );
                    EXIT_TIMEOUT
                }
                ServeOutcome::Failed(m) => {
                    eprintln!("commander: {m}");
                    1
                }
            }
        }
        _ => {
            eprintln!("{USAGE}");
            EXIT_USAGE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pid_parses_from_start_reply() {
        assert_eq!(
            parse_pid("Process started with PID 352522 (shell: /bin/bash)\nInitial output:\n"),
            Some(352522)
        );
        assert_eq!(parse_pid("no pid here"), None);
    }

    #[test]
    fn chunk_parses_header_output_and_exit() {
        let text = "[Reading 3 new lines (total: 3 lines)]\n\nprobe-out\nprobe-err\n\
                    \u{2705} Process completed with exit code 3 (runtime: 1.01s)";
        let chunk = parse_read_chunk(text, 0);
        assert_eq!(chunk.total_lines, 3);
        assert_eq!(chunk.exit_code, Some(3));
        assert!(chunk.output.contains("probe-out"));
        assert!(!chunk.output.contains("Process completed"));
    }

    #[test]
    fn chunk_without_header_counts_lines() {
        let chunk = parse_read_chunk("one\ntwo", 5);
        assert_eq!(chunk.total_lines, 7);
        assert_eq!(chunk.exit_code, None);
    }
}
