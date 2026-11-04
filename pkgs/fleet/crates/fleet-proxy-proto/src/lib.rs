//! fleet-proxy-proto — the fleet proxy wire contract
//! (design note §4, epic developing-today/code#5412).
//!
//! Design rule, taken from atlas-proto's discipline: **the wire
//! contract lives here, is versioned from the first frame, and
//! outlives any variant's feature set.** Domain crates and the bin
//! speak these types; nothing redefines the framing elsewhere.
//!
//! Framing is today's proven shape, unchanged: an **8-byte
//! big-endian length prefix** followed by one JSON object, both
//! directions — the exact framing the Python session broker runs
//! (`struct.pack(">Q", …)` in laptop-agent-broker.py, mirrored by
//! laptop-agent.py and by fleet-queue's session-proxy transport).
//!
//! ERRATUM (step 3, #5431): the design note §4 and this crate's
//! step-1 implementation both said *4-byte*. That was a census
//! error — no live peer has ever spoken 4-byte framing; the
//! Python broker, its client, and fleet-queue all use 8 bytes.
//! Step 1's 4-byte `write_frame`/`read_frame` only ever talked to
//! this crate's own tests (and silently broke fleet-secrets'
//! broker source against the live broker, masked by its bws
//! fallback). The framing below is the live wire, corrected to
//! match the protocol it claims to describe; the hello shapes and
//! the versioning policy are unchanged from the note.
//!
//! On top of the framing, the first frame on every client↔broker
//! connection is the hello exchange (fleet peers only — see
//! [`PROTO_LEGACY`] for the pre-hello dialect):
//!
//! ```text
//! client → { "hello": { "proto": 1, "client": "fleet-connect 0.1.0",
//!                       "capabilities": ["proxy-connect"] } }
//! broker → { "hello": { "proto": 1,
//!                       "services": { "4799": {"name":"laptop-agent","version":"…"}, … },
//!                       "capabilities": ["proxy-serve"] } }
//! ```
//!
//! The services map is the broker's **actual routing table**, not a
//! promise — capability negotiation *is* the services map. A client
//! must treat an unadvertised port as "not here" (the proxy-level
//! analogue of the exit-69 capability stub).
//!
//! Versioning policy: the integer is the protocol *major*; a broker
//! accepts majors N and N−1 (see [`broker_accepts`]); fields are
//! additive within a major and unknown fields are ignored.

use std::collections::BTreeMap;
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};

/// The current protocol major: the hello-bearing dialect.
pub const PROTO_MAJOR: u32 = 1;

/// The pre-hello dialect — today's Python broker and client: framed
/// requests from the first frame, no hello, no services map. It is
/// major 0 in the version window: a fleet broker serves it by
/// peeking at the first frame (a request frame opens a legacy
/// session), and a fleet client detects it when its hello is
/// answered by a response frame instead of a hello-response.
pub const PROTO_LEGACY: u32 = 0;

/// How many majors back a broker accepts (the N/N−1 window, §4).
pub const PROTO_WINDOW: u32 = 1;

/// Frame size ceiling. The broker's payloads are HTTP bodies for local
/// services; 64 MiB is generous headroom, and a ceiling turns a corrupt
/// length prefix into an error instead of an allocation.
pub const MAX_FRAME_BYTES: usize = 64 * 1024 * 1024;

/// Does a broker speaking [`PROTO_MAJOR`] accept a client hello at
/// `client_major`? (§4: majors N and N−1; older gets a clean
/// `proto_too_old` error naming the minimum — the frame a broker
/// sends is built by [`proto_too_old_frame`].)
pub fn broker_accepts(client_major: u32) -> bool {
    client_major <= PROTO_MAJOR && client_major + PROTO_WINDOW >= PROTO_MAJOR
}

/// The minimum major this broker accepts.
pub fn min_accepted_major() -> u32 {
    PROTO_MAJOR - PROTO_WINDOW
}

/// Error code a broker returns for an out-of-window hello.
pub const PROTO_TOO_OLD: &str = "proto_too_old";

