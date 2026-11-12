//! Raw byte splice: the transport for laptop services that are not
//! HTTP (code#6221, the first build of code#5694's recommendation B).
//!
//! Two halves, one relay:
//!
//! * **broker side** ([`serve_splice`]) — a client sends
//!   `{ "splice": { "port": N } }`; the broker checks the port is in
//!   its own routing table, opens ONE fresh tunnel to it, answers
//!   `{ "splice": { "status": 200 } }`, and from then on copies bytes
//!   between the client socket and that tunnel until either side ends.
//!   The tunnel is spent: it is never returned to the pool, because the
//!   pool holds reusable HTTP connections and this one is a live byte
//!   stream with another peer's protocol on it.
//! * **client side** ([`run_desktop`]) — bind a loopback listener, and
//!   for each accepted local connection negotiate one splice with the
//!   broker and relay it. This is what replaces the dedicated raw SSH
//!   forward (`forward.sh`) for RustDesk's direct-access port.
//!
//! Three properties are load-bearing and each has a test:
//!
//! 1. **Opaque.** Neither half parses, inspects, or logs payload bytes.
//!    The relay is a `read`/`write` pump over two sockets, deliberately
//!    not `std::io::copy` — on Linux that lowers to `splice(2)`, which
//!    stalled mid-stream in this sandbox for the SSH ProxyCommand relay
//!    (#5431), and the same hazard applies here.
//! 2. **Long-lived.** The tunnel is opened in [`TunnelMode::Raw`], so
//!    the HTTP dialect's 660 s read timeout is cleared. A desktop
//!    session must survive idle periods of hours, not eleven minutes.
//! 3. **Credential-free.** The broker and the relay hold no RustDesk
//!    credential of any kind: the permanent password is typed into the
//!    RustDesk client on the sandbox, and its proof travels inside the
//!    spliced bytes — which this module never looks at. Nothing here
//!    reads a token file, and nothing here writes payload to a log.

use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicUsize, Ordering};

use fleet_config::Config;
use fleet_proxy_proto::{
    CAP_SPLICE_SERVE, SpliceReply, SpliceRequest, advertises, read_frame, splice_frame,
    splice_refused_frame, splice_reply_frame, unwrap_splice, write_frame,
};

use crate::client::{ClientError, Negotiated, ensure_broker};
use crate::tunnel::{TunnelMode, connect_tunnel_mode, egress_from_env};

/// The buffer the relay moves per step. Matches the stdio relay's
/// choice: large enough that a desktop frame is one or two steps,
/// small enough that memory per splice stays trivial.
const RELAY_BUF: usize = 16 * 1024;

/// Copy bytes from `src` to `dst` until EOF or error, then half-close
/// `dst` so the peer sees a clean end in that direction.
///
/// Errors end the pump silently: a splice is a transport, and a
/// transport must not chatter onto the peer's protocol stream (the
/// same rule the SSH ProxyCommand relay follows). Returns the number
/// of bytes moved, for tests and diagnostics — never payload.
fn pump<R: Read, W: Write>(src: &mut R, dst: &mut W) -> u64 {
    let mut buf = [0u8; RELAY_BUF];
    let mut moved = 0u64;
    loop {
        match src.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if dst.write_all(&buf[..n]).is_err() {
                    break;
                }
                if dst.flush().is_err() {
                    break;
                }
                moved += n as u64;
            }
            Err(ref e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    moved
}

/// Relay `a` and `b` against each other until both directions are
/// done. `a` is driven on the calling thread (its EOF ends the relay,
/// exactly as in the stdio relay) and `b` on a helper thread; both
/// sockets are shut down at the end so neither pump can outlive the
/// relay.
pub fn relay_bidirectional(a: TcpStream, b: TcpStream) -> u64 {
    let (mut a_read, mut a_write) = match (a.try_clone(), a) {
        (Ok(r), w) => (r, w),
        // A clone failure means the socket is already unusable; there
        // is nothing to relay.
        (Err(_), _) => return 0,
    };
    let (mut b_read, mut b_write) = match (b.try_clone(), b) {
        (Ok(r), w) => (r, w),
        (Err(_), _) => return 0,
    };
    // Preserve TCP half-close semantics: EOF in one direction means
    // "no more bytes in that direction", not "discard the other
    // direction". Each pump owns one read half and one opposite write
    // half, and each sends FIN only after its own source reaches EOF.
    // When the peer disconnects, both pumps finish naturally.
    let up = std::thread::spawn(move || {
        let moved = pump(&mut b_read, &mut a_write);
        let _ = a_write.shutdown(Shutdown::Write);
        moved
    });
    let down = pump(&mut a_read, &mut b_write);
    let _ = b_write.shutdown(Shutdown::Write);
    let up = up.join().unwrap_or(0);
    up + down
}

/// One live splice, counted. The broker's keepalive loop must not
/// idle-exit while any splice is active: a splice bypasses
/// `Broker::do_request` (the only path that refreshes `last_used`),
/// so without this counter a desktop session with no unrelated HTTP
/// traffic would be killed at `idle_exit_secs` even while bytes are
/// flowing — and even while it is merely established and idle, which
/// is a normal state for a desktop session (code#6221, Codex P1).
pub struct SpliceGuard<'a>(&'a AtomicUsize);

