//! fleet-dispatch — the dispatch domain of the fleet multicall
//! binary (design note §6, epic developing-today/code#5412;
//! step 4, #5432: the thin client).
//!
//! D6: pm-stack is a working TypeScript/Bun service suite and is
//! **not** rewritten, rehomed, or re-platformed by this epic. This
//! crate is verbs over pm-dispatch's existing HTTP interfaces —
//! and nothing else. The live API (pm-stack/dispatch/src/server.ts,
//! read directly for this step) has exactly three routes:
//!
//!   fleet dispatch status    GET  /health       (unauthenticated)
//!   fleet dispatch launch    POST /v1/sessions  (launch now)
//!   fleet dispatch enqueue   POST /v1/tasks     (enqueue only)
//!
//! Deltas against the design note, recorded on #5432: the note's
//! rendered state dir (`~/.local/state/pm-dispatch/`) does not
//! exist — pm-dispatch keeps no client-readable state files; its
//! state is the repo queue itself, so per-task status reads ride
//! the queue domain (`fleet queue list`), not this one. And of the
//! service's two auth forms, GitHub Actions OIDC is a CI shape a
//! local client cannot mint; the static bearer is the client form.
//!
//! Auth: `Authorization: Bearer <token>`, the token resolved **by
//! name** through fleet-secrets (default name
//! `pm-dispatch.api-token`; `[dispatch] token_name` overrides) —
//! the platform rule: no domain reads a token file or shells out
//! to a vault on its own. The value enters exactly one HTTP header
//! per request and is never printed, logged, or stored here.
//!
//! Transports, fleet-queue's two-shape pattern over dispatch's
//! routes:
//!   1. **direct** — an `http://` base URL from `FLEET_DISPATCH_URL`
//!      or `[dispatch] url`; also the laptop seat's zero-config
//!      path (dispatch binds loopback there; a refused loopback
//!      probe falls through to the tunnel);
//!   2. **tunnel** — a CONNECT through the fleet proxy egress to
//!      the target host's dispatch port (`[dispatch] port`,
//!      default 4879): the sandbox's route. The framed
//!      session-broker request shape forwards only an
//!      `X-Agent-Token` header, which pm-dispatch does not accept,
//!      so the client opens its own tunnel on the same egress —
//!      the construction fleet-proxy already maintains — and
//!      speaks HTTP over it.
//!
//! Exit codes follow the workspace conventions (§3's collision
//! table): 0 ok · 2 usage · 3 transport down / credentials
//! unresolved or rejected · 4 the service answered with an HTTP
//! error status. Success bodies print to stdout as the service
//! sent them (pretty JSON); errors go to stderr.

use std::io::Read;
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

use fleet_config::Config;
use fleet_secrets::{Resolver, SecretValue};
use serde_json::{Map, Value, json};

const EXIT_OK: i32 = 0;
const EXIT_USAGE: i32 = 2;
const EXIT_TRANSPORT: i32 = 3;
const EXIT_HTTP: i32 = 4;

/// The task kinds pm-dispatch accepts for `POST /v1/tasks`
/// (pm-stack/dispatch/src/validate.ts `TASK_KINDS`, read live).
const TASK_KINDS: [&str; 4] = ["triage", "small_fix", "pr_shepherd", "ci_investigation"];

#[derive(Debug)]
enum DispatchError {
    Usage(String),
    Transport(String),
    Http { status: u16, message: String },
}

impl DispatchError {
    fn exit_code(&self) -> i32 {
        match self {
            DispatchError::Usage(_) => EXIT_USAGE,
            DispatchError::Transport(_) => EXIT_TRANSPORT,
            DispatchError::Http { .. } => EXIT_HTTP,
        }
    }
    fn message(&self) -> String {
        match self {
            DispatchError::Usage(m) | DispatchError::Transport(m) => m.clone(),
            DispatchError::Http { status, message } => {
                format!("pm-dispatch answered HTTP {status}: {message}")
            }
        }
    }
}

type Result<T> = std::result::Result<T, DispatchError>;

fn usage(msg: impl Into<String>) -> DispatchError {
    DispatchError::Usage(msg.into())
}

fn transport_err(msg: impl Into<String>) -> DispatchError {
    DispatchError::Transport(msg.into())
}

// ---------------------------------------------------------------------------
// Transport selection + HTTP exchange
// ---------------------------------------------------------------------------

enum Transport {
    /// Plain HTTP to `host:port` (the authority is also the Host
    /// header).
    Direct { authority: String },
    /// CONNECT through the proxy egress to the target host.
    Tunnel { target_host: String, port: u16 },
}

impl Transport {
    fn describe(&self) -> String {
        match self {
            Transport::Direct { authority } => format!("direct http://{authority}"),
            Transport::Tunnel { target_host, port } => {
                format!("tunnel via proxy egress to {target_host}:{port}")
            }
        }
    }
    fn host_header(&self) -> String {
        match self {
            Transport::Direct { authority } => authority.clone(),
            Transport::Tunnel { target_host, port } => format!("{target_host}:{port}"),
        }
    }
}