/// The frame a broker answers to an out-of-window hello, then closes:
/// `{ "error": "proto_too_old", "min_proto": …, "proto": … }` —
/// machine-readable first, so a client can branch on it (exit 69's
/// wire analogue: "your protocol is not spoken here").
pub fn proto_too_old_frame(broker_major: u32) -> serde_json::Value {
    serde_json::json!({
        "error": PROTO_TOO_OLD,
        "min_proto": broker_major - PROTO_WINDOW.min(broker_major),
        "proto": broker_major,
    })
}

/// Is this frame a proto-too-old refusal?
pub fn is_proto_too_old(frame: &serde_json::Value) -> bool {
    frame.get("error").and_then(|e| e.as_str()) == Some(PROTO_TOO_OLD)
}

/// Is this frame a hello envelope (`{ "hello": … }`)?
pub fn is_hello_frame(frame: &serde_json::Value) -> bool {
    frame.get("hello").map(|h| h.is_object()).unwrap_or(false)
}

/// The client's first frame payload: `{ "hello": Hello }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Hello {
    pub proto: u32,
    /// Free-form client identity, e.g. `"fleet-connect 0.1.0"`.
    pub client: String,
    /// Capability ids (fleet-registry `Capability::id` strings).
    pub capabilities: Vec<String>,
}

/// The broker's hello answer payload: `{ "hello": HelloResponse }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HelloResponse {
    pub proto: u32,
    /// Port (as a string, JSON-object keys) → the service actually
    /// routed there. This map is the routing table.
    pub services: BTreeMap<String, ServiceInfo>,
    pub capabilities: Vec<String>,
}

/// One routed service in the hello map.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    /// Per-service version string, so a client can feature-detect a broker
    /// capability instead of guessing from the fleet version.
    pub version: String,
}

/// A request frame after the hello (today's shape, unchanged):
/// `{method, path, body?, port?, token_file?}`. `port` selects the
/// laptop service (fleet-config's `PORT_*` constants); `token_file`
/// names the file whose contents the broker sends as `X-Agent-Token`
/// for that request — the file is read by the broker per request; the
/// token itself never appears in the frame beyond that name.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Request {
    pub method: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub token_file: Option<String>,
}

/// A response frame: `{status, body}` — the HTTP status and parsed
/// JSON body of the routed service's reply.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    pub body: serde_json::Value,
}

/// Frame-level errors. None of these ever carry payload contents —
/// proxy payloads can be secret-bearing (the secrets broker's
/// replies), so errors name shapes and sizes only.
#[derive(Debug)]
pub enum ProtoError {
    Io(std::io::Error),
    /// The peer closed the stream mid- or pre-frame.
    Closed,
    /// Length prefix above [`MAX_FRAME_BYTES`].
    Oversize(usize),
    /// Frame was not valid JSON / not the expected shape.
    BadFrame(String),
}

impl std::fmt::Display for ProtoError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ProtoError::Io(e) => write!(f, "proxy transport: {e}"),
            ProtoError::Closed => write!(f, "proxy transport: connection closed"),
            ProtoError::Oversize(n) => {
                write!(
                    f,
                    "proxy frame of {n} bytes exceeds the {MAX_FRAME_BYTES}-byte ceiling"
                )
            }
            ProtoError::BadFrame(why) => write!(f, "proxy frame malformed: {why}"),
        }
    }
}

impl std::error::Error for ProtoError {}

impl From<std::io::Error> for ProtoError {
    fn from(e: std::io::Error) -> Self {
        ProtoError::Io(e)
    }
}

/// Write one length-prefixed JSON frame (8-byte big-endian length —
/// the live wire, see the crate docs).
pub fn write_frame<W: Write, S: Serialize>(w: &mut W, value: &S) -> Result<(), ProtoError> {
    let payload = serde_json::to_vec(value)
        .map_err(|e| ProtoError::BadFrame(format!("cannot encode frame: {e}")))?;
    if payload.len() > MAX_FRAME_BYTES {
        return Err(ProtoError::Oversize(payload.len()));
    }
    let len = payload.len() as u64;
    w.write_all(&len.to_be_bytes())?;
    w.write_all(&payload)?;
    w.flush()?;
    Ok(())
}

