//! The `fleet id` proxy modes, build slice 2 of design #10391
//! (pass 2, §5 slice 2; issue #15434): the §4.2 mode-selection rule,
//! the distinct never-downgrade dial failure modes, the CONNECT
//! request/response shape for the relay dialer in constrained
//! egress, and the opaque splice both proxy modes byte-move with.
//!
//! What this slice does NOT do: the live iroh dial itself. That
//! dial needs slice 1's key on the wire together with the relay
//! fixture, and lands with the initiator work in slices 3–4
//! (#15435, #15436). What lands here is everything about the dial
//! that is decidable without a network: which mode a request gets
//! (§4.2 first-match), how a failed dial is reported (distinctly,
//! never as a downgrade to another mode), the CONNECT bytes a
//! CONNECT-only egress proxy sees, and the byte-identical splice
//! (mode 1 byte-moves a caller-held ticket through it; the stock
//! peer needs no special code because the splice parses nothing).
//!
//! The §4.2 contract implemented here:
//!
//! - **Mode 1 — raw proxy.** Fleet splices bytes over an ordinary
//!   transport, holds no iroh key for the flow, parses nothing.
//! - **Mode 2 — relay-aware proxy.** Fleet performs the iroh dial
//!   (session key per §4.1, relay path, CONNECT-shim dialer in
//!   constrained egress) and splices the post-dial streams as
//!   opaque bytes.
//! - **Mode 3 — initiator.** Fleet terminates the connection.
//!   Only for ALPNs fleet actually speaks: [`KNOWN_ALPNS`].
//! - **Selection, first match wins:** `--raw` or an explicit raw
//!   endpoint path → mode 1. Else the target ALPN is unknown to
//!   fleet or `--opaque` was passed → mode 2. Else → mode 3. A
//!   `--mode` override beats all three (debugging); the default
//!   is the rule, not a fixed mode.
//! - **Failure modes never downgrade** (§4.1, inherited from the
//!   mcpx no-silent-downgrade rule): relay-unreachable and
//!   identity-mismatch are distinct errors with distinct codes,
//!   and neither is ever retried as a different mode or an
//!   unauthenticated route. [`DialFailure`] has no variant that
//!   means "proceed anyway".

use std::io::{Read, Write};

/// The ALPNs the initiator (mode 3) speaks. Per the design's
/// default (§8 item 5): `/iroh-meta/3` only; every other ALPN
/// goes relay-aware-proxy (mode 2) until an explicit per-ALPN
/// slice adds it here.
pub const KNOWN_ALPNS: &[&str] = &["/iroh-meta/3"];

/// One of the three §4.2 modes, in increasing fleet involvement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Mode 1: raw proxy — opaque splice, no iroh key, no parsing.
    Raw,
    /// Mode 2: relay-aware proxy — fleet dials, payload stays opaque.
    RelayAware,
    /// Mode 3: initiator — fleet terminates the connection.
    Initiator,
}

impl Mode {
    /// The §5 table's mode number (1, 2 or 3).
    pub fn number(self) -> u8 {
        match self {
            Mode::Raw => 1,
            Mode::RelayAware => 2,
            Mode::Initiator => 3,
        }
    }

    /// Parse a `--mode` override value: `1`, `2`, `3`, or the
    /// mode names `raw`, `relay-aware`, `initiator`.
    pub fn parse(s: &str) -> Option<Mode> {
        match s {
            "1" | "raw" => Some(Mode::Raw),
            "2" | "relay-aware" | "relay" => Some(Mode::RelayAware),
            "3" | "initiator" => Some(Mode::Initiator),
            _ => None,
        }
    }
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Mode::Raw => f.write_str("raw"),
            Mode::RelayAware => f.write_str("relay-aware"),
            Mode::Initiator => f.write_str("initiator"),
        }
    }
}

/// What a caller asked for, before §4.2 selection runs.
#[derive(Debug, Clone, Default)]
pub struct ProxyRequest {
    /// Caller passed `--raw`.
    pub raw: bool,
    /// Caller passed an explicit raw endpoint path.
    pub raw_endpoint: bool,
    /// Caller passed `--opaque`.
    pub opaque: bool,
    /// The target ALPN, when the caller knows it.
    pub alpn: Option<String>,
    /// A `--mode` override (debugging); beats the rule.
    pub mode_override: Option<Mode>,
}

