//! UDP forwarding shim — the client half of the CONNECT-UDP
//! capability (#6109), the follow-up the spike's design note
//! named. A real consumer does not speak CONNECT-UDP (Moonlight
//! certainly doesn't), so something on the client device has to
//! wrap its UDP into proxy sessions: this module is that
//! something, at spike grade.
//!
//! Plain local UDP sockets in; one CONNECT-UDP session per
//! (listen port, peer) out, opened lazily on the peer's first
//! datagram; replies unwrapped back to the peer. Sessions idle
//! out on the same horizon as the proxy's own relay
//! ([`PEER_IDLE_TIMEOUT`]), and a dead session is replaced on
//! the peer's next datagram — the shim is stateless forwarding,
//! not a tunnel endpoint with opinions.
//!
//! The two bins beside this module are the whole surface:
//! `fleet-connect-udp` serves the endpoint standalone (for hosts
//! where the broker proper doesn't run), and `fleet-udp-forward`
//! runs this shim. Neither is wired into any service by the
//! code; a host opts in deliberately, the same posture as the
//! endpoint itself. Nothing here changes the #5924 verdict:
//! streaming stays tailnet-direct until a cutover says otherwise.

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{IpAddr, SocketAddr, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::connect_udp::{MAX_DATAGRAM_BYTES, UdpProxyClient, UdpProxySender};

/// How long a peer session may sit with no datagrams in either
/// direction before the forwarder drops it. Matches the proxy
/// relay's own idle horizon: a peer silent longer than this
/// gets a fresh session on its next datagram either way.
pub const PEER_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Poll cadence for the forwarding loops: how often a blocked
/// read wakes to reap idle peers.
const POLL: Duration = Duration::from_millis(250);
/// Maximum concurrent peer sessions per listen socket (#6152).
/// Each peer holds one CONNECT-UDP session — one thread plus
/// one socket on each side — so an uncapped peer table lets a
/// burst of one-datagram sources pin unbounded threads and
/// sockets for the full [`PEER_IDLE_TIMEOUT`]. Past the cap a
/// new source is refused loudly (its datagram is dropped, the
/// peer and the cap are named in the log) and no session opens
/// for it.
pub const MAX_PEERS: usize = 64;
/// Minimum interval between reaps on the datagram path: the
/// receive-timeout arm already reaps on every quiet poll, so
/// the busy path reaps at most this often to bound the reap
/// cost under sustained traffic (#6152).
pub const REAP_CADENCE: Duration = Duration::from_secs(5);

/// True when the datagram path owes a reap: `now` is at least
/// [`REAP_CADENCE`] past the last reap. Split out so the
/// cadence is pinned by unit test rather than by timing.
fn reap_due(last_reap: Instant, now: Instant) -> bool {
    now.duration_since(last_reap) >= REAP_CADENCE
}

/// True when a newly seen source may open a session: the peer
/// table has room under [`MAX_PEERS`]. Split out so the cap
/// wiring is pinned by unit test rather than by thread
/// counting.
fn peer_admitted(peer_count: usize) -> bool {
    peer_count < MAX_PEERS
}

/// One forwarding rule: datagrams arriving on `listen` are
/// wrapped into CONNECT-UDP sessions to `target_host:target_port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Forward {
    pub listen: SocketAddr,
    pub target_host: String,
    pub target_port: u16,
}

/// Parse a port set: `"47998"`, `"47998-48010"`,
/// `"47998,48000-48002"`. Ranges are inclusive; port 0 is not a
/// forwarding port.
pub fn parse_port_set(spec: &str) -> Result<Vec<u16>, String> {
    let mut ports = Vec::new();
    for part in spec.split(',') {
        let part = part.trim();
        if part.is_empty() {
            return Err(format!("empty element in port set {spec:?}"));
        }
        match part.split_once('-') {
            Some((a, b)) => {
                let a: u16 = a
                    .parse()
                    .map_err(|_| format!("bad port {a:?} in port set {spec:?}"))?;
                let b: u16 = b
                    .parse()
                    .map_err(|_| format!("bad port {b:?} in port set {spec:?}"))?;
                if a == 0 || b < a {
                    return Err(format!("bad port range {part:?} in port set {spec:?}"));
                }
                ports.extend(a..=b);
            }
            None => {
                let p: u16 = part
                    .parse()
                    .map_err(|_| format!("bad port {part:?} in port set {spec:?}"))?;
                if p == 0 {
                    return Err("port 0 is not a forwarding port".to_string());
                }
                ports.push(p);
            }
        }
    }
    if ports.is_empty() {
        return Err(format!("port set {spec:?} names no ports"));
    }
    Ok(ports)
}