/// Read one length-prefixed JSON frame as raw JSON.
pub fn read_frame<R: Read>(r: &mut R) -> Result<serde_json::Value, ProtoError> {
    let mut len_buf = [0u8; 8];
    match r.read_exact(&mut len_buf) {
        Ok(()) => {}
        Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => return Err(ProtoError::Closed),
        Err(e) => return Err(ProtoError::Io(e)),
    }
    let len = u64::from_be_bytes(len_buf);
    if len > MAX_FRAME_BYTES as u64 {
        // The declared length can overflow usize on paper; report it
        // saturated — the value is diagnostic only.
        return Err(ProtoError::Oversize(len.min(usize::MAX as u64) as usize));
    }
    let mut payload = vec![0u8; len as usize];
    r.read_exact(&mut payload).map_err(|e| {
        if e.kind() == std::io::ErrorKind::UnexpectedEof {
            ProtoError::Closed
        } else {
            ProtoError::Io(e)
        }
    })?;
    serde_json::from_slice(&payload)
        .map_err(|e| ProtoError::BadFrame(format!("frame is not valid JSON: {e}")))
}

/// Read one frame and decode it as `T`.
pub fn read_typed<R: Read, T: for<'de> Deserialize<'de>>(r: &mut R) -> Result<T, ProtoError> {
    let value = read_frame(r)?;
    serde_json::from_value(value)
        .map_err(|e| ProtoError::BadFrame(format!("frame shape mismatch: {e}")))
}

/// Wrap a hello payload in its `{ "hello": … }` frame envelope.
pub fn hello_frame(hello: &Hello) -> serde_json::Value {
    serde_json::json!({ "hello": hello })
}

/// Wrap a hello-response payload in its `{ "hello": … }` envelope.
pub fn hello_response_frame(resp: &HelloResponse) -> serde_json::Value {
    serde_json::json!({ "hello": resp })
}

/// Unwrap a `{ "hello": … }` envelope.
pub fn unwrap_hello<T: for<'de> Deserialize<'de>>(
    frame: &serde_json::Value,
) -> Result<T, ProtoError> {
    let inner = frame
        .get("hello")
        .ok_or_else(|| ProtoError::BadFrame("frame has no \"hello\" object".to_string()))?;
    serde_json::from_value(inner.clone())
        .map_err(|e| ProtoError::BadFrame(format!("hello shape mismatch: {e}")))
}

