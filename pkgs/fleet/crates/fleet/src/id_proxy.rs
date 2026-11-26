//! The `fleet id proxy` verb, build slice 2 of design #10391
//! (pass 2, §5 slice 2; issue #15434): the proxy modes of §4.2.
//!
//! This sub-slice lands the parts of slice 2 that do not need an
//! iroh dial:
//!
//! - **The §4.2 selection rule, complete** ([`select_mode`]):
//!   first match wins — `--mode` overrides all; `--raw` or an
//!   explicit raw endpoint path → mode 1 (raw proxy); `--opaque`
//!   or a target ALPN/protocol unknown to fleet → mode 2
//!   (relay-aware proxy); otherwise → mode 3 (initiator).
//! - **Mode 1, implemented**: an opaque byte splice between the
//!   caller's stdio and a plain (non-iroh) target — a TCP or unix
//!   endpoint. Fleet parses nothing, holds no iroh key for the
//!   flow, and is ALPN-agnostic, per §4.2 mode 1 ("local
//!   unix-socket forward, `ssh -W`-style hop").
//! - **Mode 2, fail-closed**: the relay-aware dial (CONNECT-capable
//!   relay dialer + resolver override for the relay name) is the
//!   next sub-slice of #15434. Until it lands, a mode-2 selection
//!   is a hard `EX_UNAVAILABLE` error — never a silent downgrade
//!   to mode 1 (§4.1/§4.7: failure modes error distinctly, never
//!   downgrade). Mode 3 through this verb is a usage error: the
//!   initiator verbs are `fleet id postcard` / `fleet id stream`
//!   (slice 3+, #15435/#15436); §4.7 scopes `fleet id proxy` to
//!   modes 1–2.
//!
//! Exit codes (§4.7: distinct, never a silent fallback):
//! 0 ok · 2 usage · 69 mode selected but not yet available ·
//! 71 relay unreachable · 72 identity mismatch · 73 raw target
//! unreachable. Modes 71/72 are exercised once the mode-2 dialer
//! lands; they are defined here so the catalog is stable.

use std::io::{self, Read, Write};
use std::net::{Shutdown, TcpStream};
use std::os::unix::net::UnixStream;

/// Exit code: the selected mode is not available in this build
/// yet (`EX_UNAVAILABLE`, the house code for "not here").
pub const EXIT_MODE_UNAVAILABLE: i32 = 69;
/// Exit code: the relay could not be reached (mode 2, §4.7).
pub const EXIT_RELAY_UNREACHABLE: i32 = 71;
/// Exit code: the peer's identity did not match the pinned
/// expectation (mode 2, §4.7) — fail closed, never downgrade.
pub const EXIT_IDENTITY_MISMATCH: i32 = 72;
/// Exit code: the raw (mode 1) target could not be connected.
pub const EXIT_TARGET_UNREACHABLE: i32 = 73;

/// The ALPNs fleet's initiator knows (§4.4): the ID meta protocol.
/// Any other ALPN is "unknown to fleet" for the §4.2 rule, so a
/// proxy for it selects mode 2 (opaque splice after fleet's dial)
/// rather than mode 3 (fleet terminates a protocol it does not
/// speak).
pub const KNOWN_ALPNS: &[&str] = &["/iroh-meta/3"];

/// A §4.2 transport mode, in increasing fleet involvement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProxyMode {
    /// Mode 1 — raw proxy: fleet splices bytes over an ordinary
    /// transport, parses nothing, holds no iroh key for the flow.
    Raw,
    /// Mode 2 — relay-aware proxy: fleet performs the iroh dial
    /// (session key per §4.1, relay path incl. the CONNECT shim),
    /// then splices the post-dial streams as opaque bytes.
    RelayAware,
    /// Mode 3 — initiator: fleet terminates the iroh connection.
    /// Not a proxy mode; selection can yield it, this verb refuses
    /// it (§4.7 scopes `fleet id proxy` to modes 1–2).
    Initiator,
}

impl ProxyMode {
    /// The §5/§4.2 mode number (1, 2, or 3).
    pub fn number(self) -> u8 {
        match self {
            ProxyMode::Raw => 1,
            ProxyMode::RelayAware => 2,
            ProxyMode::Initiator => 3,
        }
    }

    /// Parse a `--mode` value: `1|raw`, `2|relay|relay-aware`,
    /// `3|initiator`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "1" | "raw" => Some(ProxyMode::Raw),
            "2" | "relay" | "relay-aware" => Some(ProxyMode::RelayAware),
            "3" | "initiator" => Some(ProxyMode::Initiator),
            _ => None,
        }
    }
}