/// Parse one `--map` value: `"<lport>=<host>:<rport>"`, binding
/// the listen side on `bind`.
pub fn parse_map(spec: &str, bind: IpAddr) -> Result<Forward, String> {
    let (lport, target) = spec
        .split_once('=')
        .ok_or_else(|| format!("map {spec:?} is not <lport>=<host>:<rport>"))?;
    let lport: u16 = lport
        .parse()
        .map_err(|_| format!("map {spec:?} has a bad listen port"))?;
    if lport == 0 {
        return Err(format!("map {spec:?} listen port is 0"));
    }
    let (host, rport) = target
        .rsplit_once(':')
        .ok_or_else(|| format!("map {spec:?} target lacks a port"))?;
    if host.is_empty() {
        return Err(format!("map {spec:?} target has an empty host"));
    }
    let rport: u16 = rport
        .parse()
        .map_err(|_| format!("map {spec:?} has a bad target port"))?;
    if rport == 0 {
        return Err(format!("map {spec:?} target port is 0"));
    }
    Ok(Forward {
        listen: SocketAddr::new(bind, lport),
        target_host: host.to_string(),
        target_port: rport,
    })
}

/// One peer's live session: the send half, shared activity
/// clock, and the liveness flag the reply thread clears on its
/// way out.
struct Peer {
    tx: Mutex<UdpProxySender>,
    last: Arc<Mutex<Instant>>,
    alive: Arc<AtomicBool>,
}

