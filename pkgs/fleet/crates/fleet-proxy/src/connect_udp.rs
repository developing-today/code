//! CONNECT-UDP (RFC 9298) — a spike capability (#6109), merged
//! dark. The listener exists only when `[proxy] connect_udp` is
//! set in fleet.toml, and nothing in the live path uses it: the
//! broker's CONNECT tunnels, the client, and the tailnet-direct
//! streaming verdict (#5924) are all exactly as they were. The
//! point of the spike is that the capability *exists* in our own
//! proxy component, proven, so a later cutover is a decision
//! rather than a research project.
//!
//! Why this component: the only HTTP CONNECT proxy *server* in
//! today's sandbox→laptop path is the Muse runtime's egress
//! proxy (named by `HTTPS_PROXY`) — not ours to modify. This
//! crate is the fleet's own proxy component (the session broker
//! plus the CONNECT *client* in [`crate::tunnel`]); a CONNECT-UDP
//! server belongs here so fleet traffic could terminate at our
//! own code later.
//!
//! Carrier, honestly stated: RFC 9298 over HTTP/1.1, datagrams
//! framed by RFC 9297's capsule protocol on the one TCP
//! connection — the carrier this crate's hand-rolled std-only
//! stack can speak today. Datagrams therefore inherit TCP
//! ordering and head-of-line blocking: loss on the wire becomes
//! latency, not drops. That is the wrong trade for a media
//! stream and the right trade for a capability proof; the media
//! carrier is HTTP/3 + QUIC DATAGRAM (RFC 9298 over QUIC,
//! unreliable datagrams, no HoL), which is follow-up work if a
//! cutover is ever called for. The burst proof in
//! `examples/connect_udp_proof.rs` measures this carrier against
//! direct UDP rather than asserting it is fine.
//!
//! The endpoint as implemented:
//!
//! * `CONNECT-UDP /.well-known/masque/udp/{host}/{port}/` with
//!   a `Capsule-Protocol: ?1` header (RFC 9298 §3, RFC 9297 §4);
//!   anything else is refused.
//! * Context ID 0 only (RFC 9298 §3.2: the datagram payload is
//!   the UDP payload, unmodified). Other contexts are a protocol
//!   error that closes the session — a spike implements the one
//!   context Moonlight-shaped traffic needs.
//! * One UDP socket per session, `connect()`ed to the target, so
//!   only the target's own packets are relayed back.
//! * A session with no datagrams in either direction for
//!   [`RELAY_IDLE_TIMEOUT`] closes.
//!
//! The follow-up surface beside this module: `udp_forward`
//! (the client shim wrapping plain UDP into these sessions)
//! and two small bins — `fleet-connect-udp`, the standalone
//! endpoint for hosts where the broker doesn't run (the
//! laptop's dark deployment), and `fleet-udp-forward`, the
//! shim. Neither bin is wired into any service by the code.
//!
//! There is deliberately **no target allowlist**: the listener
//! binds loopback by default and is off unless configured, the
//! same trust posture as the broker itself (a local capability,
//! reached by local clients). A cutover that binds it wider
//! owes an explicit target policy — noted here so the gap is a
//! decision on record, not an oversight.

use std::io::{BufRead, BufReader, Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream, ToSocketAddrs, UdpSocket};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use fleet_config::Config;

/// Capsule type carrying an HTTP Datagram (RFC 9297 §3.2).
const CAPSULE_TYPE_DATAGRAM: u64 = 0x30;
/// The well-known URI prefix naming a CONNECT-UDP target
/// (RFC 9298 §3): `/.well-known/masque/udp/{host}/{port}/`.
const WELL_KNOWN_UDP_PREFIX: &str = "/.well-known/masque/udp/";
/// Largest request head (request line + headers) accepted.
/// Legitimate CONNECT-UDP requests are a few hundred bytes; the
/// bound exists so a peer cannot make the parser accumulate
/// without limit (peer-declared sizes are never trusted, #5579).
pub const MAX_HEAD_BYTES: usize = 64 * 1024;
/// Largest single datagram relayed, either direction. A UDP
/// payload cannot exceed 65507 bytes; the cap is that plus the
/// capsule/context overhead, rounded up.
pub const MAX_DATAGRAM_BYTES: usize = 64 * 1024;
/// How long a relay session may sit with no datagrams in either
/// direction before it closes. Media sessions are chatty; two
/// minutes of silence is a dead session, not a paused one.
const RELAY_IDLE_TIMEOUT: Duration = Duration::from_secs(120);
/// Poll cadence for the relay loops: how often a blocked read
/// wakes to check the idle/closed state.
const RELAY_POLL: Duration = Duration::from_millis(250);

// ---------------------------------------------------------------------
// QUIC varints (RFC 9000 §16) — the capsule protocol's integer
// encoding, and the HTTP Datagram format's field encoding.

