//! Stream multiplexing over one tunnel — the `proxy-mux` frame family
//! (issue #5756; seam record
//! `doc/2026-11-16T10-40-00Z_design_id5756_multiplexed_streams_seam.md`
//! §4, decision D1).
//!
//! The mux is a new frame family inside this crate's existing
//! contract — same 8-byte length prefix, same JSON frame objects,
//! discriminated by a `kind` field — negotiated by the hello
//! capability [`CAP_PROXY_MUX`]. It carries many independently
//! identified request/response streams over one persistent tunnel,
//! replacing HTTP/1.1-request-per-call over a pooled CONNECT tunnel:
//! no per-call CONNECT, no pool checkout, no head-of-line blocking
//! behind one stream's frames.
//!
//! This module is the wire contract only: frame types, the bounds
//! they carry, and the credit-window semantics as a type. No peer
//! sends these frames until both hello peers advertise
//! [`CAP_PROXY_MUX`] (see [`mux_negotiated`]) — the record's
//! no-one-sided-shipment rule (§5): absence on either side leaves the
//! pooled HTTP-per-call path byte-for-byte in charge, permanently, as
//! the fallback. The proxy, client, and broker-dispatch halves that
//! speak these frames are the record's follow-on steps (§7 items
//! 2–4); nothing here changes any existing behavior.
//!
//! Frame family (record §4):
//!
//! - `open` — client→broker: stream id (client-chosen, odd), target
//!   port, method/path or raw-connect intent, trip id.
//! - `data` — either direction: stream id + payload chunk, bounded
//!   by [`MAX_DATA_PAYLOAD_BYTES`]; `fin` marks end-of-data in the
//!   sender's direction; the broker's first response frame of a
//!   request stream also carries the HTTP `status`.
//! - `window` — either direction: stream id + byte credit. A stream's
//!   receive window starts at [`INITIAL_STREAM_WINDOW_BYTES`]; a
//!   sender must not exceed granted credit. This is the backpressure
//!   bound: a slow consumer stalls its own stream, never the tunnel
//!   and never other streams.
//! - `rst` — either direction: stream id + reason. Cancellation and
//!   timeout are `rst` ([`StreamCeiling`]).
//! - `ping`/`pong` — application-level keepalive on the tunnel
//!   ([`MUX_KEEPALIVE_S`], [`MUX_IDLE_EXIT_S`]).
//!
//! A dead tunnel fails every in-flight stream immediately with an
//! `rst`-equivalent tunnel-lost error preserving each stream's trip
//! id (the ≤2 s bound of PR #6490 applies); reconnect is
//! single-flight in the proxy half. Those behaviors live in the
//! follow-on steps; the constants and reasons they key on live here.

use std::time::Duration;

use base64::Engine;
use serde::{Deserialize, Serialize};

use crate::ProtoError;

/// The hello capability id advertising the stream-multiplexing frame
/// family. A wire-negotiation capability, deliberately not a
/// fleet-registry build capability: the mux is negotiated behavior
/// between two peers, not a build surface a binary has or lacks —
/// both proxy halves advertise it once their mux code lands, and it
/// stays dormant until a peer advertises it back.
pub const CAP_PROXY_MUX: &str = "proxy-mux";

/// Payload bound for one `data` frame, in bytes (record §4: 64 KiB).
/// Larger bodies are multiple frames, so no stream can head-of-line
/// block the tunnel behind one frame for long. This bounds the
/// *decoded* payload; the frame itself still answers to the crate's
/// [`crate::MAX_FRAME_BYTES`] ceiling.
pub const MAX_DATA_PAYLOAD_BYTES: usize = 64 * 1024;

/// A stream's initial receive window, in bytes (record §4:
/// 256 KiB). A sender starts with this much credit per stream and
/// may send more only as `window` frames grant it.
pub const INITIAL_STREAM_WINDOW_BYTES: u64 = 256 * 1024;

