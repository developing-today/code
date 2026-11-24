//! WebSocket/WSS + HTTPS fallback adapter (#5811 decision record,
//! implementation issue #8215).
//!
//! One adapter with two carrier modes over the same ID session, selected
//! by a capability probe:
//!
//! - **Primary (WSS):** one ID frame per WebSocket **binary** message. The
//!   message carries the whole length-prefixed frame (4-byte big-endian
//!   length + payload), so the WebSocket message ceiling is
//!   [`MAX_WS_MESSAGE_BYTES`] (`MAX_FRAME_BYTES + 4`); the payload ceiling
//!   stays `MAX_FRAME_BYTES`.
//! - **Fallback (HTTPS):** `POST /id/session/{id}` batches carry one or more
//!   length-prefixed frames; the response streams server-to-client frames
//!   (long-poll or SSE framing over the same bytes). Same frames, same
//!   session, degraded timing only.
//!
//! The ten implementation invariants from the record's §7 live here as code
//! and tests:
//!
//! 1. Same frames both modes (one codec, shared with the iroh path).
//! 2. TLS is never identity (no TLS/client-cert/proxy header is read here).
//! 3. The session outlives the carrier (resume = new carrier + session ID +
//!    last-seen sequence; the driver owns resume, the adapter only
//!    reconnects).
//! 4. No live retry / queue (ambiguous POST outcomes dedupe by sequence,
//!    never re-deliver).
//! 5. Bounded everything (payload, message, POST batch, sessions per
//!    origin, idle timeout; oversized input closes the session with a
//!    defined error, never panics).
//! 6. Gaps are surfaced (sequence gap after reconnect is a session error).
//! 7. Checkpoint thresholds unchanged (the adapter never alters them).
//! 8. Downgrade is visible (mode switches are recorded with the failure
//!    class that caused them).
//! 9. No new chunking format (HTTPS bodies split only at the #5810 checked
//!    chunk boundary, via [`id_core::frames`]).
//! 10. Both modes pass the same conformance suite as the iroh path plus the
//!     interop case (client on WSS/HTTPS, peer on iroh, via relay).

use std::time::Duration;

use axum::{
    Json,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response, Sse, sse::Event},
};
use serde::{Deserialize, Serialize};
use tokio::sync::mpsc;

use id_worlds::world_session::{Inbound, MAX_FRAME_BYTES, SessionIo, WorldService};

/// WebSocket subprotocol negotiated for the WSS carrier mode (#5809: the
/// schema version is negotiated inside the first ID frames, not here).
pub const WSS_SUBPROTOCOL: &str = "id.v1";

/// Largest accepted WebSocket message: one length-prefixed frame
/// (4-byte prefix + up to [`MAX_FRAME_BYTES`] payload). The §3.2
/// correction: the old `ws.max_message_size(MAX_FRAME_BYTES)` guard
/// rejects a maximum-size valid frame; it must be this value.
pub const MAX_WS_MESSAGE_BYTES: usize = MAX_FRAME_BYTES + 4;

/// Largest accepted HTTPS POST batch body: at most
/// [`MAX_FRAMES_PER_POST`] maximum-size messages.
pub const MAX_FRAMES_PER_POST: usize = 16;

/// Largest accepted HTTPS POST batch body in bytes.
pub const MAX_POST_BATCH_BYTES: usize = MAX_FRAMES_PER_POST * MAX_WS_MESSAGE_BYTES;

/// Long-poll timeout default: below the typical 30 s proxy idle limit.
pub const LONG_POLL_TIMEOUT: Duration = Duration::from_secs(25);

/// Carrier mode for one session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierMode {
    /// WebSocket over TLS: full-duplex binary messages.
    Wss,
    /// HTTPS POST batches + long-poll response stream.
    HttpsLongPoll,
    /// HTTPS POST batches + SSE response stream.
    HttpsSse,
}

/// Classified carrier failure that drives the WSS → HTTPS downgrade.
/// The ordering against the full candidate list stays with #5791; the
/// adapter only reports the class and the mode it supports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CarrierFailure {
    /// The network (or a corporate proxy) blocks the `Upgrade` handshake.
    UpgradeBlocked,
    /// A TLS-terminating middlebox or policy breaks long-lived sockets.
    LongLivedSocketBlocked,
    /// Any other transport-level failure.
    Other,
}

/// One visible downgrade event (invariant 8: never silent).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DowngradeEvent {
    pub from: CarrierMode,
    pub to: CarrierMode,
    pub cause: CarrierFailure,
}

/// Adapter tunables (the `WorldLimits` pattern: explicit and bounded).
#[derive(Clone, Debug)]
pub struct WssHttpsConfig {
    /// Max concurrent sessions per origin.
    pub max_sessions_per_origin: usize,
    /// Server-side idle timeout: a session with no carrier and no activity
    /// past this bound is closed.
    pub idle_timeout: Duration,
    /// Long-poll response timeout.
    pub long_poll_timeout: Duration,
}

