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
//! * Two reaches carry tunnels to the laptop (code#10356, Drew's
//!   2026-11-15 ruling that sandbox↔laptop transport lives in
//!   this binary): the Cloudflare leg — TLS through the platform
//!   egress proxy to the named tunnel's edge hostname, Access
//!   service-token headers riding the HTTP request — is PRIMARY
//!   for the laptop-agent daemon port when its Access file
//!   exists, and the tailnet leg (CONNECT through the session
//!   proxy to the tailnet host) is the automatic fallback, on
//!   connection failure and on a Cloudflare edge error status
//!   (502/503/530) alike. The attempt plan is the Python broker's
//!   exactly (see [`attempt_plan`]); pools and recovery gates key
//!   on (port, transport) so the legs' warmth and failure state
//!   never share a bucket.
//!
//! The broker persists nothing: routing table from config,
//! tunnels in memory, tokens in their files.

use std::collections::HashMap;
use std::io::Write;
use std::net::TcpStream;
use std::os::unix::io::AsRawFd;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use fleet_config::Config;
use fleet_proxy_proto::{
    Hello, HelloResponse, ProtoError, Request, Response, ServiceInfo, is_hello_frame,
    proto_too_old_frame, read_frame, unwrap_hello, write_frame,
};

use crate::http::{HttpConn, HttpError};
use crate::tunnel::{
    Egress, connect_cf_tunnel, connect_raw_tunnel, connect_tunnel, egress_from_env,
    egress_from_env_url_port,
};

/// Keepalive cadence (the Python's KEEPALIVE_S).
const KEEPALIVE_S: u64 = 180;
/// How often the idle check wakes. The Python checks inside its
/// 180 s keepalive loop; the semantics (exit once idle exceeds the
/// configured span) are the same, the granularity is finer so a
/// short configured idle is observable in tests.
const IDLE_TICK_S: u64 = 5;

/// Dial a colocated service's Unix socket bounded by `deadline`
/// (#6219, Codex review on PR #8388). `UnixStream::connect` has no
/// timeout variant and can block on a listener that stopped
/// accepting with a full backlog, so the connect runs on a scratch
/// thread and the caller waits only for the deadline: on expiry
/// the caller falls back to TCP while the scratch thread stays
/// parked in the kernel until the connect resolves or errors
/// (bounded by the listener coming back or going away, and never
/// holding broker state). The dial IS the probe here -- a socket
/// that answers within the deadline is used, any other outcome
/// selects TCP within the same attempt.
fn connect_unix_bounded(path: &std::path::Path, deadline: Duration) -> Option<UnixStream> {
    let (tx, rx) = std::sync::mpsc::channel();
    let path = path.to_path_buf();
    std::thread::spawn(move || {
        let _ = tx.send(UnixStream::connect(&path));
    });
    match rx.recv_timeout(deadline) {
        Ok(Ok(stream)) => Some(stream),
        _ => None,
    }
}

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

/// Which reach carries a tunnel (code#10356): the Cloudflare
/// leg (TLS to the named tunnel's edge hostname through the
/// platform egress proxy) or the tailnet leg (CONNECT through
/// the session proxy to the tailnet host). Pools and recovery
/// gates key on the pair (port, transport): the legs fail
/// independently, so their warmth and their recovery state
/// must never share a bucket.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Transport {
    Cf,
    Ts,
}

impl Transport {
    /// The wire name, stamped on response frames and spoken by
    /// request frames' `transport` preference.
    fn as_str(&self) -> &'static str {
        match self {
            Transport::Cf => "cf",
            Transport::Ts => "ts",
        }
    }
}

/// Where one attempt's connection comes from: the (port,
/// transport) idle pool, or a fresh open. The Python broker's
/// attempt plan names both sources.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Pool,
    Fresh,
}

/// The pool key: one laptop service on one transport leg.
type PoolKey = (u16, Transport);

