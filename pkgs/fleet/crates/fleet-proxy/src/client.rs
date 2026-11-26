//! The client half: `fleet proxy connect` and `fleet init`.
//!
//! Both verbs ensure the broker (spawning `fleet serve proxy`
//! detached when nothing answers at the socket), negotiate the
//! hello, and regenerate the PATH shim dir from the negotiated
//! services map. Talking to a pre-hello (legacy, proto 0) broker
//! — the Python one still deployed — works identically from the
//! user's side: the shims come from the built-in table the Python
//! brokers serve today, and the fact is said out loud.
//!
//! `fleet init` is the autostart door (§8): the strict no-op when
//! a broker already answers is the entire point — init lines run
//! on every shell/tool start and must never disturb a live broker.
//!
//! A `--request` call rides the mux stream family when the
//! session negotiated it (#5756 §7 step 3, [`mux_roundtrip`]):
//! the client offers `proxy-mux` in its hello, and only a broker
//! that advertises it back switches the session to streams.
//! Every other session — legacy brokers included — keeps the
//! pooled frame round-trip, byte-for-byte.

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::time::Duration;

use fleet_config::{Config, ConfigError, InitMode, ResolvedHost};
use fleet_proxy_proto::mux::{CAP_PROXY_MUX, OpenIntent, RST_PROTOCOL_ERROR, RST_TIMEOUT};
use fleet_proxy_proto::{
    Hello, HelloResponse, Request, Response, hello_frame, is_proto_too_old, read_frame,
    unwrap_hello, write_frame,
};
use serde::Deserialize;

use crate::mux::{MuxConfig, MuxSession, StreamError, StreamHandler};
use crate::shims;
use crate::{EXIT_TRANSPORT, EXIT_UNAVAILABLE, EXIT_USAGE};

/// What a hello exchange produced.
pub enum Negotiated {
    /// A fleet broker answered with its hello-response.
    Fleet(Box<HelloResponse>),
    /// A legacy broker answered: it consumed the hello frame as a
    /// (bad) request and replied with a response frame — proto 0,
    /// the Python dialect. `first_reply` is that response.
    Legacy(Box<Response>),
}

#[derive(Debug)]
pub enum ClientError {
    /// Nothing usable at the other end (connect/spawn/protocol
    /// transport failures) — exit 3.
    Transport(String),
    /// The broker's protocol is outside our window — exit 69.
    ProtoTooOld,
    /// Bad invocation — exit 2.
    Usage(String),
    /// The fleet config (file or the `FLEET_HOST_CONFIG` manifest)
    /// does not parse or fails validation — exit 2, the design's
    /// loud-malformed rule (#5687 §2). Carries the message in the
    /// `ConfigError` shape, minus its own `fleet:` prefix (the
    /// caller adds one).
    Config(String),
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Transport(why) => write!(f, "{why}"),
            ClientError::ProtoTooOld => {
                write!(f, "the broker's protocol is older than this build accepts")
            }
            ClientError::Usage(why) => write!(f, "{why}"),
            ClientError::Config(why) => write!(f, "{why}"),
        }
    }
}

impl ClientError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ClientError::Transport(_) => EXIT_TRANSPORT,
            ClientError::ProtoTooOld => EXIT_UNAVAILABLE,
            ClientError::Usage(_) => EXIT_USAGE,
            ClientError::Config(_) => EXIT_USAGE,
        }
    }
}

fn say_hello(
    stream: &mut UnixStream,
    client: &str,
    capabilities: &[String],
) -> Result<Negotiated, ClientError> {
    let hello = Hello {
        proto: fleet_proxy_proto::PROTO_MAJOR,
        client: client.to_string(),
        capabilities: capabilities.to_vec(),
    };
    write_frame(stream, &hello_frame(&hello))
        .map_err(|e| ClientError::Transport(format!("hello write: {e}")))?;
    let reply =
        read_frame(stream).map_err(|e| ClientError::Transport(format!("hello reply: {e}")))?;
    if is_proto_too_old(&reply) {
        return Err(ClientError::ProtoTooOld);
    }
    if reply.get("hello").is_some() {
        let resp: HelloResponse = unwrap_hello(&reply)
            .map_err(|e| ClientError::Transport(format!("hello reply: {e}")))?;
        return Ok(Negotiated::Fleet(Box::new(resp)));
    }
    // No hello envelope: a legacy broker. The frame is its response
    // to having eaten our hello as a (bad) request — proto 0.
    let resp: Response = serde_json::from_value(reply)
        .map_err(|e| ClientError::Transport(format!("unreadable reply: {e}")))?;
    Ok(Negotiated::Legacy(Box::new(resp)))
}

/// One request over an open legacy-mode stream (the caller already
/// consumed the hello-eating reply if there was one).
pub fn roundtrip_on(stream: &mut UnixStream, req: &Request) -> Result<Response, ClientError> {
    write_frame(stream, &serde_json::to_value(req).unwrap())
        .map_err(|e| ClientError::Transport(format!("request write: {e}")))?;
    let frame =
        read_frame(stream).map_err(|e| ClientError::Transport(format!("response read: {e}")))?;
    serde_json::from_value(frame)
        .map_err(|e| ClientError::Transport(format!("unreadable response: {e}")))
}

/// Whether a negotiated session speaks the mux stream family
/// for its requests (record §5): the client offered
/// [`CAP_PROXY_MUX`] and a fleet broker advertised it back.
/// A legacy broker (proto 0) never does — capability absence
/// is the fallback selection, and the pooled frame path stays
/// in charge of those sessions, byte-for-byte.
pub fn mux_selected(client_capabilities: &[String], negotiated: &Negotiated) -> bool {
    match negotiated {
        Negotiated::Fleet(resp) => {
            fleet_proxy_proto::mux::mux_negotiated(client_capabilities, &resp.capabilities)
        }
        Negotiated::Legacy(_) => false,
    }
}