/// Default per-stream timeout ceiling (record §4: 10 s). Cancellation
/// and timeout are `rst` ([`RST_TIMEOUT`]); on expiry the client
/// frees the stream and may retry once on a fresh stream if the
/// tunnel is alive.
pub const DEFAULT_STREAM_TIMEOUT: Duration = Duration::from_secs(10);

/// Hard cap on a per-stream timeout ceiling — well under the
/// historical 16–26 s failure cluster (#5756 §4), so no configured
/// ceiling can recreate it.
pub const MAX_STREAM_TIMEOUT: Duration = Duration::from_secs(15);

/// Application-level keepalive interval on a mux tunnel, in seconds
/// (record §4, carrying the #5756 starting constant KEEPALIVE_S).
pub const MUX_KEEPALIVE_S: u64 = 180;

/// Broker idle-exit horizon for a mux tunnel, in seconds (record
/// §4): the broker must not idle-exit while a tunnel with healthy
/// streams is up — exit only when the tunnel is dead AND no calls
/// for this long.
pub const MUX_IDLE_EXIT_S: u64 = 3600;

/// `rst` reason: the stream's owner cancelled it.
pub const RST_CANCELLED: &str = "cancelled";
/// `rst` reason: the per-stream timeout ceiling expired.
pub const RST_TIMEOUT: &str = "timeout";
/// `rst` reason: the tunnel died under the stream.
pub const RST_TUNNEL_LOST: &str = "tunnel_lost";
/// `rst` reason: the target port is not in the broker's services
/// map — the stream fails fast (the proxy-level capability stub),
/// never the tunnel.
pub const RST_PORT_NOT_SERVED: &str = "port_not_served";
/// `rst` reason: the peer violated the stream contract (frame
/// bound, window, or id rules).
pub const RST_PROTOCOL_ERROR: &str = "protocol_error";

/// Is the mux negotiated for this connection? True only when BOTH
/// hello peers advertised [`CAP_PROXY_MUX`] (record §5). Absence on
/// either side — an older peer, or the legacy major-0 dialect, which
/// has no hello at all — leaves the pooled path in charge.
pub fn mux_negotiated(client_capabilities: &[String], broker_capabilities: &[String]) -> bool {
    let has = |caps: &[String]| caps.iter().any(|c| c == CAP_PROXY_MUX);
    has(client_capabilities) && has(broker_capabilities)
}

/// A stream identifier. Client-initiated streams are odd (record
/// §4); ids are scoped to one tunnel and freed by `rst` or by both
/// directions reaching `fin`. Id 0 is never a stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct StreamId(pub u64);

impl StreamId {
    /// A client-initiated stream id: nonzero and odd. Constructing
    /// one is how the odd rule is enforced — a broker that receives
    /// an `open` whose id fails [`StreamId::is_client_initiated`]
    /// answers `rst` ([`RST_PROTOCOL_ERROR`]).
    pub fn client(n: u64) -> Result<StreamId, ProtoError> {
        let id = StreamId(n);
        if n == 0 || !id.is_client_initiated() {
            return Err(ProtoError::BadFrame(format!(
                "client stream id {n} is not a nonzero odd id"
            )));
        }
        Ok(id)
    }

    /// Was this stream initiated by the client (odd id)?
    pub fn is_client_initiated(self) -> bool {
        self.0 % 2 == 1
    }
}

/// What an `open` asks for: an HTTP-style request against the routed
/// service, or a raw byte stream to the port. Derived from the
/// frame's fields by [`StreamFrame::intent`]; the frame itself stays
/// flat, in the shape of this crate's existing [`crate::Request`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenIntent {
    Request { method: String, path: String },
    RawConnect,
}