/// The caller's selection inputs, exactly as §4.2 names them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Selection {
    /// `--mode` was passed: overrides the whole rule (debugging).
    pub mode_override: Option<ProxyMode>,
    /// `--raw` was passed.
    pub raw_flag: bool,
    /// The target is an explicit raw endpoint path (a bare
    /// `host:port` or filesystem path, not an iroh ticket/NodeId).
    pub raw_endpoint: bool,
    /// `--opaque` was passed.
    pub opaque_flag: bool,
    /// The target ALPN/protocol the caller named, if any.
    pub alpn: Option<String>,
}

/// The §4.2 selection rule, first match wins:
///
/// 1. `--mode` overrides all three modes.
/// 2. `--raw`, or an explicit raw endpoint path → mode 1.
/// 3. `--opaque`, or the target ALPN/protocol is unknown to fleet
///    (including unnamed) → mode 2.
/// 4. Otherwise (a known ALPN, no forcing flag) → mode 3.
pub fn select_mode(sel: &Selection) -> ProxyMode {
    if let Some(m) = sel.mode_override {
        return m;
    }
    if sel.raw_flag || sel.raw_endpoint {
        return ProxyMode::Raw;
    }
    let alpn_known = match &sel.alpn {
        Some(a) => KNOWN_ALPNS.contains(&a.as_str()),
        None => false,
    };
    if sel.opaque_flag || !alpn_known {
        return ProxyMode::RelayAware;
    }
    ProxyMode::Initiator
}

/// Why a proxy request failed, with the §4.7 distinct exit code
/// for each kind. No variant ever falls back to another mode.
/// The relay/identity variants are constructed by the mode-2
/// dialer (next sub-slice); today the exit-code catalog test
/// constructs them, so the codes cannot drift before the dialer
/// lands.
#[derive(Debug)]
#[allow(dead_code)]
pub enum ProxyError {
    /// The relay could not be reached (mode 2).
    RelayUnreachable(String),
    /// The dialed peer's NodeId did not match `--expect-node-id`.
    IdentityMismatch,
    /// The mode-1 target could not be connected.
    TargetUnreachable(String),
    /// The selected mode is not available in this build yet.
    ModeUnavailable(ProxyMode),
}

impl ProxyError {
    pub fn exit_code(&self) -> i32 {
        match self {
            ProxyError::RelayUnreachable(_) => EXIT_RELAY_UNREACHABLE,
            ProxyError::IdentityMismatch => EXIT_IDENTITY_MISMATCH,
            ProxyError::TargetUnreachable(_) => EXIT_TARGET_UNREACHABLE,
            ProxyError::ModeUnavailable(_) => EXIT_MODE_UNAVAILABLE,
        }
    }
}

impl std::fmt::Display for ProxyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProxyError::RelayUnreachable(what) => {
                write!(f, "relay-unreachable: {what}")
            }
            ProxyError::IdentityMismatch => f.write_str(
                "identity-mismatch: peer NodeId does not match --expect-node-id; failing closed (no downgrade)",
            ),
            ProxyError::TargetUnreachable(what) => {
                write!(f, "target-unreachable: {what}")
            }
            ProxyError::ModeUnavailable(m) => write!(
                f,
                "mode-unavailable: mode {} ({m:?}) selected but its dialer has not landed yet (#15434); refusing to downgrade",
                m.number()
            ),
        }
    }
}

impl std::error::Error for ProxyError {}

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
        let hi = nibble(pair[0])?;
        let lo = nibble(pair[1])?;
        out[i] = (hi << 4) | lo;
    }
    Ok(out)
}

fn nibble(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err("--expect-node-id must be hex".to_string()),
    }
}

/// A mode-1 target: a plain endpoint fleet byte-moves to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Tcp(String),
    Unix(String),
}

/// Parse a target argument. `tcp://host:port`, `unix:///path`, a
/// bare `host:port`, or a bare `/path` are accepted; the bare
/// forms are "an explicit raw endpoint path" for the §4.2 rule.
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

/// One end of a mode-1 splice. Fleet parses nothing on these
/// streams; they exist so the splice can half-close a direction
/// (EOF propagates as a write shutdown, so the peer sees the
/// caller's end-of-input instead of a hung stream).
pub enum End {
    Tcp(TcpStream),
    Unix(UnixStream),
    /// The caller's own stdin/stdout (the `ssh -W`-style hop).
    Stdio {
        input: io::Stdin,
        output: io::Stdout,
    },
}