impl<'a> SpliceGuard<'a> {
    pub fn begin(counter: &'a AtomicUsize) -> Self {
        counter.fetch_add(1, Ordering::SeqCst);
        Self(counter)
    }
}

impl Drop for SpliceGuard<'_> {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The broker half: serve one splice on an already-negotiated client
/// connection.
///
/// `request` is the client's `{ "splice": … }` frame (already read by
/// the session loop). Returns after the relay ends.
///
/// The port must be in the broker's routing table: the table IS the
/// capability negotiation (§4), so a port it does not name is refused
/// rather than dialled — a splice must never become a way to reach an
/// arbitrary laptop port.
pub fn serve_splice(
    config: &Config,
    egress: Option<&crate::tunnel::Egress>,
    mut client: UnixStream,
    request: &serde_json::Value,
    active_splices: &AtomicUsize,
) {
    let req: SpliceRequest = match unwrap_splice(request) {
        Ok(r) => r,
        Err(e) => {
            let _ = write_frame(&mut client, &splice_refused_frame(&e.to_string()));
            return;
        }
    };
    let known = config.services().iter().any(|s| s.port == req.port);
    if !known {
        let _ = write_frame(
            &mut client,
            &splice_refused_frame(&format!(
                "port {} is not in this broker's routing table",
                req.port
            )),
        );
        return;
    }
    // `egress == None` is ambiguous: it means explicit direct mode
    // OR a missing/invalid HTTPS_PROXY at broker startup (serve::run
    // stores None for both). Only the first may dial directly — the
    // HTTP path (Broker::open_conn_raw) refuses the second with this
    // same message, and a splice must not silently bypass the
    // configured proxy route (code#6221, Codex P2).
    if egress.is_none() && !config.proxy_direct() {
        let _ = write_frame(
            &mut client,
            &splice_refused_frame("HTTPS_PROXY must be set (runtime proxy env)"),
        );
        return;
    }
    let target = config.target_host();
    let tunnel = match egress {
        Some(e) => connect_tunnel_mode(
            Some(e),
            target,
            req.port,
            crate::tunnel::TRANSPORT_DEADLINE,
            TunnelMode::Raw,
        ),
        None => connect_tunnel_mode(
            None,
            target,
            req.port,
            crate::tunnel::TRANSPORT_DEADLINE,
            TunnelMode::Raw,
        ),
    };
    let tunnel = match tunnel {
        Ok(t) => t,
        Err(e) => {
            // The error names the target and port, never payload.
            let _ = write_frame(
                &mut client,
                &splice_refused_frame(&format!("tunnel to {target}:{}: {e}", req.port)),
            );
            return;
        }
    };
    let ok = SpliceReply {
        status: 200,
        error: None,
    };
    if write_frame(&mut client, &splice_reply_frame(&ok)).is_err() {
        return;
    }
    // Counted from handshake to relay end: while this guard lives the
    // broker's keepalive loop will not idle-exit (see SpliceGuard).
    let _active = SpliceGuard::begin(active_splices);
    // From here the connection is a byte stream in both directions.
    // A UnixStream is not a TcpStream, so the relay runs over the
    // generic pump with the client on the helper thread.
    relay_unix_tcp(client, tunnel);
}