/// One frame of the `proxy-mux` family, discriminated on the wire by
/// its `kind` field. Frames are additive within protocol major 1:
/// unknown fields are ignored, and a peer that never advertised
/// [`CAP_PROXY_MUX`] never receives one of these at all.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum StreamFrame {
    /// Open a stream. `method`+`path` together mean a request;
    /// both absent means a raw connect. `trip` is the caller's
    /// per-request id, echoed through the stream's life so trip
    /// logs on both sides keep the same id (#5756 acceptance).
    Open {
        stream: StreamId,
        port: u16,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        method: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        trip: Option<String>,
    },
    /// A payload chunk. `data` is the base64 of at most
    /// [`MAX_DATA_PAYLOAD_BYTES`] raw bytes. `fin` marks the last
    /// chunk in the sender's direction. `status` is set only on the
    /// broker's first response frame of a request stream: the HTTP
    /// status of the routed service's reply (the mux analogue of
    /// [`crate::Response`]'s status).
    Data {
        stream: StreamId,
        data: String,
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        fin: bool,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        status: Option<u16>,
    },
    /// Grant `credit` more bytes of send window on a stream.
    Window { stream: StreamId, credit: u64 },
    /// Abort a stream: cancellation, timeout, or a fast failure.
    /// `reason` is one of the `RST_*` constants, or a peer's own
    /// short machine-readable string.
    Rst { stream: StreamId, reason: String },
    /// Tunnel keepalive probe; the peer answers [`StreamFrame::Pong`]
    /// with the same nonce.
    Ping { nonce: u64 },
    /// Tunnel keepalive answer.
    Pong { nonce: u64 },
}

impl StreamFrame {
    /// An `open` for an HTTP-style request stream.
    pub fn open_request(
        stream: StreamId,
        port: u16,
        method: impl Into<String>,
        path: impl Into<String>,
        trip: Option<String>,
    ) -> StreamFrame {
        StreamFrame::Open {
            stream,
            port,
            method: Some(method.into()),
            path: Some(path.into()),
            trip,
        }
    }

    /// An `open` for a raw-connect stream.
    pub fn open_connect(stream: StreamId, port: u16, trip: Option<String>) -> StreamFrame {
        StreamFrame::Open {
            stream,
            port,
            method: None,
            path: None,
            trip,
        }
    }

    /// A `data` frame carrying `payload` (at most
    /// [`MAX_DATA_PAYLOAD_BYTES`] bytes — enforced here, so a frame
    /// built through this constructor is always within the bound).
    pub fn data(stream: StreamId, payload: &[u8], fin: bool) -> Result<StreamFrame, ProtoError> {
        if payload.len() > MAX_DATA_PAYLOAD_BYTES {
            return Err(ProtoError::Oversize(payload.len()));
        }
        Ok(StreamFrame::Data {
            stream,
            data: base64::engine::general_purpose::STANDARD.encode(payload),
            fin,
            status: None,
        })
    }

    /// The stream this frame belongs to, if any (keepalive frames
    /// are tunnel-scoped and have none).
    pub fn stream(&self) -> Option<StreamId> {
        match self {
            StreamFrame::Open { stream, .. }
            | StreamFrame::Data { stream, .. }
            | StreamFrame::Window { stream, .. }
            | StreamFrame::Rst { stream, .. } => Some(*stream),
            StreamFrame::Ping { .. } | StreamFrame::Pong { .. } => None,
        }
    }

    /// The intent of an `open` frame. Errors when exactly one of
    /// method/path is present — half a request head is malformed,
    /// never a raw connect.
    pub fn intent(&self) -> Result<Option<OpenIntent>, ProtoError> {
        match self {
            StreamFrame::Open { method, path, .. } => match (method, path) {
                (Some(m), Some(p)) => Ok(Some(OpenIntent::Request {
                    method: m.clone(),
                    path: p.clone(),
                })),
                (None, None) => Ok(Some(OpenIntent::RawConnect)),
                _ => Err(ProtoError::BadFrame(
                    "open frame carries exactly one of method/path".to_string(),
                )),
            },
            _ => Ok(None),
        }
    }

