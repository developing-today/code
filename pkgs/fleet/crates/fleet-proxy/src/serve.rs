//! `fleet proxy serve` — the session broker, succeeding
//! laptop-agent-broker.py behavior for behavior:
//!
//! * Unix socket at the historical path; if a live broker already
//!   owns it, exit quietly; if the socket file is stale, unlink
//!   and bind.
//! * Per-port pools of warm CONNECT tunnels (pool size from
//!   `LAPTOP_AGENT_POOL`, default 50); a request takes a pooled
//!   connection, and on any failure retries once on a fresh one.
//!   Overflow policy at the pool: idle → grow (below the live
//!   ceiling) → wait briefly for a returned tunnel → grow anyway;
//!   a request never fails because of the pool (see
//!   [`TunnelPool::checkout`]).
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
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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

/// Compiled default for the per-port tunnel-pool size — the
/// number of live tunnels per laptop service the broker keeps
/// warm and hands out before the bounded wait applies.
///
/// 50 is sized for the fleet's own work: the concurrency
/// benchmark on #5412 showed the old default of 12 cliffing at
/// ~12 concurrent consumers (the lanes run dozens), while 32
/// already restored direct-class latency; 50 covers the burst
/// with headroom. The compiled default is the operative surface
/// on purpose: brokers are auto-spawned by `fleet proxy
/// connect` (`ensure_broker`) from arbitrary consumers, whose
/// environments do not carry `LAPTOP_AGENT_POOL` — an env-only
/// raise silently reverts the first time a random consumer
/// respawns the broker (observed live, 2026-11-04). The env
/// var remains an override for deliberate tuning.
const DEFAULT_POOL_SIZE: usize = 50;

/// How long `pool_get` may wait for a tunnel to be returned
/// before opening a new one anyway. The benchmark's warm
/// requests return their tunnels in ~100–300 ms, so a 300 ms
/// budget catches the large majority of returns; the worst
/// case adds 300 ms ahead of the new-tunnel open that is
/// today's immediate behavior (and which costs seconds under
/// burst). Waiting is strictly an optimization: on timeout
/// the pool grows, exactly as it did before the wait existed.
const POOL_WAIT_BUDGET: Duration = Duration::from_millis(300);

fn pool_size() -> usize {
    std::env::var("LAPTOP_AGENT_POOL")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|n: &usize| *n > 0)
        .unwrap_or(DEFAULT_POOL_SIZE)
}

/// The per-port tunnel pools: an idle cache plus a live count
/// (idle + checked out) per port, behind one mutex that is
/// never held during network I/O — the same lock discipline
/// the pre-fix pool had (checkout/open/put all release it
/// before touching a socket).
struct TunnelPool {
    state: Mutex<PoolState>,
    /// Signalled whenever a tunnel returns or a live slot
    /// frees, so a waiting checkout can re-check its port.
    returned: Condvar,
    /// Idle-retention cap AND the live level above which
    /// checkouts wait (bounded) before growing.
    size: usize,
}

#[derive(Default)]
struct PoolState {
    idle: HashMap<u16, Vec<HttpConn>>,
    live: HashMap<u16, usize>,
}

/// What a checkout decided: reuse this tunnel, or open a new
/// one (a live slot has already been reserved for it — call
/// [`TunnelPool::release`] if the open fails).
enum Checkout {
    Idle(HttpConn),
    Grow,
}

impl TunnelPool {
    fn new(size: usize) -> TunnelPool {
        TunnelPool {
            state: Mutex::new(PoolState::default()),
            returned: Condvar::new(),
            size,
        }
    }

    fn size(&self) -> usize {
        self.size
    }

    fn live_of(state: &PoolState, port: u16) -> usize {
        state.live.get(&port).copied().unwrap_or(0)
    }

    fn reserve(state: &mut PoolState, port: u16) {
        *state.live.entry(port).or_insert(0) += 1;
    }

    fn unreserve(&self, state: &mut PoolState, port: u16) {
        let live = TunnelPool::live_of(state, port);
        if live <= 1 {
            state.live.remove(&port);
        } else {
            state.live.insert(port, live - 1);
        }
        self.returned.notify_all();
    }