/// One request over a mux session on an already-negotiated
/// stream (#5756 §7 step 3): the client half of the stream
/// family step 2 terminated. The session is single-use here —
/// `fleet proxy connect` ensures a fresh broker session per
/// invocation — so the reader thread runs for exactly this
/// call and the session is closed at its end.
///
/// Failure shape (record §7 step 3): a stream that fails
/// before any response data arrives is retried exactly once
/// on a fresh stream; a failure after partial response data
/// is terminal. The per-stream wait is the session ceiling —
/// its expiry resets the stream with the record's `timeout`
/// reason. A dead tunnel fails the attempt fast (the mux's
/// fail-all), never after a long wait.
pub fn mux_roundtrip(stream: UnixStream, req: &Request) -> Result<Response, ClientError> {
    let port = req.port.unwrap_or(fleet_config::PORT_LAPTOP_AGENT);
    let writer = stream
        .try_clone()
        .map_err(|e| ClientError::Transport(format!("mux writer clone: {e}")))?;
    let hook_stream = stream
        .try_clone()
        .map_err(|e| ClientError::Transport(format!("mux hook clone: {e}")))?;
    let session = MuxSession::new(
        Box::new(writer),
        MuxConfig::default(),
        None,
        Some(Arc::new(move || {
            let _ = hook_stream.shutdown(std::net::Shutdown::Both);
        })),
    );
    let reader_session = session.clone();
    let reader = std::thread::spawn(move || {
        // A client originates streams; it never terminates
        // them. An inbound open violates the session's
        // direction convention — reset it on sight.
        let handler: StreamHandler = Arc::new(|stream, _info| {
            stream.reset(RST_PROTOCOL_ERROR);
        });
        reader_session.run(stream, handler);
    });
    let result = mux_request(&session, req, port);
    session.close();
    let _ = reader.join();
    result
}

/// The request against the session, with the record's single
/// retry: attempt two runs only when attempt one failed
/// before any response data arrived.
fn mux_request(session: &MuxSession, req: &Request, port: u16) -> Result<Response, ClientError> {
    let body = match &req.body {
        Some(body) => Some(
            serde_json::to_vec(body)
                .map_err(|e| ClientError::Transport(format!("request body encode: {e}")))?,
        ),
        None => None,
    };
    let mut attempt = 0;
    loop {
        attempt += 1;
        match mux_attempt(session, req, port, body.as_deref()) {
            Ok(resp) => return Ok(resp),
            Err(failure) => {
                if failure.retryable && attempt == 1 {
                    continue;
                }
                return Err(failure.error);
            }
        }
    }
}

/// Why one mux attempt failed. `retryable` is the record's
/// fresh-stream retry gate: nothing of the response arrived,
/// so the caller saw no answer a retry could duplicate. A
/// send-side failure is also retryable — the terminating
/// side dispatches a request stream only after its `fin`,
/// so a stream that never finished sending was never
/// dispatched.
struct AttemptFailure {
    retryable: bool,
    error: ClientError,
}

fn mux_attempt(
    session: &MuxSession,
    req: &Request,
    port: u16,
    body: Option<&[u8]>,
) -> Result<Response, AttemptFailure> {
    let fail = |retryable: bool, e: StreamError| AttemptFailure {
        retryable,
        error: ClientError::Transport(e.to_string()),
    };
    let intent = OpenIntent::Request {
        method: req.method.clone(),
        path: req.path.clone(),
    };
    // The pooled frame's per-request routing fields ride the open
    // (#15552): without them a mux request silently lost the
    // caller's token file, transport preference and context.
    let meta = fleet_proxy_proto::mux::RequestMeta {
        token_file: req.token_file.clone(),
        transport: req.transport.clone(),
        cf_ok: req.cf_ok,
        ctx: req.ctx.clone(),
    };
    let stream = session
        .open_stream_with(port, &intent, req.trip.clone(), meta)
        .map_err(|e| fail(true, e))?;
    if let Some(body) = body {
        if !body.is_empty() {
            stream.send_chunk(body).map_err(|e| fail(true, e))?;
        }
    }
    stream.close_send().map_err(|e| fail(true, e))?;
    // Read the response to fin, granting credit back as bytes
    // are consumed — the same assembly as
    // `MuxStream::read_to_fin`, spelled out so the retry gate
    // can see whether any response data arrived before a
    // failure.
    let ceiling = fleet_proxy_proto::mux::StreamCeiling::default().timeout();
    let mut data = Vec::new();
    let mut saw_data = false;
    loop {
        match stream.recv_chunk(ceiling) {
            Ok(Some(bytes)) => {
                saw_data = true;
                let len = bytes.len() as u64;
                data.extend_from_slice(&bytes);
                stream.consumed(len);
                if data.len() > fleet_proxy_proto::MAX_FRAME_BYTES {
                    stream.reset(RST_PROTOCOL_ERROR);
                    return Err(AttemptFailure {
                        retryable: false,
                        error: ClientError::Transport(
                            "mux response exceeds the frame bound".to_string(),
                        ),
                    });
                }
            }
            Ok(None) => break,
            Err(StreamError::Timeout) => {
                stream.reset(RST_TIMEOUT);
                return Err(fail(!saw_data, StreamError::Timeout));
            }
            Err(e) => return Err(fail(!saw_data, e)),
        }
    }
    let status = stream.status().ok_or_else(|| AttemptFailure {
        retryable: false,
        error: ClientError::Transport(
            "mux response carried no status on its first frame".to_string(),
        ),
    })?;
    // Trip-id echo (#15552): a broker that echoes a trip must echo
    // ours. A different id means the reply is not for this call —
    // never hand it to the caller as if it were.
    if let (Some(sent), Some(echoed)) = (req.trip.as_deref(), stream.echoed_trip()) {
        if sent != echoed {
            return Err(AttemptFailure {
                retryable: false,
                error: ClientError::Transport(format!(
                    "mux response echoed trip {echoed:?}, not this call's {sent:?}"
                )),
            });
        }
    }
    let body = if data.is_empty() {
        serde_json::Value::Null
    } else {
        serde_json::from_slice(&data).map_err(|e| AttemptFailure {
            retryable: false,
            error: ClientError::Transport(format!("mux response is not JSON: {e}")),
        })?
    };
    Ok(Response {
        status,
        body,
        transport: stream.transport(),
    })
}

