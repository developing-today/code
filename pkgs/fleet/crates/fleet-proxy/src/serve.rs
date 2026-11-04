//! `fleet proxy serve` — the session broker, succeeding
//! laptop-agent-broker.py behavior for behavior:
//!
//! * Unix socket at the historical path; if a live broker already
//!   owns it, exit quietly; if the socket file is stale, unlink
//!   and bind.
//! * Per-port pools of warm CONNECT tunnels (pool size from
//!   `LAPTOP_AGENT_POOL`, default 12); a request takes a pooled
//!   connection, and on any failure retries once on a fresh one.
//! * Token files read per request, never stored, never logged.
//! * A keepalive thread pings the laptop-agent's /health every
//!   180 s and exits the process once client requests have been
//!   idle for `idle_exit_secs` (default 3600) — keepalive traffic
//!   does NOT count as use; a warmth cache that exits when cold
//!   is a feature on a shared sandbox.
//! * Sessions speak the legacy dialect (proto 0: request frames
//!   from the first frame, the Python clients) or negotiate the
//!   hello first (proto 1, fleet clients) — decided per connection
//!   by peeking at the first frame, never by configuration.
//!
//! The broker persists nothing: routing table from config,
//! tunnels in memory, tokens in their files.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use fleet_config::Config;
use fleet_proxy_proto::{
    Hello, HelloResponse, ProtoError, Request, Response, ServiceInfo, is_hello_frame,
    proto_too_old_frame, read_frame, unwrap_hello, write_frame,
};

use crate::http::{HttpConn, HttpError};
use crate::tunnel::{Egress, connect_tunnel, egress_from_env};