/// Append `value` to `out` as a QUIC varint. Values are 62-bit;
/// anything larger is encoded mod 2^62 by the format itself.
pub fn encode_varint(value: u64, out: &mut Vec<u8>) {
    match value {
        0..=63 => out.push(value as u8),
        64..=16_383 => out.extend_from_slice(&(value as u16 | 0x4000).to_be_bytes()),
        16_384..=1_073_741_823 => {
            out.extend_from_slice(&(value as u32 | 0x8000_0000).to_be_bytes())
        }
        _ => out.extend_from_slice(&(value | 0xc000_0000_0000_0000).to_be_bytes()),
    }
}

/// Decode a QUIC varint from the front of `bytes`, returning the
/// value and the bytes consumed — or `None` when `bytes` does not
/// yet hold the whole varint (the streaming case, not an error).
pub fn decode_varint(bytes: &[u8]) -> Option<(u64, usize)> {
    let first = *bytes.first()?;
    let len = 1usize << (first >> 6);
    if bytes.len() < len {
        return None;
    }
    let mut value = u64::from(first & 0x3f);
    for &b in &bytes[1..len] {
        value = (value << 8) | u64::from(b);
    }
    Some((value, len))
}

// ---------------------------------------------------------------------
// Capsules (RFC 9297 §3.2) carrying HTTP Datagrams (RFC 9297 §3,
// RFC 9298 §3.2).

/// Frame one UDP payload as a DATAGRAM capsule ready for the
/// connection: capsule type + length + the HTTP Datagram, which
/// is Quarter Stream ID (0 — HTTP/1.1 has exactly one stream),
/// Context ID (0 — the payload is the UDP datagram itself), then
/// the payload.
pub fn encode_datagram_capsule(udp_payload: &[u8], out: &mut Vec<u8>) {
    let mut datagram = Vec::with_capacity(udp_payload.len() + 2);
    datagram.push(0); // Quarter Stream ID 0 (varint)
    datagram.push(0); // Context ID 0 (varint)
    datagram.extend_from_slice(udp_payload);
    encode_varint(CAPSULE_TYPE_DATAGRAM, out);
    encode_varint(datagram.len() as u64, out);
    out.extend_from_slice(&datagram);
}

/// Incremental parser for one connection's capsule stream.
/// Bytes are pushed as they arrive; complete datagrams are
/// popped one at a time. Unknown capsule types are skipped per
/// RFC 9297 §3.2; a DATAGRAM capsule naming a nonzero quarter
/// stream or context is a protocol error (the spike speaks
/// exactly one stream and one context).
pub struct CapsuleParser {
    buf: Vec<u8>,
}

impl Default for CapsuleParser {
    fn default() -> Self {
        Self::new()
    }
}

impl CapsuleParser {
    pub fn new() -> Self {
        CapsuleParser { buf: Vec::new() }
    }

    pub fn push(&mut self, bytes: &[u8]) {
        self.buf.extend_from_slice(bytes);
    }

    /// Pop the next complete datagram's UDP payload, if one is
    /// fully buffered. `Ok(None)` means "need more bytes".
    pub fn next_datagram(&mut self) -> Result<Option<Vec<u8>>, String> {
        loop {
            let Some((capsule_type, n1)) = decode_varint(&self.buf) else {
                return Ok(None);
            };
            let Some((len, n2)) = decode_varint(&self.buf[n1..]) else {
                return Ok(None);
            };
            let len = usize::try_from(len).map_err(|_| "capsule length overflows".to_string())?;
            if len > MAX_DATAGRAM_BYTES {
                return Err(format!("capsule of {len} bytes exceeds the relay cap"));
            }
            let head = n1 + n2;
            if self.buf.len() - head < len {
                return Ok(None); // value not fully buffered yet
            }
            let value: Vec<u8> = self.buf[head..head + len].to_vec();
            self.buf.drain(..head + len);
            if capsule_type != CAPSULE_TYPE_DATAGRAM {
                continue; // unknown capsules are skipped, per RFC 9297
            }
            let (quarter, m1) =
                decode_varint(&value).ok_or("datagram missing quarter stream id")?;
            if quarter != 0 {
                return Err(format!("datagram for quarter stream {quarter}, expected 0"));
            }
            let (context, m2) = decode_varint(&value[m1..]).ok_or("datagram missing context id")?;
            if context != 0 {
                return Err(format!("datagram for context {context}, expected 0"));
            }
            return Ok(Some(value[m1 + m2..].to_vec()));
        }
    }
}

// ---------------------------------------------------------------------
// The request head: parse + validate a CONNECT-UDP request.

/// The target a CONNECT-UDP request names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UdpTarget {
    pub host: String,
    pub port: u16,
}