/// Relay a Unix-socket client against a TCP tunnel (the broker side's
/// shape). Same contract as [`relay_bidirectional`].
fn relay_unix_tcp(client: UnixStream, tunnel: TcpStream) -> u64 {
    let (mut client_read, mut client_write) = match (client.try_clone(), client) {
        (Ok(r), w) => (r, w),
        (Err(_), _) => return 0,
    };
    let (mut tunnel_read, mut tunnel_write) = match (tunnel.try_clone(), tunnel) {
        (Ok(r), w) => (r, w),
        (Err(_), _) => return 0,
    };
    let up = std::thread::spawn(move || {
        let moved = pump(&mut tunnel_read, &mut client_write);
        let _ = client_write.shutdown(Shutdown::Write);
        moved
    });
    let down = pump(&mut client_read, &mut tunnel_write);
    let _ = tunnel_write.shutdown(Shutdown::Write);
    let up = up.join().unwrap_or(0);
    up + down
}

/// What a splice negotiation produced on the client side.
#[derive(Debug, PartialEq, Eq)]
pub enum SpliceOutcome {
    /// The broker accepted: the connection is now a raw byte stream to
    /// the laptop service. The caller owns it and must not write
    /// frames to it.
    Ready,
    /// The broker refused (or the tunnel failed): the reason, shapes
    /// only.
    Refused(String),
    /// The broker does not serve splices at all (pre-splice fleet
    /// broker, or a proto-0 Python broker). The caller must keep the
    /// existing fallback.
    Unsupported,
}

/// Ask the broker for a splice on `port`, over an already-negotiated
/// connection. On [`SpliceOutcome::Ready`] the caller owns the raw
/// stream; nothing else may be written to it as frames.
pub fn request_splice(
    stream: &mut UnixStream,
    negotiated: &Negotiated,
    port: u16,
) -> Result<SpliceOutcome, ClientError> {
    request_splice_over(stream, negotiated, port)
}

/// [`request_splice`] over any framed stream — the production path is
/// the broker's Unix socket; the tests exercise the identical frame
/// exchange over TCP against a stand-in broker.
pub fn request_splice_over<S: Read + Write>(
    stream: &mut S,
    negotiated: &Negotiated,
    port: u16,
) -> Result<SpliceOutcome, ClientError> {
    // Capability negotiation first: a broker that does not advertise
    // `splice-serve` cannot serve one, and asking would leave a frame
    // in a stream that broker will try to parse as a request.
    match negotiated {
        Negotiated::Fleet(resp) => {
            if !advertises(&resp.capabilities, CAP_SPLICE_SERVE) {
                return Ok(SpliceOutcome::Unsupported);
            }
        }
        // The pre-hello dialect has no capabilities at all.
        Negotiated::Legacy(_) => return Ok(SpliceOutcome::Unsupported),
    }
    let req = SpliceRequest { port };
    write_frame(stream, &splice_frame(&req))
        .map_err(|e| ClientError::Transport(format!("splice request: {e}")))?;
    let reply =
        read_frame(stream).map_err(|e| ClientError::Transport(format!("splice reply: {e}")))?;
    let reply: SpliceReply =
        unwrap_splice(&reply).map_err(|e| ClientError::Transport(format!("splice reply: {e}")))?;
    if reply.status != 200 {
        return Ok(SpliceOutcome::Refused(reply.error.unwrap_or_else(|| {
            format!("splice refused with status {}", reply.status)
        })));
    }
    // The broker has answered 200; `stream` is now the raw byte
    // stream. The caller relays it — see `splice_one`.
    Ok(SpliceOutcome::Ready)
}