/// Parse an `http://host[:port]` base URL into its authority. No
/// path prefix: pm-dispatch's routes are rooted, and a smuggled
/// path would silently re-root every request.
fn parse_direct_url(url: &str, default_port: u16) -> Result<String> {
    let rest = url
        .strip_prefix("http://")
        .ok_or_else(|| usage(format!("dispatch URL {url:?} must be http://host[:port]")))?;
    let authority = rest.trim_end_matches('/');
    if authority.is_empty() || authority.contains('/') || authority.contains(char::is_whitespace) {
        return Err(usage(format!(
            "dispatch URL {url:?} must be http://host[:port] with no path"
        )));
    }
    if authority.contains(':') {
        Ok(authority.to_string())
    } else {
        Ok(format!("{authority}:{default_port}"))
    }
}

fn pick_transport(config: &Config) -> Result<Transport> {
    let port = config.dispatch_port();
    if let Some(url) = config.dispatch_url() {
        return Ok(Transport::Direct {
            authority: parse_direct_url(&url, port)?,
        });
    }
    // The laptop seat needs no configuration: pm-dispatch binds
    // loopback there. Probe it (briefly) before paying for a
    // tunnel — on the sandbox nothing answers and the probe is a
    // fast refusal.
    let loopback = ("127.0.0.1", port)
        .to_socket_addrs()
        .ok()
        .and_then(|mut a| a.next());
    if let Some(addr) = loopback {
        if TcpStream::connect_timeout(&addr, Duration::from_millis(750)).is_ok() {
            return Ok(Transport::Direct {
                authority: format!("127.0.0.1:{port}"),
            });
        }
    }
    if config.proxy_direct() {
        return Ok(Transport::Direct {
            authority: format!("{}:{port}", config.target_host()),
        });
    }
    Ok(Transport::Tunnel {
        target_host: config.target_host().to_string(),
        port,
    })
}

fn open_stream(config: &Config, transport: &Transport) -> Result<TcpStream> {
    match transport {
        Transport::Direct { authority } => {
            let addr = authority
                .to_socket_addrs()
                .map_err(|e| {
                    transport_err(format!("cannot resolve dispatch address {authority}: {e}"))
                })?
                .next()
                .ok_or_else(|| {
                    transport_err(format!("cannot resolve dispatch address {authority}"))
                })?;
            let stream =
                TcpStream::connect_timeout(&addr, Duration::from_secs(10)).map_err(|e| {
                    transport_err(format!("cannot connect to pm-dispatch at {authority}: {e}"))
                })?;
            stream.set_read_timeout(Some(Duration::from_secs(300))).ok();
            stream.set_write_timeout(Some(Duration::from_secs(60))).ok();
            Ok(stream)
        }
        Transport::Tunnel { target_host, port } => {
            let egress = fleet_proxy::tunnel::egress_from_env(config.egress_proxy_port())
                .map_err(transport_err)?;
            fleet_proxy::tunnel::connect_tunnel(Some(&egress), target_host, *port).map_err(|e| {
                transport_err(format!(
                    "cannot open tunnel to pm-dispatch at {target_host}:{port}: {e}"
                ))
            })
        }
    }
}

/// How a response body is delimited. Exactly one shape is chosen
/// from the headers, and it is chosen unambiguously: conflicting
/// or malformed framing headers are refused while parsing the
/// head, before any body byte is trusted (#8433).
enum Framing {
    Length(usize),
    Chunked,
    /// No framing headers: the body runs to connection close.
    Eof,
}