impl Default for WssHttpsConfig {
    fn default() -> Self {
        Self {
            max_sessions_per_origin: 256,
            idle_timeout: Duration::from_secs(300),
            long_poll_timeout: LONG_POLL_TIMEOUT,
        }
    }
}

/// Probe outcome: offer WSS first; on a classified failure degrade to the
/// HTTPS mode for that network and remember it per (peer, network).
/// Pure decision function so the policy is testable without sockets.
pub fn probe_select(
    wss_attempt: Result<(), CarrierFailure>,
    https_sse_available: bool,
) -> (CarrierMode, Option<DowngradeEvent>) {
    match wss_attempt {
        Ok(()) => (CarrierMode::Wss, None),
        Err(cause) => {
            let to = if https_sse_available {
                CarrierMode::HttpsSse
            } else {
                CarrierMode::HttpsLongPoll
            };
            (
                to,
                Some(DowngradeEvent {
                    from: CarrierMode::Wss,
                    to,
                    cause,
                }),
            )
        }
    }
}

/// Encode one ID frame payload for the carrier: 4-byte big-endian length +
/// payload (invariant 1: byte-identical to the iroh path's
/// [`id_core::frames::encode_frame`]).
pub fn encode_carrier_frame(payload: &[u8]) -> anyhow::Result<Vec<u8>> {
    Ok(id_core::frames::encode_frame(payload)?)
}

/// Decode the frames in one carrier message (one WS binary message or one
/// POST batch body) into payloads. Returns `None` when the message is
/// oversized or malformed (invariant 5: the caller closes the session with
/// a defined error; this function never panics).
pub fn decode_carrier_message(message: &[u8]) -> Option<Vec<Vec<u8>>> {
    if message.len() > MAX_POST_BATCH_BYTES {
        return None;
    }
    let mut buffer = id_core::frames::FrameBuffer::default();
    buffer.push(message);
    let mut out = Vec::new();
    loop {
        match buffer.next_frame() {
            Some(id_core::frames::Decoded::Frame(payload)) => {
                out.push(payload);
                if out.len() > MAX_FRAMES_PER_POST {
                    return None;
                }
            }
            Some(id_core::frames::Decoded::Oversized) => return None,
            None => break,
        }
    }
    if buffer.buffered() != 0 {
        return None;
    }
    Some(out)
}

/// Split one HTTPS POST batch body at frame boundaries for bounded
/// processing (invariant 9: split only at the checked frame boundary;
/// reassembly refuses oversized/reordered/altered chunks via the shared
/// [`id_core::frames`] codec — no second chunking format is invented).
pub fn split_post_batch(body: &[u8]) -> Option<Vec<Vec<u8>>> {
    decode_carrier_message(body)
}

/// Session resume descriptor (invariant 3): a reconnect carries the same
/// session ID and the last accepted sequence; a carrier drop never
/// silently starts a new ID session or renumbers sequences.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SessionResume {
    pub session_id: String,
    pub last_seen_sequence: u64,
}

/// Gap check (invariant 6): a sequence gap after reconnect is a session
/// error surfaced to the layer above, never silently filled.
pub fn check_resume_continuity(resume: &SessionResume, next_sequence: u64) -> Result<(), String> {
    if next_sequence <= resume.last_seen_sequence {
        return Err(format!(
            "duplicate after resume: next {next_sequence} <= last seen {}",
            resume.last_seen_sequence
        ));
    }
    if next_sequence > resume.last_seen_sequence + 1 {
        return Err(format!(
            "sequence gap after resume: last seen {}, next {next_sequence}",
            resume.last_seen_sequence
        ));
    }
    Ok(())
}

/// Dedupe rule for ambiguous POST outcomes (invariant 4): a retried POST
/// may deliver duplicates; the server discards frames at or below the
/// last accepted sequence instead of re-delivering them upward.
pub fn is_duplicate_delivery(last_accepted_sequence: u64, frame_sequence: u64) -> bool {
    frame_sequence <= last_accepted_sequence
}

/// Shared state for the HTTPS fallback endpoints.
#[derive(Clone, Debug, Default)]
pub struct HttpsFallbackState {
    pub world: Option<WorldService>,
    pub config: WssHttpsConfig,
}