    /// Take a tunnel for `port`.
    ///
    /// Overflow policy, in order:
    /// 1. **idle** — a returned tunnel is reused at once;
    /// 2. **grow** — below the live ceiling (`size`), reserve
    ///    a slot and open a new tunnel;
    /// 3. **wait, bounded** — at the ceiling with nothing
    ///    idle, wait up to [`POOL_WAIT_BUDGET`] for a return
    ///    (a `pool_put` or a discard wakes us);
    /// 4. **grow anyway** — the budget expired: reserve past
    ///    the ceiling and open new. A request must never
    ///    hard-fail because of the pool; the wait only ever
    ///    trades a probable ~100–300 ms return against a
    ///    certain seconds-long tunnel establishment.
    fn checkout(&self, port: u16) -> Checkout {
        let deadline = Instant::now() + POOL_WAIT_BUDGET;
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(conn) = state.idle.get_mut(&port).and_then(|pool| pool.pop()) {
                return Checkout::Idle(conn);
            }
            if TunnelPool::live_of(&state, port) < self.size {
                TunnelPool::reserve(&mut state, port);
                return Checkout::Grow;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                TunnelPool::reserve(&mut state, port);
                return Checkout::Grow;
            }
            let (guard, _) = self.returned.wait_timeout(state, remaining).unwrap();
            state = guard;
        }
    }

    /// Reserve a live slot unconditionally — the retry path's
    /// fresh connection. No idle pop (the retry contract is a
    /// *fresh* tunnel), no wait (the request is already on its
    /// second attempt); accounting only.
    fn reserve_fresh(&self, port: u16) {
        let mut state = self.state.lock().unwrap();
        TunnelPool::reserve(&mut state, port);
    }

    /// A reserved slot whose tunnel never opened: hand it back.
    fn release(&self, port: u16) {
        let mut state = self.state.lock().unwrap();
        self.unreserve(&mut state, port);
    }

    /// Return a tunnel after use. Retained as idle when it is
    /// reusable and the port's idle vec has room; otherwise it
    /// stops counting as live and is dropped — after the lock
    /// is released, like every other socket touch in the pool.
    /// Every return wakes waiters (a retained tunnel is
    /// immediately checkout-able; a dropped one frees live
    /// headroom).
    fn put(&self, port: u16, conn: HttpConn) {
        let mut state = self.state.lock().unwrap();
        if conn.reusable() && state.idle.get(&port).map_or(0, |p| p.len()) < self.size {
            state.idle.entry(port).or_default().push(conn);
            self.returned.notify_all();
            return;
        }
        self.unreserve(&mut state, port);
        drop(state);
        drop(conn);
    }

    /// A checked-out tunnel died in use and will never be
    /// returned: it stops counting as live, and waiters wake
    /// (headroom appeared).
    fn discard(&self, port: u16) {
        let mut state = self.state.lock().unwrap();
        self.unreserve(&mut state, port);
    }

    #[cfg(test)]
    fn live_count(&self, port: u16) -> usize {
        TunnelPool::live_of(&self.state.lock().unwrap(), port)
    }

    #[cfg(test)]
    fn idle_count(&self, port: u16) -> usize {
        self.state
            .lock()
            .unwrap()
            .idle
            .get(&port)
            .map_or(0, |p| p.len())
    }
}

struct Broker {
    config: Config,
    egress: Option<Egress>,
    pool: TunnelPool,
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

    /// Take a tunnel: the pool's checkout decides reuse vs
    /// grow (see [`TunnelPool::checkout`] for the overflow
    /// policy); a grow reservation is handed back if the open
    /// fails so the live count never leaks.
    fn pool_get(&self, port: u16) -> Result<HttpConn, String> {
        match self.pool.checkout(port) {
            Checkout::Idle(conn) => Ok(conn),
            Checkout::Grow => match self.open_conn(port) {
                Ok(conn) => Ok(conn),
                Err(e) => {
                    self.pool.release(port);
                    Err(e)
                }
            },
        }
    }