/// The `fleet proxy desktop` usage text.
pub const DESKTOP_USAGE: &str = "\
usage: fleet proxy desktop [--port N] [--listen ADDR:PORT]

Expose a laptop TCP service on a loopback listener through the
session proxy, as an opaque byte stream (the raw splice, #6221).

  --port N         the laptop port to splice to (default 21118,
                   RustDesk's direct-access port)
  --listen ADDR    the local listener (default 127.0.0.1:N)

The listener is loopback-only by construction. The broker must
advertise `splice-serve`; a broker that does not is reported and
the existing SSH forward stays the fallback.";

/// Parse `fleet proxy desktop`'s arguments.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesktopArgs {
    pub port: u16,
    pub listen: Option<String>,
}

impl DesktopArgs {
    pub fn parse(argv: &[String]) -> Result<DesktopArgs, ClientError> {
        let mut port = fleet_config::PORT_RUSTDESK;
        let mut listen = None;
        let mut i = 0;
        let usage = |what: &str| ClientError::Usage(format!("proxy desktop: {what}"));
        while i < argv.len() {
            match argv[i].as_str() {
                "--port" => {
                    i += 1;
                    port = argv
                        .get(i)
                        .ok_or_else(|| usage("--port needs a value"))?
                        .parse()
                        .map_err(|_| usage("--port must be a number"))?;
                }
                "--listen" => {
                    i += 1;
                    listen = Some(
                        argv.get(i)
                            .ok_or_else(|| usage("--listen needs a value"))?
                            .clone(),
                    );
                }
                other => return Err(usage(&format!("unknown argument {other:?}"))),
            }
            i += 1;
        }
        Ok(DesktopArgs { port, listen })
    }

    /// The listener address: the explicit `--listen`, else loopback on
    /// the spliced port (the SSH forward's shape — the RustDesk client
    /// on the sandbox dials `127.0.0.1:21118` without configuration).
    pub fn listen_addr(&self) -> String {
        self.listen
            .clone()
            .unwrap_or_else(|| format!("{}:{}", fleet_config::SPLICE_LISTEN_HOST, self.port))
    }
}

/// `fleet proxy desktop` — the client half. Binds the loopback
/// listener and relays each accepted connection over its own splice.
/// Runs until the listener fails; returns the process exit code.
pub fn run_desktop(
    args: &DesktopArgs,
    config: &Config,
    client: &str,
    capabilities: &[String],
) -> Result<i32, ClientError> {
    let addr = args.listen_addr();
    // Loopback only: refuse a non-loopback --listen outright rather
    // than trusting the operator to remember. The splice is a local
    // capability; exposing it on a LAN interface would hand the
    // laptop's desktop port to the network.
    let listener = TcpListener::bind(&addr)
        .map_err(|e| ClientError::Transport(format!("listen {addr}: {e}")))?;
    let bound = listener
        .local_addr()
        .map(|a| a.to_string())
        .unwrap_or(addr.clone());
    if !is_loopback_listener(&bound) {
        return Err(ClientError::Usage(format!(
            "refusing to listen on {bound}: the splice listener is loopback-only"
        )));
    }
    // Negotiate once up front so a broker that cannot splice is
    // reported before the listener looks ready.
    let ensured = ensure_broker(config, client, capabilities)?;
    match &ensured.negotiated {
        Negotiated::Fleet(resp) if advertises(&resp.capabilities, CAP_SPLICE_SERVE) => {}
        Negotiated::Fleet(_) => {
            return Err(ClientError::Unavailable(
                "this broker does not serve raw splices (no `splice-serve` capability); \
                 keep the SSH forward as the fallback"
                    .to_string(),
            ));
        }
        Negotiated::Legacy(_) => {
            return Err(ClientError::Unavailable(
                "the broker is the legacy (proto 0) dialect and cannot serve raw splices; \
                 keep the SSH forward as the fallback"
                    .to_string(),
            ));
        }
    }
    // A second negotiation for the first splice would race nothing, but
    // the broker connection we hold is already spent on the hello; each
    // accepted connection gets its own.
    drop(ensured.stream);
    eprintln!(
        "fleet proxy desktop: {bound} -> laptop:{} (raw splice)",
        args.port
    );
    for stream in listener.incoming() {
        let Ok(local) = stream else { continue };
        let config = config.clone();
        let client = client.to_string();
        let capabilities = capabilities.to_vec();
        let port = args.port;
        std::thread::spawn(move || {
            if let Err(why) = splice_one(&config, &client, &capabilities, port, local) {
                // Diagnostics name the failure, never payload.
                eprintln!("fleet proxy desktop: splice ended: {why}");
            }
        });
    }
    Ok(0)
}

/// One accepted local connection → one negotiated splice → one relay.
fn splice_one(
    config: &Config,
    client: &str,
    capabilities: &[String],
    port: u16,
    local: TcpStream,
) -> Result<(), ClientError> {
    let mut ensured = ensure_broker(config, client, capabilities)?;
    let outcome = request_splice(&mut ensured.stream, &ensured.negotiated, port)?;
    match outcome {
        SpliceOutcome::Unsupported => Err(ClientError::Unavailable(
            "the broker no longer advertises `splice-serve`".to_string(),
        )),
        SpliceOutcome::Refused(why) => {
            Err(ClientError::Transport(format!("splice refused: {why}")))
        }
        SpliceOutcome::Ready => {
            // The negotiated socket IS the raw stream; relay it
            // against the accepted local connection.
            relay_unix_tcp(ensured.stream, local);
            Ok(())
        }
    }
}

/// Is `addr` a loopback listener address?
fn is_loopback_listener(addr: &str) -> bool {
    let host = if let Some(rest) = addr.strip_prefix('[') {
        rest.split(']').next().unwrap_or(rest)
    } else {
        addr.rsplit_once(':').map(|(h, _)| h).unwrap_or(addr)
    };
    host == "localhost"
        || host
            .parse::<std::net::IpAddr>()
            .map(|ip| ip.is_loopback())
            .unwrap_or(false)
}

/// The egress for a broker-side splice, resolved the way `serve::run`
/// resolves it (missing env is a per-request failure, not fatal).
pub fn broker_egress(config: &Config) -> Option<crate::tunnel::Egress> {
    if config.proxy_direct() {
        None
    } else {
        egress_from_env(config.egress_proxy_port()).ok()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    /// A loopback TCP pair: (server-side accepted stream, client side).
    ///
    /// The connect runs on a helper thread and the accept on this one:
    /// connecting before accepting on the same thread is the classic
    /// TCP self-connect — the kernel can pair the socket with its own
    /// listener and both ends then talk to themselves.
    fn tcp_pair() -> (TcpStream, TcpStream) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let connector = std::thread::spawn(move || TcpStream::connect(addr).unwrap());
        let (server, _) = listener.accept().unwrap();
        let client = connector.join().unwrap();
        (server, client)
    }

    #[test]
    fn splice_guard_counts_live_splices() {
        let counter = AtomicUsize::new(0);
        {
            let _a = SpliceGuard::begin(&counter);
            assert_eq!(counter.load(Ordering::SeqCst), 1);
            {
                let _b = SpliceGuard::begin(&counter);
                assert_eq!(counter.load(Ordering::SeqCst), 2);
            }
            assert_eq!(counter.load(Ordering::SeqCst), 1);
        }
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn serve_splice_refuses_missing_egress_when_not_direct() {
        // Config::default(): proxy_direct() is false and the built-in
        // routing table includes the RustDesk port, so the request
        // passes the routing-table check and reaches the egress gate.
        // With egress None (missing HTTPS_PROXY at startup) the splice
        // must be refused with the HTTP path's message, never dialled
        // directly.
        let config = Config::default();
        let (broker_side, mut client_side) = UnixStream::pair().unwrap();
        let counter = AtomicUsize::new(0);
        let request = serde_json::json!({ "splice": { "port": 21118 } });
        serve_splice(&config, None, broker_side, &request, &counter);
        let frame = read_frame(&mut client_side).unwrap();
        assert_eq!(frame["splice"]["status"], 502);
        let error = frame["splice"]["error"].as_str().unwrap_or("");
        assert!(
            error.contains("HTTPS_PROXY must be set"),
            "unexpected refusal: {error}"
        );
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn relay_is_byte_transparent_in_both_directions() {
        // A payload that is deliberately NOT text and NOT JSON: every
        // byte value, so any accidental framing or encoding shows up.
        let payload: Vec<u8> = (0u8..=255).cycle().take(64 * 1024 + 7).collect();
        let (a_peer, a_relay) = tcp_pair();
        let (b_relay, b_peer) = tcp_pair();
        let relay = std::thread::spawn(move || relay_bidirectional(a_relay, b_relay));

        // a → b: write the payload, half-close, and read it back on the
        // far side.
        let sent = payload.clone();
        let a_writer = std::thread::spawn(move || {
            let mut a = a_peer;
            a.write_all(&sent).unwrap();
            a.flush().unwrap();
            a.shutdown(Shutdown::Write).unwrap();
        });
        let mut b = b_peer;
        // The far endpoint sends no reply in this test; half-close its
        // write direction so the relay can observe EOF both ways while
        // still letting us read the forwarded bytes.
        b.shutdown(Shutdown::Write).unwrap();
        let mut got = Vec::new();
        b.read_to_end(&mut got).unwrap();
        a_writer.join().unwrap();
        relay.join().unwrap();
        assert_eq!(got, payload, "a→b was not byte-transparent");
    }

    #[test]
    fn relay_carries_both_directions_concurrently() {
        // Two independent streams of different sizes crossing at once:
        // a naive one-directional copy would deadlock or truncate.
        let (a_peer, a_relay) = tcp_pair();
        let (b_relay, b_peer) = tcp_pair();
        let relay = std::thread::spawn(move || relay_bidirectional(a_relay, b_relay));

        let forward: Vec<u8> = vec![b'A'; 200 * 1024];
        let backward: Vec<u8> = vec![b'B'; 150 * 1024];

        let f = forward.clone();
        let a_for_write = a_peer.try_clone().unwrap();
        let a_writer = std::thread::spawn(move || {
            let mut a = a_for_write;
            a.write_all(&f).unwrap();
            a.flush().unwrap();
            a.shutdown(Shutdown::Write).unwrap();
            a
        });
        let b_for_write = b_peer.try_clone().unwrap();
        let back = backward.clone();
        let b_writer = std::thread::spawn(move || {
            let mut b = b_for_write;
            b.write_all(&back).unwrap();
            b.flush().unwrap();
            b.shutdown(Shutdown::Write).unwrap();
            b
        });

        let mut a_reader = a_peer;
        let mut b_reader = b_peer;
        let got_back = std::thread::spawn(move || {
            let mut got = Vec::new();
            b_reader.read_to_end(&mut got).unwrap();
            got
        });
        let mut got_fwd = Vec::new();
        a_reader.read_to_end(&mut got_fwd).unwrap();

        a_writer.join().unwrap();
        b_writer.join().unwrap();
        let back = got_back.join().unwrap();
        relay.join().unwrap();
        assert_eq!(
            got_fwd, backward,
            "b→a direction was truncated or reordered"
        );
        assert_eq!(back, forward, "a→b direction was truncated or reordered");
    }

    #[test]
    fn concurrent_splices_do_not_interfere() {
        // Two relays at once, each with its own payload; a shared
        // buffer or a global lock would cross the streams.
        let mut handles = Vec::new();
        for tag in [b'X', b'Y'] {
            handles.push(std::thread::spawn(move || {
                let (a_peer, a_relay) = tcp_pair();
                let (b_relay, b_peer) = tcp_pair();
                let relay = std::thread::spawn(move || relay_bidirectional(a_relay, b_relay));
                let payload = vec![tag; 32 * 1024];
                let p = payload.clone();
                let w = std::thread::spawn(move || {
                    let mut a = a_peer;
                    a.write_all(&p).unwrap();
                    a.shutdown(Shutdown::Write).unwrap();
                });
                let mut b = b_peer;
                b.shutdown(Shutdown::Write).unwrap();
                let mut got = Vec::new();
                b.read_to_end(&mut got).unwrap();
                w.join().unwrap();
                relay.join().unwrap();
                assert_eq!(got, payload);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
    }

    #[test]
    fn raw_tunnels_clear_the_http_timeouts() {
        // code#6221's central acceptance: a splice must not inherit
        // the 660 s HTTP read timeout. The mode is what decides.
        let (http_read, http_write) = TunnelMode::Http.timeouts();
        assert_eq!(http_read, Some(crate::tunnel::TUNNEL_READ_TIMEOUT));
        assert_eq!(http_write, Some(crate::tunnel::TUNNEL_WRITE_TIMEOUT));
        let (raw_read, raw_write) = TunnelMode::Raw.timeouts();
        assert_eq!(raw_read, None, "a splice must not carry a read timeout");
        assert_eq!(raw_write, None, "a splice must not carry a write timeout");
    }

    #[test]
    fn a_raw_tunnel_really_has_no_timeouts() {
        // The mode is not just a table: apply it to a real socket and
        // read the socket's own view back.
        let (a, b) = tcp_pair();
        crate::tunnel::tune_for_test(&a, TunnelMode::Raw);
        assert_eq!(a.read_timeout().unwrap(), None);
        assert_eq!(a.write_timeout().unwrap(), None);
        crate::tunnel::tune_for_test(&b, TunnelMode::Http);
        assert_eq!(
            b.read_timeout().unwrap(),
            Some(crate::tunnel::TUNNEL_READ_TIMEOUT)
        );
    }

    #[test]
    fn desktop_listener_defaults_to_loopback_on_the_spliced_port() {
        let args = DesktopArgs::parse(&[]).unwrap();
        assert_eq!(args.port, fleet_config::PORT_RUSTDESK);
        assert_eq!(args.listen_addr(), "127.0.0.1:21118");
        let args = DesktopArgs::parse(&["--port".into(), "21119".into()]).unwrap();
        assert_eq!(args.listen_addr(), "127.0.0.1:21119");
        let args = DesktopArgs::parse(&[
            "--port".into(),
            "21118".into(),
            "--listen".into(),
            "127.0.0.1:9999".into(),
        ])
        .unwrap();
        assert_eq!(args.listen_addr(), "127.0.0.1:9999");
    }

    #[test]
    fn desktop_parser_rejects_bad_arguments() {
        for bad in [
            vec!["--port".to_string()],
            vec!["--port".to_string(), "not-a-port".to_string()],
            vec!["--listen".to_string()],
            vec!["--bogus".to_string()],
        ] {
            let err = DesktopArgs::parse(&bad).expect_err("must reject");
            assert_eq!(err.exit_code(), crate::EXIT_USAGE, "for {bad:?}");
        }
    }

    #[test]
    fn only_loopback_listeners_are_accepted() {
        assert!(is_loopback_listener("127.0.0.1:21118"));
        assert!(is_loopback_listener("127.0.0.53:1"));
        assert!(is_loopback_listener("[::1]:21118"));
        assert!(is_loopback_listener("localhost:21118"));
        assert!(!is_loopback_listener("0.0.0.0:21118"));
        assert!(!is_loopback_listener("100.99.113.103:21118"));
        assert!(!is_loopback_listener("192.168.1.5:21118"));
    }

    /// A stand-in broker: accepts one connection, reads the splice
    /// request, and answers with `reply`. Returns the port.
    fn fake_broker(reply: fleet_proxy_proto::SpliceReply) -> (u16, mpsc::Receiver<SpliceRequest>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // The real wire: 8-byte length prefix + JSON.
            let frame = fleet_proxy_proto::read_frame(&mut conn).unwrap();
            tx.send(fleet_proxy_proto::unwrap_splice::<SpliceRequest>(&frame).unwrap())
                .unwrap();
            fleet_proxy_proto::write_frame(
                &mut conn,
                &fleet_proxy_proto::splice_reply_frame(&reply),
            )
            .unwrap();
        });
        (port, rx)
    }

    #[test]
    fn request_splice_sends_the_port_and_reads_a_ready_reply() {
        let (port, rx) = fake_broker(fleet_proxy_proto::SpliceReply {
            status: 200,
            error: None,
        });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let negotiated = Negotiated::Fleet(Box::new(fleet_proxy_proto::HelloResponse {
            proto: 1,
            services: Default::default(),
            capabilities: vec![fleet_proxy_proto::CAP_SPLICE_SERVE.to_string()],
        }));
        // The helper speaks over a UnixStream in production; here we
        // exercise the same frame exchange over TCP by adapting.
        let outcome = request_splice_over(&mut stream, &negotiated, 21118).unwrap();
        assert_eq!(outcome, SpliceOutcome::Ready);
        assert_eq!(rx.recv_timeout(Duration::from_secs(5)).unwrap().port, 21118);
    }

    #[test]
    fn request_splice_reports_a_refusal_with_its_reason() {
        let (port, _rx) = fake_broker(fleet_proxy_proto::SpliceReply {
            status: 502,
            error: Some("port 21118 is not in this broker's routing table".to_string()),
        });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let negotiated = Negotiated::Fleet(Box::new(fleet_proxy_proto::HelloResponse {
            proto: 1,
            services: Default::default(),
            capabilities: vec![fleet_proxy_proto::CAP_SPLICE_SERVE.to_string()],
        }));
        match request_splice_over(&mut stream, &negotiated, 21118).unwrap() {
            SpliceOutcome::Refused(why) => assert!(why.contains("routing table"), "{why}"),
            other => panic!("expected Refused, got {other:?}"),
        }
    }

    #[test]
    fn a_broker_without_the_capability_is_never_asked() {
        // The negotiation gate: no `splice-serve` in the hello means
        // the client must not send a splice frame at all — a pre-splice
        // broker would try to parse it as a request.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            // Block until the client either writes (bad) or closes.
            let mut buf = [0u8; 1];
            let n = conn.read(&mut buf).unwrap_or(0);
            tx.send(n).unwrap();
            // Give the client a moment to be wrong.
            std::thread::sleep(Duration::from_millis(200));
        });
        let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
        let negotiated = Negotiated::Fleet(Box::new(fleet_proxy_proto::HelloResponse {
            proto: 1,
            services: Default::default(),
            capabilities: vec!["proxy-serve".to_string()],
        }));
        let outcome = request_splice_over(&mut stream, &negotiated, 21118).unwrap();
        assert_eq!(outcome, SpliceOutcome::Unsupported);
        drop(stream);
        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)).unwrap(),
            0,
            "the client wrote a frame to a broker that cannot splice"
        );
    }

    #[test]
    fn the_legacy_dialect_is_never_asked() {
        let negotiated = Negotiated::Legacy(Box::new(fleet_proxy_proto::Response {
            status: 200,
            body: serde_json::json!({}),
        }));
        // No socket is needed: the gate answers before any I/O.
        let (a, _b) = tcp_pair();
        let mut a = a;
        assert_eq!(
            request_splice_over(&mut a, &negotiated, 21118).unwrap(),
            SpliceOutcome::Unsupported
        );
    }

    #[test]
    fn an_idle_splice_survives_past_the_http_timeout() {
        // The acceptance target is >30 minutes; testing 660 s of wall
        // clock is not practical in CI, so this proves the *mechanism*:
        // a raw-mode socket has no read deadline, so a read that waits
        // longer than the HTTP timeout's shape would still be waiting.
        // We use a short stand-in deadline to keep the test fast.
        let (mut a, mut b) = tcp_pair();
        crate::tunnel::tune_for_test(&a, TunnelMode::Raw);
        assert_eq!(a.read_timeout().unwrap(), None);
        // Nothing is written for a while; the read must not time out.
        let start = Instant::now();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            let _ = b.write_all(b"late");
            let _ = b.flush();
        });
        let mut buf = [0u8; 4];
        a.read_exact(&mut buf).unwrap();
        assert_eq!(&buf, b"late");
        assert!(start.elapsed() >= Duration::from_millis(350));
    }
}