/// True when fleet's initiator speaks `alpn`. An absent ALPN is
/// unknown by definition — fleet cannot terminate a protocol it
/// cannot name.
pub fn alpn_known(alpn: Option<&str>) -> bool {
    match alpn {
        Some(a) => KNOWN_ALPNS.contains(&a),
        None => false,
    }
}

/// The §4.2 selection rule, first match wins. Pure: the same
/// request always selects the same mode, and the property tests
/// check this function against an independently written oracle
/// of the rule as stated in the design.
pub fn select_mode(req: &ProxyRequest) -> Mode {
    if let Some(m) = req.mode_override {
        return m;
    }
    if req.raw || req.raw_endpoint {
        return Mode::Raw;
    }
    if req.opaque || !alpn_known(req.alpn.as_deref()) {
        return Mode::RelayAware;
    }
    Mode::Initiator
}

/// Why a mode 2/3 dial failed. The variants are the acceptance
/// criterion's distinct failure modes; [`DialFailure::code`] is
/// the stable string a caller matches on. There is deliberately
/// no fallback variant and no conversion from one variant to
/// another: a failure ends the dial (§4.1 fail-closed).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DialFailure {
    /// The relay (or the CONNECT shim in front of it) could not
    /// be reached, refused the CONNECT, or answered non-200.
    RelayUnreachable,
    /// The peer's NodeId is not the NodeId the caller pinned from
    /// the ticket/configuration. Fail closed; never downgrade.
    IdentityMismatch,
}

impl DialFailure {
    /// Stable machine-readable code for this failure.
    pub fn code(&self) -> &'static str {
        match self {
            DialFailure::RelayUnreachable => "relay_unreachable",
            DialFailure::IdentityMismatch => "identity_mismatch",
        }
    }
}

impl std::fmt::Display for DialFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            DialFailure::RelayUnreachable => {
                f.write_str("relay_unreachable: relay could not be reached")
            }
            DialFailure::IdentityMismatch => {
                f.write_str("identity_mismatch: peer NodeId does not match the pinned NodeId")
            }
        }
    }
}

impl std::error::Error for DialFailure {}

/// Fail-closed identity check for a completed dial: the peer must
/// present exactly the pinned NodeId (§4.1 self-certifying
/// NodeId). Any difference is [`DialFailure::IdentityMismatch`] —
/// the caller gets an error, never a downgraded connection.
pub fn ensure_peer_identity(pinned: &[u8; 32], presented: &[u8; 32]) -> Result<(), DialFailure> {
    if pinned == presented {
        Ok(())
    } else {
        Err(DialFailure::IdentityMismatch)
    }
}

/// The CONNECT request head the relay dialer sends through a
/// CONNECT-only egress proxy (the probe-2 shape: stock relay
/// dials are redirected to such a proxy in constrained egress).
/// `target` is the relay's `host:port` authority.
pub fn connect_request(target: &str) -> String {
    format!("CONNECT {target} HTTP/1.1\r\nHost: {target}\r\nProxy-Connection: keep-alive\r\n\r\n")
}

/// Judge a CONNECT response head: only a 200 status opens the
/// tunnel. Anything else — a refusal, an auth demand, a malformed
/// head — is [`DialFailure::RelayUnreachable`]; the dialer does
/// not retry the target over a plainer route (no downgrade).
pub fn check_connect_response(head: &str) -> Result<(), DialFailure> {
    let status: Option<u32> = head
        .lines()
        .next()
        .and_then(|line| line.split_whitespace().nth(1))
        .and_then(|code| code.parse().ok());
    match status {
        Some(200) => Ok(()),
        _ => Err(DialFailure::RelayUnreachable),
    }
}

/// The opaque splice: byte-move `src` to `dst` verbatim, parsing
/// nothing, and report how many bytes moved. This is the whole
/// data path of mode 1 (a caller-held ticket moves through it
/// unchanged) and the post-dial data path of mode 2.
pub fn splice<R: Read, W: Write>(src: &mut R, dst: &mut W) -> std::io::Result<u64> {
    let n = std::io::copy(src, dst)?;
    dst.flush()?;
    Ok(n)
}

const USAGE: &str = "\
Usage: fleet id proxy-mode [--raw] [--raw-endpoint] [--opaque]
                           [--alpn <alpn>] [--mode <1|2|3|name>]

Print the mode design #10391 §4.2 selects for a proxy request
(first match wins: --mode override, then --raw / --raw-endpoint
-> mode 1 raw; then --opaque or an ALPN fleet does not speak
-> mode 2 relay-aware; else mode 3 initiator). The initiator
speaks /iroh-meta/3; every other ALPN selects mode 2.";