impl End {
    pub fn stdio() -> Self {
        End::Stdio {
            input: io::stdin(),
            output: io::stdout(),
        }
    }

    fn try_clone(&self) -> io::Result<End> {
        match self {
            End::Tcp(s) => Ok(End::Tcp(s.try_clone()?)),
            End::Unix(s) => Ok(End::Unix(s.try_clone()?)),
            End::Stdio { .. } => Ok(End::stdio()),
        }
    }

    fn shutdown_write(&self) {
        match self {
            End::Tcp(s) => {
                let _ = s.shutdown(Shutdown::Write);
            }
            End::Unix(s) => {
                let _ = s.shutdown(Shutdown::Write);
            }
            End::Stdio { .. } => {}
        }
    }
}

impl Read for End {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        match self {
            End::Tcp(s) => s.read(buf),
            End::Unix(s) => s.read(buf),
            End::Stdio { input, .. } => input.read(buf),
        }
    }
}

impl Write for End {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            End::Tcp(s) => s.write(buf),
            End::Unix(s) => s.write(buf),
            End::Stdio { output, .. } => output.write(buf),
        }
    }
    fn flush(&mut self) -> io::Result<()> {
        match self {
            End::Tcp(s) => s.flush(),
            End::Unix(s) => s.flush(),
            End::Stdio { output, .. } => output.flush(),
        }
    }
}

/// Splice two ends together, byte for byte, in both directions at
/// once (mode 1). Returns `(left→right, right→left)` byte counts.
/// A direction that reaches EOF half-closes the other end's write
/// side; the splice ends when both directions do.
pub fn splice(mut left: End, mut right: End) -> io::Result<(u64, u64)> {
    let mut left_clone = left.try_clone()?;
    let mut right_clone = right.try_clone()?;
    let forward = std::thread::spawn(move || {
        let n = io::copy(&mut left_clone, &mut right_clone);
        right_clone.shutdown_write();
        n
    });
    let backward_n = io::copy(&mut right, &mut left);
    left.shutdown_write();
    let forward_n = forward
        .join()
        .unwrap_or(Err(io::Error::other("splice thread panicked")));
    Ok((forward_n?, backward_n?))
}

/// Connect a mode-1 target. Failure is
/// [`ProxyError::TargetUnreachable`] — distinct from a relay
/// failure, per §4.7.
pub fn connect_target(target: &Target) -> Result<End, ProxyError> {
    match target {
        Target::Tcp(addr) => TcpStream::connect(addr)
            .map(End::Tcp)
            .map_err(|e| ProxyError::TargetUnreachable(format!("tcp {addr}: {e}"))),
        Target::Unix(path) => UnixStream::connect(path)
            .map(End::Unix)
            .map_err(|e| ProxyError::TargetUnreachable(format!("unix {path}: {e}"))),
    }
}

const USAGE: &str = "\
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
byte for byte; fleet parses nothing. Mode 2's dialer (CONNECT
relay shim + resolver override) is the next sub-slice of #15434:
until it lands a mode-2 selection exits 69 rather than silently
downgrading. Exit codes: 0 ok · 2 usage · 69 mode unavailable ·
71 relay unreachable · 72 identity mismatch · 73 target
unreachable.";