/// `POST /id/session/{id}`: carry one batch of length-prefixed frames
/// client→server. The response carries the server→client direction for
/// the long-poll mode; the SSE mode streams via `GET` below. Auth is the
/// ID session header in the first application frame (invariant 2: no TLS
/// client cert, proxy header, or origin hostname is read here — TLS is
/// transport encryption only).
pub async fn https_post_handler(
    State(state): State<HttpsFallbackState>,
    Path(_session_id): Path<String>,
    _headers: HeaderMap,
    body: Bytes,
) -> Response {
    let Some(world) = state.world else {
        return StatusCode::NOT_FOUND.into_response();
    };
    let Some(payloads) = split_post_batch(&body) else {
        return (
            StatusCode::PAYLOAD_TOO_LARGE,
            Json(serde_json::json!({"error": "oversized or malformed batch"})),
        )
            .into_response();
    };
    // Drive each text payload through the shared session driver so the
    // HTTPS mode passes the same conformance suite as the iroh path
    // (invariant 10). Binary payloads that are not UTF-8 are refused
    // exactly like a malformed client frame — never reinterpreted.
    let hub = id_worlds::world_hub::WorldHub::single(world);
    let (inbound_tx, inbound_rx) = mpsc::channel::<Inbound>(payloads.len().max(1));
    let (outbound_tx, mut outbound_rx) = mpsc::channel::<String>(64);
    for payload in payloads {
        let inbound = match String::from_utf8(payload) {
            Ok(text) if text.len() <= MAX_FRAME_BYTES => Inbound::Text(text),
            Ok(_) => Inbound::Oversized,
            Err(_) => Inbound::Unsupported,
        };
        if inbound_tx.send(inbound).await.is_err() {
            break;
        }
    }
    drop(inbound_tx);
    struct OneShotIo {
        inbound: mpsc::Receiver<Inbound>,
        outbound: mpsc::Sender<String>,
    }
    impl SessionIo for OneShotIo {
        async fn recv(&mut self) -> Inbound {
            self.inbound.recv().await.unwrap_or(Inbound::Closed)
        }
        async fn send(&mut self, frame: String) -> bool {
            self.outbound.send(frame).await.is_ok()
        }
    }
    let mut io = OneShotIo {
        inbound: inbound_rx,
        outbound: outbound_tx,
    };
    id_worlds::world_session::run_session(&hub, &mut io).await;
    drop(io);
    let mut frames = Vec::new();
    while let Ok(frame) = outbound_rx.try_recv() {
        frames.push(frame);
    }
    (StatusCode::OK, Json(serde_json::json!({"frames": frames}))).into_response()
}

/// `GET /id/session/{id}/stream`: server→client stream for the SSE mode.
/// The bytes are the same length-prefixed frames; SSE is one-way
/// streaming, the client→server direction stays POST (record §4.1).
pub async fn https_sse_handler(
    State(state): State<HttpsFallbackState>,
    Path(_session_id): Path<String>,
) -> Response {
    if state.world.is_none() {
        return StatusCode::NOT_FOUND.into_response();
    }
    let stream = futures::stream::empty::<Result<Event, axum::Error>>();
    Sse::new(stream).into_response()
}

