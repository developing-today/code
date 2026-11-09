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
    let mut content_length: Option<usize> = None;
    let mut chunked = false;
    for line in head.lines().skip(1) {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        if name.eq_ignore_ascii_case("content-length") {
            content_length = value.parse().ok();
        } else if name.eq_ignore_ascii_case("transfer-encoding")
            && value.to_ascii_lowercase().contains("chunked")
        {
            chunked = true;
        }
    }
    let mut body: Vec<u8> = raw[header_end..].to_vec();
    // Fill the body per its framing. `Connection: close` was sent,
    // so EOF is a valid terminator for every shape.
    let complete = |body: &[u8]| -> bool {
        if chunked {
            dechunked_len(body).is_some()
        } else if let Some(n) = content_length {
            body.len() >= n
        } else {
            false
        }
    };
    while !complete(&body) {
        let n = stream
            .read(&mut chunk)
            .map_err(|e| transport_err(format!("pm-dispatch read failed: {e}")))?;
        if n == 0 {
            break;
        }
        body.extend_from_slice(&chunk[..n]);
        if body.len() > 64 * 1024 * 1024 {
            return Err(transport_err("pm-dispatch response body exceeds 64 MiB"));
        }
    }
    let body = if chunked {
        dechunk(&body)
    } else if let Some(n) = content_length {
        body[..n.min(body.len())].to_vec()
    } else {
        body
    };
    Ok((status, body))
}

/// Length of the decoded body if `body` holds at least one full
/// chunked stream, else None (more bytes needed).
fn dechunked_len(body: &[u8]) -> Option<usize> {
    let mut total = 0usize;
    let mut rest = body;
    loop {
        let line_end = find_subslice(rest, b"\r\n")?;
        let size_str = String::from_utf8_lossy(&rest[..line_end]).into_owned();
        let size = usize::from_str_radix(size_str.trim().split(';').next()?.trim(), 16).ok()?;
        rest = rest.get(line_end.checked_add(2)?..)?;
        if size == 0 {
            return Some(total);
        }
        // The chunk size is peer-declared and untrusted: use
        // checked arithmetic so usize::MAX, usize::MAX - 1, or any
        // other huge declared size yields incomplete (None) instead
        // of panicking in debug or wrapping in release into a
        // false-complete body (#6512).
        let need = size.checked_add(2)?;
        if rest.len() < need {
            return None;
        }
        total = total.checked_add(size)?;
        rest = &rest[need..];
    }
}

fn dechunk(body: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut rest = body;
    while let Some(line_end) = find_subslice(rest, b"\r\n") {
        let size_str = String::from_utf8_lossy(&rest[..line_end]).into_owned();
        let Ok(size) = usize::from_str_radix(size_str.trim().split(';').next().unwrap_or(""), 16)
        else {
            break;
        };
        rest = &rest[line_end + 2..];
        if size == 0 || rest.len() < size {
            break;
        }
        out.extend_from_slice(&rest[..size]);
        rest = &rest[size..];
        if rest.starts_with(b"\r\n") {
            rest = &rest[2..];
        }
    }
    out
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
            let err = parse_args(&args, &["repo"], &[]).err().expect("duplicate must error");
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

    /// Checked-arithmetic pins for #6512 (replaces the probe
    /// characterization test from archive PR #7524, which pinned
    /// the overflow-panic): peer-declared chunk sizes must never
    /// panic, wrap, or yield a false-complete body. Impossible
    /// sizes read as incomplete (None); valid decoding is
    /// preserved.
    #[test]
    fn dechunked_len_rejects_untrusted_chunk_sizes() {
        // controls: valid decoding is preserved, including a
        // chunk extension on the size line
        assert_eq!(
            dechunked_len(b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n"),
            Some(9)
        );
        assert_eq!(dechunked_len(b"4;ext=1\r\nWiki\r\n0\r\n\r\n"), Some(4));
        assert_eq!(dechunked_len(b"4\r\nWi"), None);
        // dechunk() itself guards the declared size before slicing:
        assert_eq!(dechunk(b"ffffffffffffffff\r\nXX"), Vec::<u8>::new());
        // usize::MAX and usize::MAX - 1 declared sizes: incomplete,
        // never completion and never a panic (this test runs in
        // debug, where the old `size + 2` panicked)
        assert_eq!(dechunked_len(b"ffffffffffffffff\r\nXX"), None);
        assert_eq!(dechunked_len(b"fffffffffffffffe\r\nXX"), None);
        // a large declared chunk with only a couple of body bytes
        // present is incomplete, not complete
        assert_eq!(dechunked_len(b"1000000\r\nXX"), None);
        // a huge declared size is still incomplete even when the
        // header line itself is well-formed with extensions
        assert_eq!(dechunked_len(b"ffffffffffffffff;ext=1\r\nXX"), None);
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