/// The per-port tunnel pools: an idle cache plus a live count
/// (idle + checked out) per (port, transport), behind one
/// mutex that is never held during network I/O — the same
/// lock discipline the pre-fix pool had (checkout/open/put
/// all release it before touching a socket).
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
    idle: HashMap<PoolKey, Vec<HttpConn>>,
    live: HashMap<PoolKey, usize>,
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

    fn live_of(state: &PoolState, key: PoolKey) -> usize {
        state.live.get(&key).copied().unwrap_or(0)
    }

    fn reserve(state: &mut PoolState, key: PoolKey) {
        *state.live.entry(key).or_insert(0) += 1;
    }

    fn unreserve(&self, state: &mut PoolState, key: PoolKey) {
        let live = TunnelPool::live_of(state, key);
        if live <= 1 {
            state.live.remove(&key);
        } else {
            state.live.insert(key, live - 1);
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
    fn checkout(&self, key: PoolKey) -> Checkout {
        let deadline = Instant::now() + POOL_WAIT_BUDGET;
        let mut state = self.state.lock().unwrap();
        loop {
            if let Some(conn) = state.idle.get_mut(&key).and_then(|pool| pool.pop()) {
                return Checkout::Idle(conn);
            }
            if TunnelPool::live_of(&state, key) < self.size {
                TunnelPool::reserve(&mut state, key);
                return Checkout::Grow;
            }
            let remaining = deadline.saturating_duration_since(Instant::now());
            if remaining.is_zero() {
                TunnelPool::reserve(&mut state, key);
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
    fn reserve_fresh(&self, key: PoolKey) {
        let mut state = self.state.lock().unwrap();
        TunnelPool::reserve(&mut state, key);
    }

    /// A reserved slot whose tunnel never opened: hand it back.
    fn release(&self, key: PoolKey) {
        let mut state = self.state.lock().unwrap();
        self.unreserve(&mut state, key);
    }

    /// Return a tunnel after use. Retained as idle when it is
    /// reusable and the key's idle vec has room; otherwise it
    /// stops counting as live and is dropped — after the lock
    /// is released, like every other socket touch in the pool.
    /// Every return wakes waiters (a retained tunnel is
    /// immediately checkout-able; a dropped one frees live
    /// headroom).
    fn put(&self, key: PoolKey, conn: HttpConn) {
        let mut state = self.state.lock().unwrap();
        if conn.reusable() && state.idle.get(&key).map_or(0, |p| p.len()) < self.size {
            state.idle.entry(key).or_default().push(conn);
            self.returned.notify_all();
            return;
        }
        self.unreserve(&mut state, key);
        drop(state);
        drop(conn);
    }

    /// A checked-out tunnel died in use and will never be
    /// returned: it stops counting as live, and waiters wake
    /// (headroom appeared).
    fn discard(&self, key: PoolKey) {
        let mut state = self.state.lock().unwrap();
        self.unreserve(&mut state, key);
    }

    #[cfg(test)]
    fn live_count(&self, key: PoolKey) -> usize {
        TunnelPool::live_of(&self.state.lock().unwrap(), key)
    }

    #[cfg(test)]
    fn idle_count(&self, key: PoolKey) -> usize {
        self.state
            .lock()
            .unwrap()
            .idle
            .get(&key)
            .map_or(0, |p| p.len())
    }
}

/// How long a caller waits for an in-flight recovery of the same
/// port before failing fast — the transport deadline, so waiting
/// never costs more than one bounded attempt.
const RECOVERY_WAIT: Duration = crate::tunnel::TRANSPORT_DEADLINE;

#[derive(Default)]
struct GateState {
    /// A tunnel open failed and no recovery has succeeded since:
    /// the next opener is the single recovery attempt.
    needs_recovery: bool,
    recovery_in_flight: bool,
    active_generation: u64,
    next_generation: u64,
    /// Verdict of each completed recovery generation (None =
    /// success), kept per generation so a waiter always reads the
    /// verdict of the generation IT waited on, never a successor's.
    /// Pruned to the most recent generations (see `record`).
    verdicts: std::collections::HashMap<u64, Option<String>>,
}

/// What [`decide`] tells a caller to do next — the gate's pure
/// decision core, driven synchronously by the model-based
/// property test and under the condvar by [`RecoveryGate::run`].
#[derive(Debug, PartialEq, Eq)]
enum Decision {
    /// Healthy path: open now, ungated.
    OpenNormal,
    /// You are the one recovery attempt, generation attached.
    OpenRecovery(u64),
    /// A recovery is in flight: wait for this generation.
    Wait(u64),
    /// The generation you waited on failed: share its verdict.
    ShareFailure(String),
    /// The generation you waited on succeeded: open ungated.
    ProceedAfterRecovery,
}

/// Pure gate decision for one caller. `waiting_on` is the
/// generation the caller has been waiting for, if any — its
/// verdict is consumed FIRST, before any newer in-flight attempt
/// is considered (Codex P2, PR #6490).
fn decide(entry: &mut GateState, waiting_on: Option<u64>) -> Decision {
    if let Some(wgen) = waiting_on {
        if let Some(verdict) = entry.verdicts.get(&wgen) {
            return match verdict {
                Some(err) => Decision::ShareFailure(err.clone()),
                None => Decision::ProceedAfterRecovery,
            };
        }
    }
    if entry.recovery_in_flight {
        return Decision::Wait(entry.active_generation);
    }
    if entry.needs_recovery {
        entry.next_generation += 1;
        entry.active_generation = entry.next_generation;
        entry.recovery_in_flight = true;
        entry.needs_recovery = false;
        return Decision::OpenRecovery(entry.active_generation);
    }
    Decision::OpenNormal
}

/// Pure gate bookkeeping for a finished attempt. Any failure
/// arms recovery for the next opener; a recovery attempt also
/// records its generation's verdict (consumed by its waiters via
/// [`decide`]) and disarms the gate on success. Verdicts are
/// pruned to the last 64 generations — far beyond any live
/// waiter cohort.
fn record(entry: &mut GateState, recovery_generation: Option<u64>, error: Option<&String>) {
    if error.is_some() {
        entry.needs_recovery = true;
    }
    if let Some(generation) = recovery_generation {
        entry.recovery_in_flight = false;
        entry.verdicts.insert(generation, error.cloned());
        if error.is_none() {
            entry.needs_recovery = false;
        }
        if entry.verdicts.len() > 64 {
            let floor = generation.saturating_sub(64);
            entry.verdicts.retain(|gnum, _| *gnum > floor);
        }
    }
}

/// Single-flight tunnel RECOVERY (#5758, shaped by PR #6490's
/// Codex review): healthy opens are NOT gated — a cold pool or a
/// burst opens concurrently exactly as before. The gate engages
/// only after a failure: the next opener for that port becomes
/// the one recovery attempt, and concurrent callers wait (bounded
/// by [`RECOVERY_WAIT`]) for its verdict — failure is shared with
/// the waiters (no CONNECT stampede against a dead target);
/// success clears the gate and waiters proceed to their own,
/// concurrent opens. A waiter always consumes the verdict of the
/// generation it waited on BEFORE looking at any newer attempt,
/// so an opener's immediate retry can never steal or overwrite
/// the failure its cohort observed.
struct RecoveryGate {
    state: Mutex<std::collections::HashMap<PoolKey, GateState>>,
    finished: Condvar,
}

impl RecoveryGate {
    fn new() -> RecoveryGate {
        RecoveryGate {
            state: Mutex::new(std::collections::HashMap::new()),
            finished: Condvar::new(),
        }
    }

    fn run<T>(
        &self,
        key: PoolKey,
        attempt: impl FnOnce() -> Result<T, String>,
    ) -> Result<T, String> {
        let deadline = Instant::now() + RECOVERY_WAIT;
        let mut waiting_on: Option<u64> = None;
        let recovery_generation = loop {
            let mut state = self.state.lock().unwrap();
            let entry = state.entry(key).or_default();
            match decide(entry, waiting_on) {
                Decision::ShareFailure(err) => return Err(err),
                Decision::ProceedAfterRecovery => break None,
                Decision::OpenNormal => break None,
                Decision::OpenRecovery(generation) => break Some(generation),
                Decision::Wait(generation) => {
                    waiting_on = Some(generation);
                    let left = deadline.saturating_duration_since(Instant::now());
                    if left.is_zero() {
                        return Err(format!(
                            "tunnel recovery for port {} via {} still in flight; failing fast",
                            key.0,
                            key.1.as_str()
                        ));
                    }
                    let (guard, _) = self.finished.wait_timeout(state, left).unwrap();
                    state = guard;
                    continue;
                }
            }
        };
        let got = attempt();
        let mut state = self.state.lock().unwrap();
        let entry = state.entry(key).or_default();
        record(entry, recovery_generation, got.as_ref().err());
        if recovery_generation.is_some() {
            self.finished.notify_all();
        }
        got
    }
}

/// `YYYY-MM-DDTHH:MM:SSZ` for a unix timestamp — the broker's log
/// stamp, computed without a date crate (civil-from-days).
fn format_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let tod = secs % 86_400;
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = if m <= 2 { y + 1 } else { y };
    format!(
        "{year:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        tod / 3600,
        (tod % 3600) / 60,
        tod % 60
    )
}

/// The one startup stamp every broker start writes first
/// (#5758): UTC time, implementation, version, pool size, and
/// pid — so which broker owned an incident window (and with what
/// pool) is reconstructable from broker.log alone. Credentials
/// are never part of it.
fn startup_line(pool_size: usize, socket_path: &std::path::Path) -> String {
    format!(
        "{} fleet proxy serve start implementation=fleet-proxy version={} pool_size={} pid={} socket={}",
        format_utc(now_secs()),
        env!("CARGO_PKG_VERSION"),
        pool_size,
        std::process::id(),
        socket_path.display()
    )
}

struct Broker {
    config: Config,
    /// The tailnet egress parsed at startup. Tunnel opens
    /// re-resolve it (see [`Broker::tailnet_egress`]); this parse
    /// is the fallback for an open whose fresh resolution fails,
    /// and its absence is what the startup warning reports.
    egress: Option<Egress>,
    /// The Cloudflare leg's egress: the same proxy as `egress`,
    /// dialled at the proxy URL's own port (see
    /// [`crate::tunnel::egress_from_env_url_port`]). Parsed at
    /// startup like `egress`; None only when the env is missing,
    /// in which case the leg fails per-request until it appears.
    cf_egress: Option<Egress>,
    pool: TunnelPool,
    /// Single-flight gate for tunnel opens (recovery stampedes).
    recovery: RecoveryGate,
    /// Last client-request time (unix secs). Keepalive health
    /// checks deliberately never write this.
    last_used: AtomicU64,
}

/// The tailnet-egress preference at tunnel-open time (the
/// oracle for #12426's fix): a fresh resolution of the seat's
/// current state wins; the startup parse is the fallback when
/// the fresh resolution fails; with neither, the fresh error —
/// the same message the startup warning prints — is the
/// failure.
fn resolve_tailnet_egress(
    fresh: Result<Egress, String>,
    startup: &Option<Egress>,
) -> Result<Egress, String> {
    match fresh {
        Ok(e) => Ok(e),
        Err(why) => startup.clone().ok_or(why),
    }
}

impl Broker {
    /// The tailnet leg's egress for one tunnel open, resolved
    /// fresh — never just the startup parse. The seat's recorded
    /// tailnet proxy (`fleet-proxy.env`, #12555) can appear or
    /// change after this broker started: a broker auto-spawned
    /// while tailscaled was down was born with the ambient
    /// egress value (or none) and would otherwise keep it for
    /// its whole life, so tailnet tunnels kept failing after
    /// the tailnet recovered until a manual restart (#12426).
    /// Opens are pooled and recovery-gated, so re-resolving
    /// per open costs one small file read per new tunnel, not
    /// per request.
    fn tailnet_egress(&self) -> Result<Egress, String> {
        resolve_tailnet_egress(
            egress_from_env(self.config.egress_proxy_port()),
            &self.egress,
        )
    }

    fn open_raw_tunnel(&self, port: u16) -> Result<TcpStream, String> {
        let target = self.config.target_host();
        let stream = if self.config.proxy_direct() {
            connect_raw_tunnel(None, target, port)
        } else {
            let egress = self.tailnet_egress()?;
            connect_raw_tunnel(Some(&egress), target, port)
        }
        .map_err(|e| format!("raw tunnel to {target}:{port}: {e}"))?;
        Ok(stream)
    }

    fn open_conn(&self, port: u16, transport: Transport) -> Result<HttpConn, String> {
        // Tunnel opens pass through the recovery gate: ungated
        // while healthy, single-flight once an open has failed —
        // see [`RecoveryGate`].
        self.recovery
            .run((port, transport), || self.open_conn_raw(port, transport))
    }

    fn open_conn_raw(&self, port: u16, transport: Transport) -> Result<HttpConn, String> {
        if transport == Transport::Cf {
            // The Cloudflare leg (code#10356): TLS to the named
            // tunnel's edge hostname through the platform egress
            // proxy. It never takes the colocated direct path —
            // direct mode is a same-box arrangement, and the leg
            // exists exactly for the cross-box case (the attempt
            // plan never offers it in direct mode anyway).
            let egress = self
                .cf_egress
                .as_ref()
                .ok_or_else(|| "HTTPS_PROXY must be set (runtime proxy env)".to_string())?;
            let cf_host = self.config.cf_host();
            let stream = connect_cf_tunnel(egress, cf_host)
                .map_err(|e| format!("cloudflare tunnel to {cf_host}:443: {e}"))?;
            return HttpConn::from_tls(stream)
                .map_err(|e| format!("cloudflare tunnel to {cf_host}:443: {e}"));
        }
        let target = self.config.target_host();
        if self.config.proxy_direct() {
            // Colocated direct mode (#6219): when the service on
            // this port is a broker role whose Unix socket answers,
            // dial the socket instead of TCP loopback. A socket
            // that does not answer (absent or stale, or a failed
            // connect) falls back to the TCP dial below within the
            // same attempt; non-colocated ports never take this
            // path, and the tunnel (non-direct) path is unchanged.
            if let Some((socket, pref)) = self.config.service_socket_resolution(port) {
                // A "tcp" preference skips the socket entirely; any
                // other case dials it, bounded by the same setup
                // deadline the tunnel path enforces (Codex review
                // on PR #8388: a wedged listener with a full backlog
                // can block a bare connect indefinitely, and the
                // read/write timeouts only apply after connect).
                if pref.as_deref() != Some("tcp") {
                    if let Some(stream) =
                        connect_unix_bounded(&socket, crate::tunnel::TRANSPORT_DEADLINE)
                    {
                        let _ = stream.set_read_timeout(Some(crate::tunnel::TUNNEL_READ_TIMEOUT));
                        let _ = stream.set_write_timeout(Some(crate::tunnel::TUNNEL_WRITE_TIMEOUT));
                        return HttpConn::from_unix(stream).map_err(|e| {
                            format!("socket {} for port {port}: {e}", socket.display())
                        });
                    }
                }
            }
            let stream: TcpStream = connect_tunnel(None, target, port)
                .map_err(|e| format!("tunnel to {target}:{port}: {e}"))?;
            return HttpConn::new(stream).map_err(|e| format!("tunnel to {target}:{port}: {e}"));
        }
        let egress = self.tailnet_egress()?;
        let stream: TcpStream = connect_tunnel(Some(&egress), target, port)
            .map_err(|e| format!("tunnel to {target}:{port}: {e}"))?;
        HttpConn::new(stream).map_err(|e| format!("tunnel to {target}:{port}: {e}"))
    }

    /// Take a tunnel: the pool's checkout decides reuse vs
    /// grow (see [`TunnelPool::checkout`] for the overflow
    /// policy); a grow reservation is handed back if the open
    /// fails so the live count never leaks.
    fn pool_get(&self, port: u16, transport: Transport) -> Result<HttpConn, String> {
        let key = (port, transport);
        match self.pool.checkout(key) {
            Checkout::Idle(conn) => Ok(conn),
            Checkout::Grow => match self.open_conn(port, transport) {
                Ok(conn) => Ok(conn),
                Err(e) => {
                    self.pool.release(key);
                    Err(e)
                }
            },
        }
    }

    /// The retry path's fresh tunnel: reserved in the live
    /// count, opened immediately (no idle pop, no wait — the
    /// request is already on its second attempt).
    fn pool_fresh(&self, port: u16, transport: Transport) -> Result<HttpConn, String> {
        let key = (port, transport);
        self.pool.reserve_fresh(key);
        match self.open_conn(port, transport) {
            Ok(conn) => Ok(conn),
            Err(e) => {
                self.pool.release(key);
                Err(e)
            }
        }
    }

    fn pool_put(&self, port: u16, transport: Transport, conn: HttpConn) {
        self.pool.put((port, transport), conn);
    }

    /// Whether the Cloudflare leg can be attempted at all: the
    /// egress env parsed at startup AND the Access file exists.
    /// The file's existence is the enable (the Python broker's
    /// rule) and is checked per request — dropping the file in,
    /// or removing it, takes effect without a broker restart.
    fn cf_available(&self) -> bool {
        self.cf_egress.is_some() && self.config.cf_access_file().exists()
    }

    /// One routed request: the request's attempt plan (see
    /// [`attempt_plan`]) executed in order — pooled Cloudflare,
    /// fresh Cloudflare, fresh tailnet when the leg is in play;
    /// pooled tailnet, fresh tailnet otherwise. A Cloudflare
    /// edge error status (502/503/530 — the edge answering for a
    /// tunnel that is down) is not the service's reply: the
    /// connection is dropped and the plan falls to the next leg.
    /// Returns the status, the parsed body, and the transport
    /// that actually served, which the caller stamps on the
    /// response frame so a client's trip record proves which leg
    /// a request rode. `client` distinguishes real client
    /// traffic (resets the idle timer) from the broker's own
    /// keepalive (never does).
    fn do_request(
        &self,
        req: &Request,
        client: bool,
    ) -> Result<(u16, serde_json::Value, Transport), String> {
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
        let pref = req.transport.as_deref().unwrap_or("auto");
        let cf_ok = req.cf_ok.unwrap_or_else(|| cf_request_ok(req));
        let plan = attempt_plan(
            port,
            pref,
            cf_ok,
            self.cf_available(),
            self.config.proxy_direct(),
        );
        // The Access headers are read once per request (the file
        // is the rotation surface) and ride Cloudflare attempts
        // only. The trip id, when the frame carries one, rides
        // every attempt — the daemon logs it per request, joining
        // the client's trip record to the daemon's own log.
        let cf_headers = if plan.iter().any(|(t, _)| *t == Transport::Cf) {
            read_cf_access_headers(&self.config.cf_access_file())
        } else {
            Vec::new()
        };
        let mut last_err: Option<String> = None;
        for (transport, source) in &plan {
            let key = (port, *transport);
            let mut conn = match match source {
                Source::Pool => self.pool_get(port, *transport),
                Source::Fresh => self.pool_fresh(port, *transport),
            } {
                Ok(c) => c,
                Err(e) => {
                    last_err = Some(e);
                    continue;
                }
            };
            let host_header = match transport {
                Transport::Cf => self.config.cf_host().to_string(),
                Transport::Ts => format!("{}:{}", self.config.target_host(), port),
            };
            let mut headers: Vec<(String, String)> = Vec::new();
            if let Some(trip) = &req.trip {
                headers.push(("X-Trip-Id".to_string(), trip.clone()));
            }
            if *transport == Transport::Cf {
                headers.extend(cf_headers.iter().cloned());
            }
            match conn.request_with_extra_headers(
                &req.method,
                &req.path,
                &host_header,
                token,
                payload.as_deref(),
                &headers,
            ) {
                Ok((status, data)) => {
                    if *transport == Transport::Cf && CF_EDGE_STATUSES.contains(&status) {
                        // The edge answered for a dead tunnel:
                        // not the service's reply. Drop the
                        // connection and fall to the next leg,
                        // exactly as the Python broker does.
                        eprintln!(
                            "fleet proxy serve: cloudflare edge status {status} for {} {}; falling to next leg",
                            req.method, req.path
                        );
                        self.pool.discard(key);
                        last_err = Some(format!("cloudflare edge status {status}"));
                        continue;
                    }
                    self.pool_put(port, *transport, conn);
                    let body = serde_json::from_slice(&data)
                        .map_err(|e| format!("service on port {port} answered non-JSON: {e}"))?;
                    return Ok((status, body, *transport));
                }
                Err(e) => {
                    last_err = Some(match e {
                        HttpError::Stale => "connection closed before any response".to_string(),
                        other => other.to_string(),
                    });
                    // conn drops: never re-pooled after an error,
                    // and it stops counting against the pool's
                    // live level.
                    self.pool.discard(key);
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

/// HTTP statuses that mean the Cloudflare EDGE answered, not
/// the service: the tunnel is down, or the edge cannot reach it.
/// The Python broker's set — the connection is dropped and the
/// plan falls to the next leg.
const CF_EDGE_STATUSES: [u16; 3] = [502, 503, 530];

/// The attempt plan for one request, the Python broker's plan
/// exactly (code#10356). The Cloudflare leg is in play only for
/// the laptop-agent daemon port, only when the caller's
/// per-request verdict allows the request to ride it (`cf_ok`),
/// the transport preference is `auto` or `cf`, the Access file
/// exists (`cf_available`), and the broker is not in colocated
/// direct mode (same-box there; the leg exists for the
/// cross-box case). In play, the plan is pooled-CF, fresh-CF,
/// and — under `auto` only — fresh tailnet. Every other case is
/// the historical plan: pooled tailnet, fresh tailnet. A `cf`
/// preference with the leg out of play degrades to the tailnet
/// plan: the preference is a preference, never a hard failure
/// (the Python's rule).
fn attempt_plan(
    port: u16,
    pref: &str,
    cf_ok: bool,
    cf_available: bool,
    direct: bool,
) -> Vec<(Transport, Source)> {
    let cf_in_play = !direct
        && port == fleet_config::PORT_LAPTOP_AGENT
        && cf_ok
        && cf_available
        && (pref == "auto" || pref == "cf");
    if cf_in_play {
        let mut plan = vec![
            (Transport::Cf, Source::Pool),
            (Transport::Cf, Source::Fresh),
        ];
        if pref == "auto" {
            plan.push((Transport::Ts, Source::Fresh));
        }
        plan
    } else {
        vec![
            (Transport::Ts, Source::Pool),
            (Transport::Ts, Source::Fresh),
        ]
    }
}

/// The broker's own verdict on whether a request may ride the
/// Cloudflare leg, used when the frame carries no `cf_ok`: the
/// edge's response window makes a long /exec unsuitable — the
/// Python client's 660 s safety valve, computed from the same
/// body field. A frame that DOES carry `cf_ok` is honored
/// verbatim (the client computes the same rule, plus its own
/// knowledge of the request).
fn cf_request_ok(req: &Request) -> bool {
    if req.path == "/exec" {
        if let Some(timeout_s) = req
            .body
            .as_ref()
            .and_then(|b| b.get("timeout_s"))
            .and_then(|v| v.as_f64())
        {
            if timeout_s > 660.0 {
                return false;
            }
        }
    }
    true
}

/// The Cloudflare Access service-token headers for one request,
/// read fresh from the Access file (rotation without a restart;
/// the values are never stored on the broker and never logged).
/// Any failure — missing file, bad JSON, missing fields — yields
/// no headers, the Python's rule: the request still rides the
/// leg and the edge's 403 is the service-visible answer.
fn read_cf_access_headers(path: &Path) -> Vec<(String, String)> {
    let Ok(text) = std::fs::read_to_string(path) else {
        return Vec::new();
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Vec::new();
    };
    let (Some(id), Some(secret)) = (
        value.get("client_id").and_then(|v| v.as_str()),
        value.get("client_secret").and_then(|v| v.as_str()),
    ) else {
        return Vec::new();
    };
    vec![
        ("CF-Access-Client-Id".to_string(), id.to_string()),
        ("CF-Access-Client-Secret".to_string(), secret.to_string()),
    ]
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
                transport: None,
            })
            .unwrap();
        }
    };
    match broker.do_request(&req, true) {
        Ok((status, body, transport)) => serde_json::to_value(Response {
            status,
            body,
            transport: Some(transport.as_str().to_string()),
        })
        .unwrap(),
        Err(why) => serde_json::to_value(Response {
            status: 502,
            body: serde_json::json!({ "error": format!("broker: {why}") }),
            transport: None,
        })
        .unwrap(),
    }
}

/// Request parsing, mirroring the Python's dict access:
/// `method`/`path` are required (their absence is the KeyError the
/// Python handler turns into a 502), and the remaining fields take
/// the same defaults the Python applies (body/token_file None, an
/// absent `port` defaults downstream) -- except `port`, which is
/// strict at this boundary (#6516): a present-but-unusable port is
/// refused, never silently defaulted.
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
    let port = parse_port(frame)?;
    let token_file = parse_opt_string(frame, "token_file")?;
    // The transport fields (code#10356) ride the same strict
    // boundary: a present-but-misshapen value is refused, never
    // silently reinterpreted — a `transport` that is not a string
    // must not quietly become the default preference.
    let trip = parse_opt_string(frame, "trip")?;
    let transport = parse_opt_string(frame, "transport")?;
    let cf_ok = match frame.get("cf_ok") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::Bool(b)) => Some(*b),
        Some(_) => return Err("request \"cf_ok\" must be a boolean or null".to_string()),
    };
    Ok(Request {
        method,
        path,
        body,
        port,
        token_file,
        trip,
        transport,
        cf_ok,
    })
}

/// An optional string frame field (`token_file`, and the
/// code#10356 `trip` / `transport` fields): absent or null is
/// None; a string is taken verbatim; anything else is refused
/// with the shape named — the #6516 strict-boundary rule.
fn parse_opt_string(frame: &serde_json::Value, field: &str) -> Result<Option<String>, String> {
    match frame.get(field) {
        None | Some(serde_json::Value::Null) => Ok(None),
        Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
        Some(value) => {
            let shape = match value {
                serde_json::Value::Bool(_) => "boolean",
                serde_json::Value::Number(_) => "number",
                serde_json::Value::Array(_) => "array",
                serde_json::Value::Object(_) => "object",
                serde_json::Value::Null => "null",
                serde_json::Value::String(_) => "string",
            };
            Err(format!(
                "request \"{field}\" must be a string or null, got {shape}"
            ))
        }
    }
}

/// The frame's `port`, strictly. Absent -- or null, mirroring how
/// `body` treats null -- means no port was named: the caller's
/// default applies downstream (`Broker::do_request` substitutes
/// PORT_LAPTOP_AGENT). A port that is present but unusable -- a
/// string, a float, a bool/container, a negative, zero, or above
/// 65535 -- is refused here at the frame boundary (#6516): the
/// lenient chain this replaces collapsed every one of those shapes
/// to None, and None became the default port, silently rerouting
/// the request to a service the caller never named. Method/path
/// already refuse malformed shapes at this boundary (#5750); the
/// port now does too. The error names the shape that arrived, and
/// `handle_request` wraps it in the standard 502 broker frame; no
/// connection is opened for a refused port.
fn parse_port(frame: &serde_json::Value) -> Result<Option<u16>, String> {
    let Some(value) = frame.get("port").filter(|v| !v.is_null()) else {
        return Ok(None);
    };
    let Some(n) = value.as_u64().filter(|_| value.is_u64()) else {
        let shape = if value.is_string() {
            format!("string {value}")
        } else if value.is_f64() {
            format!("non-integer number {value}")
        } else if value.is_i64() {
            format!("negative number {value}")
        } else {
            format!("{value}")
        };
        return Err(format!("request \"port\" {shape} is not an integer port"));
    };
    if n == 0 || n > u64::from(u16::MAX) {
        return Err(format!(
            "request \"port\" {n} is outside the usable port range 1..=65535"
        ));
    }
    Ok(Some(n as u16))
}

fn serve_session(broker: &Broker, stream: UnixStream) {
    serve_session_with_read_timeout(broker, stream, crate::tunnel::TUNNEL_READ_TIMEOUT);
}

fn serve_session_with_read_timeout(
    broker: &Broker,
    mut stream: UnixStream,
    read_timeout: Duration,
) {
    if stream.set_read_timeout(Some(read_timeout)).is_err() {
        return;
    }
    let mut client_supports_raw_splice = false;
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
                        transport: None,
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
        client_supports_raw_splice = hello
            .capabilities
            .iter()
            .any(|cap| cap == fleet_registry::Capability::ProxyRawSplice.id());
        let services = broker.services_map();
        let raw_splice_available = services
            .get(&fleet_config::PORT_RUSTDESK.to_string())
            .is_some_and(|service| service.name == "rustdesk");
        let mut capabilities = vec![fleet_registry::Capability::ProxyServe.id().to_string()];
        if raw_splice_available {
            capabilities.push(fleet_registry::Capability::ProxyRawSplice.id().to_string());
        }
        let resp = HelloResponse {
            proto: fleet_proxy_proto::PROTO_MAJOR,
            services,
            capabilities,
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
                transport: None,
            })
            .unwrap(),
        );
        return;
    }
    loop {
        match read_frame(&mut stream) {
            Ok(frame) => {
                if let Some(raw_splice) = frame.get("raw_splice") {
                    let port = raw_splice
                        .get("port")
                        .and_then(|value| value.as_u64())
                        .and_then(|value| u16::try_from(value).ok());
                    let service_available = broker
                        .services_map()
                        .get(&fleet_config::PORT_RUSTDESK.to_string())
                        .is_some_and(|service| service.name == "rustdesk");
                    if !client_supports_raw_splice
                        || port != Some(fleet_config::PORT_RUSTDESK)
                        || !service_available
                    {
                        let _ = write_frame(
                            &mut stream,
                            &serde_json::json!({
                                "raw_splice": {"error": "raw-splice capability or service unavailable"}
                            }),
                        );
                        return;
                    }
                    broker.last_used.store(now_secs(), Ordering::Relaxed);
                    match broker.open_raw_tunnel(fleet_config::PORT_RUSTDESK) {
                        Ok(tunnel) => {
                            if write_frame(
                                &mut stream,
                                &serde_json::json!({"raw_splice": {"ready": true}}),
                            )
                            .is_err()
                            {
                                return;
                            }
                            // Session framing is bounded, but the raw
                            // relay carries long-lived connections that
                            // may legitimately remain idle for hours.
                            if stream.set_read_timeout(None).is_err() {
                                return;
                            }
                            // A raw relay can be the broker's only
                            // activity for hours (a live RustDesk
                            // session): refresh last_used while it
                            // runs, or keepalive_loop's idle exit
                            // would kill the broker — and the session
                            // — after idle_exit_secs (Codex P2, PR
                            // #8618).
                            let relay_done = std::sync::atomic::AtomicBool::new(false);
                            std::thread::scope(|scope| {
                                scope.spawn(|| {
                                    while !relay_done.load(Ordering::Relaxed) {
                                        std::thread::sleep(Duration::from_secs(60));
                                        if !relay_done.load(Ordering::Relaxed) {
                                            broker.last_used.store(now_secs(), Ordering::Relaxed);
                                        }
                                    }
                                });
                                let _ = crate::raw_stream::relay(stream, tunnel);
                                relay_done.store(true, Ordering::Relaxed);
                            });
                        }
                        Err(why) => {
                            let _ = write_frame(
                                &mut stream,
                                &serde_json::json!({"raw_splice": {"error": why}}),
                            );
                        }
                    }
                    return;
                }
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

fn keepalive_loop(broker: Arc<Broker>, idle_exit: bool) {
    let idle = broker.config.idle_exit_secs();
    let health_req = Request {
        method: "GET".to_string(),
        path: "/health".to_string(),
        body: None,
        port: Some(fleet_config::PORT_LAPTOP_AGENT),
        token_file: None,
        trip: None,
        transport: None,
        cf_ok: None,
    };
    let mut last_health = now_secs();
    loop {
        std::thread::sleep(Duration::from_secs(IDLE_TICK_S));
        let now = now_secs();
        if idle_exit && now.saturating_sub(broker.last_used.load(Ordering::Relaxed)) > idle {
            // The Python exits via os._exit from its keepalive
            // loop; the same abruptness is fine — the broker holds
            // nothing that needs draining. Standalone brokers only:
            // a co-hosted proxy (run_cohosted) must never exit the
            // shared process out from under its sibling roles
            // (#8394 Codex P1).
            std::process::exit(0);
        }
        if now.saturating_sub(last_health) >= KEEPALIVE_S {
            last_health = now;
            let _ = broker.do_request(&health_req, false);
        }
    }
}

/// #9611 acceptance 1 (fleet-proxy leg): the cross-implementation
/// singleton lock. The Python broker holds an exclusive flock on
/// `broker.lock` beside the socket for its whole lifetime; this
/// implementation must play by the same rule, or the two can both
/// pass the stale-socket probe and race to bind (episode 1: Errno 98
/// churn; episode 2 tail: a silent takeover after which broker.pid
/// named a dead process). Returns the lock file, to be kept alive
/// for the process lifetime, or None when another implementation
/// already holds the singleton — the caller then converges on that
/// owner instead of competing for the socket. The flock releases
/// automatically when the holder dies, so a crashed broker never
/// wedges the path.
fn acquire_singleton_lock(socket_dir: &Path) -> Option<std::fs::File> {
    let lock_path = socket_dir.join("broker.lock");
    let file = std::fs::OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)
        .ok()?;
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc != 0 {
        return None;
    }
    Some(file)
}

/// Run the broker co-hosted with other serve roles in one process
/// (`fleet serve <role> [<role> ...]`, #6264): same broker, but the
/// idle-exit is disabled — the proxy's standalone lifetime contract
/// (auto-spawned, exits when idle) must not terminate the sibling
/// roles sharing the process (design #5698 point 5, #8394 Codex P1).
/// The serve-list supervisor owns this process's lifetime instead.
pub fn run_cohosted(config: &Config) -> i32 {
    run_inner(config, false)
}

/// Run the broker in the foreground. Returns the process exit code.
///
/// Startup mirrors the Python exactly: if the socket file exists
/// and a connect succeeds, a live broker owns it — exit 0 quietly.
/// If the connect fails, the file is stale — unlink and bind.
pub fn run(config: &Config) -> i32 {
    run_inner(config, true)
}

fn run_inner(config: &Config, idle_exit: bool) -> i32 {
    let socket_path = config.broker_socket();
    // #9611 acceptance 1: converge on the cross-implementation
    // singleton before touching the socket. Held for the whole
    // process lifetime (the serve loop below never returns while
    // this binding is alive).
    let _singleton_lock = match socket_path.parent() {
        Some(dir) => match acquire_singleton_lock(dir) {
            Some(f) => Some(f),
            None => {
                eprintln!(
                    "fleet proxy serve: broker singleton lock held by another                      implementation; not competing for {}",
                    socket_path.display()
                );
                return 0;
            }
        },
        None => None,
    };
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
    // broker.pid names the socket owner, written only after this
    // process actually owns the listening socket (#9611 acc. 1) —
    // previously only the Python broker wrote it, so a fleet-proxy
    // takeover left it naming a dead pid.
    if let Some(dir) = socket_path.parent() {
        let _ = std::fs::write(dir.join("broker.pid"), std::process::id().to_string());
    }
    let size = pool_size();
    // The stamp is the FIRST line a starting broker writes, so it
    // precedes any per-request tunnel failure the egress setup
    // below may log.
    println!("{}", startup_line(size, &socket_path));
    let _ = std::io::stdout().flush();
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
    let cf_egress = if config.proxy_direct() {
        None
    } else {
        match egress_from_env_url_port() {
            Ok(e) => Some(e),
            Err(why) => {
                eprintln!("fleet proxy serve: {why} (cloudflare leg will fail until it is set)");
                None
            }
        }
    };
    let broker = Arc::new(Broker {
        config: config.clone(),
        egress,
        cf_egress,
        pool: TunnelPool::new(size),
        recovery: RecoveryGate::new(),
        last_used: AtomicU64::new(now_secs()),
    });
    // Preconnect: warm the laptop-agent pool through the same
    // checkout accounting as requests, best effort (the Python
    // does the same in a daemon thread). The warmth goes to the
    // pool the first requests will draw from: the Cloudflare
    // pool when the leg is in play, switching to tailnet warmth
    // if a Cloudflare open fails (the Python preconnect's rule);
    // the tailnet pool otherwise, as before.
    {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || {
            let port = fleet_config::PORT_LAPTOP_AGENT;
            let mut transport = if broker.cf_available() {
                Transport::Cf
            } else {
                Transport::Ts
            };
            for _ in 0..broker.pool.size() {
                match broker.pool_get(port, transport) {
                    Ok(conn) => broker.pool_put(port, transport, conn),
                    Err(_) => {
                        if transport == Transport::Cf {
                            transport = Transport::Ts;
                            continue;
                        }
                        break;
                    }
                }
            }
        });
    }
    {
        let broker = Arc::clone(&broker);
        std::thread::spawn(move || keepalive_loop(broker, idle_exit));
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

    #[test]
    fn stalled_accepted_session_is_dropped_within_read_timeout() {
        let broker = Arc::new(Broker {
            config: Config::default(),
            egress: None,
            cf_egress: None,
            pool: TunnelPool::new(0),
            recovery: RecoveryGate::new(),
            last_used: AtomicU64::new(now_secs()),
        });
        let (stream, _peer) = UnixStream::pair().unwrap();
        let (done_tx, done_rx) = mpsc::channel();
        let timeout = Duration::from_millis(200);
        let started = Instant::now();

        std::thread::spawn(move || {
            serve_session_with_read_timeout(&broker, stream, timeout);
            let _ = done_tx.send(());
        });

        done_rx
            .recv_timeout(Duration::from_secs(2))
            .expect("stalled accepted stream should time out and return");
        assert!(started.elapsed() >= timeout);
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    const PORT: u16 = 4799;

    /// The pool/gate key the pre-transport tests exercise: the
    /// daemon port on the tailnet leg. The Cloudflare leg's own
    /// key separation is covered by
    /// `pool_keys_separate_the_transports`.
    const KEY: PoolKey = (PORT, Transport::Ts);

    #[test]
    fn parse_request_validates_token_file_shape() {
        let mut absent = serde_json::json!({"method": "GET", "path": "/health"});
        assert!(parse_request(&absent).unwrap().token_file.is_none());

        absent["token_file"] = serde_json::Value::Null;
        assert!(parse_request(&absent).unwrap().token_file.is_none());

        absent["token_file"] = serde_json::json!("/tmp/request-token");
        assert_eq!(
            parse_request(&absent).unwrap().token_file.as_deref(),
            Some("/tmp/request-token")
        );

        for (value, shape) in [
            (serde_json::json!(123), "number"),
            (serde_json::json!(true), "boolean"),
            (serde_json::json!({"path": "token"}), "object"),
            (serde_json::json!(["token"]), "array"),
        ] {
            let frame = serde_json::json!({
                "method": "GET",
                "path": "/health",
                "token_file": value,
            });
            let error = parse_request(&frame).unwrap_err();
            assert!(
                error.contains(shape),
                "expected token_file error to name {shape}, got: {error}"
            );
        }
    }

    #[test]
    fn parse_request_token_file_matches_the_shape_model_on_generated_values() {
        use proptest::prelude::*;
        // Oracle: the #6518 acceptance rules stated as a model over the
        // JSON value. Absent and null yield None; an arbitrary string is
        // preserved verbatim; every other shape -- numbers, floats,
        // bools, containers -- is refused.
        fn model(value: Option<&serde_json::Value>) -> Result<Option<String>, ()> {
            match value {
                None | Some(serde_json::Value::Null) => Ok(None),
                Some(serde_json::Value::String(s)) => Ok(Some(s.clone())),
                Some(_) => Err(()),
            }
        }
        let shapes = prop_oneof![
            Just(None),
            Just(Some(serde_json::Value::Null)),
            any::<String>().prop_map(|s| Some(serde_json::json!(s))),
            any::<i64>().prop_map(|n| Some(serde_json::json!(n))),
            (-1e6f64..=1e6).prop_map(|f| Some(serde_json::json!(f))),
            any::<bool>().prop_map(|b| Some(serde_json::json!(b))),
            prop::collection::vec(any::<String>(), 0..4).prop_map(|v| Some(serde_json::json!(v))),
            prop::collection::btree_map(any::<String>(), any::<String>(), 0..4)
                .prop_map(|m| Some(serde_json::json!(m))),
        ];
        proptest!(|(value in shapes)| {
            let mut frame = serde_json::json!({"method": "GET", "path": "/health"});
            if let Some(v) = &value {
                frame["token_file"] = v.clone();
            }
            let got = parse_request(&frame)
                .map(|req| req.token_file)
                .map_err(|_| ());
            prop_assert_eq!(got, model(value.as_ref()), "token_file value: {:?}", value);
        });
    }

    #[test]
    fn attempt_plan_puts_cloudflare_first_when_the_leg_is_in_play() {
        // Daemon port, auto, suitable request, Access file
        // present: pooled CF, fresh CF, fresh tailnet.
        assert_eq!(
            attempt_plan(PORT, "auto", true, true, false),
            vec![
                (Transport::Cf, Source::Pool),
                (Transport::Cf, Source::Fresh),
                (Transport::Ts, Source::Fresh),
            ]
        );
        // cf pinned: no tailnet leg at all.
        assert_eq!(
            attempt_plan(PORT, "cf", true, true, false),
            vec![
                (Transport::Cf, Source::Pool),
                (Transport::Cf, Source::Fresh),
            ]
        );
    }

    #[test]
    fn attempt_plan_degrades_to_the_tailnet_plan() {
        let ts_plan = vec![
            (Transport::Ts, Source::Pool),
            (Transport::Ts, Source::Fresh),
        ];
        // ts pinned.
        assert_eq!(attempt_plan(PORT, "ts", true, true, false), ts_plan);
        // No Access file: the leg is not in play, even for cf.
        assert_eq!(attempt_plan(PORT, "cf", true, false, false), ts_plan);
        assert_eq!(attempt_plan(PORT, "auto", true, false, false), ts_plan);
        // The request may not ride the edge (long /exec).
        assert_eq!(attempt_plan(PORT, "auto", false, true, false), ts_plan);
        // Other ports never take the leg.
        assert_eq!(
            attempt_plan(fleet_config::PORT_SECRETS_BROKER, "auto", true, true, false),
            ts_plan
        );
        // Colocated direct mode never takes the leg.
        assert_eq!(attempt_plan(PORT, "auto", true, true, true), ts_plan);
        // An unrecognized preference is not auto/cf: tailnet.
        assert_eq!(attempt_plan(PORT, "warp", true, true, false), ts_plan);
    }

    #[test]
    fn pool_keys_separate_the_transports() {
        // The legs' warmth must not share a bucket: a tunnel
        // pooled under the Cloudflare key is invisible to a
        // tailnet checkout for the same port, and the live
        // counts are independent.
        let pool = TunnelPool::new(2);
        let cf_key = (PORT, Transport::Cf);
        assert!(matches!(pool.checkout(cf_key), Checkout::Grow));
        pool.put(cf_key, dummy_conn());
        assert_eq!(pool.idle_count(cf_key), 1);
        assert_eq!(pool.idle_count(KEY), 0);
        assert!(matches!(pool.checkout(KEY), Checkout::Grow));
        assert_eq!(pool.live_count(KEY), 1);
        assert_eq!(pool.live_count(cf_key), 1);
    }

    #[test]
    fn parse_request_reads_the_transport_fields() {
        let frame = serde_json::json!({
            "method": "POST",
            "path": "/exec",
            "trip": "t-123",
            "transport": "cf",
            "cf_ok": false,
        });
        let req = parse_request(&frame).unwrap();
        assert_eq!(req.trip.as_deref(), Some("t-123"));
        assert_eq!(req.transport.as_deref(), Some("cf"));
        assert_eq!(req.cf_ok, Some(false));

        // Absent fields stay absent (pre-field frames parse
        // exactly as before).
        let bare = serde_json::json!({"method": "GET", "path": "/health"});
        let req = parse_request(&bare).unwrap();
        assert!(req.trip.is_none() && req.transport.is_none() && req.cf_ok.is_none());

        // Misshapen fields are refused at the boundary.
        let bad = serde_json::json!({"method": "GET", "path": "/health", "transport": 3});
        assert!(parse_request(&bad).unwrap_err().contains("transport"));
        let bad = serde_json::json!({"method": "GET", "path": "/health", "cf_ok": "yes"});
        assert!(parse_request(&bad).unwrap_err().contains("cf_ok"));
    }

    #[test]
    fn cf_request_ok_applies_the_exec_safety_valve() {
        let mk = |path: &str, body: serde_json::Value| Request {
            method: "POST".to_string(),
            path: path.to_string(),
            body: Some(body),
            port: None,
            token_file: None,
            trip: None,
            transport: None,
            cf_ok: None,
        };
        // A long /exec may not ride the edge.
        assert!(!cf_request_ok(&mk(
            "/exec",
            serde_json::json!({"timeout_s": 3600})
        )));
        assert!(cf_request_ok(&mk(
            "/exec",
            serde_json::json!({"timeout_s": 600})
        )));
        assert!(cf_request_ok(&mk(
            "/exec",
            serde_json::json!({"timeout_s": 660})
        )));
        // No timeout named: the daemon's own default applies,
        // which is inside the valve.
        assert!(cf_request_ok(&mk("/exec", serde_json::json!({}))));
        // Other paths are unaffected by a timeout-shaped field.
        assert!(cf_request_ok(&mk(
            "/run",
            serde_json::json!({"timeout_s": 3600})
        )));
    }

    #[test]
    fn cf_access_headers_read_the_file_and_fail_closed_to_none() {
        let dir = std::env::temp_dir().join(format!("fleet-cf-access-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("access.json");
        // Missing file: no headers (the request still rides the
        // leg; the edge's 403 is the visible answer).
        assert!(read_cf_access_headers(&path).is_empty());
        std::fs::write(
            &path,
            r#"{"client_id": "id-1", "client_secret": "secret-1"}"#,
        )
        .unwrap();
        assert_eq!(
            read_cf_access_headers(&path),
            vec![
                ("CF-Access-Client-Id".to_string(), "id-1".to_string()),
                (
                    "CF-Access-Client-Secret".to_string(),
                    "secret-1".to_string()
                ),
            ]
        );
        // Bad JSON / missing fields: no headers, never a panic.
        std::fs::write(&path, "not json").unwrap();
        assert!(read_cf_access_headers(&path).is_empty());
        std::fs::write(&path, r#"{"client_id": "id-1"}"#).unwrap();
        assert!(read_cf_access_headers(&path).is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn checkout_grows_below_ceiling_then_reuses_idle() {
        let pool = TunnelPool::new(2);
        assert!(matches!(pool.checkout(KEY), Checkout::Grow));
        assert_eq!(pool.live_count(KEY), 1);
        pool.put(KEY, dummy_conn());
        assert_eq!(pool.idle_count(KEY), 1);
        assert!(matches!(pool.checkout(KEY), Checkout::Idle(_)));
        // Checking out an idle tunnel does not change the
        // live count: the tunnel existed all along.
        assert_eq!(pool.live_count(KEY), 1);
        assert_eq!(pool.idle_count(KEY), 0);
    }

    #[test]
    fn waiter_is_woken_by_put_and_gets_the_returned_tunnel() {
        let pool = Arc::new(TunnelPool::new(1));
        // Occupy the only live slot.
        assert!(matches!(pool.checkout(KEY), Checkout::Grow));
        let (tx, rx) = mpsc::channel();
        let waiter_pool = Arc::clone(&pool);
        let waiter = std::thread::spawn(move || {
            let got = waiter_pool.checkout(KEY);
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
        pool.put(KEY, dummy_conn());
        match rx.recv_timeout(Duration::from_secs(5)) {
            Ok(true) => {}
            other => panic!("waiter did not get the returned tunnel: {other:?}"),
        }
        waiter.join().unwrap();
        assert_eq!(pool.live_count(KEY), 1);
    }

    #[test]
    fn waiter_times_out_and_grows_anyway() {
        let pool = TunnelPool::new(1);
        assert!(matches!(pool.checkout(KEY), Checkout::Grow));
        let t0 = Instant::now();
        let got = pool.checkout(KEY);
        let elapsed = t0.elapsed();
        // Nobody returns anything: after the bounded wait the
        // checkout grows past the ceiling rather than failing.
        assert!(matches!(got, Checkout::Grow));
        assert!(
            elapsed >= POOL_WAIT_BUDGET.mul_f64(0.8),
            "waited only {elapsed:?} of the {POOL_WAIT_BUDGET:?} budget"
        );
        assert!(elapsed < Duration::from_secs(5), "waited {elapsed:?}");
        assert_eq!(pool.live_count(KEY), 2);
    }

    #[test]
    fn release_and_discard_keep_the_live_count_honest() {
        let pool = TunnelPool::new(2);
        assert!(matches!(pool.checkout(KEY), Checkout::Grow));
        assert_eq!(pool.live_count(KEY), 1);
        pool.release(KEY); // the open failed: reservation back
        assert_eq!(pool.live_count(KEY), 0);

        assert!(matches!(pool.checkout(KEY), Checkout::Grow));
        pool.put(KEY, dummy_conn());
        assert!(matches!(pool.checkout(KEY), Checkout::Idle(_)));
        pool.discard(KEY); // the checked-out tunnel died in use
        assert_eq!(pool.live_count(KEY), 0);
        assert_eq!(pool.idle_count(KEY), 0);
    }

    #[test]
    fn put_drops_when_idle_is_full_or_conn_not_reusable() {
        let pool = TunnelPool::new(1);
        pool.reserve_fresh(KEY);
        pool.put(KEY, dummy_conn()); // retained: idle 1, live 1
        assert_eq!(pool.idle_count(KEY), 1);
        assert_eq!(pool.live_count(KEY), 1);

        pool.reserve_fresh(KEY); // live 2 (retry path counts too)
        pool.put(KEY, dummy_conn()); // idle full: dropped, live 1
        assert_eq!(pool.idle_count(KEY), 1);
        assert_eq!(pool.live_count(KEY), 1);

        assert!(matches!(pool.checkout(KEY), Checkout::Idle(_)));
        pool.discard(KEY);
        assert_eq!(pool.live_count(KEY), 0);

        pool.reserve_fresh(KEY);
        pool.put(KEY, close_conn()); // not reusable: never pooled
        assert_eq!(pool.idle_count(KEY), 0);
        assert_eq!(pool.live_count(KEY), 0);
    }

    #[test]
    fn recovery_failure_is_shared_by_all_waiters() {
        // Fault injection (#5758): one failure arms the gate;
        // then N concurrent callers produce exactly ONE recovery
        // attempt and every waiter fails fast with its verdict —
        // no CONNECT stampede against a dead target.
        let gate = Arc::new(RecoveryGate::new());
        let attempts = Arc::new(AtomicU64::new(0));
        let attempt = |attempts: &Arc<AtomicU64>| {
            attempts.fetch_add(1, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(150));
            Err::<(), String>("tunnel to laptop:4799: refused".to_string())
        };
        // Prime: a normal-mode failure arms single-flight recovery.
        let priming = gate.run(KEY, || attempt(&attempts));
        assert!(priming.is_err());
        assert_eq!(attempts.load(Ordering::Relaxed), 1);
        let mut handles = Vec::new();
        for _ in 0..8 {
            let gate = Arc::clone(&gate);
            let attempts = Arc::clone(&attempts);
            handles.push(std::thread::spawn(move || {
                gate.run(KEY, || attempt(&attempts))
            }));
        }
        for h in handles {
            assert_eq!(
                h.join().unwrap().unwrap_err(),
                "tunnel to laptop:4799: refused".to_string()
            );
        }
        // Priming + exactly one shared recovery attempt.
        assert_eq!(attempts.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn healthy_opens_proceed_concurrently() {
        // Codex P1 on PR #6490: the gate must NOT serialize the
        // healthy path — a cold-pool burst opens in parallel, so
        // nobody times out queued behind successful CONNECTs.
        let gate = Arc::new(RecoveryGate::new());
        let in_flight = Arc::new(AtomicU64::new(0));
        let peak = Arc::new(AtomicU64::new(0));
        let mut handles = Vec::new();
        for _ in 0..4 {
            let gate = Arc::clone(&gate);
            let in_flight = Arc::clone(&in_flight);
            let peak = Arc::clone(&peak);
            handles.push(std::thread::spawn(move || {
                gate.run(KEY, || {
                    let now = in_flight.fetch_add(1, Ordering::Relaxed) + 1;
                    peak.fetch_max(now, Ordering::Relaxed);
                    std::thread::sleep(Duration::from_millis(100));
                    in_flight.fetch_sub(1, Ordering::Relaxed);
                    Ok::<(), String>(())
                })
            }));
        }
        for h in handles {
            assert!(h.join().unwrap().is_ok());
        }
        assert!(
            peak.load(Ordering::Relaxed) > 1,
            "healthy opens were serialized (peak {})",
            peak.load(Ordering::Relaxed)
        );
    }

    #[test]
    fn waiter_keeps_its_generations_verdict_despite_a_retry() {
        // Codex P2 on PR #6490: a waiter on a failed recovery
        // generation must consume THAT verdict even if the
        // opener's immediate retry starts a newer generation
        // before the waiter reacquires the lock.
        let gate = Arc::new(RecoveryGate::new());
        // Prime the gate into recovery mode (attempt #1).
        let n = Arc::new(AtomicU64::new(0));
        let mk = |n: &Arc<AtomicU64>, ms: u64| {
            let k = n.fetch_add(1, Ordering::Relaxed) + 1;
            std::thread::sleep(Duration::from_millis(ms));
            Err::<(), String>(format!("failure-{k}"))
        };
        assert_eq!(gate.run(KEY, || mk(&n, 0)).unwrap_err(), "failure-1");
        // Leader: recovery attempt #2 (slow fail), then an
        // immediate retry = attempt #3 (even slower fail).
        let leader = {
            let gate = Arc::clone(&gate);
            let n = Arc::clone(&n);
            std::thread::spawn(move || {
                let first = gate.run(KEY, || mk(&n, 200));
                let second = gate.run(KEY, || mk(&n, 600));
                (first, second)
            })
        };
        // Waiter arrives while attempt #2 is in flight.
        std::thread::sleep(Duration::from_millis(50));
        let waited = gate.run(KEY, || mk(&n, 0));
        // The waiter shares attempt #2's verdict — it neither
        // waits for nor reports attempt #3's.
        assert_eq!(waited.unwrap_err(), "failure-2");
        let (first, second) = leader.join().unwrap();
        assert_eq!(first.unwrap_err(), "failure-2");
        assert_eq!(second.unwrap_err(), "failure-3");
    }

    #[test]
    fn successful_recovery_disarms_the_gate() {
        // After a recovery succeeds, opens are concurrent again.
        let gate = Arc::new(RecoveryGate::new());
        assert!(
            gate.run(KEY, || Err::<(), String>("dead".to_string()))
                .is_err()
        );
        assert!(gate.run(KEY, || Ok::<(), String>(())).is_ok());
        // needs_recovery cleared: a later failure is a fresh
        // normal-mode failure, and the gate re-arms only then.
        assert!(gate.run(KEY, || Ok::<(), String>(())).is_ok());
    }

    // Model-based property test for the RecoveryGate decision
    // core (Codex P1 round 2 on PR #6490; the repo testing
    // doctrine: a state machine's invariants get a model and a
    // generator, not only handwritten schedules). The MODEL is
    // the oracle: an independent, minimal restatement of the
    // gate spec — armed flag, the single in-flight recovery
    // generation, and per-generation verdicts. Generated step
    // sequences advance four simulated callers through the REAL
    // `decide`/`record` functions; after every step the gate
    // state must equal the model's, the decision must be the one
    // the model predicts, and at most one simulated caller may
    // hold a recovery opening.
    #[derive(Debug, Clone, PartialEq)]
    enum SimCaller {
        New,
        Waiting(u64),
        Opening(Option<u64>, bool, String),
        Done(Result<(), String>),
    }

    #[test]
    fn recovery_gate_matches_the_model_on_generated_interleavings() {
        use proptest::prelude::*;
        #[derive(Default)]
        struct Model {
            armed: bool,
            in_flight: Option<u64>,
            next_gen: u64,
            verdicts: std::collections::HashMap<u64, Option<String>>,
        }
        fn model_decide(m: &Model, waiting_on: Option<u64>) -> Decision {
            if let Some(wgen) = waiting_on {
                if let Some(verdict) = m.verdicts.get(&wgen) {
                    return match verdict {
                        Some(err) => Decision::ShareFailure(err.clone()),
                        None => Decision::ProceedAfterRecovery,
                    };
                }
            }
            if let Some(g) = m.in_flight {
                return Decision::Wait(g);
            }
            if m.armed {
                return Decision::OpenRecovery(m.next_gen + 1);
            }
            Decision::OpenNormal
        }
        let strategy = proptest::collection::vec((0usize..4, any::<bool>()), 0..48usize);
        proptest!(|(
            steps in strategy,
        )| {
            let mut entry = GateState::default();
            let mut model = Model::default();
            let mut callers = vec![SimCaller::New; 4];
            let mut attempts = 0u64;
            let advance = |entry: &mut GateState,
                           model: &mut Model,
                           callers: &mut Vec<SimCaller>,
                           ci: usize,
                           succeed: bool,
                           attempts: &mut u64| {
                match callers[ci].clone() {
                    SimCaller::New | SimCaller::Waiting(_) => {
                        let waiting_on = match callers[ci] {
                            SimCaller::Waiting(g) => Some(g),
                            _ => None,
                        };
                        let expected = model_decide(model, waiting_on);
                        let got = decide(entry, waiting_on);
                        prop_assert_eq!(&got, &expected);
                        match got {
                            Decision::ShareFailure(err) => {
                                // Oracle check: the shared error is
                                // exactly the waited generation's
                                // recorded verdict.
                                let wgen = waiting_on.unwrap();
                                prop_assert_eq!(
                                    Some(&Some(err.clone())),
                                    model.verdicts.get(&wgen)
                                );
                                callers[ci] = SimCaller::Done(Err(err));
                            }
                            Decision::ProceedAfterRecovery | Decision::OpenNormal => {
                                *attempts += 1;
                                let label = format!("err-c{ci}-a{attempts}");
                                callers[ci] = SimCaller::Opening(None, succeed, label);
                            }
                            Decision::OpenRecovery(g) => {
                                model.in_flight = Some(g);
                                model.next_gen = g;
                                model.armed = false;
                                *attempts += 1;
                                let label = format!("err-c{ci}-a{attempts}");
                                callers[ci] = SimCaller::Opening(Some(g), succeed, label);
                            }
                            Decision::Wait(g) => {
                                callers[ci] = SimCaller::Waiting(g);
                            }
                        }
                    }
                    SimCaller::Opening(gnum, succeed, label) => {
                        let err = if succeed { None } else { Some(label.clone()) };
                        record(entry, gnum, err.as_ref());
                        if err.is_some() {
                            model.armed = true;
                        }
                        if let Some(g) = gnum {
                            model.in_flight = None;
                            model.verdicts.insert(g, err.clone());
                            if err.is_none() {
                                model.armed = false;
                            }
                        }
                        callers[ci] = SimCaller::Done(if succeed {
                            Ok(())
                        } else {
                            Err(label)
                        });
                    }
                    SimCaller::Done(_) => {}
                }
                // State equality with the model after every step.
                prop_assert_eq!(entry.needs_recovery, model.armed);
                prop_assert_eq!(entry.recovery_in_flight, model.in_flight.is_some());
                prop_assert_eq!(entry.verdicts.len(), model.verdicts.len());
                for (g, v) in &model.verdicts {
                    prop_assert_eq!(entry.verdicts.get(g), Some(v));
                }
                // Invariant: at most one recovery opening exists.
                let recovery_openings = callers
                    .iter()
                    .filter(|c| matches!(c, SimCaller::Opening(Some(_), _, _)))
                    .count();
                prop_assert!(recovery_openings <= 1);
                Ok(())
            };
            for (ci, succeed) in &steps {
                advance(&mut entry, &mut model, &mut callers, *ci, *succeed, &mut attempts)?;
            }
            // Drain: with cooperative (succeeding) steps every
            // caller must reach a terminal state — no caller is
            // ever stranded waiting on a generation.
            for _ in 0..200 {
                if callers.iter().all(|c| matches!(c, SimCaller::Done(_))) {
                    break;
                }
                for ci in 0..4 {
                    advance(&mut entry, &mut model, &mut callers, ci, true, &mut attempts)?;
                }
            }
            prop_assert!(
                callers.iter().all(|c| matches!(c, SimCaller::Done(_))),
                "callers stranded: {callers:?}"
            );
        });
    }

    #[test]
    fn singleton_lock_is_exclusive_across_open_descriptions() {
        let dir =
            std::env::temp_dir().join(format!("fleet-proxy-singleton-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let first = acquire_singleton_lock(&dir);
        assert!(first.is_some(), "first acquisition must succeed");
        // flock is per open-file-description: a second open in the
        // same process still conflicts — the property the
        // cross-implementation singleton relies on.
        assert!(
            acquire_singleton_lock(&dir).is_none(),
            "second acquisition must fail while the first is held"
        );
        drop(first);
        assert!(
            acquire_singleton_lock(&dir).is_some(),
            "acquisition must succeed again after the holder drops"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn startup_line_carries_the_incident_stamps() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(951_782_400 + 3661), "2000-02-29T01:01:01Z");
        // Pool size default is untouched by this change (#5758.5).
        assert_eq!(DEFAULT_POOL_SIZE, 50);
        let line = startup_line(50, std::path::Path::new("/tmp/broker.sock"));
        assert!(line.contains("implementation=fleet-proxy"), "{line}");
        assert!(
            line.contains(&format!("version={}", env!("CARGO_PKG_VERSION"))),
            "{line}"
        );
        assert!(line.contains("pool_size=50"), "{line}");
        assert!(
            line.contains(&format!("pid={}", std::process::id())),
            "{line}"
        );
        assert!(line.contains("socket=/tmp/broker.sock"), "{line}");
        // Shape: "<ISO-8601 UTC> fleet proxy serve start …".
        let stamp = line.split(' ').next().unwrap();
        assert!(stamp.ends_with('Z') && stamp.contains('T'), "{line}");
    }

    fn frame_with_port(port: serde_json::Value) -> serde_json::Value {
        serde_json::json!({"method": "GET", "path": "/health", "port": port})
    }

    #[test]
    fn parse_request_absent_port_means_the_default() {
        // No "port" key at all: None, which do_request turns into
        // PORT_LAPTOP_AGENT (4799) -- the absent-port route.
        let req = parse_request(&serde_json::json!({"method": "GET", "path": "/health"})).unwrap();
        assert_eq!(req.port, None);
        // An explicit null is absent too, mirroring `body`.
        let req = parse_request(&frame_with_port(serde_json::Value::Null)).unwrap();
        assert_eq!(req.port, None);
    }

    #[test]
    fn parse_request_valid_port_is_kept() {
        let req = parse_request(&frame_with_port(serde_json::json!(4801))).unwrap();
        assert_eq!(req.port, Some(4801));
        let req = parse_request(&frame_with_port(serde_json::json!(65535))).unwrap();
        assert_eq!(req.port, Some(65535));
    }

    #[test]
    fn parse_request_refuses_present_but_invalid_ports() {
        // Every shape the pre-fix chain silently collapsed to the
        // 4799 default (#6516): out of range, negative, float,
        // string, zero, and a bool. Each must be refused at the
        // frame boundary with an error naming the port field, so
        // handle_request answers the 502 broker frame and no
        // connection is ever opened.
        for bad in [
            serde_json::json!(70000),
            serde_json::json!(-1),
            serde_json::json!(4801.5),
            serde_json::json!("4801"),
            serde_json::json!(0),
            serde_json::json!(true),
        ] {
            let err = parse_request(&frame_with_port(bad.clone()))
                .expect_err("present-but-invalid port must be refused");
            assert!(
                err.contains("\"port\""),
                "error must name the port field, got: {err}"
            );
        }
    }

    #[test]
    fn parse_request_port_matches_the_shape_model_on_generated_values() {
        use proptest::prelude::*;
        // Oracle: the #6516 acceptance rules stated as a model over
        // the JSON value. Null yields None (absent: the 4799 default
        // applies downstream); an integer in 1..=65535 is preserved;
        // every other shape -- out-of-range or negative integers,
        // floats, strings, bools, containers -- is refused.
        fn model(value: &serde_json::Value) -> Result<Option<u16>, ()> {
            match value {
                serde_json::Value::Null => Ok(None),
                serde_json::Value::Number(n) => match n.as_u64() {
                    Some(u) if n.is_u64() && (1..=65535).contains(&u) => Ok(Some(u as u16)),
                    _ => Err(()),
                },
                _ => Err(()),
            }
        }
        let shapes = prop_oneof![
            (1u64..=65535).prop_map(|n| serde_json::json!(n)),
            Just(serde_json::json!(0)),
            (65536u64..=u64::MAX).prop_map(|n| serde_json::json!(n)),
            (-1_000_000i64..=-1).prop_map(|n| serde_json::json!(n)),
            (-1e6f64..=1e6).prop_map(|f| serde_json::json!(f)),
            any::<String>().prop_map(|s| serde_json::json!(s)),
            any::<bool>().prop_map(|b| serde_json::json!(b)),
            Just(serde_json::json!([])),
            Just(serde_json::json!({})),
            Just(serde_json::Value::Null),
        ];
        proptest!(|(value in shapes)| {
            let got = parse_request(&frame_with_port(value.clone()))
                .map(|req| req.port)
                .map_err(|_| ());
            prop_assert_eq!(got, model(&value), "port value: {}", value);
        });
    }
    fn egress(host: &str) -> Egress {
        Egress {
            host: host.to_string(),
            port: 3130,
            authorization: "Basic Og==".to_string(),
        }
    }

    #[test]
    fn tailnet_egress_fresh_resolution_wins_over_the_startup_parse() {
        // The seat's state appeared (or changed) after startup:
        // the open must ride the current value, not the one the
        // broker was born with (#12426).
        let startup = Some(egress("hatch-egress-proxy"));
        let got = resolve_tailnet_egress(Ok(egress("127.0.0.1")), &startup).unwrap();
        assert_eq!(got, egress("127.0.0.1"));
    }

    #[test]
    fn tailnet_egress_falls_back_to_the_startup_parse() {
        // Fresh resolution failed (no recorded proxy, no ambient
        // env): the startup value still serves the open.
        let startup = Some(egress("127.0.0.1"));
        let got =
            resolve_tailnet_egress(Err("HTTPS_PROXY must be set".to_string()), &startup).unwrap();
        assert_eq!(got, egress("127.0.0.1"));
    }

    #[test]
    fn tailnet_egress_without_either_surfaces_the_resolution_error() {
        let got = resolve_tailnet_egress(
            Err("HTTPS_PROXY must be set (runtime proxy env)".to_string()),
            &None,
        );
        assert_eq!(
            got.unwrap_err(),
            "HTTPS_PROXY must be set (runtime proxy env)"
        );
    }
}