/// The built-in services table in hello-map shape — what a legacy
/// broker serves, used for shims when it answers.
fn built_in_services(
    config: &Config,
) -> std::collections::BTreeMap<String, fleet_proxy_proto::ServiceInfo> {
    config
        .services()
        .into_iter()
        .map(|s| {
            (
                s.port.to_string(),
                fleet_proxy_proto::ServiceInfo {
                    name: s.name,
                    version: s.version.unwrap_or_default(),
                },
            )
        })
        .collect()
}

pub struct Ensured {
    pub stream: UnixStream,
    pub negotiated: Negotiated,
    pub spawned: bool,
}

/// Ensure a broker answers at the configured socket: connect, and
/// on failure spawn `fleet serve proxy` detached and wait for it
/// (the Python client's auto-start, fleet-shaped). The hello is
/// spoken on the returned stream, so callers own the negotiation
/// result either way.
pub fn ensure_broker(
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<Ensured, ClientError> {
    let socket = config.broker_socket();
    if let Ok(mut stream) = UnixStream::connect(&socket) {
        let negotiated = say_hello(&mut stream, client, capabilities)?;
        return Ok(Ensured {
            stream,
            negotiated,
            spawned: false,
        });
    }
    spawn_serve(config)?;
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if let Ok(mut stream) = UnixStream::connect(&socket) {
            match say_hello(&mut stream, client, capabilities) {
                Ok(negotiated) => {
                    return Ok(Ensured {
                        stream,
                        negotiated,
                        spawned: true,
                    });
                }
                Err(ClientError::ProtoTooOld) => return Err(ClientError::ProtoTooOld),
                Err(_) => continue,
            }
        }
    }
    Err(ClientError::Transport(format!(
        "broker did not come up at {}",
        socket.display()
    )))
}