    /// The decoded payload of a `data` frame, enforcing the frame
    /// bound. Errors name shapes and sizes only, never contents
    /// (this crate's error discipline).
    pub fn payload(&self) -> Result<Option<Vec<u8>>, ProtoError> {
        match self {
            StreamFrame::Data { data, .. } => {
                let bytes = base64::engine::general_purpose::STANDARD
                    .decode(data)
                    .map_err(|_| {
                        ProtoError::BadFrame("data frame payload is not base64".to_string())
                    })?;
                if bytes.len() > MAX_DATA_PAYLOAD_BYTES {
                    return Err(ProtoError::Oversize(bytes.len()));
                }
                Ok(Some(bytes))
            }
            _ => Ok(None),
        }
    }

    /// Check a frame against the stream contract's shape rules:
    /// nonzero stream ids, a whole request head on `open`, the
    /// payload bound on `data`, nonzero credit on `window`, a named
    /// reason on `rst`. Peers run this on frames they receive;
    /// failing it is a [`RST_PROTOCOL_ERROR`] against the stream
    /// (or a dropped keepalive, for tunnel-scoped frames).
    pub fn validate(&self) -> Result<(), ProtoError> {
        if let Some(id) = self.stream() {
            if id.0 == 0 {
                return Err(ProtoError::BadFrame(
                    "stream frame carries stream id 0".to_string(),
                ));
            }
        }
        match self {
            StreamFrame::Open { .. } => self.intent().map(|_| ()),
            StreamFrame::Data { .. } => self.payload().map(|_| ()),
            StreamFrame::Window { credit, .. } => {
                if *credit == 0 {
                    Err(ProtoError::BadFrame(
                        "window frame grants zero credit".to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
            StreamFrame::Rst { reason, .. } => {
                if reason.is_empty() {
                    Err(ProtoError::BadFrame(
                        "rst frame carries no reason".to_string(),
                    ))
                } else {
                    Ok(())
                }
            }
            StreamFrame::Ping { .. } | StreamFrame::Pong { .. } => Ok(()),
        }
    }
}

/// Is this raw frame a member of the `proxy-mux` family (does it
/// carry one of the family's `kind` discriminators)? Hello frames,
/// request/response frames, and error frames are not.
pub fn is_stream_frame(frame: &serde_json::Value) -> bool {
    matches!(
        frame.get("kind").and_then(|k| k.as_str()),
        Some("open" | "data" | "window" | "rst" | "ping" | "pong")
    )
}

/// Decode a raw frame as a [`StreamFrame`] and validate it against
/// the contract. Unknown fields are ignored (additive within the
/// major); an unknown `kind` is a malformed frame here — callers
/// that merely route frames should test [`is_stream_frame`] first.
pub fn parse_stream_frame(frame: &serde_json::Value) -> Result<StreamFrame, ProtoError> {
    let parsed: StreamFrame = serde_json::from_value(frame.clone())
        .map_err(|e| ProtoError::BadFrame(format!("stream frame shape mismatch: {e}")))?;
    parsed.validate()?;
    Ok(parsed)
}

/// Per-stream credit window — the backpressure bound as a type
/// (record §4). One instance tracks one stream in one direction
/// pair: how much this side may still send, and how much the peer
/// has sent that this side has not yet credited back.
///
/// A sender must not exceed granted credit: [`StreamWindow::send`]
/// refuses a chunk that would overrun it. A receiver whose peer
/// overruns the window it granted learns so from
/// [`StreamWindow::receive`] and answers `rst`
/// ([`RST_PROTOCOL_ERROR`]). Consumed bytes become credit to return
/// in a `window` frame via [`StreamWindow::consume`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamWindow {
    send_credit: u64,
    recv_ungranted: u64,
}

impl Default for StreamWindow {
    fn default() -> Self {
        Self::new()
    }
}

impl StreamWindow {
    /// A fresh window: [`INITIAL_STREAM_WINDOW_BYTES`] of send
    /// credit, nothing received.
    pub fn new() -> StreamWindow {
        StreamWindow {
            send_credit: INITIAL_STREAM_WINDOW_BYTES,
            recv_ungranted: 0,
        }
    }

    /// Bytes this side may still send before more credit arrives.
    pub fn send_credit(&self) -> u64 {
        self.send_credit
    }

    /// Account for sending one chunk of `len` bytes. Refuses a
    /// chunk over the frame bound, or one that exceeds the credit
    /// the peer has granted — the sender waits for a `window`
    /// frame instead of overrunning.
    pub fn send(&mut self, len: usize) -> Result<(), ProtoError> {
        if len > MAX_DATA_PAYLOAD_BYTES {
            return Err(ProtoError::Oversize(len));
        }
        if len as u64 > self.send_credit {
            return Err(ProtoError::BadFrame(format!(
                "stream send of {len} bytes exceeds granted credit {}",
                self.send_credit
            )));
        }
        self.send_credit -= len as u64;
        Ok(())
    }

    /// Account for a `window` frame granting `credit` more bytes.
    pub fn grant(&mut self, credit: u64) {
        self.send_credit = self.send_credit.saturating_add(credit);
    }

    /// Account for receiving one chunk of `len` bytes. Refuses a
    /// chunk over the frame bound, or one that overruns the window
    /// this side granted — the peer is in protocol violation.
    pub fn receive(&mut self, len: usize) -> Result<(), ProtoError> {
        if len > MAX_DATA_PAYLOAD_BYTES {
            return Err(ProtoError::Oversize(len));
        }
        let total = self.recv_ungranted + len as u64;
        if total > INITIAL_STREAM_WINDOW_BYTES {
            return Err(ProtoError::BadFrame(format!(
                "stream receive overruns the {INITIAL_STREAM_WINDOW_BYTES}-byte window"
            )));
        }
        self.recv_ungranted = total;
        Ok(())
    }

    /// Mark `len` received bytes consumed; returns the credit to
    /// grant back in a `window` frame (never more than was
    /// received-and-ungranted).
    pub fn consume(&mut self, len: u64) -> u64 {
        let credit = len.min(self.recv_ungranted);
        self.recv_ungranted -= credit;
        credit
    }
}

/// A per-stream timeout ceiling, checked against the record's
/// bounds at construction: positive, defaulting to
/// [`DEFAULT_STREAM_TIMEOUT`], never above [`MAX_STREAM_TIMEOUT`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamCeiling(Duration);

impl Default for StreamCeiling {
    fn default() -> Self {
        StreamCeiling(DEFAULT_STREAM_TIMEOUT)
    }
}

impl StreamCeiling {
    pub fn new(timeout: Duration) -> Result<StreamCeiling, ProtoError> {
        if timeout.is_zero() || timeout > MAX_STREAM_TIMEOUT {
            return Err(ProtoError::BadFrame(format!(
                "stream timeout ceiling {timeout:?} outside (0, {MAX_STREAM_TIMEOUT:?}]"
            )));
        }
        Ok(StreamCeiling(timeout))
    }

    pub fn timeout(&self) -> Duration {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn caps(ids: &[&str]) -> Vec<String> {
        ids.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn mux_requires_both_peers_to_advertise() {
        assert!(mux_negotiated(
            &caps(&["proxy-connect", CAP_PROXY_MUX]),
            &caps(&["proxy-serve", CAP_PROXY_MUX])
        ));
        // Either side missing it — including the legacy dialect,
        // which advertises nothing — leaves the pooled path in
        // charge (record §5).
        assert!(!mux_negotiated(
            &caps(&[CAP_PROXY_MUX]),
            &caps(&["proxy-serve"])
        ));
        assert!(!mux_negotiated(
            &caps(&["proxy-connect"]),
            &caps(&[CAP_PROXY_MUX])
        ));
        assert!(!mux_negotiated(&caps(&[]), &caps(&[])));
    }

    #[test]
    fn stream_frame_kinds_match_the_record() {
        let open = StreamFrame::open_request(
            StreamId::client(1).unwrap(),
            4799,
            "GET",
            "/health",
            Some("trip-1".to_string()),
        );
        let frame = serde_json::to_value(&open).unwrap();
        assert_eq!(frame["kind"], "open");
        assert_eq!(frame["stream"], 1);
        assert_eq!(frame["port"], 4799);
        assert_eq!(frame["method"], "GET");
        assert_eq!(frame["trip"], "trip-1");
        assert!(is_stream_frame(&frame));
        assert_eq!(parse_stream_frame(&frame).unwrap(), open);

        let ping = serde_json::to_value(StreamFrame::Ping { nonce: 7 }).unwrap();
        assert_eq!(ping["kind"], "ping");
        assert!(is_stream_frame(&ping));
        // Not stream frames: hello, request, response, error shapes.
        assert!(!is_stream_frame(&serde_json::json!({"hello": {}})));
        assert!(!is_stream_frame(
            &serde_json::json!({"method": "GET", "path": "/"})
        ));
        assert!(!is_stream_frame(&serde_json::json!({"status": 200})));
    }

    #[test]
    fn open_intent_distinguishes_request_from_raw_connect() {
        let req = StreamFrame::open_request(StreamId::client(3).unwrap(), 4800, "POST", "/x", None);
        assert_eq!(
            req.intent().unwrap(),
            Some(OpenIntent::Request {
                method: "POST".to_string(),
                path: "/x".to_string()
            })
        );
        let raw = StreamFrame::open_connect(StreamId::client(3).unwrap(), 4800, None);
        assert_eq!(raw.intent().unwrap(), Some(OpenIntent::RawConnect));
        // Half a request head is malformed, on parse and on demand.
        let half = serde_json::json!({"kind": "open", "stream": 3, "port": 4800, "method": "GET"});
        assert!(parse_stream_frame(&half).is_err());
    }

    #[test]
    fn client_stream_ids_are_nonzero_and_odd() {
        assert!(StreamId::client(1).is_ok());
        assert!(StreamId::client(0).is_err());
        assert!(StreamId::client(2).is_err());
        assert!(StreamId(5).is_client_initiated());
        assert!(!StreamId(4).is_client_initiated());
        // Stream id 0 on the wire fails validation whatever the kind.
        let zero = serde_json::json!({"kind": "rst", "stream": 0, "reason": "cancelled"});
        assert!(parse_stream_frame(&zero).is_err());
    }

    #[test]
    fn data_payload_is_bounded_at_64_kib() {
        let id = StreamId::client(1).unwrap();
        let at = vec![7u8; MAX_DATA_PAYLOAD_BYTES];
        let frame = StreamFrame::data(id, &at, true).unwrap();
        assert_eq!(frame.payload().unwrap().unwrap(), at);
        let over = vec![7u8; MAX_DATA_PAYLOAD_BYTES + 1];
        assert!(StreamFrame::data(id, &over, false).is_err());
        // A hand-built frame over the bound fails validation too.
        let big = StreamFrame::Data {
            stream: id,
            data: base64::engine::general_purpose::STANDARD.encode(&over),
            fin: false,
            status: None,
        };
        assert!(big.validate().is_err());
        // Garbage payload encoding is a shape error, not a panic.
        let junk = StreamFrame::Data {
            stream: id,
            data: "!!not-base64!!".to_string(),
            fin: false,
            status: None,
        };
        assert!(junk.validate().is_err());
    }

    #[test]
    fn data_frame_omits_absent_optionals() {
        let frame = StreamFrame::data(StreamId::client(1).unwrap(), b"hi", false).unwrap();
        let value = serde_json::to_value(&frame).unwrap();
        assert_eq!(value["kind"], "data");
        assert!(value.get("fin").is_none());
        assert!(value.get("status").is_none());
        let fin = StreamFrame::data(StreamId::client(1).unwrap(), b"hi", true).unwrap();
        assert_eq!(serde_json::to_value(&fin).unwrap()["fin"], true);
    }

    #[test]
    fn unknown_fields_are_tolerated() {
        // Additive within the major: a frame from a newer peer with
        // fields this build does not know still parses.
        let frame = serde_json::json!({
            "kind": "window", "stream": 9, "credit": 1024, "future_field": {"x": 1}
        });
        let parsed = parse_stream_frame(&frame).unwrap();
        assert_eq!(
            parsed,
            StreamFrame::Window {
                stream: StreamId(9),
                credit: 1024
            }
        );
        // An unknown kind is not a stream frame at all.
        let other = serde_json::json!({"kind": "teleport", "stream": 9});
        assert!(!is_stream_frame(&other));
        assert!(parse_stream_frame(&other).is_err());
    }

    #[test]
    fn window_credit_bounds_both_directions() {
        let mut w = StreamWindow::new();
        assert_eq!(w.send_credit(), INITIAL_STREAM_WINDOW_BYTES);
        // Send up to the granted credit, then no further.
        w.send(MAX_DATA_PAYLOAD_BYTES).unwrap();
        let rest = (INITIAL_STREAM_WINDOW_BYTES as usize) - MAX_DATA_PAYLOAD_BYTES;
        for _ in 0..(rest / MAX_DATA_PAYLOAD_BYTES) {
            w.send(MAX_DATA_PAYLOAD_BYTES).unwrap();
        }
        assert_eq!(w.send_credit(), 0);
        assert!(w.send(1).is_err());
        // A window frame reopens sending.
        w.grant(100);
        w.send(100).unwrap();
        // An over-bound chunk is refused even with credit.
        w.grant(INITIAL_STREAM_WINDOW_BYTES);
        assert!(w.send(MAX_DATA_PAYLOAD_BYTES + 1).is_err());

        // Receiving: the peer may fill the window, never overrun it.
        let mut r = StreamWindow::new();
        for _ in 0..(INITIAL_STREAM_WINDOW_BYTES as usize / MAX_DATA_PAYLOAD_BYTES) {
            r.receive(MAX_DATA_PAYLOAD_BYTES).unwrap();
        }
        assert!(r.receive(1).is_err());
        // Consuming returns exactly that much credit to grant back.
        assert_eq!(r.consume(512), 512);
        r.receive(512).unwrap();
        // The window is full again, so consuming everything grants
        // back the whole window.
        assert_eq!(r.consume(u64::MAX), INITIAL_STREAM_WINDOW_BYTES);
    }

    #[test]
    fn ceiling_defaults_and_bounds() {
        assert_eq!(StreamCeiling::default().timeout(), DEFAULT_STREAM_TIMEOUT);
        assert_eq!(DEFAULT_STREAM_TIMEOUT, Duration::from_secs(10));
        assert!(StreamCeiling::new(Duration::from_secs(15)).is_ok());
        assert!(StreamCeiling::new(Duration::from_secs(16)).is_err());
        assert!(StreamCeiling::new(Duration::ZERO).is_err());
        // The hard cap stays well under the 16–26 s failure cluster.
        assert!(MAX_STREAM_TIMEOUT < Duration::from_secs(16));
    }

    #[test]
    fn frames_roundtrip_through_the_crate_framing() {
        let frames = [
            StreamFrame::open_request(
                StreamId::client(11).unwrap(),
                4799,
                "GET",
                "/",
                Some("t".to_string()),
            ),
            StreamFrame::data(StreamId(11), b"chunk", false).unwrap(),
            StreamFrame::Window {
                stream: StreamId(11),
                credit: 4096,
            },
            StreamFrame::Rst {
                stream: StreamId(11),
                reason: RST_TIMEOUT.to_string(),
            },
            StreamFrame::Ping { nonce: 1 },
            StreamFrame::Pong { nonce: 1 },
        ];
        for frame in frames {
            let mut buf = Vec::new();
            crate::write_frame(&mut buf, &frame).unwrap();
            let raw = crate::read_frame(&mut std::io::Cursor::new(buf)).unwrap();
            assert!(is_stream_frame(&raw));
            assert_eq!(parse_stream_frame(&raw).unwrap(), frame);
        }
    }
}