/// Open a session for `peer` and spawn its reply thread:
/// datagrams from the proxy go back out the listen socket to
/// the peer, verbatim.
fn open_peer(
    proxy_addr: &str,
    fwd: &Forward,
    socket: &Arc<UdpSocket>,
    peer: SocketAddr,
) -> std::io::Result<Peer> {
    let client = UdpProxyClient::connect(proxy_addr, &fwd.target_host, fwd.target_port)?;
    let (tx, mut rx) = client.split();
    rx.set_read_timeout(Some(POLL))?;
    let last = Arc::new(Mutex::new(Instant::now()));
    let alive = Arc::new(AtomicBool::new(true));
    {
        let socket = Arc::clone(socket);
        let last = Arc::clone(&last);
        let alive = Arc::clone(&alive);
        std::thread::spawn(move || {
            loop {
                match rx.recv() {
                    Ok(d) => {
                        *last.lock().unwrap() = Instant::now();
                        if socket.send_to(&d, peer).is_err() {
                            break;
                        }
                    }
                    Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                        if !alive.load(Ordering::SeqCst) {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            alive.store(false, Ordering::SeqCst);
        });
    }
    Ok(Peer {
        tx: Mutex::new(tx),
        last,
        alive,
    })
}

/// Drop peers that died (reply thread gone) or outlived the
/// idle horizon. Dropping a peer drops its send half; the reply
/// thread sees the liveness flag and exits, closing the
/// session.
fn reap(peers: &mut HashMap<SocketAddr, Peer>) {
    peers.retain(|_, p| {
        let keep =
            p.alive.load(Ordering::SeqCst) && p.last.lock().unwrap().elapsed() <= PEER_IDLE_TIMEOUT;
        if !keep {
            p.alive.store(false, Ordering::SeqCst);
        }
        keep
    });
}

/// The forwarding loop for one already-bound listen socket.
fn serve_socket(proxy_addr: &str, fwd: &Forward, socket: Arc<UdpSocket>) {
    let mut peers: HashMap<SocketAddr, Peer> = HashMap::new();
    let mut buf = vec![0u8; MAX_DATAGRAM_BYTES];
    let mut last_reap = Instant::now();
    loop {
        match socket.recv_from(&mut buf) {
            Ok((n, peer_addr)) => {
                // Sustained traffic never visits the quiet arm,
                // so reap here too — at a bounded cadence, so
                // idle peers die during a burst, not only after
                // it (#6152).
                if reap_due(last_reap, Instant::now()) {
                    reap(&mut peers);
                    last_reap = Instant::now();
                }
                let live = peers
                    .get(&peer_addr)
                    .map(|p| p.alive.load(Ordering::SeqCst))
                    .unwrap_or(false);
                if !live {
                    peers.remove(&peer_addr);
                    if !peer_admitted(peers.len()) {
                        eprintln!(
                            "fleet udp-forward: at peer cap ({MAX_PEERS} peers) on {}; refusing new source {peer_addr}: dropping datagram, no session opened",
                            fwd.listen
                        );
                        continue;
                    }
                    match open_peer(proxy_addr, fwd, &socket, peer_addr) {
                        Ok(p) => {
                            peers.insert(peer_addr, p);
                        }
                        Err(e) => {
                            eprintln!(
                                "fleet udp-forward: session to {}:{} for {peer_addr} via {proxy_addr} failed: {e}",
                                fwd.target_host, fwd.target_port
                            );
                            continue;
                        }
                    }
                }
                let p = peers.get(&peer_addr).expect("peer just ensured");
                *p.last.lock().unwrap() = Instant::now();
                let sent = p.tx.lock().unwrap().send(&buf[..n]);
                if sent.is_err() {
                    // Dead session: drop it; the peer's next
                    // datagram opens a fresh one.
                    p.alive.store(false, Ordering::SeqCst);
                    peers.remove(&peer_addr);
                }
            }
            Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                reap(&mut peers);
            }
            Err(e) => {
                eprintln!("fleet udp-forward: recv on {} failed: {e}", fwd.listen);
            }
        }
    }
}

/// Bind every forward's listen socket and serve them all,
/// one thread each. Bind failures surface before any thread
/// starts, so a misconfigured shim fails whole and loud.
pub fn run_forwards(proxy_addr: &str, forwards: &[Forward]) -> Result<(), String> {
    use std::io::Write as _;
    if forwards.is_empty() {
        return Err("no forwards configured".to_string());
    }
    let mut bound = Vec::new();
    for fwd in forwards {
        let socket = UdpSocket::bind(fwd.listen)
            .map_err(|e| format!("cannot bind UDP {}: {e}", fwd.listen))?;
        socket
            .set_read_timeout(Some(POLL))
            .map_err(|e| format!("cannot set read timeout on {}: {e}", fwd.listen))?;
        println!(
            "fleet udp-forward: udp:{} -> {}:{} via {proxy_addr}",
            fwd.listen, fwd.target_host, fwd.target_port
        );
        bound.push((fwd.clone(), Arc::new(socket)));
    }
    let _ = std::io::stdout().flush();
    let mut handles = Vec::new();
    for (fwd, socket) in bound {
        let proxy_addr = proxy_addr.to_string();
        handles.push(std::thread::spawn(move || {
            serve_socket(&proxy_addr, &fwd, socket);
        }));
    }
    for h in handles {
        let _ = h.join();
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn port_set_parsing() {
        assert_eq!(parse_port_set("47998").unwrap(), vec![47998]);
        assert_eq!(
            parse_port_set("47998-48000").unwrap(),
            vec![47998, 47999, 48000]
        );
        assert_eq!(
            parse_port_set("47998, 48010-48011").unwrap(),
            vec![47998, 48010, 48011]
        );
        assert!(parse_port_set("").is_err());
        assert!(parse_port_set("0").is_err());
        assert!(parse_port_set("48010-47998").is_err());
        assert!(parse_port_set("abc").is_err());
        assert!(parse_port_set("47998,,47999").is_err());
    }

    #[test]
    fn map_parsing() {
        let bind: IpAddr = "127.0.0.1".parse().unwrap();
        assert_eq!(
            parse_map("47998=192.0.2.10:47999", bind).unwrap(),
            Forward {
                listen: "127.0.0.1:47998".parse().unwrap(),
                target_host: "192.0.2.10".to_string(),
                target_port: 47999,
            }
        );
        assert!(parse_map("47998", bind).is_err());
        assert!(parse_map("0=127.0.0.1:47999", bind).is_err());
        assert!(parse_map("47998=127.0.0.1", bind).is_err());
        assert!(parse_map("47998=127.0.0.1:0", bind).is_err());
        assert!(parse_map("47998=:47999", bind).is_err());
    }

    /// A loopback UDP echo server on an ephemeral port.
    fn spawn_echo() -> u16 {
        let sock = UdpSocket::bind("127.0.0.1:0").unwrap();
        let port = sock.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let mut buf = vec![0u8; MAX_DATAGRAM_BYTES];
            loop {
                match sock.recv_from(&mut buf) {
                    Ok((n, from)) => {
                        let _ = sock.send_to(&buf[..n], from);
                    }
                    Err(_) => break,
                }
            }
        });
        port
    }

    #[test]
    fn forward_round_trip() {
        let echo_port = spawn_echo();
        let listener = crate::connect_udp::listen("127.0.0.1:0").unwrap();
        let proxy_addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || crate::connect_udp::serve(listener));

        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap());
        let fwd_addr = sock.local_addr().unwrap();
        let fwd = Forward {
            listen: fwd_addr,
            target_host: "127.0.0.1".to_string(),
            target_port: echo_port,
        };
        std::thread::spawn(move || serve_socket(&proxy_addr, &fwd, sock));

        let client = UdpSocket::bind("127.0.0.1:0").unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut buf = [0u8; 64];
        // First datagram opens the session; the second rides it.
        for payload in [&b"hello"[..], b"again"] {
            client.send_to(payload, fwd_addr).unwrap();
            let (n, _) = client.recv_from(&mut buf).unwrap();
            assert_eq!(&buf[..n], payload);
        }
    }

    #[test]
    fn reap_cadence_and_peer_cap_wiring() {
        let now = Instant::now();
        assert!(!reap_due(now, now));
        assert!(reap_due(now, now + REAP_CADENCE));
        assert!(reap_due(now, now + REAP_CADENCE + Duration::from_secs(1)));
        assert!(peer_admitted(0));
        assert!(peer_admitted(MAX_PEERS - 1));
        assert!(!peer_admitted(MAX_PEERS));
        assert!(!peer_admitted(MAX_PEERS + 100));
    }

    /// The shim cap (#6152): distinct sources well past
    /// [`MAX_PEERS`] each send one datagram; exactly the cap
    /// count gets an echo, the rest are refused (no session, no
    /// reply). The endpoint here is uncapped relative to the
    /// probe size, so every refusal is the shim's.
    #[test]
    fn peer_cap_refuses_beyond_max() {
        let echo_port = spawn_echo();
        let listener = crate::connect_udp::listen("127.0.0.1:0").unwrap();
        let proxy_addr = listener.local_addr().unwrap().to_string();
        // Keep the endpoint cap above this probe's source count;
        // otherwise its own limit could make the shim-cap test pass.
        std::thread::spawn(move || crate::connect_udp::serve_with_cap(listener, MAX_PEERS + 6));

        let sock = Arc::new(UdpSocket::bind("127.0.0.1:0").unwrap());
        let fwd_addr = sock.local_addr().unwrap();
        let fwd = Forward {
            listen: fwd_addr,
            target_host: "127.0.0.1".to_string(),
            target_port: echo_port,
        };
        std::thread::spawn(move || serve_socket(&proxy_addr, &fwd, sock));

        let sources: Vec<UdpSocket> = (0..MAX_PEERS + 6)
            .map(|_| {
                let s = UdpSocket::bind("127.0.0.1:0").unwrap();
                s.set_read_timeout(Some(Duration::from_millis(300)))
                    .unwrap();
                s
            })
            .collect();
        for (i, s) in sources.iter().enumerate() {
            let payload = format!("cap-probe-{i}");
            s.send_to(payload.as_bytes(), fwd_addr).unwrap();
        }
        // Session opens are handshake round-trips; give all of
        // them a beat before draining the replies.
        std::thread::sleep(Duration::from_secs(3));
        let mut buf = vec![0u8; MAX_DATAGRAM_BYTES];
        let mut echoed = 0;
        for s in &sources {
            if s.recv_from(&mut buf).is_ok() {
                echoed += 1;
            }
        }
        assert_eq!(
            echoed, MAX_PEERS,
            "exactly the cap admits sessions; the rest are refused"
        );
    }
}