/// `fleet id proxy-mode` — see [`USAGE`]. Returns a process exit
/// code (0 ok, 2 usage).
pub fn run_proxy_mode(args: &[String]) -> i32 {
    let mut req = ProxyRequest::default();
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" | "help" => {
                println!("{USAGE}");
                return 0;
            }
            "--raw" => req.raw = true,
            "--raw-endpoint" => req.raw_endpoint = true,
            "--opaque" => req.opaque = true,
            "--alpn" => match rest.next() {
                Some(v) => req.alpn = Some(v.clone()),
                None => {
                    eprintln!("error: --alpn needs a value");
                    return 2;
                }
            },
            "--mode" => match rest.next().and_then(|v| Mode::parse(v)) {
                Some(m) => req.mode_override = Some(m),
                None => {
                    eprintln!("error: --mode needs one of 1, 2, 3, raw, relay-aware, initiator");
                    return 2;
                }
            },
            other => {
                eprintln!("error: unknown argument '{other}'\n\n{USAGE}");
                return 2;
            }
        }
    }
    let mode = select_mode(&req);
    println!("{} {}", mode.number(), mode);
    0
}

/// Exit codes for `fleet id proxy` (§4.7: distinct, never a
/// silent fallback or downgrade). 2 is usage; 69 is the house
/// `EX_UNAVAILABLE` for a mode selected but not yet landed.
pub const EXIT_MODE_UNAVAILABLE: i32 = 69;
/// The relay (or the CONNECT shim in front of it) could not be
/// reached, refused the CONNECT, or answered non-200 (mode 2).
pub const EXIT_RELAY_UNREACHABLE: i32 = 71;
/// The dialed peer's NodeId did not match `--expect-node-id`
/// (mode 2) — fail closed, never downgrade.
pub const EXIT_IDENTITY_MISMATCH: i32 = 72;
/// The mode-1 raw target could not be connected.
pub const EXIT_TARGET_UNREACHABLE: i32 = 73;

impl DialFailure {
    /// The process exit code for this fail-closed dial failure.
    pub fn exit_code(&self) -> i32 {
        match self {
            DialFailure::RelayUnreachable => EXIT_RELAY_UNREACHABLE,
            DialFailure::IdentityMismatch => EXIT_IDENTITY_MISMATCH,
        }
    }
}

/// A mode-1 target: a plain endpoint fleet byte-moves to. Fleet
/// parses nothing on these streams and holds no iroh key for the
/// flow (§4.2 mode 1, the `ssh -W`-style hop).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Tcp(String),
    Unix(String),
}

/// Parse a target argument. `tcp://host:port`, `unix:///path`,
/// a bare `host:port`, or a bare `/path` are accepted; the bare
/// forms count as "an explicit raw endpoint path" for the §4.2
/// rule (second element of the return).
pub fn parse_target(s: &str) -> Result<(Target, bool), String> {
    if let Some(rest) = s.strip_prefix("tcp://") {
        if rest.is_empty() {
            return Err("tcp:// target needs a host:port".to_string());
        }
        Ok((Target::Tcp(rest.to_string()), false))
    } else if let Some(rest) = s.strip_prefix("unix://") {
        if rest.is_empty() {
            return Err("unix:// target needs a path".to_string());
        }
        Ok((Target::Unix(rest.to_string()), false))
    } else if s.contains("://") {
        Err(format!(
            "unsupported target scheme in '{s}' (want tcp://, unix://, host:port, or /path)"
        ))
    } else if s.starts_with('/') {
        Ok((Target::Unix(s.to_string()), true))
    } else if s.contains(':') {
        Ok((Target::Tcp(s.to_string()), true))
    } else {
        Err(format!(
            "cannot tell what target '{s}' is (want tcp://, unix://, host:port, or /path)"
        ))
    }
}