/// Read one HTTP/1.1 response: status line, headers, and a body
/// framed by chunked encoding, Content-Length, or connection
/// close — the three shapes a Bun.serve JSON reply can take.
fn read_response(stream: &mut TcpStream) -> Result<(u16, Vec<u8>)> {
    let mut raw: Vec<u8> = Vec::with_capacity(8192);
    let mut chunk = [0u8; 16384];
    let header_end = loop {
        if let Some(pos) = find_subslice(&raw, b"\r\n\r\n") {
            break pos + 4;
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|e| transport_err(format!("pm-dispatch read failed: {e}")))?;
        if n == 0 {
            return Err(transport_err(
                "pm-dispatch closed the connection without a response",
            ));
        }
        raw.extend_from_slice(&chunk[..n]);
        if raw.len() > 1024 * 1024 {
            return Err(transport_err("pm-dispatch response headers exceed 1 MiB"));
        }
    };
    let head = String::from_utf8_lossy(&raw[..header_end]).into_owned();
    let status: u16 = head
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| transport_err("pm-dispatch answered with a malformed status line"))?;
    // Framing is validated where it is read (#6582, #8433): a
    // present-but-invalid Content-Length or Transfer-Encoding is
    // refused, never silently treated as absent — a partial body
    // can be valid JSON, so lenient framing would make truncation
    // invisible to the caller.
    let mut content_length: Option<usize> = None;
    let mut transfer_codings: Vec<String> = Vec::new();
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            // Repeated headers and comma-joined values are the same
            // claim stated several times: every statement must parse
            // and they must all agree.
            for part in value.split(',') {
                let part = part.trim();
                let parsed: usize = part.parse().map_err(|_| {
                    transport_err(format!(
                        "pm-dispatch answered with a malformed Content-Length: {value:?}"
                    ))
                })?;
                match content_length {
                    None => content_length = Some(parsed),
                    Some(prev) if prev == parsed => {}
                    Some(prev) => {
                        return Err(transport_err(format!(
                            "pm-dispatch answered with conflicting Content-Length values: {prev} and {parsed}"
                        )));
                    }
                }
            }
        } else if name.eq_ignore_ascii_case("transfer-encoding") {
            transfer_codings.extend(
                value
                    .split(',')
                    .map(|c| c.trim().to_ascii_lowercase())
                    .filter(|c| !c.is_empty()),
            );
        }
    }
    let framing = match (content_length, transfer_codings.as_slice()) {
        (Some(n), []) => Framing::Length(n),
        (None, []) => Framing::Eof,
        (None, [only]) if only == "chunked" => Framing::Chunked,
        (Some(_), _) => {
            return Err(transport_err(
                "pm-dispatch answered with both Content-Length and Transfer-Encoding",
            ));
        }
        _ => {
            return Err(transport_err(format!(
                "pm-dispatch answered with an unsupported Transfer-Encoding: {}",
                transfer_codings.join(", ")
            )));
        }
    };
    let mut body: Vec<u8> = raw[header_end..].to_vec();
    // Fill the body per its framing. `Connection: close` was sent,
    // so EOF terminates an unframed body; for a declared length or
    // a chunked stream, EOF first is truncation, not completion.
    loop {
        match framing {
            Framing::Length(n) if body.len() >= n => break,
            Framing::Chunked => match parse_chunked(&body) {
                Ok(Some(_)) => break,
                Ok(None) => {}
                Err(e) => return Err(e),
            },
            _ => {}
        }
        let n = stream
            .read(&mut chunk)
            .map_err(|e| transport_err(format!("pm-dispatch read failed: {e}")))?;
        if n == 0 {
            match framing {
                Framing::Eof => break,
                Framing::Length(want) => {
                    return Err(transport_err(format!(
                        "pm-dispatch response truncated: Content-Length {want}, connection closed after {} body bytes",
                        body.len()
                    )));
                }
                Framing::Chunked => {
                    return Err(transport_err(
                        "pm-dispatch response truncated: connection closed inside a chunked body",
                    ));
                }
            }
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > 64 * 1024 * 1024 {
            return Err(transport_err("pm-dispatch response body exceeds 64 MiB"));
        }
    }
    let body = match framing {
        Framing::Chunked => parse_chunked(&body)?.unwrap_or_default(),
        Framing::Length(n) => body[..n].to_vec(),
        Framing::Eof => body,
    };
    Ok((status, body))
}

/// Strictly decode a chunked body. `Ok(Some(decoded))` when `body`
/// holds one complete chunked stream — terminal chunk and the
/// trailer section's closing CRLF included; `Ok(None)` when more
/// bytes could still complete it; `Err` when the bytes present are
/// already malformed (a size line that is not hex, or a chunk not
/// followed by its CRLF terminator). The chunk size is
/// peer-declared and never trusted (#5579): it is checked with
/// `checked_add` before it is used to slice or extend anything, so
/// a declared size near `usize::MAX` reads as "not complete"
/// instead of panicking in debug builds or wrapping in release
/// builds (#6512). The decoded output never exceeds the input, so
/// the caller's body bound covers it.
fn parse_chunked(body: &[u8]) -> Result<Option<Vec<u8>>> {
    let mut out: Vec<u8> = Vec::new();
    let mut rest = body;
    loop {
        let Some(line_end) = find_subslice(rest, b"\r\n") else {
            return Ok(None);
        };
        let size_str = String::from_utf8_lossy(&rest[..line_end]).into_owned();
        let size_field = size_str.trim().split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_field, 16).map_err(|_| {
            transport_err(format!(
                "pm-dispatch answered with a malformed chunk size: {size_field:?}"
            ))
        })?;
        rest = &rest[line_end + 2..];
        if size == 0 {
            // Trailer section: header-like lines ended by an empty
            // line. The stream is complete only once that closing
            // CRLF has arrived; anything less is truncation.
            loop {
                if rest.starts_with(b"\r\n") {
                    return Ok(Some(out));
                }
                let Some(trailer_end) = find_subslice(rest, b"\r\n") else {
                    return Ok(None);
                };
                rest = &rest[trailer_end + 2..];
            }
        }
        let Some(chunk_end) = size.checked_add(2) else {
            return Ok(None);
        };
        if rest.len() < chunk_end {
            return Ok(None);
        }
        if &rest[size..chunk_end] != b"\r\n" {
            return Err(transport_err(
                "pm-dispatch answered with a chunk missing its CRLF terminator",
            ));
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[chunk_end..];
    }
}

/// Length of the decoded body if `body` holds at least one full
/// chunked stream, else None (more bytes needed, or malformed).
fn dechunked_len(body: &[u8]) -> Option<usize> {
    parse_chunked(body).ok().flatten().map(|v| v.len())
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    parse_chunked(body).ok().flatten().unwrap_or_default()
}