    /// The retry path's fresh tunnel: reserved in the live
    /// count, opened immediately (no idle pop, no wait — the
    /// request is already on its second attempt).
    fn pool_fresh(&self, port: u16) -> Result<HttpConn, String> {
        self.pool.reserve_fresh(port);
        match self.open_conn(port) {
            Ok(conn) => Ok(conn),
            Err(e) => {
                self.pool.release(port);
                Err(e)
            }
        }
    }

    fn pool_put(&self, port: u16, conn: HttpConn) {
        self.pool.put(port, conn);
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
                self.pool_fresh(port)
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
                    // conn drops: never re-pooled after an error,
                    // and it stops counting against the pool's
                    // live level.
                    self.pool.discard(port);
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
    // The method/path are interpolated into an HTTP request line
    // by HttpConn::request: reject the shapes that would inject
    // headers or smuggle a second request here, at the frame
    // boundary, before any pool checkout happens (#5750).
    if !crate::http::valid_method(&method) {
        return Err(format!(
            "request \"method\" {method:?} is not an HTTP token"
        ));
    }
    if !crate::http::valid_path(&path) {
        return Err(format!(
            "request \"path\" {path:?} is not a clean origin-form path"
        ));
    }
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
        pool: TunnelPool::new(pool_size()),
        last_used: AtomicU64::new(now_secs()),
    });
    // Preconnect: warm the laptop-agent pool through the same
    // checkout accounting as requests, best effort (the Python
    // does the same in a daemon thread).
    {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || {
            for _ in 0..broker.pool.size() {
                if let Ok(conn) = broker.pool_get(fleet_config::PORT_LAPTOP_AGENT) {
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
    // CONNECT-UDP (RFC 9298) spike (#6109): only when the
    // config opts in — the flag defaults off and nothing in
    // the live path uses the endpoint. A startup failure is
    // logged and the broker carries on: an opt-in spike must
    // not take the broker it rides along with down.
    if config.connect_udp_enabled() {
        let config = config.clone();
        std::thread::spawn(move || {
            if let Err(why) = crate::connect_udp::run(&config) {
                eprintln!("fleet proxy serve: connect-udp not serving: {why}");
            }
        });
    }
    println!(
        "fleet proxy serve on {} (pool: {} per port, overflow wait {} ms)",
        socket_path.display(),
        broker.pool.size(),
        POOL_WAIT_BUDGET.as_millis()
    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read as _;
    use std::net::TcpListener;
    use std::sync::mpsc;

    /// An HttpConn over a loopback pair. The pool does no I/O
    /// with the conns it holds in these tests; they are opaque
    /// tokens whose `reusable` flag is what matters.
    fn dummy_conn() -> HttpConn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream = TcpStream::connect(addr).unwrap();
        let (_peer, _) = listener.accept().unwrap();
        HttpConn::new(stream).unwrap()
    }

    /// A conn that completed one exchange against a peer that
    /// answered `Connection: close` — a successful request
    /// whose connection must never re-pool (`reusable` false),
    /// produced deterministically through the real HTTP path.
    fn close_conn() -> HttpConn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let stream = TcpStream::connect(addr).unwrap();
        let (mut peer, _) = listener.accept().unwrap();
        let server = std::thread::spawn(move || {
            let mut buf = [0u8; 1024];
            let mut seen = Vec::new();
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = peer.read(&mut buf).unwrap();
                if n == 0 {
                    break;
                }
                seen.extend_from_slice(&buf[..n]);
            }
            use std::io::Write as _;
            peer.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
                .unwrap();
        });
        let mut conn = HttpConn::new(stream).unwrap();
        let (status, _) = conn
            .request("GET", "/health", "127.0.0.1", "token", None)
            .unwrap();
        assert_eq!(status, 200);
        assert!(!conn.reusable());
        server.join().unwrap();
        conn
    }

    const PORT: u16 = 4799;

    #[test]
    fn checkout_grows_below_ceiling_then_reuses_idle() {
        let pool = TunnelPool::new(2);
        assert!(matches!(pool.checkout(PORT), Checkout::Grow));
        assert_eq!(pool.live_count(PORT), 1);
        pool.put(PORT, dummy_conn());
        assert_eq!(pool.idle_count(PORT), 1);
        assert!(matches!(pool.checkout(PORT), Checkout::Idle(_)));
        // Checking out an idle tunnel does not change the
        // live count: the tunnel existed all along.
        assert_eq!(pool.live_count(PORT), 1);
        assert_eq!(pool.idle_count(PORT), 0);
    }

    #[test]
    fn waiter_is_woken_by_put_and_gets_the_returned_tunnel() {
        let pool = Arc::new(TunnelPool::new(1));
        // Occupy the only live slot.
        assert!(matches!(pool.checkout(PORT), Checkout::Grow));
        let (tx, rx) = mpsc::channel();
        let waiter_pool = Arc::clone(&pool);
        let waiter = std::thread::spawn(move || {
            let got = waiter_pool.checkout(PORT);
            tx.send(matches!(got, Checkout::Idle(_))).unwrap();
        });
        // At the ceiling with nothing idle, the waiter must
        // still be waiting well into the budget — the pre-fix
        // pool would have returned Grow immediately.
        match rx.recv_timeout(Duration::from_millis(100)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            other => panic!("waiter returned before any put: {other:?}"),
        }
        // Return a tunnel: the waiter wakes and reuses it.
        pool.put(PORT, dummy_conn());
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(true) => {}
            other => panic!("waiter did not get the returned tunnel: {other:?}"),
        }
        waiter.join().unwrap();
        assert_eq!(pool.live_count(PORT), 1);
    }

    #[test]
    fn waiter_times_out_and_grows_anyway() {
        let pool = TunnelPool::new(1);
        assert!(matches!(pool.checkout(PORT), Checkout::Grow));
        let t0 = Instant::now();
        let got = pool.checkout(PORT);
        let elapsed = t0.elapsed();
        // Nobody returns anything: after the bounded wait the
        // checkout grows past the ceiling rather than failing.
        assert!(matches!(got, Checkout::Grow));
        assert!(
            elapsed >= POOL_WAIT_BUDGET.mul_f64(0.8),
            "waited only {elapsed:?} of the {POOL_WAIT_BUDGET:?} budget"
        );
        assert!(elapsed < Duration::from_secs(5), "waited {elapsed:?}");
        assert_eq!(pool.live_count(PORT), 2);
    }

    #[test]
    fn release_and_discard_keep_the_live_count_honest() {
        let pool = TunnelPool::new(2);
        assert!(matches!(pool.checkout(PORT), Checkout::Grow));
        assert_eq!(pool.live_count(PORT), 1);
        pool.release(PORT); // the open failed: reservation back
        assert_eq!(pool.live_count(PORT), 0);

        assert!(matches!(pool.checkout(PORT), Checkout::Grow));
        pool.put(PORT, dummy_conn());
        assert!(matches!(pool.checkout(PORT), Checkout::Idle(_)));
        pool.discard(PORT); // the checked-out tunnel died in use
        assert_eq!(pool.live_count(PORT), 0);
        assert_eq!(pool.idle_count(PORT), 0);
    }

    #[test]
    fn put_drops_when_idle_is_full_or_conn_not_reusable() {
        let pool = TunnelPool::new(1);
        pool.reserve_fresh(PORT);
        pool.put(PORT, dummy_conn()); // retained: idle 1, live 1
        assert_eq!(pool.idle_count(PORT), 1);
        assert_eq!(pool.live_count(PORT), 1);

        pool.reserve_fresh(PORT); // live 2 (retry path counts too)
        pool.put(PORT, dummy_conn()); // idle full: dropped, live 1
        assert_eq!(pool.idle_count(PORT), 1);
        assert_eq!(pool.live_count(PORT), 1);

        assert!(matches!(pool.checkout(PORT), Checkout::Idle(_)));
        pool.discard(PORT);
        assert_eq!(pool.live_count(PORT), 0);

        pool.reserve_fresh(PORT);
        pool.put(PORT, close_conn()); // not reusable: never pooled
        assert_eq!(pool.idle_count(PORT), 0);
        assert_eq!(pool.live_count(PORT), 0);
    }
}