/// Keepalive cadence (the Python's KEEPALIVE_S).
const KEEPALIVE_S: u64 = 180;
/// How often the idle check wakes. The Python checks inside its
/// 180 s keepalive loop; the semantics (exit once idle exceeds the
/// configured span) are the same, the granularity is finer so a
/// short configured idle is observable in tests.
const IDLE_TICK_S: u64 = 5;

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn pool_size() -> usize {
    std::env::var("LAPTOP_AGENT_POOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(12)
}

struct Broker {
    config: Config,
    egress: Option<Egress>,
    pools: Mutex<HashMap<u16, Vec<HttpConn>>>,
    pool_size: usize,
    /// Last client-request time (unix secs). Keepalive health
    /// checks deliberately never write this.
    last_used: AtomicU64,
}

impl Broker {
    fn open_conn(&self, port: u16) -> Result<HttpConn, String> {
        let target = self.config.target_host();
        let stream: TcpStream = if self.config.proxy_direct() {
            connect_tunnel(None, target, port)
        } else {
            let egress = self
                .egress
                .as_ref()
                .ok_or_else(|| "HTTPS_PROXY must be set (runtime proxy env)".to_string())?;
            connect_tunnel(Some(egress), target, port)
        }
        .map_err(|e| format!("tunnel to {target}:{port}: {e}"))?;
        HttpConn::new(stream).map_err(|e| format!("tunnel to {target}:{port}: {e}"))
    }

    fn pool_get(&self, port: u16) -> Result<HttpConn, String> {
        if let Some(conn) = self
            .pools
            .lock()
            .unwrap()
            .get_mut(&port)
            .and_then(|pool| pool.pop())
        {
            return Ok(conn);
        }
        self.open_conn(port)
    }

    fn pool_put(&self, port: u16, conn: HttpConn) {
        if !conn.reusable() {
            return; // drops here — a dead connection never re-pools
        }
        let mut pools = self.pools.lock().unwrap();
        let pool = pools.entry(port).or_default();
        if pool.len() < self.pool_size {
            pool.push(conn);
        }
    }

    /// One routed request, Python semantics: pooled connection
    /// first, one retry on a fresh connection on any failure.
    /// `client` distinguishes real client traffic (resets the idle
    /// timer) from the broker's own keepalive (never does).
    fn do_request(&self, req: &Request, client: bool) -> Result<(u16, serde_json::Value), String> {
        if client {
            self.last_used.store(now_secs(), Ordering::Relaxed);
        }
        let port = req.port.unwrap_or(fleet_config::PORT_LAPTOP_AGENT);
        let token_path = match &req.token_file {
            Some(p) => std::path::PathBuf::from(p),
            None => self.config.default_token_file(),
        };
        let token = std::fs::read_to_string(&token_path)
            .map_err(|e| format!("cannot read token file {}: {e}", token_path.display()))?;
        let token = token.trim();
        let payload = match &req.body {
            Some(body) => Some(serde_json::to_vec(body).map_err(|e| e.to_string())?),
            None => None,
        };
        let host_header = format!("{}:{}", self.config.target_host(), port);
        let mut last_err: Option<String> = None;
        for attempt in 0..2 {
            let mut conn = match if attempt == 0 {
                self.pool_get(port)
            } else {
                self.open_conn(port)
            } {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            match conn.request(
                &req.method,
                &req.path,
                &host_header,
                token,
                payload.as_deref(),
            ) {
                Ok((status, data)) => {
                    self.pool_put(port, conn);
                    let body = serde_json::from_slice(&data)
                        .map_err(|e| format!("service on port {port} answered non-JSON: {e}"))?;
                    return Ok((status, body));
                }
                Err(e) => {
                    last_err = Some(match e {
                        HttpError::Stale => "connection closed before any response".to_string(),
                        other => other.to_string(),
                    });
                    // conn drops: never re-pooled after an error.
                }
            }
        }
        Err(last_err.unwrap_or_else(|| "request failed".to_string()))
    }

    fn services_map(&self) -> std::collections::BTreeMap<String, ServiceInfo> {
        self.config
            .services()
            .into_iter()
            .map(|s| {
                (
                    s.port.to_string(),
                    ServiceInfo {
                        name: s.name,
                        version: s.version.unwrap_or_default(),
                    },
                )
            })
            .collect()
    }
}

/// Handle one request frame value; always answers a response frame
/// value (errors ride the Python's `{status: 502, body:{error}}`
/// shape — consumers branch on status, not on text).
fn handle_request(broker: &Broker, frame: &serde_json::Value) -> serde_json::Value {
    let req = match parse_request(frame) {
        Ok(r) => r,
        Err(why) => {
            return serde_json::to_value(Response {
                status: 502,
                body: serde_json::json!({ "error": format!("broker: {why}") }),
            })
            .unwrap();
        }
    };
    match broker.do_request(&req, true) {
        Ok((status, body)) => serde_json::to_value(Response { status, body }).unwrap(),
        Err(why) => serde_json::to_value(Response {
            status: 502,
            body: serde_json::json!({ "error": format!("broker: {why}") }),
        })
        .unwrap(),
    }
}

/// Lenient request parsing, mirroring the Python's dict access:
/// `method`/`path` are required (their absence is the KeyError the
/// Python handler turns into a 502), everything else has the same
/// defaults the Python applies (`port` 4799, body/token_file None).
fn parse_request(frame: &serde_json::Value) -> Result<Request, String> {
    let method = frame
        .get("method")
        .and_then(|v| v.as_str())
        .ok_or("request is missing \"method\"")?
        .to_string();
    let path = frame
        .get("path")
        .and_then(|v| v.as_str())
        .ok_or("request is missing \"path\"")?
        .to_string();
    let body = frame.get("body").filter(|b| !b.is_null()).cloned();
    let port = frame
        .get("port")
        .and_then(|v| v.as_u64())
        .and_then(|p| u16::try_from(p).ok());
    let token_file = frame
        .get("token_file")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(Request {
        method,
        path,
        body,
        port,
        token_file,
    })
}

fn serve_session(broker: &Broker, mut stream: UnixStream) {
    let first = match read_frame(&mut stream) {
        Ok(f) => f,
        Err(_) => return, // probe connects and closes; the Python exits the same way
    };
    if is_hello_frame(&first) {
        let hello: Hello = match unwrap_hello(&first) {
            Ok(h) => h,
            Err(_) => {
                let _ = write_frame(
                    &mut stream,
                    &serde_json::to_value(Response {
                        status: 502,
                        body: serde_json::json!({ "error": "broker: malformed hello" }),
                    })
                    .unwrap(),
                );
                return;
            }
        };
        if !fleet_proxy_proto::broker_accepts(hello.proto) {
            let _ = write_frame(
                &mut stream,
                &proto_too_old_frame(fleet_proxy_proto::PROTO_MAJOR),
            );
            return;
        }
        let resp = HelloResponse {
            proto: fleet_proxy_proto::PROTO_MAJOR,
            services: broker.services_map(),
            capabilities: vec![fleet_registry::Capability::ProxyServe.id().to_string()],
        };
        if write_frame(&mut stream, &fleet_proxy_proto::hello_response_frame(&resp)).is_err() {
            return;
        }
    } else if first.get("method").is_some() {
        // Legacy session (proto 0): the first frame is a request.
        let reply = handle_request(broker, &first);
        if write_frame(&mut stream, &reply).is_err() {
            return;
        }
    } else {
        let _ = write_frame(
            &mut stream,
            &serde_json::to_value(Response {
                status: 502,
                body: serde_json::json!({
                    "error": "broker: first frame is neither hello nor request"
                }),
            })
            .unwrap(),
        );
        return;
    }
    loop {
        match read_frame(&mut stream) {
            Ok(frame) => {
                let reply = handle_request(broker, &frame);
                if write_frame(&mut stream, &reply).is_err() {
                    return;
                }
            }
            Err(ProtoError::Closed) => return,
            Err(_) => return,
        }
    }
}

fn keepalive_loop(broker: Arc<Broker>) {
    let idle = broker.config.idle_exit_secs();
    let health_req = Request {
        method: "GET".to_string(),
        path: "/health".to_string(),
        body: None,
        port: Some(fleet_config::PORT_LAPTOP_AGENT),
        token_file: None,
    };
    let mut last_health = now_secs();
    loop {
        std::thread::sleep(Duration::from_secs(IDLE_TICK_S));
        let now = now_secs();
        if now.saturating_sub(broker.last_used.load(Ordering::Relaxed)) > idle {
            // The Python exits via os._exit from its keepalive
            // loop; the same abruptness is fine — the broker holds
            // nothing that needs draining.
            std::process::exit(0);
        }
        if now.saturating_sub(last_health) >= KEEPALIVE_S {
            last_health = now;
            let _ = broker.do_request(&health_req, false);
        }
    }
}

/// Run the broker in the foreground. Returns the process exit code.
///
/// Startup mirrors the Python exactly: if the socket file exists
/// and a connect succeeds, a live broker owns it — exit 0 quietly.
/// If the connect fails, the file is stale — unlink and bind.
pub fn run(config: &Config) -> i32 {
    let socket_path = config.broker_socket();
    if let Some(parent) = socket_path.parent() {
        let _ = std::fs::create_dir_all(parent);
        // The Python chmods its socket dir 0700 (best effort).
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
    }
    if socket_path.exists() {
        match UnixStream::connect(&socket_path) {
            Ok(_) => return 0, // a live broker already owns the socket
            Err(_) => {
                let _ = std::fs::remove_file(&socket_path);
            }
        }
    }
    let listener = match UnixListener::bind(&socket_path) {
        Ok(l) => l,
        Err(e) => {
            eprintln!(
                "fleet proxy serve: cannot bind {}: {e}",
                socket_path.display()
            );
            return crate::EXIT_TRANSPORT;
        }
    };
    let egress = if config.proxy_direct() {
        None
    } else {
        match egress_from_env(config.egress_proxy_port()) {
            Ok(e) => Some(e),
            Err(why) => {
                // Like the Python broker, a missing proxy env is
                // not fatal at startup: tunnels fail per-request
                // with the same message until the env appears.
                eprintln!("fleet proxy serve: {why} (tunnels will fail until it is set)");
                None
            }
        }
    };
    let broker = Arc::new(Broker {
        config: config.clone(),
        egress,
        pools: Mutex::new(HashMap::new()),
        pool_size: pool_size(),
        last_used: AtomicU64::new(now_secs()),
    });
    // Preconnect: warm the laptop-agent pool, best effort (the
    // Python does the same in a daemon thread).
    {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || {
            for _ in 0..broker.pool_size {
                if let Ok(conn) = broker.open_conn(fleet_config::PORT_LAPTOP_AGENT) {
                    broker.pool_put(fleet_config::PORT_LAPTOP_AGENT, conn);
                } else {
                    break;
                }
            }
        });
    }
    {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || keepalive_loop(broker));
    }
    println!("fleet proxy serve on {}", socket_path.display());
    let _ = std::io::stdout().flush();
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                let broker = Arc::clone(&broker);
                std::thread::spawn(move || serve_session(&broker, stream));
            }
            Err(_) => continue,
        }
    }
    0
}