fn find_subslice(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

/// One request/response exchange. The bearer token, when given,
/// enters exactly this one header — its only appearance anywhere.
fn exchange(
    config: &Config,
    transport: &Transport,
    method: &str,
    path: &str,
    token: Option<&SecretValue>,
    body: Option<&Value>,
) -> Result<(u16, Value)> {
    let mut stream = open_stream(config, transport)?;
    let payload = body
        .map(serde_json::to_vec)
        .transpose()
        .map_err(|e| transport_err(format!("cannot encode request body: {e}")))?;
    let mut head = format!(
        "{method} {path} HTTP/1.1\r\nHost: {}\r\n",
        transport.host_header()
    );
    if let Some(token) = token {
        head.push_str(&format!("Authorization: Bearer {}\r\n", token.expose()));
    }
    head.push_str("Content-Type: application/json\r\n");
    if let Some(payload) = &payload {
        head.push_str(&format!("Content-Length: {}\r\n", payload.len()));
    }
    head.push_str("Connection: close\r\n\r\n");
    use std::io::Write;
    stream
        .write_all(head.as_bytes())
        .and_then(|_| match &payload {
            Some(p) => stream.write_all(p),
            None => Ok(()),
        })
        .map_err(|e| transport_err(format!("pm-dispatch write failed: {e}")))?;
    let (status, raw) = read_response(&mut stream)?;
    let parsed: Value = if raw.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&raw).map_err(|_| {
            transport_err(format!(
                "pm-dispatch answered HTTP {status} with a non-JSON body"
            ))
        })?
    };
    Ok((status, parsed))
}

/// The shared response tail: 2xx prints the service's JSON; 401 is
/// a credentials failure (exit 3, the repair-the-transport branch);
/// any other error status is exit 4 with the service's own message.
fn finish(verb: &str, result: Result<(u16, Value)>) -> Result<Value> {
    let (status, body) = result?;
    match status {
        200..=299 => Ok(body),
        401 => Err(transport_err(
            "pm-dispatch rejected the bearer token (HTTP 401): the resolved secret \
             does not match the service's DISPATCH_API_TOKEN",
        )),
        _ => {
            let error_text = body.get("error").and_then(|e| e.as_str());
            let mut message = match error_text {
                Some(e) => e.to_string(),
                None => serde_json::to_string(&body).unwrap_or_default(),
            };
            // Error bodies can carry ids the caller needs (a failed
            // launch's 502 names its cancelled task_id) — keep the
            // whole body when it says more than the error string.
            if error_text.is_some() && body.as_object().is_some_and(|m| m.len() > 1) {
                message.push_str(&format!(
                    " [{}]",
                    serde_json::to_string(&body).unwrap_or_default()
                ));
            }
            if message.chars().count() > 600 {
                message = format!("{}…", message.chars().take(600).collect::<String>());
            }
            Err(DispatchError::Http {
                status,
                message: format!("{verb}: {message}"),
            })
        }
    }
}

fn print_json(v: &Value) {
    println!(
        "{}",
        serde_json::to_string_pretty(v).unwrap_or_else(|_| "null".to_string())
    );
}

/// Resolve the bearer token by name through fleet-secrets — the
/// one secrets path (§1). Failure is a transport-class error: the
/// client cannot even present credentials.
fn bearer_token(config: &Config) -> Result<SecretValue> {
    let name = config.dispatch_token_name();
    Resolver::default_chain(config)
        .resolve(name)
        .map_err(|e| transport_err(e.to_string()))
}

// ---------------------------------------------------------------------------
// Verbs
// ---------------------------------------------------------------------------

const STATUS_USAGE: &str = "\
Usage: fleet dispatch status

GET /health on pm-dispatch: is the service up, and which build.
Prints the service's health body plus the transport it arrived by.

Takes no arguments.";

fn verb_status(config: &Config, args: &[String]) -> Result<Value> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{STATUS_USAGE}");
        return Ok(Value::Null);
    }
    if let Some(arg) = args.first() {
        return Err(usage(format!(
            "status takes no arguments, got {arg:?}\n\n{STATUS_USAGE}"
        )));
    }
    let transport_sel = pick_transport(config)?;
    let described = transport_sel.describe();
    let body = finish(
        "status",
        exchange(config, &transport_sel, "GET", "/health", None, None),
    )?;
    // The service's health body, plus the route it arrived by — the
    // two facts a status verb exists to report.
    let mut out: Map<String, Value> = match body {
        Value::Object(m) => m,
        other => {
            let mut m = Map::new();
            m.insert("health".to_string(), other);
            m
        }
    };
    out.insert("transport".to_string(), Value::String(described));
    Ok(Value::Object(out))
}

const LAUNCH_USAGE: &str = "\
Usage: fleet dispatch launch --repo <owner/name> --prompt <text> [options]

Launch an OpenCode session on the laptop NOW via pm-dispatch
(POST /v1/sessions): creates the tracking issue and queue task,
starts the session, and assigns the task to it.

  --repo <owner/name>     required; an allowlisted fleet repo
  --prompt <text>         required (or --prompt-file)
  --prompt-file <path>    read the prompt from a file
  --title <text>          default: the prompt's first line
  --model <provider/id>   model hint for the session
  --directory <path>      must be the repo's laptop primary or inside it
  --priority <p>          integer 10-90, or critical|high|normal|low
  --issue / --no-issue    create the tracking issue (default: issue)
  --wait / --no-wait      wait for the launch to complete (default: wait)";

const ENQUEUE_USAGE: &str = "\
Usage: fleet dispatch enqueue --repo <owner/name> --title <text> --prompt <text> [options]

Enqueue work into the repo queue via pm-dispatch (POST /v1/tasks)
without launching a session.

  --repo <owner/name>     required; an allowlisted fleet repo
  --title <text>          required
  --prompt <text>         required (or --prompt-file)
  --prompt-file <path>    read the prompt from a file
  --kind <kind>           triage|small_fix|pr_shepherd|ci_investigation (default: triage)
  --priority <p>          integer 10-90, or critical|high|normal|low
  --model <provider/id>   stored as the payload's model hint only
  --issue                 create a tracking issue first (default: no issue)
  --issue-number <n>      attach to this existing issue instead
  --dedupe-key <key>      default: canonical key from the issue, else unique";