/// Spawn the proxy serve role detached (setsid, output appended
/// to broker.log beside the socket — the Python auto-start's
/// shape). The argv is the post-rename spelling, `fleet serve
/// proxy` (design #5687 §5); `fleet proxy serve` remains as the
/// same leaf under the proxy domain.
fn spawn_serve(config: &Config) -> Result<(), ClientError> {
    use std::os::unix::process::CommandExt;
    let socket = config.broker_socket();
    let sock_dir = socket
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&sock_dir)
        .map_err(|e| ClientError::Transport(format!("socket dir: {e}")))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(sock_dir.join("broker.log"))
        .map_err(|e| ClientError::Transport(format!("broker.log: {e}")))?;
    let exe =
        std::env::current_exe().map_err(|e| ClientError::Transport(format!("current exe: {e}")))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["serve", "proxy"])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log.try_clone().map_err(|e| {
            ClientError::Transport(format!("broker.log: {e}"))
        })?))
        .stderr(std::process::Stdio::from(log));
    unsafe {
        cmd.pre_exec(|| {
            // Detach from the caller's session, exactly like the
            // Python client's start_new_session=True.
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn()
        .map_err(|e| ClientError::Transport(format!("spawn broker: {e}")))?;
    Ok(())
}

fn sync_shims_from(
    services: &std::collections::BTreeMap<String, fleet_proxy_proto::ServiceInfo>,
) -> Result<shims::ShimSync, ClientError> {
    let exe = shims::current_fleet_exe()
        .map_err(|e| ClientError::Transport(format!("current exe: {e}")))?;
    shims::sync_shims(&Config::shims_dir(), services, &exe)
        .map_err(|e| ClientError::Transport(format!("shim sync: {e}")))
}

fn print_warnings(sync: &shims::ShimSync) {
    for w in &sync.warnings {
        eprintln!("fleet: warning: {w}");
    }
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ConnectArgs {
    pub print_path: bool,
    pub stdio: bool,
    pub host: Option<String>,
    pub port: Option<u16>,
    pub request: Option<RequestArgs>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RequestArgs {
    pub method: String,
    pub path: String,
    pub service_port: Option<u16>,
    pub token_file: Option<String>,
    pub body: Option<serde_json::Value>,
}

/// The `fleet proxy connect` usage text, shared by the parser's
/// help arm and the bin's help intercept, which prints it to
/// stdout at exit 0 (#5619).
pub const CONNECT_USAGE: &str = "\
usage: fleet proxy connect [--print-path] [--stdio [--host H] \
[--port N]] [--request METHOD PATH [--service-port N] \
[--token-file F] [--body JSON]]

Connect to the session proxy: ensure a broker answers (starting
one when none does), sync the PATH shims from its services map,
and optionally issue one request over the negotiated stream.

Each flag may be given at most once. --stdio and --print-path are
mutually exclusive with --request, and --host/--port require
--stdio.";

impl ConnectArgs {
    pub fn parse(argv: &[String]) -> Result<ConnectArgs, ClientError> {
        let mut out = ConnectArgs::default();
        let mut i = 0;
        let usage = |what: &str| ClientError::Usage(format!("proxy connect: {what}"));
        let repeated = |flag: &str| usage(&format!("{flag} given more than once"));
        while i < argv.len() {
            match argv[i].as_str() {
                "--print-path" => out.print_path = true,
                "--stdio" => out.stdio = true,
                "--host" => {
                    if out.host.is_some() {
                        return Err(repeated("--host"));
                    }
                    i += 1;
                    out.host = Some(
                        argv.get(i)
                            .ok_or_else(|| usage("--host needs a value"))?
                            .clone(),
                    );
                }
                "--port" => {
                    if out.port.is_some() {
                        return Err(repeated("--port"));
                    }
                    i += 1;
                    out.port = Some(
                        argv.get(i)
                            .ok_or_else(|| usage("--port needs a value"))?
                            .parse()
                            .map_err(|_| usage("--port must be a number"))?,
                    );
                }
                "--request" => {
                    if out.request.is_some() {
                        return Err(repeated("--request"));
                    }
                    i += 1;
                    let method = argv
                        .get(i)
                        .ok_or_else(|| usage("--request needs METHOD PATH"))?
                        .clone();
                    i += 1;
                    let path = argv
                        .get(i)
                        .ok_or_else(|| usage("--request needs METHOD PATH"))?
                        .clone();
                    out.request = Some(RequestArgs {
                        method,
                        path,
                        service_port: None,
                        token_file: None,
                        body: None,
                    });
                }
                "--service-port" => {
                    if out
                        .request
                        .as_ref()
                        .is_some_and(|r| r.service_port.is_some())
                    {
                        return Err(repeated("--service-port"));
                    }
                    i += 1;
                    let value = argv
                        .get(i)
                        .ok_or_else(|| usage("--service-port needs a value"))?
                        .parse()
                        .map_err(|_| usage("--service-port must be a number"))?;
                    out.request
                        .as_mut()
                        .ok_or_else(|| usage("--service-port must follow --request"))?
                        .service_port = Some(value);
                }
                "--token-file" => {
                    if out.request.as_ref().is_some_and(|r| r.token_file.is_some()) {
                        return Err(repeated("--token-file"));
                    }
                    i += 1;
                    let value = argv
                        .get(i)
                        .ok_or_else(|| usage("--token-file needs a value"))?
                        .clone();
                    out.request
                        .as_mut()
                        .ok_or_else(|| usage("--token-file must follow --request"))?
                        .token_file = Some(value);
                }
                "--body" => {
                    if out.request.as_ref().is_some_and(|r| r.body.is_some()) {
                        return Err(repeated("--body"));
                    }
                    i += 1;
                    let value = argv
                        .get(i)
                        .ok_or_else(|| usage("--body needs a JSON value"))?;
                    let body: serde_json::Value = serde_json::from_str(value)
                        .map_err(|e| usage(&format!("--body is not JSON: {e}")))?;
                    out.request
                        .as_mut()
                        .ok_or_else(|| usage("--body must follow --request"))?
                        .body = Some(body);
                }
                "--help" | "-h" => {
                    return Err(ClientError::Usage(CONNECT_USAGE.to_string()));
                }
                other => return Err(usage(&format!("unknown argument {other:?}"))),
            }
            i += 1;
        }
        out.validate()?;
        Ok(out)
    }

    /// Reject flag combinations that would silently drop an
    /// argument: `run_connect` returns early for `--stdio` and
    /// `--print-path` (never sending a `--request`), and
    /// `--host`/`--port` only feed the stdio relay (#5929).
    pub fn validate(&self) -> Result<(), ClientError> {
        let usage = |what: &str| ClientError::Usage(format!("proxy connect: {what}"));
        if self.request.is_some() {
            if self.stdio {
                return Err(usage("--stdio cannot be combined with --request"));
            }
            if self.print_path {
                return Err(usage("--print-path cannot be combined with --request"));
            }
        }
        if !self.stdio {
            if self.host.is_some() {
                return Err(usage("--host requires --stdio"));
            }
            if self.port.is_some() {
                return Err(usage("--port requires --stdio"));
            }
        }
        Ok(())
    }
}

/// `fleet proxy connect` — see the module docs. Returns the exit
/// code; errors are printed by the caller via `err_line`.
pub fn run_connect(
    args: &ConnectArgs,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    args.validate()?;
    if args.stdio {
        let host = args
            .host
            .clone()
            .unwrap_or_else(|| config.target_host().to_string());
        let port = args.port.unwrap_or(22);
        return crate::stdio::relay_stdio(config, &host, port);
    }
    // The client half of the mux landed with this code
    // (#5756 §7 step 3): the client offers `proxy-mux` in its
    // hello from here. The offer is not activation — a session
    // speaks streams only when the broker advertises the
    // capability back (see `mux_selected`), and every other
    // session keeps the pooled frame path.
    let mut offered = capabilities.to_vec();
    if !offered.iter().any(|c| c == CAP_PROXY_MUX) {
        offered.push(CAP_PROXY_MUX.to_string());
    }
    let mut ensured = ensure_broker(config, client, &offered)?;
    let (services, broker_desc) = match &ensured.negotiated {
        Negotiated::Fleet(resp) => (
            resp.services.clone(),
            format!("fleet broker, proto {}", resp.proto),
        ),
        Negotiated::Legacy(_) => (
            built_in_services(config),
            "legacy broker (proto 0, pre-hello)".to_string(),
        ),
    };
    let sync = sync_shims_from(&services)?;
    print_warnings(&sync);
    if args.print_path {
        println!("{}", Config::shims_dir().display());
        return Ok(0);
    }
    if let Some(req) = &args.request {
        let request = Request {
            method: req.method.clone(),
            path: req.path.clone(),
            body: req.body.clone(),
            port: req.service_port,
            token_file: req.token_file.clone(),
            trip: None,
            transport: None,
            cf_ok: None,
            // #14188: the caller's FLEET_CTX_* record rides the frame.
            ctx: fleet_config::ctx::Ctx::from_env().to_frame(),
        };
        let resp = if mux_selected(&offered, &ensured.negotiated) {
            mux_roundtrip(ensured.stream, &request)?
        } else {
            // A legacy broker already spent its first reply eating
            // the hello; the round-trip proceeds identically from
            // here.
            roundtrip_on(&mut ensured.stream, &request)?
        };
        println!(
            "{}",
            serde_json::json!({ "status": resp.status, "body": resp.body })
        );
        return Ok(0);
    }
    println!(
        "fleet proxy: connected ({broker_desc}{})",
        if ensured.spawned {
            ", broker started"
        } else {
            ""
        }
    );
    println!("  socket: {}", config.broker_socket().display());
    println!(
        "  services: {}",
        services
            .iter()
            .map(|(port, s)| format!("{port}={}", s.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    println!(
        "  shims: {}{}",
        Config::shims_dir().display(),
        if sync.tools.is_empty() {
            " (none advertised)".to_string()
        } else {
            format!(" [{}]", sync.tools.join(", "))
        }
    );
    let _ = std::io::stdout().flush();
    Ok(0)
}

/// The `fleet init` usage text (#5619).
pub const INIT_USAGE: &str = "\
Usage: fleet init

Autostart this host's fleet roles if they are not running. The
[host] manifest in fleet config (or FLEET_LOCATION /
FLEET_HOST_CONFIG) names this machine's location and roles: on
the sandbox that is the session proxy (probe the broker socket,
sync the PATH shims when a broker answers, start one when none
does — a strict no-op when a broker is already running) plus the
standing secrets broker (probe its Unix socket, start one bound
to loopback when none answers). An unknown location reports
state, starts nothing, and exits 69.

Takes no arguments.";

/// `fleet init` — autostart semantics (§8): probe first; a live
/// broker (either dialect) means status + shim sync and out,
/// never a spawn, never a restart. Only an unanswered socket
/// spawns, and then init reports what it started.
pub fn run_init(
    argv: &[String],
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    // A help flag asks for the usage: print it and succeed, like
    // the other fleet leaves (#5619) - it is not an illegal
    // argument, and asking must never probe, spawn, or sync shims.
    if argv.iter().any(|a| matches!(a.as_str(), "--help" | "-h")) {
        println!("{INIT_USAGE}");
        return Ok(0);
    }
    if !argv.is_empty() {
        return Err(ClientError::Usage(format!(
            "fleet init takes no arguments (got {:?})",
            argv[0]
        )));
    }
    // Location resolution (design #5687 §4, impl #6770): exactly
    // one init implementation lives here, so `fleet init` and
    // `fleet proxy init` resolve identically (§6 step 4).
    let resolved = config.resolve_host().map_err(|e| match e {
        ConfigError::Parse { path, message } => {
            ClientError::Config(format!("cannot parse config {}: {message}", path.display()))
        }
        ConfigError::Read { path, message } => {
            ClientError::Config(format!("cannot read config {}: {message}", path.display()))
        }
    })?;
    run_init_resolved(resolved, config, client, capabilities)
}

/// `fleet init` after location resolution. Unknown location is
/// the design's core failure rule (#5687 §4): report state, start
/// nothing, exit 69 — never guess-start a role. A resolved
/// location reports its source, applies the manifest's own skips
/// and then the `[domains]` kill switch (which wins over the
/// manifest), and ensures each remaining role per its `init_mode`.
fn run_init_resolved(
    resolved: Option<ResolvedHost>,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    let Some(host) = resolved else {
        print_unknown_location_report();
        return Ok(EXIT_UNAVAILABLE);
    };
    println!(
        "fleet: location {} (source: {})",
        host.location,
        host.source.describe(&Config::file_path())
    );
    for (role, reason) in &host.skipped_roles {
        println!("fleet: role {role}: skipped ({reason})");
    }
    let mut active: Vec<String> = Vec::new();
    for role in &host.roles {
        // The kill switch is keyed by domain: the proxy role is
        // the proxy domain; the daemon roles live under serve.
        let domain = if role == "proxy" { "proxy" } else { "serve" };
        if config.domain_enabled(domain) {
            active.push(role.clone());
        } else {
            println!("fleet: role {role}: skipped (disabled by [domains] on this host)");
        }
    }
    match host.init_mode {
        InitMode::Spawn => {
            let mut proxy_active = false;
            let mut secrets_broker_active = false;
            for role in &active {
                if role == "proxy" {
                    proxy_active = true;
                } else if role == "secrets-broker" {
                    secrets_broker_active = true;
                } else {
                    // v1 spawn semantics exist for the proxy and the
                    // standing sandbox secrets broker (stage-2 broker
                    // resilience) only; any other role in a spawn
                    // manifest is reported, not spawned.
                    println!(
                        "fleet: role {role}: not spawn-managed in this build — reported, skipped"
                    );
                }
            }
            let mut code = 0;
            if proxy_active {
                code = run_init_proxy(config, client, capabilities)?;
            }
            if secrets_broker_active {
                let sb_code = run_init_secrets_broker(config)?;
                if code == 0 {
                    code = sb_code;
                }
            }
            Ok(code)
        }
        InitMode::Systemd => {
            for role in &active {
                ensure_role_systemd(role);
            }
            Ok(0)
        }
    }
}

/// The unknown-location report (#5687 §4): every source consulted
/// and what detection found. Detection informs this report only —
/// it never selects roles (§3).
fn print_unknown_location_report() {
    let path = Config::file_path();
    println!("fleet: location: unknown — starting nothing");
    println!(
        "fleet:   manifest: none (checked $FLEET_HOST_CONFIG, $FLEET_LOCATION, {} [host])",
        path.display()
    );
    println!(
        "fleet:   detected: hostname={}, os={}, systemd={}, tailnet=unknown",
        detect_hostname(),
        detect_os(),
        if std::path::Path::new("/run/systemd/system").is_dir() {
            "present"
        } else {
            "absent"
        }
    );
    println!(
        "fleet: to provision this machine, add a [host] section to {} or set FLEET_LOCATION / FLEET_HOST_CONFIG",
        path.display()
    );
}

fn detect_hostname() -> String {
    if let Ok(h) = std::env::var("HOSTNAME") {
        if !h.trim().is_empty() {
            return h.trim().to_string();
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .and_then(|text| text.lines().next().map(|l| l.trim().to_string()))
        .filter(|h| !h.is_empty())
        .unwrap_or_else(|| "unknown".to_string())
}

fn detect_os() -> String {
    if let Ok(text) = std::fs::read_to_string("/etc/os-release") {
        for key in ["PRETTY_NAME=", "NAME="] {
            for line in text.lines() {
                if let Some(value) = line.strip_prefix(key) {
                    return value.trim_matches('"').to_string();
                }
            }
        }
    }
    std::env::consts::OS.to_string()
}

/// One `systemctl` call: `(success, stdout-or-stderr)`, or `None`
/// when systemctl cannot be run at all.
fn systemctl(args: &[&str]) -> Option<(bool, String)> {
    let out = std::process::Command::new("systemctl")
        .args(args)
        .output()
        .ok()?;
    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if text.trim().is_empty() {
        text = String::from_utf8_lossy(&out.stderr).into_owned();
    }
    Some((out.status.success(), text))
}

/// Ensure one role under `init_mode = "systemd"` (#5687 §5): init
/// never detached-spawns a daemon systemd owns. It reports the
/// unit's state; starting a stopped role means asking systemd,
/// and only for a unit that exists — a missing unit is a report,
/// not an error (the role may land later).
fn ensure_role_systemd(role: &str) {
    let unit = format!("fleet-{role}.service");
    let Some((_, out)) = systemctl(&["--user", "is-active", &unit]) else {
        println!(
            "fleet: role {role}: unit {unit} state unknown (systemctl unavailable) — nothing spawned"
        );
        return;
    };
    let state = out.trim().to_string();
    if state == "active" {
        println!("fleet: role {role}: unit {unit} active — nothing spawned");
        return;
    }
    match systemctl(&["--user", "cat", &unit]) {
        Some((true, _)) => match systemctl(&["--user", "start", &unit]) {
            Some((true, _)) => {
                println!("fleet: role {role}: unit {unit} {state} — started via systemctl --user")
            }
            Some((_, err)) => println!(
                "fleet: role {role}: unit {unit} {state} — start failed: {}",
                err.lines().next().unwrap_or("unknown error")
            ),
            None => println!(
                "fleet: role {role}: unit {unit} state unknown (systemctl unavailable) — nothing spawned"
            ),
        },
        _ => println!("fleet: role {role}: unit {unit} not found — reported, skipped"),
    }
}

/// The proxy role's spawn-mode ensure (the pre-#6770 `fleet init`
/// body, unchanged): probe first; a live broker (either dialect)
/// means status + shim sync and out, never a spawn, never a
/// restart. Only an unanswered socket spawns, and then init
/// reports what it started.
fn run_init_proxy(
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    let socket = config.broker_socket();
    // Probe: connect and say hello. A legacy answer still means
    // "running" — init never fights an incumbent broker.
    if let Ok(mut stream) = UnixStream::connect(&socket) {
        return match say_hello(&mut stream, client, capabilities) {
            Ok(Negotiated::Fleet(resp)) => {
                let sync = sync_shims_from(&resp.services)?;
                print_warnings(&sync);
                println!(
                    "fleet: proxy already running (proto {}); shims {}",
                    resp.proto,
                    if sync.changed { "refreshed" } else { "current" }
                );
                Ok(0)
            }
            Ok(Negotiated::Legacy(_)) => {
                let sync = sync_shims_from(&built_in_services(config))?;
                print_warnings(&sync);
                println!(
                    "fleet: proxy already running (legacy broker, proto 0); shims {}",
                    if sync.changed { "refreshed" } else { "current" }
                );
                Ok(0)
            }
            Err(ClientError::ProtoTooOld) => Err(ClientError::ProtoTooOld),
            Err(e) => Err(e),
        };
    }
    let ensured = ensure_broker(config, client, capabilities)?;
    let services = match &ensured.negotiated {
        Negotiated::Fleet(resp) => resp.services.clone(),
        Negotiated::Legacy(_) => built_in_services(config),
    };
    let sync = sync_shims_from(&services)?;
    print_warnings(&sync);
    println!(
        "fleet: proxy started; shims at {} [{}]",
        Config::shims_dir().display(),
        sync.tools.join(", ")
    );
    Ok(0)
}

/// The secrets-broker role's spawn-mode ensure (stage-2 broker
/// resilience): probe the broker's own Unix socket first — a live
/// owner means status and out, never a spawn, never a restart
/// (the init strict no-op, §8). Only an unanswered socket spawns
/// `fleet serve secrets-broker` detached, bound to loopback: the
/// sandbox broker is a local capability, never a LAN surface.
///
/// The loopback bind is enforced twice: the spawn sets
/// `SECRETS_BROKER_BIND=127.0.0.1:4800` explicitly, and the broker
/// itself forces loopback when `FLEET_LOCATION=sandbox`, so a stale
/// environment can never widen it.
fn run_init_secrets_broker(config: &Config) -> Result<i32, ClientError> {
    let socket = config.secrets_broker_socket();
    if fleet_config::probe_socket(&socket) {
        println!(
            "fleet: secrets-broker already running ({}); nothing spawned",
            socket.display()
        );
        return Ok(0);
    }
    ensure_secrets_broker(config)?;
    println!(
        "fleet: secrets-broker started ({}; loopback only)",
        socket.display()
    );
    Ok(0)
}

/// Wait for the spawned secrets broker's socket to answer, like
/// `ensure_broker`'s post-spawn wait: the spawn returns as soon as
/// the daemon execs, and callers (and the proof) need the socket
/// to be dialable before init reports success.
fn ensure_secrets_broker(config: &Config) -> Result<(), ClientError> {
    spawn_secrets_broker(config)?;
    let socket = config.secrets_broker_socket();
    for _ in 0..50 {
        std::thread::sleep(Duration::from_millis(100));
        if fleet_config::probe_socket(&socket) {
            return Ok(());
        }
    }
    Err(ClientError::Transport(format!(
        "secrets-broker did not come up at {}",
        socket.display()
    )))
}

/// Spawn the secrets-broker serve role detached (setsid, output
/// appended to secrets-broker.log beside the socket — the proxy
/// spawn's shape). The argv is the post-rename spelling, `fleet
/// serve secrets-broker`.
fn spawn_secrets_broker(config: &Config) -> Result<(), ClientError> {
    use std::os::unix::process::CommandExt;
    let socket = config.secrets_broker_socket();
    let sock_dir = socket
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    std::fs::create_dir_all(&sock_dir)
        .map_err(|e| ClientError::Transport(format!("socket dir: {e}")))?;
    let log = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(sock_dir.join("secrets-broker.log"))
        .map_err(|e| ClientError::Transport(format!("secrets-broker.log: {e}")))?;
    let exe =
        std::env::current_exe().map_err(|e| ClientError::Transport(format!("current exe: {e}")))?;
    let mut cmd = std::process::Command::new(exe);
    cmd.args(["serve", "secrets-broker"])
        .env("SECRETS_BROKER_BIND", "127.0.0.1:4800")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log.try_clone().map_err(|e| {
            ClientError::Transport(format!("secrets-broker.log: {e}"))
        })?))
        .stderr(std::process::Stdio::from(log));
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    cmd.spawn()
        .map_err(|e| ClientError::Transport(format!("spawn secrets-broker: {e}")))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn fleet_negotiated(broker_caps: &[&str]) -> Negotiated {
        Negotiated::Fleet(Box::new(HelloResponse {
            proto: fleet_proxy_proto::PROTO_MAJOR,
            services: Default::default(),
            capabilities: strings(broker_caps),
        }))
    }

    #[test]
    fn mux_selected_only_when_both_sides_advertise() {
        // #5756 §5/§7 step 3: the fallback selection. Both
        // advertisements present → streams; either side silent
        // → the pooled frame path stays in charge.
        let offered = strings(&["proxy-connect", CAP_PROXY_MUX]);
        assert!(mux_selected(&offered, &fleet_negotiated(&[CAP_PROXY_MUX])));
        assert!(!mux_selected(&offered, &fleet_negotiated(&["proxy-serve"])));
        assert!(!mux_selected(
            &strings(&["proxy-connect"]),
            &fleet_negotiated(&[CAP_PROXY_MUX])
        ));
    }

    #[test]
    fn mux_never_selected_for_a_legacy_broker() {
        let legacy = Negotiated::Legacy(Box::new(Response {
            status: 200,
            body: serde_json::Value::Null,
            transport: None,
        }));
        assert!(!mux_selected(&strings(&[CAP_PROXY_MUX]), &legacy));
    }

    #[test]
    fn init_help_succeeds_without_touching_the_broker() {
        // #5619: --help/-h print the init usage and return Ok(0)
        // before the takes-no-arguments rejection and before any
        // socket probe, so these calls double as the
        // no-side-effect pins.
        let config = Config::default();
        for flag in ["--help", "-h"] {
            assert_eq!(
                run_init(&strings(&[flag]), &config, "test-client", &[]).ok(),
                Some(0),
                "{flag} must print the init usage and succeed"
            );
        }
        // A non-help argument is still a usage error (exit 2).
        let err = run_init(&strings(&["--bogus"]), &config, "test-client", &[])
            .expect_err("a non-help argument must be rejected");
        assert_eq!(err.exit_code(), EXIT_USAGE);
        assert!(err.to_string().contains("--bogus"));
    }

    #[test]
    fn init_unknown_location_reports_and_starts_nothing() {
        // #6770 / design #5687 §4: no manifest resolves to the
        // report + exit 69, with no socket probe and no spawn —
        // this call returning at all (instead of a broker wait)
        // is half the assertion.
        let config = Config::default();
        let code = run_init_resolved(None, &config, "test-client", &[]).unwrap();
        assert_eq!(code, EXIT_UNAVAILABLE);
    }

    #[test]
    fn init_systemd_roles_are_reported_never_spawned() {
        // The laptop map under systemd: init reports unit state
        // and returns 0 on any machine (unit active, inactive,
        // missing, or no systemctl at all) — and never spawns a
        // detached daemon.
        let host = ResolvedHost {
            location: "laptop".to_string(),
            init_mode: InitMode::Systemd,
            roles: vec!["search".to_string()],
            skipped_roles: vec![],
            source: fleet_config::HostSource::EnvContents,
        };
        let config = Config::default();
        let code = run_init_resolved(Some(host), &config, "test-client", &[]).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn init_spawn_mode_reports_non_proxy_roles_without_a_spawn() {
        let host = ResolvedHost {
            location: "lab-01".to_string(),
            init_mode: InitMode::Spawn,
            roles: vec!["search".to_string()],
            skipped_roles: vec![("proxy".to_string(), "disabled in [host.roles]".to_string())],
            source: fleet_config::HostSource::File,
        };
        let config = Config::default();
        let code = run_init_resolved(Some(host), &config, "test-client", &[]).unwrap();
        assert_eq!(code, 0);
    }

    #[test]
    fn init_domains_gate_wins_over_the_manifest() {
        // [domains] proxy=false + a sandbox spawn manifest: the
        // proxy role is skipped, so init returns 0 without ever
        // probing or spawning a broker (a probe of the default
        // socket would either error or spawn — neither may happen
        // for a gated-off role).
        let mut config = Config::default();
        config.domains.insert("proxy".to_string(), false);
        let host = ResolvedHost {
            location: "sandbox".to_string(),
            init_mode: InitMode::Spawn,
            roles: vec!["proxy".to_string()],
            skipped_roles: vec![],
            source: fleet_config::HostSource::EnvLocation,
        };
        let code = run_init_resolved(Some(host), &config, "test-client", &[]).unwrap();
        assert_eq!(code, 0);
    }

    fn parse_err(list: &[&str]) -> String {
        let err = ConnectArgs::parse(&strings(list)).expect_err("must be rejected");
        assert_eq!(err.exit_code(), EXIT_USAGE, "{list:?}");
        err.to_string()
    }

    #[test]
    fn connect_rejects_repeated_value_flags() {
        // #5929: last-wins used to discard earlier values silently.
        let cases: [(&[&str], &str); 6] = [
            (&["--stdio", "--host", "a", "--host", "b"], "--host"),
            (&["--stdio", "--port", "1", "--port", "2"], "--port"),
            (
                &["--request", "GET", "/a", "--request", "GET", "/b"],
                "--request",
            ),
            (
                &[
                    "--request",
                    "GET",
                    "/a",
                    "--service-port",
                    "1",
                    "--service-port",
                    "2",
                ],
                "--service-port",
            ),
            (
                &[
                    "--request",
                    "GET",
                    "/a",
                    "--token-file",
                    "x",
                    "--token-file",
                    "y",
                ],
                "--token-file",
            ),
            (
                &["--request", "POST", "/a", "--body", "1", "--body", "2"],
                "--body",
            ),
        ];
        for (argv, flag) in cases {
            let msg = parse_err(argv);
            assert!(
                msg.contains(flag) && msg.contains("more than once"),
                "{msg}"
            );
        }
    }

    #[test]
    fn connect_rejects_conflicting_modes() {
        let msg = parse_err(&["--stdio", "--request", "GET", "/a"]);
        assert!(
            msg.contains("--stdio") && msg.contains("--request"),
            "{msg}"
        );
        let msg = parse_err(&["--request", "GET", "/a", "--stdio"]);
        assert!(
            msg.contains("--stdio") && msg.contains("--request"),
            "{msg}"
        );
        let msg = parse_err(&["--print-path", "--request", "GET", "/a"]);
        assert!(
            msg.contains("--print-path") && msg.contains("--request"),
            "{msg}"
        );
    }

    #[test]
    fn connect_rejects_host_and_port_without_stdio() {
        assert!(parse_err(&["--host", "h"]).contains("--host requires --stdio"));
        assert!(parse_err(&["--port", "22"]).contains("--port requires --stdio"));
        assert!(parse_err(&["--print-path", "--host", "h"]).contains("--stdio"));
    }

    #[test]
    fn connect_valid_shapes_still_parse() {
        let a = ConnectArgs::parse(&strings(&[
            "--request",
            "POST",
            "/a",
            "--service-port",
            "8080",
            "--token-file",
            "t",
            "--body",
            "{\"k\":1}",
        ]))
        .unwrap();
        let r = a.request.unwrap();
        assert_eq!((r.method.as_str(), r.path.as_str()), ("POST", "/a"));
        assert_eq!(r.service_port, Some(8080));
        assert_eq!(r.token_file.as_deref(), Some("t"));
        assert!(r.body.is_some());

        let a = ConnectArgs::parse(&strings(&["--stdio", "--host", "h", "--port", "22"])).unwrap();
        assert!(a.stdio && a.host.as_deref() == Some("h") && a.port == Some(22));

        let a = ConnectArgs::parse(&strings(&["--print-path"])).unwrap();
        assert!(a.print_path && !a.stdio && a.request.is_none());
        assert!(ConnectArgs::parse(&[]).is_ok());
    }
}