/// The capability ids a hello advertises, from registry capabilities.
pub fn capability_ids(caps: &fleet_registry::CapabilitySet) -> Vec<String> {
    caps.iter().map(|c| c.id().to_string()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;

    fn roundtrip<S: Serialize + for<'de> Deserialize<'de> + PartialEq + std::fmt::Debug>(
        value: &S,
    ) -> S {
        let mut buf = Vec::new();
        write_frame(&mut buf, value).unwrap();
        read_typed(&mut Cursor::new(buf)).unwrap()
    }

    #[test]
    fn hello_frame_shape_matches_the_design_note() {
        let hello = Hello {
            proto: 1,
            client: "fleet-connect 0.1.0".to_string(),
            capabilities: vec!["proxy-connect".to_string()],
        };
        let frame = hello_frame(&hello);
        assert_eq!(frame["hello"]["proto"], 1);
        assert_eq!(frame["hello"]["client"], "fleet-connect 0.1.0");
        assert_eq!(frame["hello"]["capabilities"][0], "proxy-connect");
        let back: Hello = unwrap_hello(&frame).unwrap();
        assert_eq!(back, hello);
    }

    #[test]
    fn hello_response_carries_the_services_map() {
        let mut services = BTreeMap::new();
        services.insert(
            "4800".to_string(),
            ServiceInfo {
                name: "secrets-broker".to_string(),
                version: "0.1.0".to_string(),
            },
        );
        let resp = HelloResponse {
            proto: 1,
            services,
            capabilities: vec!["proxy-serve".to_string()],
        };
        let back: HelloResponse = unwrap_hello(&hello_response_frame(&resp)).unwrap();
        assert_eq!(back, resp);
        assert_eq!(back.services["4800"].name, "secrets-broker");
    }

    #[test]
    fn request_omits_absent_optional_fields() {
        let req = Request {
            method: "GET".to_string(),
            path: "/health".to_string(),
            body: None,
            port: None,
            token_file: None,
        };
        let frame = serde_json::to_value(&req).unwrap();
        assert_eq!(frame["method"], "GET");
        assert!(frame.get("body").is_none());
        assert!(frame.get("port").is_none());
        assert!(frame.get("token_file").is_none());
        assert_eq!(roundtrip(&req), req);
    }

    #[test]
    fn framing_is_eight_byte_big_endian_like_the_live_broker() {
        // The live Python broker packs ">Q" (8-byte big-endian). This
        // test is the regression guard for the step-1 census error
        // that made this crate speak 4-byte framing no peer used.
        let mut buf = Vec::new();
        write_frame(&mut buf, &serde_json::json!({"a": 1})).unwrap();
        // {"a":1} is 7 bytes; the prefix must be 00 00 00 00 00 00 00 07.
        assert_eq!(&buf[..8], &[0, 0, 0, 0, 0, 0, 0, 7]);
        assert_eq!(&buf[8..], b"{\"a\":1}");
    }

    #[test]
    fn response_roundtrip() {
        let resp = Response {
            status: 200,
            body: serde_json::json!({"ok": true}),
        };
        assert_eq!(roundtrip(&resp), resp);
    }

    #[test]
    fn major_window_is_n_and_n_minus_1() {
        assert!(broker_accepts(PROTO_MAJOR));
        assert!(broker_accepts(PROTO_LEGACY));
        assert!(broker_accepts(PROTO_MAJOR - PROTO_WINDOW));
        if PROTO_MAJOR > PROTO_WINDOW {
            // One major below the window is too old. (At PROTO_MAJOR 1
            // the window reaches 0, so no representable major is too
            // old yet — the branch keeps the property tested as the
            // major grows.)
            assert!(!broker_accepts(PROTO_MAJOR - PROTO_WINDOW - 1));
        }
        assert!(!broker_accepts(PROTO_MAJOR + 1));
        assert_eq!(min_accepted_major(), PROTO_MAJOR - PROTO_WINDOW);
    }

    #[test]
    fn proto_too_old_frame_names_the_minimum() {
        let frame = proto_too_old_frame(PROTO_MAJOR + 1);
        assert!(is_proto_too_old(&frame));
        assert_eq!(frame["min_proto"], PROTO_MAJOR + 1 - PROTO_WINDOW);
        assert_eq!(frame["proto"], PROTO_MAJOR + 1);
        assert!(!is_proto_too_old(&serde_json::json!({"status": 200})));
    }

    #[test]
    fn hello_frame_detection() {
        assert!(is_hello_frame(&hello_frame(&Hello {
            proto: 1,
            client: "x".to_string(),
            capabilities: vec![],
        })));
        assert!(!is_hello_frame(&serde_json::json!({"method": "GET"})));
        assert!(!is_hello_frame(&serde_json::json!({"hello": 3})));
    }

    #[test]
    fn closed_stream_is_a_clean_error() {
        let mut empty = Cursor::new(Vec::new());
        match read_frame(&mut empty) {
            Err(ProtoError::Closed) => {}
            other => panic!("expected Closed, got {other:?}"),
        }
    }

    #[test]
    fn oversize_prefix_is_an_error_not_an_allocation() {
        let mut buf = Vec::new();
        buf.extend_from_slice(&(MAX_FRAME_BYTES as u64 + 1).to_be_bytes());
        match read_frame(&mut Cursor::new(buf)) {
            Err(ProtoError::Oversize(_)) => {}
            other => panic!("expected Oversize, got {other:?}"),
        }
    }
}