/// A minimal `--flag value` / `--flag=value` parser, workspace
/// style (the queue client parses the same way).
struct Args {
    values: Map<String, Value>,
    flags: Map<String, Value>,
}

fn parse_args(args: &[String], value_flags: &[&str], bool_flags: &[&str]) -> Result<Args> {
    let mut values = Map::new();
    let mut flags = Map::new();
    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];
        let Some(long) = arg.strip_prefix("--") else {
            return Err(usage(format!("unexpected argument {arg:?}")));
        };
        let (name, inline) = match long.split_once('=') {
            Some((n, v)) => (n, Some(v.to_string())),
            None => (long, None),
        };
        if bool_flags.contains(&name) {
            if inline.is_some() {
                return Err(usage(format!("--{name} takes no value")));
            }
            flags.insert(name.to_string(), Value::Bool(true));
        } else if value_flags.contains(&name) {
            let value = match inline {
                Some(v) => v,
                None => {
                    i += 1;
                    args.get(i)
                        .cloned()
                        .ok_or_else(|| usage(format!("--{name} needs a value")))?
                }
            };
            // A repeated value flag is malformed input, not a
            // last-value-wins override: silently dropping the first
            // value can launch against the wrong repo or prompt
            // (#5713). Name the flag and stop, like every other
            // malformed shape this parser rejects.
            if values
                .insert(name.to_string(), Value::String(value))
                .is_some()
            {
                return Err(usage(format!("--{name} given more than once")));
            }
        } else {
            return Err(usage(format!("unknown option --{name}")));
        }
        i += 1;
    }
    Ok(Args { values, flags })
}

impl Args {
    fn get(&self, name: &str) -> Option<&str> {
        self.values.get(name).and_then(|v| v.as_str())
    }
    fn require(&self, name: &str) -> Result<&str> {
        self.get(name)
            .ok_or_else(|| usage(format!("--{name} is required")))
    }
    fn flag(&self, name: &str) -> bool {
        self.flags
            .get(name)
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
    }
}

fn prompt_of(args: &Args) -> Result<String> {
    // Both prompt sources supplied: preferring the file would
    // silently discard the explicitly typed --prompt text (#5713).
    if args.get("prompt").is_some() && args.get("prompt-file").is_some() {
        return Err(usage(
            "--prompt and --prompt-file are mutually exclusive: supply only one".to_string(),
        ));
    }
    if let Some(path) = args.get("prompt-file") {
        return std::fs::read_to_string(path)
            .map_err(|e| usage(format!("cannot read prompt file {path}: {e}")));
    }
    args.require("prompt").map(str::to_string)
}

/// Priority crosses the wire as a number when it parses as one —
/// the service also accepts decimal strings and the named scale,
/// and validating the scale is its job (validate.ts), not a thin
/// client's.
fn priority_value(raw: &str) -> Value {
    match raw.parse::<i64>() {
        Ok(n) => json!(n),
        Err(_) => Value::String(raw.to_string()),
    }
}

fn verb_launch(config: &Config, args: &[String]) -> Result<Value> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{LAUNCH_USAGE}");
        return Ok(Value::Null);
    }
    let parsed = parse_args(
        args,
        &[
            "repo",
            "prompt",
            "prompt-file",
            "title",
            "model",
            "directory",
            "priority",
        ],
        &["issue", "no-issue", "wait", "no-wait"],
    )?;
    // Contradictory pairs resolve by whichever the user meant, and
    // check order cannot know it: --issue --no-issue and
    // --no-issue --issue are the same ambiguity (#5713). Reject the
    // pair instead of silently letting the negative always win.
    for (pos, neg) in [("issue", "no-issue"), ("wait", "no-wait")] {
        if parsed.flag(pos) && parsed.flag(neg) {
            return Err(usage(format!(
                "--{pos} and --{neg} are contradictory: supply only one"
            )));
        }
    }
    let mut body = Map::new();
    body.insert(
        "repo".into(),
        Value::String(parsed.require("repo")?.to_string()),
    );
    body.insert("prompt".into(), Value::String(prompt_of(&parsed)?));
    if let Some(v) = parsed.get("title") {
        body.insert("title".into(), Value::String(v.to_string()));
    }
    if let Some(v) = parsed.get("model") {
        body.insert("model".into(), Value::String(v.to_string()));
    }
    if let Some(v) = parsed.get("directory") {
        body.insert("directory".into(), Value::String(v.to_string()));
    }
    if let Some(v) = parsed.get("priority") {
        body.insert("priority".into(), priority_value(v));
    }
    if parsed.flag("no-issue") {
        body.insert("issue".into(), Value::Bool(false));
    } else if parsed.flag("issue") {
        body.insert("issue".into(), Value::Bool(true));
    }
    if parsed.flag("no-wait") {
        body.insert("wait".into(), Value::Bool(false));
    } else if parsed.flag("wait") {
        body.insert("wait".into(), Value::Bool(true));
    }
    let token = bearer_token(config)?;
    let transport_sel = pick_transport(config)?;
    finish(
        "launch",
        exchange(
            config,
            &transport_sel,
            "POST",
            "/v1/sessions",
            Some(&token),
            Some(&Value::Object(body)),
        ),
    )
}

