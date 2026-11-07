//! The client half: `fleet proxy connect` and `fleet init`.
//!
//! Both verbs ensure the broker (spawning `fleet proxy serve`
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

use std::io::Write;
use std::os::unix::net::UnixStream;
use std::time::Duration;

use fleet_config::Config;
use fleet_proxy_proto::{
    Hello, HelloResponse, Request, Response, hello_frame, is_proto_too_old, read_frame,
    unwrap_hello, write_frame,
};
use serde::Deserialize;

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
}

impl std::fmt::Display for ClientError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ClientError::Transport(why) => write!(f, "{why}"),
            ClientError::ProtoTooOld => {
                write!(f, "the broker's protocol is older than this build accepts")
            }
            ClientError::Usage(why) => write!(f, "{why}"),
        }
    }
}

impl ClientError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ClientError::Transport(_) => EXIT_TRANSPORT,
            ClientError::ProtoTooOld => EXIT_UNAVAILABLE,
            ClientError::Usage(_) => EXIT_USAGE,
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
/// on failure spawn `fleet proxy serve` detached and wait for it
/// (the Python client's auto-start, fleet-shaped). The hello is
/// spoken on the returned stream, so callers own the negotiation
/// result either way.
pub fn ensure_broker(
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<Ensured, ClientError> {
    let socket = config
        .broker_socket()
        .map_err(|e| ClientError::Transport(e.to_string()))?;
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

/// Spawn `fleet proxy serve` detached (setsid, output appended to
/// broker.log beside the socket — the Python auto-start's shape).
fn spawn_serve(config: &Config) -> Result<(), ClientError> {
    use std::os::unix::process::CommandExt;
    let socket = config
        .broker_socket()
        .map_err(|e| ClientError::Transport(e.to_string()))?;
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
    cmd.args(["proxy", "serve"])
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
    let shims_dir = Config::shims_dir().map_err(|e| ClientError::Transport(e.to_string()))?;
    shims::sync_shims(&shims_dir, services, &exe)
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
and optionally issue one request over the negotiated stream.";

impl ConnectArgs {
    pub fn parse(argv: &[String]) -> Result<ConnectArgs, ClientError> {
        let mut out = ConnectArgs::default();
        let mut i = 0;
        let usage = |what: &str| ClientError::Usage(format!("proxy connect: {what}"));
        while i < argv.len() {
            match argv[i].as_str() {
                "--print-path" => out.print_path = true,
                "--stdio" => out.stdio = true,
                "--host" => {
                    i += 1;
                    out.host = Some(
                        argv.get(i)
                            .ok_or_else(|| usage("--host needs a value"))?
                            .clone(),
                    );
                }
                "--port" => {
                    i += 1;
                    out.port = Some(
                        argv.get(i)
                            .ok_or_else(|| usage("--port needs a value"))?
                            .parse()
                            .map_err(|_| usage("--port must be a number"))?,
                    );
                }
                "--request" => {
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
        Ok(out)
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
    if args.stdio {
        let host = args
            .host
            .clone()
            .unwrap_or_else(|| config.target_host().to_string());
        let port = args.port.unwrap_or(22);
        return crate::stdio::relay_stdio(config, &host, port);
    }
    let mut ensured = ensure_broker(config, client, capabilities)?;
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
        let shims_dir = Config::shims_dir().map_err(|e| ClientError::Transport(e.to_string()))?;
        println!("{}", shims_dir.display());
        return Ok(0);
    }
    if let Some(req) = &args.request {
        let request = Request {
            method: req.method.clone(),
            path: req.path.clone(),
            body: req.body.clone(),
            port: req.service_port,
            token_file: req.token_file.clone(),
        };
        // A legacy broker already spent its first reply eating the
        // hello; the round-trip proceeds identically from here.
        let resp = roundtrip_on(&mut ensured.stream, &request)?;
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
    let socket = config
        .broker_socket()
        .map_err(|e| ClientError::Transport(e.to_string()))?;
    println!("  socket: {}", socket.display());
    println!(
        "  services: {}",
        services
            .iter()
            .map(|(port, s)| format!("{port}={}", s.name))
            .collect::<Vec<_>>()
            .join(", ")
    );
    let shims_dir = Config::shims_dir().map_err(|e| ClientError::Transport(e.to_string()))?;
    println!(
        "  shims: {}{}",
        shims_dir.display(),
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

Autostart the session proxy if it is not running: probe the broker
socket, sync the PATH shims when a broker answers, and start one
when none does. A strict no-op when a broker is already running.

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
    let socket = config
        .broker_socket()
        .map_err(|e| ClientError::Transport(e.to_string()))?;
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
    let shims_dir = Config::shims_dir().map_err(|e| ClientError::Transport(e.to_string()))?;
    println!(
        "fleet: proxy started; shims at {} [{}]",
        shims_dir.display(),
        sync.tools.join(", ")
    );
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
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
}
