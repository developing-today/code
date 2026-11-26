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
}