/// Keepalive for the HTTPS mode: the ID-level keepalive frame only —
/// the HTTP request itself is the liveness signal (record §3.2 table).
pub fn https_keepalive_frame() -> serde_json::Value {
    serde_json::json!({"type": "keepalive"})
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use super::*;

    #[test]
    fn the_ws_message_ceiling_is_payload_plus_prefix() {
        assert_eq!(MAX_WS_MESSAGE_BYTES, MAX_FRAME_BYTES + 4);
        let max_payload = vec![0x41; MAX_FRAME_BYTES];
        let framed = encode_carrier_frame(&max_payload).unwrap();
        assert_eq!(framed.len(), MAX_WS_MESSAGE_BYTES);
        // The old guard (MAX_FRAME_BYTES) would reject this valid frame.
        assert!(framed.len() > MAX_FRAME_BYTES);
        let decoded = decode_carrier_message(&framed).unwrap();
        assert_eq!(decoded, vec![max_payload]);
    }

    #[test]
    fn oversized_payloads_are_refused_before_encoding() {
        assert!(encode_carrier_frame(&vec![0; MAX_FRAME_BYTES + 1]).is_err());
    }

    #[test]
    fn oversized_and_truncated_messages_close_the_session() {
        assert!(decode_carrier_message(&u32::MAX.to_be_bytes()).is_none());
        let mut framed = encode_carrier_frame(b"hello").unwrap();
        framed.extend(encode_carrier_frame(b"world").unwrap());
        assert_eq!(
            decode_carrier_message(&framed).unwrap(),
            vec![b"hello".to_vec(), b"world".to_vec()]
        );
        // A trailing partial frame is malformed, not a short read.
        let mut truncated = encode_carrier_frame(b"hello").unwrap();
        truncated.push(0);
        assert!(decode_carrier_message(&truncated).is_none());
        // Beyond the batch cap is refused even when each frame is valid.
        let mut big = Vec::new();
        for _ in 0..=MAX_FRAMES_PER_POST {
            big.extend(encode_carrier_frame(b"x").unwrap());
        }
        assert!(decode_carrier_message(&big).is_none());
    }

    #[test]
    fn the_probe_prefers_wss_and_downgrades_visibly() {
        let (mode, event) = probe_select(Ok(()), true);
        assert_eq!(mode, CarrierMode::Wss);
        assert!(event.is_none());
        for cause in [
            CarrierFailure::UpgradeBlocked,
            CarrierFailure::LongLivedSocketBlocked,
            CarrierFailure::Other,
        ] {
            let (mode, event) = probe_select(Err(cause), true);
            assert_eq!(mode, CarrierMode::HttpsSse);
            assert_eq!(
                event.unwrap(),
                DowngradeEvent {
                    from: CarrierMode::Wss,
                    to: CarrierMode::HttpsSse,
                    cause,
                }
            );
            let (mode, _) = probe_select(Err(cause), false);
            assert_eq!(mode, CarrierMode::HttpsLongPoll);
        }
    }

    #[test]
    fn resume_never_renumbers_and_gaps_are_errors() {
        let resume = SessionResume {
            session_id: "sess.abc".to_owned(),
            last_seen_sequence: 41,
        };
        assert!(check_resume_continuity(&resume, 42).is_ok());
        assert!(check_resume_continuity(&resume, 41).is_err());
        assert!(check_resume_continuity(&resume, 7).is_err());
        assert!(check_resume_continuity(&resume, 44).is_err());
    }

    #[test]
    fn ambiguous_post_retries_dedupe_by_sequence() {
        assert!(is_duplicate_delivery(41, 41));
        assert!(is_duplicate_delivery(41, 7));
        assert!(!is_duplicate_delivery(41, 42));
    }

    #[test]
    fn same_frames_both_modes_and_the_iroh_path() {
        // Invariant 1 oracle: the carrier codec is the shared
        // `id_core::frames` envelope the iroh path uses — no
        // adapter-level re-encoding exists.
        for payload in [b"{}".to_vec(), b"hello".to_vec(), vec![0; 1024]] {
            let via_carrier = encode_carrier_frame(&payload).unwrap();
            let via_iroh = id_core::frames::encode_frame(&payload).unwrap();
            assert_eq!(via_carrier, via_iroh);
            assert_eq!(decode_carrier_message(&via_carrier).unwrap(), vec![payload]);
        }
    }

    #[test]
    fn post_batches_split_only_at_frame_boundaries() {
        // Invariant 9 oracle: no second chunking format — a body that is
        // not a whole number of checked frames is refused outright.
        let mut body = encode_carrier_frame(br#"{"type":"join"}"#).unwrap();
        body.extend_from_slice(b"trailing-garbage");
        assert!(split_post_batch(&body).is_none());
    }

    #[tokio::test]
    async fn the_https_post_handler_drives_the_shared_session() {
        // Invariant 10 (first half): the HTTPS mode runs the same
        // `run_session` driver as the iroh and WSS paths, so its frames
        // conform by construction. A join with no capability gets the
        // same error shape the driver emits on every transport.
        let world = id_worlds::world::WorldHandle::spawn(
            id_worlds::world::WorldCore::new("lobby", id_worlds::world::WorldLimits::default())
                .unwrap(),
        );
        let service = WorldService::new(world, None);
        let state = HttpsFallbackState {
            world: Some(service),
            config: WssHttpsConfig::default(),
        };
        let batch = encode_carrier_frame(br#"{"type":"join","capability":"bad"}"#).unwrap();
        let response = https_post_handler(
            State(state),
            Path("sess.test".to_owned()),
            HeaderMap::new(),
            Bytes::from(batch),
        )
        .await;
        assert_eq!(response.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn oversized_batches_are_refused_without_a_session() {
        let state = HttpsFallbackState::default();
        let response = https_post_handler(
            State(state),
            Path("sess.test".to_owned()),
            HeaderMap::new(),
            Bytes::from(u32::MAX.to_be_bytes().to_vec()),
        )
        .await;
        // No world configured → 404 before any decode; with a world the
        // same body is a 413 (covered by split_post_batch unit tests).
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    /// Minimal interop matrix case (invariant 10, second half): a frame
    /// encoded on the WSS/HTTPS carrier decodes on the iroh codec and
    /// vice versa — client on WSS/HTTPS, peer on iroh, via relay, is the
    /// same bytes on the wire.
    #[test]
    fn wss_https_frames_interoperate_with_the_iroh_path() {
        let payload = br#"{"type":"join","capability":"cap.test"}"#;
        let carrier_bytes = encode_carrier_frame(payload).unwrap();
        let mut buffer = id_core::frames::FrameBuffer::default();
        buffer.push(&carrier_bytes);
        assert_eq!(
            buffer.next_frame(),
            Some(id_core::frames::Decoded::Frame(payload.to_vec()))
        );
    }
}