/// Parse the request-target of a CONNECT-UDP request:
/// `/.well-known/masque/udp/{host}/{port}/` (RFC 9298 §3). The
/// host variable is percent-encoded in the template; decoding
/// mirrors the egress parser's treatment in [`crate::tunnel`].
pub fn parse_target(request_target: &str) -> Result<UdpTarget, String> {
    let rest = request_target
        .strip_prefix(WELL_KNOWN_UDP_PREFIX)
        .ok_or_else(|| format!("request target is not a CONNECT-UDP URI: {request_target}"))?;
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let (host_enc, port_s) = rest
        .rsplit_once('/')
        .ok_or_else(|| format!("CONNECT-UDP URI lacks a port: {request_target}"))?;
    if host_enc.is_empty() {
        return Err("CONNECT-UDP URI has an empty host".to_string());
    }
    let port: u16 = port_s
        .parse()
        .map_err(|_| format!("CONNECT-UDP URI port is not a u16: {port_s}"))?;
    if port == 0 {
        return Err("CONNECT-UDP URI port is 0".to_string());
    }
    Ok(UdpTarget {
        host: percent_decode(host_enc),
        port,
    })
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
            if let Ok(v) = u8::from_str_radix(hex, 16) {
                out.push(v);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Read one head line (through `\n`) with the byte bound; strips
/// the trailing CR/LF. `Ok(None)` is a clean EOF before any byte.
fn read_head_line(reader: &mut impl BufRead, total: &mut usize) -> std::io::Result<Option<String>> {
    let mut line = Vec::new();
    // Bound the read itself, not just the accounting after it
    // (#6511): take() caps accumulation at one byte past the
    // remaining head budget, so a single newline-free line is
    // refused as soon as the cumulative total crosses
    // MAX_HEAD_BYTES instead of after the whole line has already
    // been allocated. The +1 lets a line that exactly fills the
    // remaining budget (total == MAX_HEAD_BYTES) still succeed.
    let remaining = MAX_HEAD_BYTES.saturating_sub(*total);
    let n = reader
        .by_ref()
        .take((remaining + 1) as u64)
        .read_until(b'\n', &mut line)?;
    *total += n;
    if *total > MAX_HEAD_BYTES {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "request head exceeds the byte bound",
        ));
    }
    if n == 0 {
        return Ok(None);
    }
    while matches!(line.last(), Some(b'\n' | b'\r')) {
        line.pop();
    }
    Ok(Some(String::from_utf8_lossy(&line).into_owned()))
}

/// Read and validate a CONNECT-UDP request head from `reader`.
/// On success the reader is positioned at the capsule stream
/// (any bytes the buffer already holds belong to it). Failures
/// come back as an HTTP status + reason for the caller to send.
fn read_request(reader: &mut impl BufRead) -> Result<UdpTarget, (u16, String)> {
    let mut total = 0usize;
    let request_line = read_head_line(reader, &mut total)
        .map_err(|e| (400, format!("unreadable request line: {e}")))?
        .ok_or((400, "empty request".to_string()))?;
    let mut parts = request_line.splitn(3, ' ');
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");
    let version = parts.next().unwrap_or("");
    if method != "CONNECT-UDP" {
        return Err((405, format!("method {method} is not CONNECT-UDP")));
    }
    if !version.starts_with("HTTP/1.") {
        return Err((400, format!("unsupported HTTP version {version}")));
    }
    let target = parse_target(target).map_err(|why| (400, why))?;
    let mut capsule_ok = false;
    loop {
        let line = read_head_line(reader, &mut total)
            .map_err(|e| (400, format!("unreadable header: {e}")))?
            .ok_or((400, "connection closed mid-head".to_string()))?;
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.trim().eq_ignore_ascii_case("capsule-protocol")
                && value.split(',').any(|v| v.trim() == "?1")
            {
                capsule_ok = true;
            }
        }
    }
    if !capsule_ok {
        return Err((400, "missing Capsule-Protocol: ?1 header".to_string()));
    }
    Ok(target)
}

fn write_status(writer: &TcpStream, status: u16, reason: &str) {
    let mut w = writer;
    let _ = write!(w, "HTTP/1.1 {status} {reason}\r\nContent-Length: 0\r\n\r\n");
    let _ = w.flush();
}

// ---------------------------------------------------------------------
// The relay: one session, two directions, std threads — the same
// shape as the broker's session handling.

struct Session {
    closed: AtomicBool,
    last_activity: Mutex<Instant>,
}

impl Session {
    fn touch(&self) {
        *self.last_activity.lock().unwrap() = Instant::now();
    }
    fn idle_expired(&self) -> bool {
        self.last_activity.lock().unwrap().elapsed() > RELAY_IDLE_TIMEOUT
    }
    fn close(&self) {
        self.closed.store(true, Ordering::SeqCst);
    }
    fn is_closed(&self) -> bool {
        self.closed.load(Ordering::SeqCst) || self.idle_expired()
    }
}