fn verb_enqueue(config: &Config, args: &[String]) -> Result<Value> {
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("{ENQUEUE_USAGE}");
        return Ok(Value::Null);
    }
    let parsed = parse_args(
        args,
        &[
            "repo",
            "title",
            "prompt",
            "prompt-file",
            "kind",
            "priority",
            "model",
            "issue-number",
            "dedupe-key",
        ],
        &["issue"],
    )?;
    let mut body = Map::new();
    body.insert(
        "repo".into(),
        Value::String(parsed.require("repo")?.to_string()),
    );
    body.insert(
        "title".into(),
        Value::String(parsed.require("title")?.to_string()),
    );
    body.insert("prompt".into(), Value::String(prompt_of(&parsed)?));
    if let Some(kind) = parsed.get("kind") {
        if !TASK_KINDS.contains(&kind) {
            return Err(usage(format!(
                "unknown task kind {kind:?}: expected one of {}",
                TASK_KINDS.join(", ")
            )));
        }
        body.insert("kind".into(), Value::String(kind.to_string()));
    }
    if let Some(v) = parsed.get("priority") {
        body.insert("priority".into(), priority_value(v));
    }
    if let Some(v) = parsed.get("model") {
        body.insert("model".into(), Value::String(v.to_string()));
    }
    if parsed.flag("issue") {
        body.insert("issue".into(), Value::Bool(true));
    }
    if let Some(v) = parsed.get("issue-number") {
        let n: i64 = v
            .parse()
            .map_err(|_| usage(format!("--issue-number must be an integer, got {v:?}")))?;
        body.insert("issue_number".into(), json!(n));
    }
    if let Some(v) = parsed.get("dedupe-key") {
        body.insert("dedupe_key".into(), Value::String(v.to_string()));
    }
    let token = bearer_token(config)?;
    let transport_sel = pick_transport(config)?;
    finish(
        "enqueue",
        exchange(
            config,
            &transport_sel,
            "POST",
            "/v1/tasks",
            Some(&token),
            Some(&Value::Object(body)),
        ),
    )
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

const USAGE: &str = "\
Usage: fleet dispatch <command> [args...]

Thin client over pm-stack dispatch (D6 — the service is not
rewritten; these are verbs over its existing HTTP interfaces):

  status            GET /health — is pm-dispatch up, which build
  launch [options]  POST /v1/sessions — launch a session now
  enqueue [options] POST /v1/tasks — enqueue work, no session

Per-task status lives in the queue: fleet queue list.
Run 'fleet dispatch launch --help' or 'fleet dispatch enqueue
--help' for a verb's options.";