/// Validate a `--expect-node-id` pin: exactly 64 hex characters
/// (a 32-byte NodeId, the §4.1 form). A malformed pin is a usage
/// error, never an identity mismatch and never ignored.
pub fn parse_expect_node_id(s: &str) -> Result<[u8; 32], String> {
    let bytes = s.as_bytes();
    if bytes.len() != 64 {
        return Err(format!(
            "--expect-node-id must be 64 hex characters (32 bytes), got {}",
            bytes.len()
        ));
    }
    let mut out = [0u8; 32];
    for (i, pair) in bytes.chunks_exact(2).enumerate() {
        let hi = hex_val(pair[0])?;
        let lo = hex_val(pair[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn hex_val(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err("--expect-node-id must be hex".to_string()),
    }
}

/// One end of a mode-1 duplex splice. Fleet parses nothing on
/// these streams; the splice half-closes a direction on EOF so
/// the peer sees the caller's end-of-input instead of a hung
/// stream.
pub enum End {
    Tcp(std::net::TcpStream),
    Unix(std::os::unix::net::UnixStream),
    /// The caller's own stdin/stdout (the `ssh -W`-style hop).
    Stdio {
        input: std::io::Stdin,
        output: std::io::Stdout,
    },
}

impl End {
    pub fn stdio() -> Self {
        End::Stdio {
            input: std::io::stdin(),
            output: std::io::stdout(),
        }
    }

    fn try_clone(&self) -> std::io::Result<End> {
        match self {
            End::Tcp(s) => Ok(End::Tcp(s.try_clone()?)),
            End::Unix(s) => Ok(End::Unix(s.try_clone()?)),
            End::Stdio { .. } => Ok(End::stdio()),
        }
    }

    fn shutdown_write(&self) {
        match self {
            End::Tcp(s) => {
                let _ = s.shutdown(std::net::Shutdown::Write);
            }
            End::Unix(s) => {
                let _ = s.shutdown(std::net::Shutdown::Write);
            }
            End::Stdio { .. } => {}
        }
    }
}

impl Read for End {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        match self {
            End::Tcp(s) => s.read(buf),
            End::Unix(s) => s.read(buf),
            End::Stdio { input, .. } => input.read(buf),
        }
    }
}

impl Write for End {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        match self {
            End::Tcp(s) => s.write(buf),
            End::Unix(s) => s.write(buf),
            End::Stdio { output, .. } => output.write(buf),
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        match self {
            End::Tcp(s) => s.flush(),
            End::Unix(s) => s.flush(),
            End::Stdio { output, .. } => output.flush(),
        }
    }
}

/// Splice two ends together, byte for byte, in both directions
/// at once (mode 1). Returns `(left→right, right→left)` byte
/// counts. A direction that reaches EOF half-closes the other
/// end's write side; the splice ends when both directions do.
pub fn splice_duplex(mut left: End, mut right: End) -> std::io::Result<(u64, u64)> {
    let mut left_clone = left.try_clone()?;
    let mut right_clone = right.try_clone()?;
    let forward = std::thread::spawn(move || {
        let n = std::io::copy(&mut left_clone, &mut right_clone);
        right_clone.shutdown_write();
        n
    });
    let backward_n = std::io::copy(&mut right, &mut left);
    left.shutdown_write();
    let forward_n = forward
        .join()
        .unwrap_or(Err(std::io::Error::other("splice thread panicked")));
    Ok((forward_n?, backward_n?))
}

/// Connect a mode-1 target. Failure is exit 73
/// ([`EXIT_TARGET_UNREACHABLE`]) — distinct from a relay
/// failure, per §4.7.
pub fn connect_target(target: &Target) -> Result<End, (i32, String)> {
    match target {
        Target::Tcp(addr) => std::net::TcpStream::connect(addr)
            .map(End::Tcp)
            .map_err(|e| {
                (
                    EXIT_TARGET_UNREACHABLE,
                    format!("target-unreachable: tcp {addr}: {e}"),
                )
            }),
        Target::Unix(path) => std::os::unix::net::UnixStream::connect(path)
            .map(End::Unix)
            .map_err(|e| {
                (
                    EXIT_TARGET_UNREACHABLE,
                    format!("target-unreachable: unix {path}: {e}"),
                )
            }),
    }
}

const PROXY_USAGE: &str = "\
Usage: fleet id proxy [--raw | --opaque | --mode <1|2|3>] [--alpn <alpn>]
                      [--relay <url>] [--expect-node-id <hex>] <target>

Byte-move to an ID peer under the design #10391 §4.2 mode rule
(first match wins): --mode overrides all; --raw or a raw endpoint
path -> mode 1 (raw proxy); --opaque or an ALPN fleet does not
know -> mode 2 (relay-aware proxy); otherwise mode 3 (initiator,
served by 'fleet id postcard'/'fleet id stream', not this verb).

  <target>        tcp://host:port | unix:///path | host:port | /path
  --alpn <alpn>   the target protocol (fleet knows /iroh-meta/3)
  --relay <url>   relay for the mode-2 dial (next sub-slice)
  --expect-node-id <hex>
                  pin the peer NodeId for the mode-2 dial; a
                  mismatch fails closed (exit 72), never downgrades

Mode 1 splices this process's stdin/stdout to the target,
byte for byte; fleet parses nothing. Mode 2's live dialer
(CONNECT relay shim + resolver override) is the next sub-slice
of #15434: until it lands a mode-2 selection exits 69 rather
than silently downgrading. Exit codes: 0 ok · 2 usage ·
69 mode unavailable · 71 relay unreachable · 72 identity
mismatch · 73 target unreachable.";

/// `fleet id proxy` — see [`PROXY_USAGE`]. Returns a process
/// exit code. The first element of `args` is the literal
/// `"proxy"` (stripped by the `id` domain router in `main.rs`).
pub fn run(args: &[String]) -> i32 {
    let mut rest = args.iter();
    match rest.next().map(|s| s.as_str()) {
        Some("proxy") => {}
        Some(other) => {
            eprintln!("error: unknown 'fleet id' command '{other}'\n\n{PROXY_USAGE}");
            return 2;
        }
        None => {
            eprintln!("error: 'fleet id proxy' needs a <target>\n\n{PROXY_USAGE}");
            return 2;
        }
    }
    let mut req = ProxyRequest::default();
    let mut target_arg: Option<String> = None;
    let mut expect: Option<[u8; 32]> = None;
    let mut relay: Option<String> = None;
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" | "help" => {
                println!("{PROXY_USAGE}");
                return 0;
            }
            "--raw" => req.raw = true,
            "--opaque" => req.opaque = true,
            "--mode" => match rest.next().and_then(|v| Mode::parse(v)) {
                Some(m) => req.mode_override = Some(m),
                None => {
                    eprintln!("error: --mode must be one of 1, 2, 3, raw, relay-aware, initiator");
                    return 2;
                }
            },
            "--alpn" => match rest.next() {
                Some(v) => req.alpn = Some(v.clone()),
                None => {
                    eprintln!("error: --alpn needs a value");
                    return 2;
                }
            },
            "--relay" => match rest.next() {
                Some(v) => relay = Some(v.clone()),
                None => {
                    eprintln!("error: --relay needs a value");
                    return 2;
                }
            },
            "--expect-node-id" => match rest.next() {
                Some(v) => match parse_expect_node_id(v) {
                    Ok(pin) => expect = Some(pin),
                    Err(e) => {
                        eprintln!("error: {e}");
                        return 2;
                    }
                },
                None => {
                    eprintln!("error: --expect-node-id needs a value");
                    return 2;
                }
            },
            other if other.starts_with("--") => {
                eprintln!("error: unknown argument '{other}'\n\n{PROXY_USAGE}");
                return 2;
            }
            other => {
                if target_arg.is_some() {
                    eprintln!("error: exactly one <target> is allowed\n\n{PROXY_USAGE}");
                    return 2;
                }
                target_arg = Some(other.to_string());
            }
        }
    }
    let target_arg = match target_arg {
        Some(t) => t,
        None => {
            eprintln!("error: 'fleet id proxy' needs a <target>\n\n{PROXY_USAGE}");
            return 2;
        }
    };
    let (target, raw_endpoint) = match parse_target(&target_arg) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("error: {e}");
            return 2;
        }
    };
    req.raw_endpoint = raw_endpoint;
    let mode = select_mode(&req);
    // The mode-2-only flags are meaningless (and refused, not
    // ignored) for a raw splice: silently accepting a NodeId pin
    // fleet cannot check would be a silent downgrade of the
    // caller's stated intent.
    if mode == Mode::Raw && (expect.is_some() || relay.is_some()) {
        eprintln!(
            "error: --expect-node-id/--relay belong to the mode-2 dial; mode 1 holds no iroh key and cannot honor them (refusing, not ignoring)"
        );
        return 2;
    }
    match mode {
        Mode::Raw => match connect_target(&target) {
            Ok(end) => match splice_duplex(End::stdio(), end) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("error: splice failed: {e}");
                    EXIT_TARGET_UNREACHABLE
                }
            },
            Err((code, msg)) => {
                eprintln!("error: {msg}");
                code
            }
        },
        Mode::RelayAware => {
            eprintln!(
                "error: mode-unavailable: mode 2 (relay-aware) selected but its live iroh dialer has not landed yet (#15434); refusing to downgrade"
            );
            EXIT_MODE_UNAVAILABLE
        }
        Mode::Initiator => {
            eprintln!(
                "error: mode 3 (initiator) is not a proxy mode; use 'fleet id postcard' or 'fleet id stream' (design #10391 §4.7)"
            );
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn req(
        raw: bool,
        raw_endpoint: bool,
        opaque: bool,
        alpn: Option<&str>,
        mode_override: Option<Mode>,
    ) -> ProxyRequest {
        ProxyRequest {
            raw,
            raw_endpoint,
            opaque,
            alpn: alpn.map(str::to_string),
            mode_override,
        }
    }

    /// Oracle for §4.2, written directly from the design text:
    /// "--raw or an explicit raw endpoint path -> mode 1. Else
    /// the target ALPN/protocol is unknown to fleet or caller
    /// passed --opaque -> mode 2. Else -> mode 3. A --mode flag
    /// overrides all three."
    fn oracle(
        raw: bool,
        raw_endpoint: bool,
        opaque: bool,
        alpn: Option<&str>,
        mode_override: Option<Mode>,
    ) -> Mode {
        if let Some(m) = mode_override {
            return m;
        }
        if raw || raw_endpoint {
            return Mode::Raw;
        }
        let known = matches!(alpn, Some("/iroh-meta/3"));
        if opaque || !known {
            return Mode::RelayAware;
        }
        Mode::Initiator
    }

    #[test]
    fn selection_examples_from_the_design() {
        // --raw wins over everything but --mode.
        assert_eq!(
            select_mode(&req(true, false, true, Some("/iroh-meta/3"), None)),
            Mode::Raw
        );
        // Explicit raw endpoint path -> mode 1.
        assert_eq!(select_mode(&req(false, true, false, None, None)), Mode::Raw);
        // Unknown ALPN -> mode 2 (forward-compat, sealed ALPNs).
        assert_eq!(
            select_mode(&req(false, false, false, Some("/experimental/9"), None)),
            Mode::RelayAware
        );
        // --opaque on a known ALPN still -> mode 2.
        assert_eq!(
            select_mode(&req(false, false, true, Some("/iroh-meta/3"), None)),
            Mode::RelayAware
        );
        // No ALPN named -> unknown -> mode 2.
        assert_eq!(
            select_mode(&req(false, false, false, None, None)),
            Mode::RelayAware
        );
        // The one known ALPN, no flags -> mode 3.
        assert_eq!(
            select_mode(&req(false, false, false, Some("/iroh-meta/3"), None)),
            Mode::Initiator
        );
        // --mode overrides the rule in both directions.
        assert_eq!(
            select_mode(&req(true, false, false, None, Some(Mode::Initiator))),
            Mode::Initiator
        );
        assert_eq!(
            select_mode(&req(
                false,
                false,
                false,
                Some("/iroh-meta/3"),
                Some(Mode::Raw)
            )),
            Mode::Raw
        );
    }

    proptest! {
        /// Selection always matches the §4.2 oracle, for every
        /// combination of flags, ALPNs and overrides.
        #[test]
        fn selection_matches_design_oracle(
            raw in any::<bool>(),
            raw_endpoint in any::<bool>(),
            opaque in any::<bool>(),
            alpn in prop_oneof![
                Just(None),
                Just(Some("/iroh-meta/3")),
                Just(Some("/iroh-blobs/2")),
                Just(Some("")),
            ],
            mode_override in prop_oneof![
                Just(None),
                Just(Some(Mode::Raw)),
                Just(Some(Mode::RelayAware)),
                Just(Some(Mode::Initiator)),
            ],
        ) {
            let got = select_mode(&req(raw, raw_endpoint, opaque, alpn, mode_override));
            prop_assert_eq!(got, oracle(raw, raw_endpoint, opaque, alpn, mode_override));
        }

        /// The splice byte-moves any payload verbatim — the mode 1
        /// acceptance shape (a caller-held ticket crosses fleet
        /// unchanged; the peer needs no special code).
        #[test]
        fn splice_moves_bytes_verbatim(payload in proptest::collection::vec(any::<u8>(), 0..65536)) {
            let mut src = std::io::Cursor::new(payload.clone());
            let mut dst: Vec<u8> = Vec::new();
            let n = splice(&mut src, &mut dst).expect("splice");
            prop_assert_eq!(n, payload.len() as u64);
            prop_assert_eq!(dst, payload);
        }
    }

    #[test]
    fn splice_moves_a_ticket_sized_payload_verbatim() {
        // A caller-held ticket is opaque text to fleet; 4 KiB of
        // it crosses the splice sha-identical (byte equality is
        // the check — no parsing happened on the way).
        let ticket: Vec<u8> = (0..4096u32).map(|i| (i % 251) as u8).collect();
        let mut src = std::io::Cursor::new(ticket.clone());
        let mut dst: Vec<u8> = Vec::new();
        assert_eq!(splice(&mut src, &mut dst).expect("splice"), 4096);
        assert_eq!(dst, ticket);
    }

    #[test]
    fn failure_modes_are_distinct_and_never_downgrade() {
        // Distinct codes a caller can match on.
        assert_ne!(
            DialFailure::RelayUnreachable.code(),
            DialFailure::IdentityMismatch.code()
        );
        // Identity: exact pin passes, any other NodeId fails
        // closed with the identity variant specifically.
        let pinned = [7u8; 32];
        assert_eq!(ensure_peer_identity(&pinned, &pinned), Ok(()));
        assert_eq!(
            ensure_peer_identity(&pinned, &[8u8; 32]),
            Err(DialFailure::IdentityMismatch)
        );
        // CONNECT: only 200 opens the tunnel; refusals and junk
        // are relay-unreachable, never a plainer retry.
        assert_eq!(
            check_connect_response("HTTP/1.1 200 Connection Established\r\n\r\n"),
            Ok(())
        );
        for head in [
            "HTTP/1.1 403 Forbidden\r\n\r\n",
            "HTTP/1.1 407 Proxy Authentication Required\r\n\r\n",
            "HTTP/1.1 502 Bad Gateway\r\n\r\n",
            "garbage",
            "",
        ] {
            assert_eq!(
                check_connect_response(head),
                Err(DialFailure::RelayUnreachable),
                "head: {head:?}"
            );
        }
    }

    #[test]
    fn connect_request_is_a_connect_head_for_the_target() {
        let head = connect_request("relay.example:443");
        assert!(head.starts_with("CONNECT relay.example:443 HTTP/1.1\r\n"));
        assert!(head.contains("Host: relay.example:443\r\n"));
        assert!(head.ends_with("\r\n\r\n"));
    }

    #[test]
    fn proxy_exit_codes_are_distinct_and_stable() {
        let mut codes = vec![
            EXIT_MODE_UNAVAILABLE,
            EXIT_RELAY_UNREACHABLE,
            EXIT_IDENTITY_MISMATCH,
            EXIT_TARGET_UNREACHABLE,
        ];
        codes.sort_unstable();
        codes.dedup();
        assert_eq!(codes.len(), 4, "proxy exit codes must be distinct");
        assert_eq!(DialFailure::RelayUnreachable.exit_code(), 71);
        assert_eq!(DialFailure::IdentityMismatch.exit_code(), 72);
    }

    #[test]
    fn target_parsing_marks_bare_forms_raw() {
        assert_eq!(
            parse_target("tcp://127.0.0.1:9"),
            Ok((Target::Tcp("127.0.0.1:9".into()), false))
        );
        assert_eq!(
            parse_target("unix:///tmp/sock"),
            Ok((Target::Unix("/tmp/sock".into()), false))
        );
        assert_eq!(
            parse_target("127.0.0.1:9"),
            Ok((Target::Tcp("127.0.0.1:9".into()), true))
        );
        assert_eq!(
            parse_target("/tmp/sock"),
            Ok((Target::Unix("/tmp/sock".into()), true))
        );
        assert!(parse_target("iroh://ticket").is_err());
        assert!(parse_target("just-a-name").is_err());
    }

    #[test]
    fn expect_node_id_pin_is_strict() {
        let ok = parse_expect_node_id(&"ab".repeat(32)).expect("64 hex");
        assert_eq!(ok, [0xab; 32]);
        for bad in [
            "",
            "ab",
            &"ab".repeat(31),
            &"ab".repeat(33),
            &"zz".repeat(32),
        ] {
            assert!(parse_expect_node_id(bad).is_err(), "input: {bad:?}");
        }
    }

    fn proxy_args(words: &[&str]) -> Vec<String> {
        words.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn proxy_mode2_without_dialer_is_fail_closed_69() {
        // No network is touched: the mode-2 live dialer has not
        // landed, so selection alone decides the exit.
        assert_eq!(
            run(&proxy_args(&["proxy", "--mode", "2", "127.0.0.1:9"])),
            EXIT_MODE_UNAVAILABLE
        );
        assert_eq!(
            run(&proxy_args(&["proxy", "--opaque", "127.0.0.1:9"])),
            EXIT_MODE_UNAVAILABLE
        );
    }

    #[test]
    fn proxy_mode1_refuses_mode2_flags_instead_of_ignoring_them() {
        let pin = "ab".repeat(32);
        assert_eq!(
            run(&proxy_args(&[
                "proxy",
                "--mode",
                "1",
                "--expect-node-id",
                &pin,
                "127.0.0.1:9"
            ])),
            2
        );
        assert_eq!(
            run(&proxy_args(&[
                "proxy",
                "--mode",
                "1",
                "--relay",
                "https://relay.example:443",
                "127.0.0.1:9"
            ])),
            2
        );
    }

    #[test]
    fn proxy_mode3_through_this_verb_is_a_usage_error() {
        assert_eq!(
            run(&proxy_args(&[
                "proxy",
                "--mode",
                "3",
                "--alpn",
                "/iroh-meta/3",
                "127.0.0.1:9"
            ])),
            2
        );
    }

    #[test]
    fn unreachable_raw_target_is_the_distinct_73_error() {
        // Bind then drop a listener to get a port nothing serves.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        drop(listener);
        match connect_target(&Target::Tcp(addr.clone())) {
            Err((code, _)) => assert_eq!(code, EXIT_TARGET_UNREACHABLE),
            Ok(_) => panic!("connect to a dead port must fail"),
        }
        // The verb itself maps the same failure to exit 73.
        assert_eq!(
            run(&proxy_args(&["proxy", "--mode", "1", &addr])),
            EXIT_TARGET_UNREACHABLE
        );
    }

    /// Deterministic byte stream (splitmix64) — the fixture's
    /// oracle is byte identity, not the generator.
    fn fixture_bytes(n: usize, seed: u64) -> Vec<u8> {
        let mut state = seed;
        (0..n)
            .map(|_| {
                state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
                let mut z = state;
                z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
                z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
                (z ^ (z >> 31)) as u8
            })
            .collect()
    }

    /// Mode 1's acceptance shape: an echo peer (a "stock peer"
    /// that needs no special code) behind the duplex splice;
    /// bytes the caller sends come back identical, in both
    /// directions, and the splice reports the moved counts. The
    /// caller side is a loopback TCP pair standing in for stdio
    /// so the test does not touch the process's real stdin/out.
    #[test]
    fn raw_splice_duplex_moves_bytes_identically_both_ways() {
        use std::io::{Read as _, Write as _};
        use std::net::{Shutdown, TcpListener, TcpStream};
        // Echo server: one accepted connection, echo until EOF.
        let echo_listener = TcpListener::bind("127.0.0.1:0").expect("bind echo");
        let echo_addr = echo_listener.local_addr().expect("addr");
        let echo = std::thread::spawn(move || {
            let (mut s, _) = echo_listener.accept().expect("accept");
            std::io::copy(&mut s, &mut s.try_clone().expect("clone")).expect("echo copy");
        });
        // Caller pair: driver <-> splice-left.
        let pair_listener = TcpListener::bind("127.0.0.1:0").expect("bind pair");
        let pair_addr = pair_listener.local_addr().expect("addr");
        let mut driver = TcpStream::connect(pair_addr).expect("connect pair");
        let (splice_left, _) = pair_listener.accept().expect("accept pair");
        let splice_right = TcpStream::connect(echo_addr).expect("connect echo");

        let splice_thread = std::thread::spawn(move || {
            splice_duplex(End::Tcp(splice_left), End::Tcp(splice_right))
        });

        let payload = fixture_bytes(1 << 20, 42);
        let mut driver_clone = driver.try_clone().expect("clone driver");
        let payload_clone = payload.clone();
        let writer = std::thread::spawn(move || {
            driver_clone
                .write_all(&payload_clone)
                .expect("write payload");
            driver_clone.shutdown(Shutdown::Write).expect("half-close");
        });
        let mut echoed = Vec::new();
        driver.read_to_end(&mut echoed).expect("read echo");
        writer.join().expect("writer");
        let (fwd, back) = splice_thread.join().expect("splice").expect("splice ok");
        echo.join().expect("echo");

        assert_eq!(echoed, payload, "bytes must survive the splice identical");
        assert_eq!(fwd, payload.len() as u64);
        assert_eq!(back, payload.len() as u64);
    }
}