/// Handle one CONNECT-UDP connection: validate the request,
/// open the target socket, relay until either side ends or the
/// session idles out.
fn serve_session(stream: TcpStream) {
    let peer = stream
        .peer_addr()
        .map(|a| a.to_string())
        .unwrap_or_else(|_| "?".to_string());
    let _ = stream.set_nodelay(true);
    let _ = stream.set_read_timeout(Some(RELAY_POLL));
    let writer = match stream.try_clone() {
        Ok(w) => w,
        Err(e) => {
            eprintln!("fleet proxy connect-udp: clone for {peer} failed: {e}");
            return;
        }
    };
    let mut reader = BufReader::new(stream);
    let target = match read_request(&mut reader) {
        Ok(t) => t,
        Err((status, why)) => {
            eprintln!("fleet proxy connect-udp: refusing {peer}: {why}");
            write_status(&writer, status, "refused");
            return;
        }
    };
    let target_addr = match (target.host.as_str(), target.port).to_socket_addrs() {
        Ok(mut addrs) => match addrs.next() {
            Some(a) => a,
            None => {
                write_status(&writer, 502, "target does not resolve");
                return;
            }
        },
        Err(e) => {
            eprintln!(
                "fleet proxy connect-udp: cannot resolve {}:{}: {e}",
                target.host, target.port
            );
            write_status(&writer, 502, "target does not resolve");
            return;
        }
    };
    let bind_addr = if target_addr.is_ipv6() {
        "[::]:0"
    } else {
        "0.0.0.0:0"
    };
    let udp = match UdpSocket::bind(bind_addr).and_then(|u| u.connect(target_addr).map(|()| u)) {
        Ok(u) => u,
        Err(e) => {
            eprintln!("fleet proxy connect-udp: cannot open UDP to {target_addr}: {e}");
            write_status(&writer, 502, "target unreachable");
            return;
        }
    };
    let _ = udp.set_read_timeout(Some(RELAY_POLL));
    // The 200 goes out only once the target socket exists: a
    // client that sees 2xx can send datagrams immediately.
    {
        let mut w = &writer;
        if write!(w, "HTTP/1.1 200 OK\r\nCapsule-Protocol: ?1\r\n\r\n").is_err() {
            return;
        }
        let _ = w.flush();
    }
    let session = Arc::new(Session {
        closed: AtomicBool::new(false),
        last_activity: Mutex::new(Instant::now()),
    });
    // UDP → TCP direction.
    let up_session = Arc::clone(&session);
    let up_writer = match writer.try_clone() {
        Ok(w) => w,
        Err(_) => return,
    };
    let up_udp = match udp.try_clone() {
        Ok(u) => u,
        Err(_) => return,
    };
    let up = std::thread::spawn(move || {
        let mut buf = vec![0u8; MAX_DATAGRAM_BYTES];
        let mut framed = Vec::with_capacity(MAX_DATAGRAM_BYTES + 16);
        loop {
            match up_udp.recv(&mut buf) {
                Ok(n) => {
                    up_session.touch();
                    framed.clear();
                    encode_datagram_capsule(&buf[..n], &mut framed);
                    let mut w = &up_writer;
                    if w.write_all(&framed).and_then(|()| w.flush()).is_err() {
                        break;
                    }
                }
                Err(ref e)
                    if e.kind() == std::io::ErrorKind::WouldBlock
                        || e.kind() == std::io::ErrorKind::TimedOut =>
                {
                    if up_session.is_closed() {
                        break;
                    }
                }
                // A connected UDP socket surfaces the target's
                // ICMP port-unreachable as ConnectionRefused on
                // recv. That is UDP weather — the target process
                // is down or restarting — not a dead session: a
                // media target that restarts mid-call must find
                // the relay still standing.
                Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                    if up_session.is_closed() {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        up_session.close();
        let _ = up_writer.shutdown(Shutdown::Both);
    });
    // TCP → UDP direction (this thread).
    let mut parser = CapsuleParser::new();
    let mut chunk = vec![0u8; 16 * 1024];
    loop {
        match reader.read(&mut chunk) {
            Ok(0) => break, // client closed the connection
            Ok(n) => {
                parser.push(&chunk[..n]);
                let mut protocol_error = false;
                loop {
                    match parser.next_datagram() {
                        Ok(Some(payload)) => {
                            session.touch();
                            match udp.send(&payload) {
                                Ok(_) => {}
                                // The pending ICMP error from a
                                // down target lands here once;
                                // drop the datagram (UDP is lossy)
                                // and keep the session.
                                Err(ref e) if e.kind() == std::io::ErrorKind::ConnectionRefused => {
                                }
                                Err(_) => {
                                    protocol_error = true;
                                    break;
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(why) => {
                            eprintln!(
                                "fleet proxy connect-udp: closing {peer}: protocol error: {why}"
                            );
                            protocol_error = true;
                            break;
                        }
                    }
                }
                if protocol_error {
                    break;
                }
            }
            Err(ref e)
                if e.kind() == std::io::ErrorKind::WouldBlock
                    || e.kind() == std::io::ErrorKind::TimedOut =>
            {
                if session.is_closed() {
                    break;
                }
            }
            Err(_) => break,
        }
    }
    session.close();
    let _ = writer.shutdown(Shutdown::Both);
    let _ = up.join();
}

/// Bind the CONNECT-UDP listener at `listen_addr` (from config,
/// default loopback — see fleet-config).
pub fn listen(listen_addr: &str) -> std::io::Result<TcpListener> {
    TcpListener::bind(listen_addr)
}

/// The accept loop: serve connections until the listener dies.
/// Spawned on its own thread by [`crate::serve`] when the
/// capability is enabled; also driven directly by the tests and
/// the proof harness.
pub fn serve(listener: TcpListener) {
    for stream in listener.incoming() {
        match stream {
            Ok(stream) => {
                std::thread::spawn(move || serve_session(stream));
            }
            Err(_) => continue,
        }
    }
}

/// The `fleet-connect-udp` usage text (#6134), printed to
/// stdout by the bin help intercept at exit 0 and to stderr
/// after a usage error. The bin takes no arguments at all:
/// behaviour is governed entirely by the loaded fleet config.
pub const CONNECT_UDP_USAGE: &str = "usage: fleet-connect-udp

The standalone CONNECT-UDP endpoint (RFC 9298). Takes no
arguments: whether the endpoint serves, and what it binds,
is governed by the loaded fleet config ($FLEET_CONFIG, else
~/.config/fleet/fleet.toml) — it serves only when that config
opts in with [proxy] connect_udp = true, and stays dark
(exit 69) otherwise.";

/// What the standalone bin should do with its argument vector,
/// decided before any config is loaded or anything is bound
/// (#6134, the #5488/#5568 class: a --help or a typo must never
/// start a network endpoint).
#[derive(Debug, PartialEq, Eq)]
pub enum BinArgs {
    /// No arguments: run the endpoint (bare behaviour unchanged).
    Run,
    /// -h/--help present: print the usage to stdout, exit 0.
    Help,
    /// Any other argument: a usage error naming that argument,
    /// exit EXIT_USAGE (2).
    UsageError(String),
}

/// Classify the bin argument vector. A help flag anywhere wins
/// (the `run_init` precedent, #5619); otherwise the first
/// argument — any argument, since the bin takes none — is the
/// usage error.
pub fn classify_bin_args(argv: &[String]) -> BinArgs {
    if argv.iter().any(|a| matches!(a.as_str(), "-h" | "--help")) {
        return BinArgs::Help;
    }
    match argv.first() {
        None => BinArgs::Run,
        Some(first) => BinArgs::UsageError(first.clone()),
    }
}

/// The serve-side entry: bind per config and serve. A bind
/// failure is returned for the caller to log — an opt-in spike
/// reports its own failure and never takes the broker down
/// with it.
pub fn run(config: &Config) -> Result<(), String> {
    let addr = config.connect_udp_listen();
    let listener =
        listen(addr).map_err(|e| format!("cannot bind CONNECT-UDP listener {addr}: {e}"))?;
    println!(
        "fleet proxy connect-udp on {} (RFC 9298, spike #6109 — capability only, not a cutover)",
        listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| addr.to_string())
    );
    let _ = std::io::stdout().flush();
    serve(listener);
    Ok(())
}

// ---------------------------------------------------------------------
// A minimal client — the tests' and the proof harness's way in.
// (A production client shim for a real Moonlight cutover is the
// follow-up work the #6109 design note names; this client exists
// to prove the server half.)

/// One CONNECT-UDP session through the proxy.
pub struct UdpProxyClient {
    reader: BufReader<TcpStream>,
    writer: TcpStream,
    parser: CapsuleParser,
}

impl UdpProxyClient {
    /// Open a session to `target_host:target_port` through the
    /// proxy at `proxy_addr` ("host:port").
    pub fn connect(proxy_addr: &str, target_host: &str, target_port: u16) -> std::io::Result<Self> {
        let stream = TcpStream::connect(proxy_addr)?;
        let _ = stream.set_nodelay(true);
        let writer = stream.try_clone()?;
        let mut reader = BufReader::new(stream);
        {
            let mut w = &writer;
            write!(
                w,
                "CONNECT-UDP {WELL_KNOWN_UDP_PREFIX}{target_host}/{target_port}/ HTTP/1.1\r\n\
                 Host: {proxy_addr}\r\nCapsule-Protocol: ?1\r\n\r\n"
            )?;
            w.flush()?;
        }
        let mut total = 0usize;
        let status_line = read_head_line(&mut reader, &mut total)?
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "no response"))?;
        let status: u16 = status_line
            .split(' ')
            .nth(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("malformed proxy response: {status_line}"),
                )
            })?;
        // Consume the remaining response headers.
        loop {
            let line = read_head_line(&mut reader, &mut total)?.ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "eof mid-response")
            })?;
            if line.is_empty() {
                break;
            }
        }
        if status != 200 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::ConnectionRefused,
                format!("proxy refused CONNECT-UDP with {status}"),
            ));
        }
        Ok(UdpProxyClient {
            reader,
            writer,
            parser: CapsuleParser::new(),
        })
    }

    /// Send one UDP datagram to the session's target.
    pub fn send(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut framed = Vec::with_capacity(payload.len() + 16);
        encode_datagram_capsule(payload, &mut framed);
        self.writer.write_all(&framed)?;
        self.writer.flush()
    }

    /// Receive the next relayed datagram. Honors the read
    /// timeout set with [`UdpProxyClient::set_read_timeout`]:
    /// a timeout surfaces as a `WouldBlock`/`TimedOut` error.
    pub fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            if let Some(d) = self
                .parser
                .next_datagram()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            {
                return Ok(d);
            }
            let n = self.reader.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "proxy closed the session",
                ));
            }
            self.parser.push(&chunk[..n]);
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.reader.get_ref().set_read_timeout(timeout)
    }

    /// Split the session into its two halves so a sender and a
    /// receiver can work concurrently — two buffered readers on
    /// one stream would split the capsule stream arbitrarily,
    /// so the halves share the socket but never a buffer: the
    /// sender holds only the write side, the receiver the only
    /// `BufReader`.
    pub fn split(self) -> (UdpProxySender, UdpProxyReceiver) {
        (
            UdpProxySender {
                writer: self.writer,
            },
            UdpProxyReceiver {
                reader: self.reader,
                parser: self.parser,
            },
        )
    }
}