/// The dispatch domain's entry point (the bin crate routes here
/// once the capability checks have passed).
pub fn run(config: &Config, args: &[String]) -> i32 {
    let verb = args.first().map(|s| s.as_str());
    let rest = if args.is_empty() { &[][..] } else { &args[1..] };
    let result = match verb {
        Some("status") => verb_status(config, rest),
        Some("launch") => verb_launch(config, rest),
        Some("enqueue") => verb_enqueue(config, rest),
        Some(other) => Err(usage(format!(
            "unknown dispatch command {other:?}\n\n{USAGE}"
        ))),
        None => Err(usage(USAGE.to_string())),
    };
    match result {
        Ok(Value::Null) => EXIT_OK,
        Ok(body) => {
            print_json(&body);
            EXIT_OK
        }
        Err(e) => {
            eprintln!("fleet dispatch: {}", e.message());
            e.exit_code()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn direct_url_parsing() {
        assert_eq!(
            parse_direct_url("http://127.0.0.1:4879", 4879).unwrap(),
            "127.0.0.1:4879"
        );
        assert_eq!(
            parse_direct_url("http://localhost", 4879).unwrap(),
            "localhost:4879"
        );
        assert_eq!(
            parse_direct_url("http://127.0.0.1:4879/", 4879).unwrap(),
            "127.0.0.1:4879"
        );
        assert!(parse_direct_url("https://dispatch.example", 4879).is_err());
        assert!(parse_direct_url("http://127.0.0.1:4879/v1", 4879).is_err());
        assert!(parse_direct_url("http://", 4879).is_err());
    }

    #[test]
    fn priority_crosses_as_number_or_string() {
        assert_eq!(priority_value("20"), json!(20));
        assert_eq!(priority_value("high"), json!("high"));
        assert_eq!(priority_value("40.5"), json!("40.5"));
    }

    #[test]
    fn launch_body_carries_only_set_fields() {
        let parsed = parse_args(
            &strings(&[
                "--repo",
                "developing-today/code",
                "--prompt",
                "fix it",
                "--no-wait",
            ]),
            &[
                "repo",
                "prompt",
                "prompt-file",
                "title",
                "model",
                "directory",
                "priority",
            ],
            &["issue", "no-issue", "wait", "no-wait"],
        )
        .unwrap();
        assert_eq!(parsed.require("repo").unwrap(), "developing-today/code");
        assert!(parsed.flag("no-wait"));
        assert!(!parsed.flag("issue"));
        assert!(parse_args(&strings(&["--repo"]), &["repo"], &[]).is_err());
        assert!(parse_args(&strings(&["--bogus", "x"]), &["repo"], &[]).is_err());
    }

    #[test]
    fn enqueue_rejects_unknown_kinds() {
        let parsed = parse_args(&strings(&["--kind", "bogus"]), &["kind"], &[]).unwrap();
        let kind = parsed.get("kind").unwrap();
        assert!(!TASK_KINDS.contains(&kind));
        assert!(TASK_KINDS.contains(&"triage"));
    }

    #[test]
    fn chunked_bodies_dechunk() {
        let wire = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(wire), b"Wikipedia");
        assert_eq!(dechunked_len(wire), Some(9));
        assert_eq!(dechunked_len(b"4\r\nWi"), None);
    }

    #[test]
    fn chunk_missing_terminator_is_malformed() {
        // The chunk data is present but its CRLF is not: the old
        // decoder skipped any two bytes and kept going (#8433).
        let wire = b"4\r\nWikiXX5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(wire), Vec::<u8>::new());
        assert_eq!(dechunked_len(wire), None);
        assert!(parse_chunked(wire).is_err());
        // A stream that ends inside the trailer section is not
        // complete either.
        assert_eq!(dechunked_len(b"4\r\nWiki\r\n0\r\nX-T: 1\r\n"), None);
        assert_eq!(dechunked_len(b"4\r\nWiki\r\n0\r\nX-T: 1\r\n\r\n"), Some(4));
    }

    /// Feed `wire` to `read_response` over a loopback socket: the
    /// writer sends the bytes and closes, which is exactly the EOF
    /// signal the framing rules key on (#8433).
    fn serve_once(wire: &[u8]) -> Result<(u16, Vec<u8>)> {
        use std::io::Write;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let bytes = wire.to_vec();
        let writer = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            sock.write_all(&bytes).unwrap();
        });
        let mut stream = TcpStream::connect(addr).unwrap();
        let result = read_response(&mut stream);
        writer.join().unwrap();
        result
    }

    fn served_err(wire: &[u8]) -> String {
        serve_once(wire).err().unwrap().message()
    }

    #[test]
    fn framed_responses_round_trip() {
        // Valid controls: Content-Length, chunked (with a trailer),
        // repeated agreeing lengths, and an unframed close body.
        assert_eq!(
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\n\r\nhello").unwrap(),
            (200, b"hello".to_vec())
        );
        assert_eq!(
            serve_once(
                b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\npedia\r\n0\r\nX-T: 1\r\n\r\n"
            )
            .unwrap(),
            (200, b"Wikipedia".to_vec())
        );
        assert_eq!(
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 5\r\n\r\nhello")
                .unwrap(),
            (200, b"hello".to_vec())
        );
        assert_eq!(
            serve_once(b"HTTP/1.1 200 OK\r\nContent-Length: 5, 5\r\n\r\nhello").unwrap(),
            (200, b"hello".to_vec())
        );
        assert_eq!(
            serve_once(b"HTTP/1.1 204 No Content\r\n\r\n").unwrap(),
            (204, Vec::new())
        );
    }

    #[test]
    fn invalid_or_conflicting_lengths_are_refused() {
        let msg = served_err(b"HTTP/1.1 200 OK\r\nContent-Length: abc\r\n\r\nhello");
        assert!(msg.contains("malformed Content-Length"), "{msg}");
        let msg =
            served_err(b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nContent-Length: 6\r\n\r\nhello!");
        assert!(msg.contains("conflicting Content-Length"), "{msg}");
        let msg = served_err(
            b"HTTP/1.1 200 OK\r\nContent-Length: 5\r\nTransfer-Encoding: chunked\r\n\r\nhello",
        );
        assert!(msg.contains("both Content-Length"), "{msg}");
        let msg = served_err(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip\r\n\r\nhello");
        assert!(msg.contains("unsupported Transfer-Encoding"), "{msg}");
        let msg = served_err(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: gzip, chunked\r\n\r\nhello");
        assert!(msg.contains("unsupported Transfer-Encoding"), "{msg}");
    }

    #[test]
    fn truncated_bodies_are_refused() {
        // Truncation after valid JSON bytes: the partial body parses
        // as JSON, which is exactly why lenient framing was silent.
        let msg = served_err(b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n{\"ok\":true}");
        assert!(msg.contains("truncated"), "{msg}");
        // Chunked stream cut mid-chunk and mid-size-line.
        let msg = served_err(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWiki\r\n5\r\nped",
        );
        assert!(msg.contains("truncated"), "{msg}");
        let msg = served_err(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r");
        assert!(msg.contains("truncated"), "{msg}");
    }

    #[test]
    fn malformed_chunks_are_refused() {
        let msg = served_err(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\nzz\r\nWiki\r\n0\r\n\r\n",
        );
        assert!(msg.contains("malformed chunk size"), "{msg}");
        let msg = served_err(
            b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n4\r\nWikiXX5\r\npedia\r\n0\r\n\r\n",
        );
        assert!(msg.contains("CRLF terminator"), "{msg}");
    }

    #[test]
    fn status_help_and_rejection_need_no_service() {
        // Help and argument rejection return before any transport is
        // opened (the default config's transport would try the
        // loopback probe / proxy egress), so these calls double as
        // the no-network pins: a regression that dials out first
        // fails or hangs here instead of returning promptly.
        let config = Config::default();
        assert_eq!(
            verb_status(&config, &strings(&["--help"])).unwrap(),
            Value::Null
        );
        assert_eq!(
            verb_status(&config, &strings(&["-h"])).unwrap(),
            Value::Null
        );
        let err = verb_status(&config, &strings(&["--bogus"])).unwrap_err();
        assert_eq!(err.exit_code(), EXIT_USAGE);
        assert!(err.message().contains("--bogus"));
    }

    #[test]
    fn duplicate_value_flags_are_rejected() {
        // Repeated value flags error naming the flag (#5713), in both
        // the spaced and inline forms, before any network call.
        for args in [
            strings(&["--repo", "a", "--repo", "b"]),
            strings(&["--repo=a", "--repo=b"]),
            strings(&["--repo", "a", "--repo=a"]),
        ] {
            let err = parse_args(&args, &["repo"], &[])
                .err()
                .expect("duplicate must error");
            assert_eq!(err.exit_code(), EXIT_USAGE);
            assert!(err.message().contains("--repo"), "{}", err.message());
            assert!(
                err.message().contains("more than once"),
                "{}",
                err.message()
            );
        }
        // The ordinary single-flag control still parses.
        let parsed = parse_args(&strings(&["--repo", "a"]), &["repo"], &[]).unwrap();
        assert_eq!(parsed.require("repo").unwrap(), "a");
    }

    #[test]
    fn contradictory_bool_pairs_are_rejected_in_both_orders() {
        // --issue/--no-issue and --wait/--no-wait together are a usage
        // error naming the pair, whichever order they were typed in
        // (#5713). Rejection happens before any transport is opened,
        // so these calls return promptly against the default config.
        let config = Config::default();
        for extra in [
            strings(&["--issue", "--no-issue"]),
            strings(&["--no-issue", "--issue"]),
            strings(&["--wait", "--no-wait"]),
            strings(&["--no-wait", "--wait"]),
        ] {
            let mut args = strings(&["--repo", "developing-today/code", "--prompt", "fix it"]);
            args.extend(extra);
            let err = verb_launch(&config, &args).unwrap_err();
            assert_eq!(err.exit_code(), EXIT_USAGE);
            assert!(err.message().contains("contradictory"), "{}", err.message());
        }
    }

    #[test]
    fn prompt_and_prompt_file_together_are_rejected() {
        // Both prompt sources: a usage error naming both, never a
        // silent preference for the file (#5713). The prompt-file
        // path does not exist, so an implementation that read it
        // first would fail with a different message.
        let config = Config::default();
        let launch_args = strings(&[
            "--repo",
            "developing-today/code",
            "--prompt",
            "typed text",
            "--prompt-file",
            "/nonexistent/prompt.txt",
        ]);
        let err = verb_launch(&config, &launch_args).unwrap_err();
        assert_eq!(err.exit_code(), EXIT_USAGE);
        assert!(err.message().contains("--prompt"), "{}", err.message());
        assert!(err.message().contains("--prompt-file"), "{}", err.message());
        let enqueue_args = strings(&[
            "--repo",
            "developing-today/code",
            "--title",
            "t",
            "--prompt",
            "typed text",
            "--prompt-file",
            "/nonexistent/prompt.txt",
        ]);
        let err = verb_enqueue(&config, &enqueue_args).unwrap_err();
        assert_eq!(err.exit_code(), EXIT_USAGE);
        assert!(err.message().contains("--prompt-file"), "{}", err.message());
    }

    /// Pins the #6512 fix: `dechunked_len` does checked arithmetic on
    /// the peer-declared chunk size, so the two largest `usize` values
    /// (and any size that cannot fit the buffered remainder) read as
    /// "not complete" — no panic in debug builds, no wrapped verdict
    /// in release builds. (This began as a characterization test
    /// salvaged from archive PR #7524 that pinned the old
    /// overflow-panic; the fix updates it deliberately.)
    #[test]
    fn dechunked_len_huge_chunk_size_is_not_complete() {
        // controls: the existing pins
        assert_eq!(
            dechunked_len(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"),
            Some(9)
        );
        assert_eq!(dechunked_len(b"4\r\nWi"), None);
        // dechunk() itself guards the declared size before slicing:
        assert_eq!(dechunk(b"ffffffffffffffff\r\nXX"), Vec::<u8>::new());
        // the two overflow sizes: not complete, and no panic — these
        // are direct calls, so a panic fails the test outright.
        assert_eq!(dechunked_len(b"ffffffffffffffff\r\nXX"), None);
        assert_eq!(dechunked_len(b"fffffffffffffffe\r\nXX"), None);
        // a huge declared size after a valid first chunk is the same:
        assert_eq!(dechunked_len(b"4\r\nWiki\r\nffffffffffffffff\r\nXX"), None);
        // a large-but-plausible size (16 MiB) with too few bytes
        // buffered is simply not complete yet:
        assert_eq!(dechunked_len(b"1000000\r\nshort"), None);
    }

    #[test]
    fn usage_errors_exit_2_and_transport_3() {
        assert_eq!(usage("x").exit_code(), 2);
        assert_eq!(transport_err("x").exit_code(), 3);
        assert_eq!(
            DispatchError::Http {
                status: 409,
                message: "m".into()
            }
            .exit_code(),
            4
        );
    }
}