/// `fleet id proxy` — see [`USAGE`]. Returns a process exit code.
pub fn run(args: &[String]) -> i32 {
    let mut rest = args.iter();
    match rest.next().map(|s| s.as_str()) {
        Some("proxy") => {}
        Some(other) => {
            eprintln!("error: unknown 'fleet id' command '{other}'\n\n{USAGE}");
            return 2;
        }
        None => {
            eprintln!("error: 'fleet id' needs a command\n\n{USAGE}");
            return 2;
        }
    }
    let mut sel = Selection::default();
    let mut target_arg: Option<String> = None;
    let mut expect: Option<[u8; 32]> = None;
    let mut relay: Option<String> = None;
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" | "help" => {
                println!("{USAGE}");
                return 0;
            }
            "--raw" => sel.raw_flag = true,
            "--opaque" => sel.opaque_flag = true,
            "--mode" => match rest.next() {
                Some(v) => match ProxyMode::parse(v) {
                    Some(m) => sel.mode_override = Some(m),
                    None => {
                        eprintln!("error: --mode must be 1|2|3|raw|relay-aware|initiator");
                        return 2;
                    }
                },
                None => {
                    eprintln!("error: --mode needs a value");
                    return 2;
                }
            },
            "--alpn" => match rest.next() {
                Some(v) => sel.alpn = Some(v.clone()),
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
                eprintln!("error: unknown argument '{other}'\n\n{USAGE}");
                return 2;
            }
            other => {
                if target_arg.is_some() {
                    eprintln!("error: exactly one <target> is allowed\n\n{USAGE}");
                    return 2;
                }
                target_arg = Some(other.to_string());
            }
        }
    }
    let target_arg = match target_arg {
        Some(t) => t,
        None => {
            eprintln!("error: 'fleet id proxy' needs a <target>\n\n{USAGE}");
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
    sel.raw_endpoint = raw_endpoint;
    let mode = select_mode(&sel);
    // The mode-2-only flags are meaningless (and refused, not
    // ignored) for a raw splice: silently accepting a NodeId pin
    // fleet cannot check would be a silent downgrade of the
    // caller's stated intent.
    if mode == ProxyMode::Raw && (expect.is_some() || relay.is_some()) {
        eprintln!(
            "error: --expect-node-id/--relay belong to the mode-2 dial; mode 1 holds no iroh key and cannot honor them (refusing, not ignoring)"
        );
        return 2;
    }
    match mode {
        ProxyMode::Raw => {
            let end = match connect_target(&target) {
                Ok(e) => e,
                Err(e) => {
                    eprintln!("error: {e}");
                    return e.exit_code();
                }
            };
            match splice(End::stdio(), end) {
                Ok(_) => 0,
                Err(e) => {
                    eprintln!("error: splice failed: {e}");
                    EXIT_TARGET_UNREACHABLE
                }
            }
        }
        ProxyMode::RelayAware => {
            let e = ProxyError::ModeUnavailable(mode);
            eprintln!("error: {e}");
            e.exit_code()
        }
        ProxyMode::Initiator => {
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
    use std::io::{Read as _, Write as _};
    use std::net::TcpListener;

    /// Reference oracle for the §4.2 rule, written directly from
    /// the design text ("first match wins: caller passed --raw or
    /// an explicit raw endpoint path → mode 1. Else the target
    /// ALPN/protocol is unknown to fleet or caller passed --opaque
    /// → mode 2. Else → mode 3. A --mode flag overrides all
    /// three"), independently of `select_mode`'s code shape.
    fn oracle(
        mode_override: Option<ProxyMode>,
        raw: bool,
        opaque: bool,
        alpn: &Option<String>,
    ) -> ProxyMode {
        if let Some(m) = mode_override {
            return m;
        }
        if raw {
            return ProxyMode::Raw;
        }
        let known = matches!(alpn, Some(a) if a == "/iroh-meta/3");
        if opaque || !known {
            return ProxyMode::RelayAware;
        }
        ProxyMode::Initiator
    }

    #[test]
    fn selection_rule_table() {
        let known = Some("/iroh-meta/3".to_string());
        let unknown = Some("/iroh-gossip/1".to_string());
        let cases: &[(Selection, ProxyMode)] = &[
            // No flags, known ALPN -> initiator.
            (
                Selection {
                    alpn: known.clone(),
                    ..Default::default()
                },
                ProxyMode::Initiator,
            ),
            // No flags, unknown or unnamed ALPN -> relay-aware.
            (
                Selection {
                    alpn: unknown.clone(),
                    ..Default::default()
                },
                ProxyMode::RelayAware,
            ),
            (Selection::default(), ProxyMode::RelayAware),
            // --raw beats everything but --mode.
            (
                Selection {
                    raw_flag: true,
                    alpn: known.clone(),
                    ..Default::default()
                },
                ProxyMode::Raw,
            ),
            // A raw endpoint path is mode 1 even with a known ALPN.
            (
                Selection {
                    raw_endpoint: true,
                    alpn: known.clone(),
                    ..Default::default()
                },
                ProxyMode::Raw,
            ),
            // --opaque forces mode 2 even for a known ALPN.
            (
                Selection {
                    opaque_flag: true,
                    alpn: known.clone(),
                    ..Default::default()
                },
                ProxyMode::RelayAware,
            ),
            // --mode overrides all, including --raw.
            (
                Selection {
                    mode_override: Some(ProxyMode::Initiator),
                    raw_flag: true,
                    ..Default::default()
                },
                ProxyMode::Initiator,
            ),
            (
                Selection {
                    mode_override: Some(ProxyMode::Raw),
                    opaque_flag: true,
                    alpn: unknown.clone(),
                    ..Default::default()
                },
                ProxyMode::Raw,
            ),
        ];
        for (sel, want) in cases {
            assert_eq!(select_mode(sel), *want, "selection: {sel:?}");
        }
    }

    proptest! {
        /// The selection matches the design-text oracle for every
        /// combination of flags and ALPNs.
        #[test]
        fn selection_matches_oracle(
            mode_override in prop::option::of(prop::sample::select(vec![
                ProxyMode::Raw, ProxyMode::RelayAware, ProxyMode::Initiator,
            ])),
            raw_flag in any::<bool>(),
            raw_endpoint in any::<bool>(),
            opaque_flag in any::<bool>(),
            alpn in prop::option::of(prop::sample::select(vec![
                "/iroh-meta/3".to_string(),
                "/iroh-meta/2".to_string(),
                "/iroh-gossip/1".to_string(),
                String::new(),
            ])),
        ) {
            let sel = Selection {
                mode_override,
                raw_flag,
                raw_endpoint,
                opaque_flag,
                alpn: alpn.clone(),
            };
            prop_assert_eq!(
                select_mode(&sel),
                oracle(mode_override, raw_flag || raw_endpoint, opaque_flag, &alpn)
            );
        }
    }

    #[test]
    fn mode_parse_round_trips_numbers() {
        for (s, m) in [
            ("1", ProxyMode::Raw),
            ("raw", ProxyMode::Raw),
            ("2", ProxyMode::RelayAware),
            ("relay-aware", ProxyMode::RelayAware),
            ("3", ProxyMode::Initiator),
            ("initiator", ProxyMode::Initiator),
        ] {
            assert_eq!(ProxyMode::parse(s), Some(m));
            assert_eq!(m.number(), s.parse::<u8>().unwrap_or(m.number()));
        }
        assert_eq!(ProxyMode::parse("4"), None);
        assert_eq!(ProxyMode::parse(""), None);
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

    #[test]
    fn failure_kinds_have_distinct_exit_codes() {
        let codes = [
            ProxyError::RelayUnreachable("x".into()).exit_code(),
            ProxyError::IdentityMismatch.exit_code(),
            ProxyError::TargetUnreachable("x".into()).exit_code(),
            ProxyError::ModeUnavailable(ProxyMode::RelayAware).exit_code(),
        ];
        let mut sorted = codes.to_vec();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            codes.len(),
            "exit codes must be distinct: {codes:?}"
        );
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
    /// that needs no special code) behind the splice; bytes the
    /// caller sends come back identical, in both directions, and
    /// the splice reports the moved counts. The caller side is a
    /// loopback TCP pair standing in for stdio so the test does
    /// not touch the process's real stdin/stdout.
    #[test]
    fn raw_splice_moves_bytes_identically_both_ways() {
        // Echo server: one accepted connection, echo until EOF.
        let echo_listener = TcpListener::bind("127.0.0.1:0").expect("bind echo");
        let echo_addr = echo_listener.local_addr().expect("addr");
        let echo = std::thread::spawn(move || {
            let (mut s, _) = echo_listener.accept().expect("accept");
            io::copy(&mut s, &mut s.try_clone().expect("clone")).expect("echo copy");
        });
        // Caller pair: driver <-> splice-left.
        let pair_listener = TcpListener::bind("127.0.0.1:0").expect("bind pair");
        let pair_addr = pair_listener.local_addr().expect("addr");
        let mut driver = TcpStream::connect(pair_addr).expect("connect pair");
        let (splice_left, _) = pair_listener.accept().expect("accept pair");
        let splice_right = TcpStream::connect(echo_addr).expect("connect echo");

        let splice_thread =
            std::thread::spawn(move || splice(End::Tcp(splice_left), End::Tcp(splice_right)));

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

    #[test]
    fn unreachable_raw_target_is_a_distinct_error() {
        // Bind then drop a listener to get a port nothing serves.
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = listener.local_addr().expect("addr").to_string();
        drop(listener);
        match connect_target(&Target::Tcp(addr)) {
            Err(e) => assert_eq!(e.exit_code(), EXIT_TARGET_UNREACHABLE),
            Ok(_) => panic!("connect to a dead port must fail"),
        }
    }
}