/// The sending half of a [`UdpProxyClient`] session.
pub struct UdpProxySender {
    writer: TcpStream,
}

impl UdpProxySender {
    pub fn send(&mut self, payload: &[u8]) -> std::io::Result<()> {
        let mut framed = Vec::with_capacity(payload.len() + 16);
        encode_datagram_capsule(payload, &mut framed);
        self.writer.write_all(&framed)?;
        self.writer.flush()
    }
}

/// The receiving half of a [`UdpProxyClient`] session.
pub struct UdpProxyReceiver {
    reader: BufReader<TcpStream>,
    parser: CapsuleParser,
}

impl UdpProxyReceiver {
    pub fn recv(&mut self) -> std::io::Result<Vec<u8>> {
        let mut chunk = vec![0u8; 16 * 1024];
        loop {
            if let Some(d) = self
                .parser
                .next_datagram()
                .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?
            {
                return Ok(d);
            }
            let n = self.reader.read(&mut chunk)?;
            if n == 0 {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "proxy closed the session",
                ));
            }
            self.parser.push(&chunk[..n]);
        }
    }

    pub fn set_read_timeout(&self, timeout: Option<Duration>) -> std::io::Result<()> {
        self.reader.get_ref().set_read_timeout(timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::ErrorKind;

    #[test]
    fn bin_args_help_usage_error_and_bare() {
        // #6134: the three argument shapes are pinned before any
        // config load or bind can happen — help succeeds, an
        // unknown argument is a usage error naming it, and bare
        // runs the endpoint exactly as before.
        let strings = |list: &[&str]| list.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        for flag in ["--help", "-h"] {
            assert_eq!(
                classify_bin_args(&strings(&[flag])),
                BinArgs::Help,
                "{flag}"
            );
        }
        assert_eq!(classify_bin_args(&strings(&[])), BinArgs::Run);
        assert_eq!(
            classify_bin_args(&strings(&["--bogus-junk"])),
            BinArgs::UsageError("--bogus-junk".to_string())
        );
        assert_eq!(
            classify_bin_args(&strings(&["extra"])),
            BinArgs::UsageError("extra".to_string())
        );
        // Help anywhere wins, even alongside another argument.
        assert_eq!(
            classify_bin_args(&strings(&["--bogus-junk", "--help"])),
            BinArgs::Help
        );
        // The usage states the contract the help shape promises.
        assert!(CONNECT_UDP_USAGE.contains("Takes no"));
        assert!(CONNECT_UDP_USAGE.contains("connect_udp = true"));
    }

    #[test]
    fn varint_round_trip() {
        for value in [
            0u64,
            1,
            63,
            64,
            300,
            16_383,
            16_384,
            1_073_741_823,
            1_073_741_824,
            (1 << 62) - 1,
        ] {
            let mut out = Vec::new();
            encode_varint(value, &mut out);
            assert_eq!(
                decode_varint(&out),
                Some((value, out.len())),
                "value {value}"
            );
        }
    }

    #[test]
    fn varint_decode_needs_the_whole_thing() {
        assert_eq!(decode_varint(&[]), None);
        assert_eq!(decode_varint(&[0x40]), None); // 2-byte form, 1 present
        assert_eq!(decode_varint(&[0x80, 0x00, 0x00]), None); // 4-byte form, 3 present
    }

    #[test]
    fn capsule_round_trip_split_feeds() {
        let payloads: Vec<Vec<u8>> = vec![
            b"x".to_vec(),
            vec![7u8; 300],
            (0..1200).map(|i| (i % 251) as u8).collect(),
        ];
        let mut wire = Vec::new();
        for p in &payloads {
            encode_datagram_capsule(p, &mut wire);
        }
        let mut parser = CapsuleParser::new();
        // Feed in two chunks split mid-capsule: nothing may be
        // emitted early or lost at the seam.
        let mid = wire.len() / 2;
        parser.push(&wire[..mid]);
        let mut got = Vec::new();
        while let Some(d) = parser.next_datagram().unwrap() {
            got.push(d);
        }
        parser.push(&wire[mid..]);
        while let Some(d) = parser.next_datagram().unwrap() {
            got.push(d);
        }
        assert_eq!(got, payloads);
    }

    #[test]
    fn parser_skips_unknown_capsules() {
        let mut wire = Vec::new();
        encode_varint(0x1234, &mut wire); // some other capsule type
        encode_varint(3, &mut wire);
        wire.extend_from_slice(b"abc");
        encode_datagram_capsule(b"real", &mut wire);
        let mut parser = CapsuleParser::new();
        parser.push(&wire);
        assert_eq!(parser.next_datagram().unwrap(), Some(b"real".to_vec()));
        assert_eq!(parser.next_datagram().unwrap(), None);
    }

    #[test]
    fn parser_rejects_nonzero_context() {
        // Hand-build a datagram capsule with context id 1.
        let mut datagram = vec![0u8]; // quarter stream 0
        datagram.push(1); // context 1 — not the UDP-payload context
        datagram.extend_from_slice(b"nope");
        let mut wire = Vec::new();
        encode_varint(CAPSULE_TYPE_DATAGRAM, &mut wire);
        encode_varint(datagram.len() as u64, &mut wire);
        wire.extend_from_slice(&datagram);
        let mut parser = CapsuleParser::new();
        parser.push(&wire);
        assert!(parser.next_datagram().is_err());
    }

    #[test]
    fn target_parsing() {
        assert_eq!(
            parse_target("/.well-known/masque/udp/127.0.0.1/47998/").unwrap(),
            UdpTarget {
                host: "127.0.0.1".to_string(),
                port: 47998
            }
        );
        assert_eq!(
            parse_target("/.well-known/masque/udp/laptop.example/48010").unwrap(),
            UdpTarget {
                host: "laptop.example".to_string(),
                port: 48010
            }
        );
        assert!(parse_target("/.well-known/masque/tcp/127.0.0.1/443/").is_err());
        assert!(parse_target("/.well-known/masque/udp/127.0.0.1/0/").is_err());
        assert!(parse_target("/.well-known/masque/udp//47998/").is_err());
        assert!(parse_target("example.com:47998").is_err());
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

    /// The proxy on an ephemeral loopback port.
    fn spawn_proxy() -> String {
        let listener = listen("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        std::thread::spawn(move || serve(listener));
        addr
    }

    #[test]
    fn relay_ping_pong_and_burst() {
        let echo_port = spawn_echo();
        let proxy_addr = spawn_proxy();
        let mut client = UdpProxyClient::connect(&proxy_addr, "127.0.0.1", echo_port).unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        client.send(b"hello").unwrap();
        assert_eq!(client.recv().unwrap(), b"hello".to_vec());

        // A burst well past one datagram in flight: 300 × 1200
        // bytes in windows of 16 (sent together, then drained).
        // The window matters: UDP buffers are finite, and an
        // unpaced flood is lossy *by design* at the echo socket
        // and at this relay's target socket alike — that loss
        // belongs to the paced measurement in the proof harness,
        // not to a correctness test. What is pinned here: with
        // the sender behaving like a media source (bounded
        // packets in flight), every sequence number comes back
        // exactly once.
        const N: u64 = 300;
        const WINDOW: u64 = 16;
        let mut seen = vec![false; N as usize];
        let mut seq = 0;
        while seq < N {
            let end = (seq + WINDOW).min(N);
            for s in seq..end {
                let mut payload = vec![0u8; 1200];
                payload[..8].copy_from_slice(&s.to_be_bytes());
                client.send(&payload).unwrap();
            }
            for _ in seq..end {
                let d = client.recv().unwrap();
                assert_eq!(d.len(), 1200);
                let got = u64::from_be_bytes(d[..8].try_into().unwrap());
                assert!(!seen[got as usize], "seq {got} echoed twice");
                seen[got as usize] = true;
            }
            seq = end;
        }
        assert!(seen.iter().all(|s| *s), "some datagrams never returned");
    }

    #[test]
    fn handshake_refusals() {
        let proxy_addr = spawn_proxy();
        // A plain TCP CONNECT is not this endpoint's business.
        let mut s = TcpStream::connect(&proxy_addr).unwrap();
        s.write_all(b"CONNECT example.com:443 HTTP/1.1\r\nHost: x\r\n\r\n")
            .unwrap();
        let mut reader = BufReader::new(s);
        let mut total = 0;
        let line = read_head_line(&mut reader, &mut total).unwrap().unwrap();
        assert!(line.contains("405"), "expected 405, got: {line}");
        // CONNECT-UDP without the capsule header is refused too.
        let mut s = TcpStream::connect(&proxy_addr).unwrap();
        s.write_all(
            b"CONNECT-UDP /.well-known/masque/udp/127.0.0.1/47998/ HTTP/1.1\r\nHost: x\r\n\r\n",
        )
        .unwrap();
        let mut reader = BufReader::new(s);
        let mut total = 0;
        let line = read_head_line(&mut reader, &mut total).unwrap().unwrap();
        assert!(line.contains("400"), "expected 400, got: {line}");
    }

    #[test]
    fn head_line_bound_applies_during_the_read() {
        // #6511: a single newline-free line must be refused once
        // the cumulative total crosses MAX_HEAD_BYTES, with the
        // accumulation itself capped -- not after the whole line
        // has already been read into memory.
        use std::io::Cursor;
        let mut reader = Cursor::new(vec![b'x'; MAX_HEAD_BYTES + 4096]);
        let mut total = 0;
        let err = read_head_line(&mut reader, &mut total).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(err.to_string().contains("exceeds the byte bound"), "{err}");
        // The reader stopped at most one byte past the bound, and
        // the accounting never ran past it either.
        assert!(
            reader.position() <= (MAX_HEAD_BYTES + 1) as u64,
            "consumed {}",
            reader.position()
        );
        assert!(total <= MAX_HEAD_BYTES + 1, "total {total}");
    }

    #[test]
    fn head_line_bound_is_cumulative_across_lines() {
        // #6511: the bound is on the whole head, so a second line
        // gets only the remaining budget plus the one probe byte.
        use std::io::Cursor;
        let mut data = vec![b'a'; MAX_HEAD_BYTES - 10];
        data.push(b'\n');
        data.extend(vec![b'b'; 4096]);
        let mut reader = Cursor::new(data);
        let mut total = 0;
        let line = read_head_line(&mut reader, &mut total).unwrap().unwrap();
        assert_eq!(line.len(), MAX_HEAD_BYTES - 10);
        let err = read_head_line(&mut reader, &mut total).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::InvalidData);
        assert!(total <= MAX_HEAD_BYTES + 1, "total {total}");
    }

    #[test]
    fn head_line_exactly_at_the_bound_still_reads() {
        // #6511: a line whose bytes (newline included) exactly
        // fill the budget is at the bound, not over it, and a
        // clean EOF right at the bound is still Ok(None).
        use std::io::Cursor;
        let mut data = vec![b'c'; MAX_HEAD_BYTES - 1];
        data.push(b'\n');
        let mut reader = Cursor::new(data);
        let mut total = 0;
        let line = read_head_line(&mut reader, &mut total).unwrap().unwrap();
        assert_eq!(line.len(), MAX_HEAD_BYTES - 1);
        assert_eq!(total, MAX_HEAD_BYTES);
        assert!(read_head_line(&mut reader, &mut total).unwrap().is_none());
    }

    #[test]
    fn request_head_over_bound_single_line_is_a_400() {
        // #6511: at the read_request boundary the refusal surfaces
        // as the existing 400 shape, promptly on the one line.
        use std::io::Cursor;
        let mut reader = Cursor::new(vec![b'x'; MAX_HEAD_BYTES + 1024]);
        let (status, _reason) = read_request(&mut reader).unwrap_err();
        assert_eq!(status, 400);
    }

    #[test]
    fn dead_target_keeps_the_session_open() {
        // A target that never answers is not an error at connect
        // time in UDP-land: the session opens, sends go nowhere,
        // and recv observes only the timeout. Even the target's
        // ICMP port-unreachable (port 9 has no listener) must
        // not kill the session — the relay tolerates it and the
        // client still just sees silence. Pin the behavior so
        // nobody "fixes" it into a handshake probe later.
        let proxy_addr = spawn_proxy();
        let mut client = UdpProxyClient::connect(&proxy_addr, "127.0.0.1", 9).unwrap();
        client
            .set_read_timeout(Some(Duration::from_millis(300)))
            .unwrap();
        client.send(b"anyone there").unwrap();
        match client.recv() {
            Err(e) => assert!(
                matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut),
                "unexpected error: {e}"
            ),
            Ok(d) => panic!("nothing should answer on port 9, got {} bytes", d.len()),
        }
    }
}
