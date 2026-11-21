//! Serve-proxy (ID broker) wire protocol — slice 1: typed frame codec.
//!
//! Implements the framing layer of the #5792 decision record (issue #7569):
//! one binary frame format, identical on both broker legs, carried inside
//! TLS 1.3 over TCP 443 (through HTTP CONNECT where egress requires a
//! proxy; the WebSocket binary-message adapter carries the same frames,
//! one frame per message, where CONNECT is denied).
//!
//! Wire layout (v1, defined here — the decision record fixes the frame
//! types, the length prefix, and the caps, not the header byte layout):
//!
//! ```text
//! +-------------------+---------------- frame body -------------------+
//! | u32 BE body length | type u8 | stream u32 BE | seq u64 BE | payload |
//! +-------------------+---------------- frame body -------------------+
//! ```
//!
//! - OPEN, DATA, and CLOSE carry a nonzero stream ID. WINDOW_UPDATE uses
//!   stream ID 0 for connection credit or a nonzero ID for stream credit.
//!   PING, PONG, RESUME, RESUME_ACK, and ERROR use stream ID 0.
//! - `seq` is the per-stream DATA sequence number (decision §5); it is 0
//!   for every non-DATA frame.
//! - DATA payloads are capped at [`MAX_DATA_PAYLOAD`] (16 KiB, decision
//!   §2 / invariant 4). The cap is enforced on encode and on decode.
//! - Stream IDs are scoped to a leg: odd IDs are opened by the leg's
//!   initiator, even IDs by the responder side (decision §3). The broker
//!   maps IDs between legs; one leg's numbering is never exposed on the
//!   other, so this codec treats IDs as opaque apart from parity.
//!
//! Later slices (tracked on issue #7569) build on this codec: leg
//! establishment (TLS / CONNECT / WebSocket adapter), broker-side stream
//! mapping and priority scheduling, keepalive (§4), and resumption (§5).
//! The broker forwards E2E ciphertext only and never logs payload bytes
//! (invariants 1–2); nothing in this module logs at all.
//!
//! Testing oracle (repo testing doctrine): the codec is a pure function
//! pair, so the oracle is decode∘encode = identity, checked with
//! property-based tests over generated frames, plus the 16 KiB DATA cap
//! invariant on both directions and a chunking oracle for the streaming
//! decoder (any split of the byte stream yields the same frames).

use anyhow::{Context, Result, bail};

/// Maximum DATA frame payload, in bytes (decision §2, invariant 4).
pub const MAX_DATA_PAYLOAD: usize = 16 * 1024;

/// Header size inside the frame body: type (1) + stream ID (4) + seq (8).
pub const HEADER_BYTES: usize = 13;

/// Largest frame body this codec accepts. Bounds broker buffering the
/// way decision §3 requires; RESUME vectors are the largest legitimate
/// non-DATA payloads and fit far below this.
pub const MAX_FRAME_BODY: usize = HEADER_BYTES + 256 * 1024;

/// Largest wire representation retained while an incomplete frame arrives.
const MAX_FRAME_WIRE_BYTES: usize = 4 + MAX_FRAME_BODY;

/// Initial per-stream credit window, in bytes (decision §3).
pub const INITIAL_STREAM_CREDIT: u64 = 64 * 1024;

/// Initial per-connection credit window, in bytes (decision §3).
pub const INITIAL_CONNECTION_CREDIT: u64 = 1024 * 1024;

/// The typed frames of the decision record §2.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum FrameType {
    /// New logical stream; names target identity and service (payload).
    Open,
    /// E2E ciphertext for one stream; payload ≤ [`MAX_DATA_PAYLOAD`].
    Data,
    /// Half-close of one direction of a stream.
    Close,
    /// Flow-control credit grant (payload: u32 BE additional bytes).
    WindowUpdate,
    /// Keepalive probe (decision §4).
    Ping,
    /// Keepalive answer.
    Pong,
    /// Session resumption request (decision §5); signed by the endpoint
    /// identity key — the signature travels in the payload, verification
    /// is the resumption slice's job, not the codec's.
    Resume,
    /// Resumption answer carrying the replay positions.
    ResumeAck,
    /// Leg-level failure with a classified reason ([`ErrorReason`]).
    Error,
}

impl FrameType {
    fn to_u8(self) -> u8 {
        match self {
            Self::Open => 1,
            Self::Data => 2,
            Self::Close => 3,
            Self::WindowUpdate => 4,
            Self::Ping => 5,
            Self::Pong => 6,
            Self::Resume => 7,
            Self::ResumeAck => 8,
            Self::Error => 9,
        }
    }

    fn from_u8(byte: u8) -> Result<Self> {
        Ok(match byte {
            1 => Self::Open,
            2 => Self::Data,
            3 => Self::Close,
            4 => Self::WindowUpdate,
            5 => Self::Ping,
            6 => Self::Pong,
            7 => Self::Resume,
            8 => Self::ResumeAck,
            9 => Self::Error,
            other => bail!("unknown serve-proxy frame type {other}"),
        })
    }

    /// Scheduling class of the frame itself (decision §3): control frames
    /// are never queued behind DATA (invariant 4).
    pub fn scheduling_class(self) -> SchedulingClass {
        match self {
            Self::Data => SchedulingClass::Data,
            _ => SchedulingClass::Control,
        }
    }
}

/// Priority scheduling classes (decision §3). DATA frames take the class
/// of their stream (interactive or bulk); every other frame type is
/// control and is scheduled ahead of already-queued DATA.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SchedulingClass {
    /// Highest: OPEN / CLOSE / PING / RESUME / flow control.
    Control = 0,
    /// SSH-class and ID query traffic.
    Interactive = 1,
    /// Sync traffic; loses to interactive frames already queued.
    Bulk = 2,
    /// Internal marker for "this frame is DATA; use the stream's class".
    Data = 3,
}

/// Classified leg-failure reasons (decision §2: the parent #5671 failure
/// classes, plus credit / protocol violations from invariant 5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ErrorReason {
    UdpBlocked,
    PeerOffline,
    ProxyAuthRequired,
    BrokerRefused,
    CreditOverrun,
    ProtocolViolation,
    Internal,
}

impl ErrorReason {
    fn to_u8(self) -> u8 {
        match self {
            Self::UdpBlocked => 1,
            Self::PeerOffline => 2,
            Self::ProxyAuthRequired => 3,
            Self::BrokerRefused => 4,
            Self::CreditOverrun => 5,
            Self::ProtocolViolation => 6,
            Self::Internal => 7,
        }
    }

    fn from_u8(byte: u8) -> Result<Self> {
        Ok(match byte {
            1 => Self::UdpBlocked,
            2 => Self::PeerOffline,
            3 => Self::ProxyAuthRequired,
            4 => Self::BrokerRefused,
            5 => Self::CreditOverrun,
            6 => Self::ProtocolViolation,
            7 => Self::Internal,
            other => bail!("unknown serve-proxy error reason {other}"),
        })
    }
}

/// One decoded serve-proxy frame.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub ty: FrameType,
    /// Leg-scoped stream ID; nonzero for OPEN/DATA/CLOSE, optionally nonzero
    /// for WINDOW_UPDATE, and 0 for leg-level frames.
    pub stream_id: u32,
    /// Per-stream DATA sequence number (decision §5); 0 for non-DATA frames.
    pub seq: u64,
    pub payload: Vec<u8>,
}

impl Frame {
    pub fn new(ty: FrameType, stream_id: u32, seq: u64, payload: Vec<u8>) -> Result<Self> {
        let frame = Self {
            ty,
            stream_id,
            seq,
            payload,
        };
        frame.validate()?;
        Ok(frame)
    }

    /// A DATA frame carrying E2E ciphertext (never plaintext the broker
    /// could read — invariant 1 is the caller's layering, enforced by the
    /// E2E slice; the codec only enforces the size cap).
    pub fn data(stream_id: u32, seq: u64, payload: Vec<u8>) -> Result<Self> {
        Self::new(FrameType::Data, stream_id, seq, payload)
    }

    /// A leg-level ERROR frame: payload is the reason byte plus a UTF-8 message.
    /// Messages are metadata for operators; they must never contain
    /// payload bytes (invariant 2).
    pub fn error(reason: ErrorReason, message: &str) -> Self {
        let mut payload = vec![reason.to_u8()];
        payload.extend_from_slice(message.as_bytes());
        Self {
            ty: FrameType::Error,
            stream_id: 0,
            seq: 0,
            payload,
        }
    }

    /// The classified reason of an ERROR frame, if this is one.
    pub fn error_reason(&self) -> Result<Option<ErrorReason>> {
        if self.ty != FrameType::Error {
            return Ok(None);
        }
        let byte = self
            .payload
            .first()
            .context("ERROR frame without a reason byte")?;
        Ok(Some(ErrorReason::from_u8(*byte)?))
    }

    /// The additional credit of a WINDOW_UPDATE frame (u32 BE payload).
    pub fn window_update_amount(&self) -> Result<Option<u32>> {
        if self.ty != FrameType::WindowUpdate {
            return Ok(None);
        }
        let bytes: [u8; 4] = self
            .payload
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("WINDOW_UPDATE payload is not 4 bytes"))?;
        Ok(Some(u32::from_be_bytes(bytes)))
    }

    fn validate(&self) -> Result<()> {
        let requires_stream = matches!(
            self.ty,
            FrameType::Open | FrameType::Data | FrameType::Close
        );
        let allows_stream = requires_stream || self.ty == FrameType::WindowUpdate;
        if requires_stream && self.stream_id == 0 {
            bail!("{:?} frame must name a nonzero stream ID", self.ty);
        }
        if !allows_stream && self.stream_id != 0 {
            bail!("{:?} frame is leg-level and must use stream ID 0", self.ty);
        }
        if self.ty != FrameType::Data && self.seq != 0 {
            bail!("{:?} frame must use sequence number 0", self.ty);
        }
        if self.ty == FrameType::WindowUpdate && self.payload.len() != 4 {
            bail!("WINDOW_UPDATE payload must be exactly 4 bytes");
        }
        if self.ty == FrameType::Data && self.payload.len() > MAX_DATA_PAYLOAD {
            bail!(
                "DATA payload of {} bytes exceeds the {MAX_DATA_PAYLOAD}-byte cap",
                self.payload.len()
            );
        }
        if HEADER_BYTES + self.payload.len() > MAX_FRAME_BODY {
            bail!(
                "frame body of {} bytes exceeds {MAX_FRAME_BODY}",
                HEADER_BYTES + self.payload.len()
            );
        }
        Ok(())
    }

    /// The scheduling class to queue this frame under: control frames
    /// always win; DATA takes its stream's class.
    pub fn scheduling_class(&self, data_class: SchedulingClass) -> SchedulingClass {
        match self.ty.scheduling_class() {
            SchedulingClass::Data => data_class,
            control => control,
        }
    }

    /// Encode to the wire format: u32 BE body length, then the body.
    pub fn encode(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let body_len = u32::try_from(HEADER_BYTES + self.payload.len()).context("frame length")?;
        let mut out = Vec::with_capacity(4 + body_len as usize);
        out.extend_from_slice(&body_len.to_be_bytes());
        out.push(self.ty.to_u8());
        out.extend_from_slice(&self.stream_id.to_be_bytes());
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.payload);
        Ok(out)
    }

    /// Decode one frame from the front of `bytes`; returns the frame and
    /// the number of bytes consumed, or `Ok(None)` when more bytes are
    /// needed. An oversized length prefix or a cap violation is an error
    /// as soon as it is visible — the body is never buffered first
    /// (invariant 5's buffering bound, applied to parsing).
    pub fn decode_prefix(bytes: &[u8]) -> Result<Option<(Self, usize)>> {
        if bytes.len() < 4 {
            return Ok(None);
        }
        let length_prefix = bytes
            .get(..4)
            .context("frame is missing its four-byte length prefix")?;
        let body_len = u32::from_be_bytes(
            length_prefix
                .try_into()
                .context("frame length prefix must be four bytes")?,
        ) as usize;
        if body_len < HEADER_BYTES {
            bail!("frame body of {body_len} bytes is shorter than the {HEADER_BYTES}-byte header");
        }
        if body_len > MAX_FRAME_BODY {
            bail!("frame body of {body_len} bytes exceeds {MAX_FRAME_BODY}");
        }
        if bytes.len() < 4 + body_len {
            return Ok(None);
        }
        let body = bytes
            .get(4..4 + body_len)
            .context("frame body is shorter than its declared length")?;
        let ty = *body.first().context("frame body is missing its type")?;
        let stream_id = body
            .get(1..5)
            .context("frame body is missing its stream ID")?;
        let seq = body
            .get(5..13)
            .context("frame body is missing its sequence number")?;
        let payload = body
            .get(HEADER_BYTES..)
            .context("frame body is missing its payload")?;
        let frame = Self {
            ty: FrameType::from_u8(ty)?,
            stream_id: u32::from_be_bytes(
                stream_id
                    .try_into()
                    .context("frame stream ID must be four bytes")?,
            ),
            seq: u64::from_be_bytes(
                seq.try_into()
                    .context("frame sequence number must be eight bytes")?,
            ),
            payload: payload.to_vec(),
        };
        frame.validate()?;
        Ok(Some((frame, 4 + body_len)))
    }
}

/// Stream IDs are odd for the leg's initiator, even for the responder
/// (decision §3). Stream 0 is the leg itself and is neither.
pub fn stream_id_is_initiator(stream_id: u32) -> bool {
    stream_id % 2 == 1
}

/// Incremental decoder for a leg's byte stream: feed bytes in any
/// chunking, take complete frames out. Oracle: for any chunk split of
/// an encoded frame sequence, the decoded frames are identical.
#[derive(Debug, Default)]
pub struct FrameDecoder {
    /// Bytes from a prior feed that have not yet completed one frame.
    buf: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    /// Append bytes and return every frame completed by them.
    pub fn feed(&mut self, bytes: &[u8]) -> Result<Vec<Frame>> {
        let mut frames = Vec::new();

        // Finish a frame carried over from the prior feed. Fill only the
        // prefix first so malformed lengths are rejected before copying an
        // arbitrary caller-provided suffix into decoder storage.
        let mut input = bytes;
        if !self.buf.is_empty() {
            if self.buf.len() < 4 {
                let take = (4 - self.buf.len()).min(input.len());
                self.buf.extend_from_slice(&input[..take]);
                input = &input[take..];
                if self.buf.len() < 4 {
                    return Ok(frames);
                }
            }

            // This validates the declared length as soon as all four prefix
            // bytes are present, without requiring any body bytes.
            let _ = Frame::decode_prefix(&self.buf)?;
            let body_len = u32::from_be_bytes(
                self.buf[..4]
                    .try_into()
                    .context("frame length prefix must be four bytes")?,
            ) as usize;
            let wire_len = 4 + body_len;
            debug_assert!(wire_len <= MAX_FRAME_WIRE_BYTES);

            let take = (wire_len - self.buf.len()).min(input.len());
            self.buf.extend_from_slice(&input[..take]);
            input = &input[take..];
            if self.buf.len() < wire_len {
                return Ok(frames);
            }

            let (frame, consumed) = Frame::decode_prefix(&self.buf)?
                .context("complete buffered frame was not decoded")?;
            debug_assert_eq!(consumed, wire_len);
            frames.push(frame);
            self.buf.clear();
        }

        // Decode directly from the caller's slice. Only the final incomplete
        // frame is copied into `buf`, so a batch may contain any number of
        // valid frames without becoming an aggregate buffer limit.
        let mut offset = 0;
        while offset < input.len() {
            match Frame::decode_prefix(&input[offset..])? {
                Some((frame, consumed)) => {
                    frames.push(frame);
                    offset += consumed;
                }
                None => {
                    let pending = &input[offset..];
                    debug_assert!(pending.len() < MAX_FRAME_WIRE_BYTES);
                    self.buf.extend_from_slice(pending);
                    break;
                }
            }
        }
        Ok(frames)
    }

    /// Bytes retained for an incomplete frame (for tests/metrics).
    pub fn pending_bytes(&self) -> usize {
        self.buf.len()
    }
}

/// A credit window (decision §3): the primitive a sender must consult
/// before queueing DATA. Credit is never exceeded (invariant 5):
/// [`CreditWindow::try_consume`] refuses rather than going negative,
/// and the leg's reaction to a peer that overruns is an ERROR frame
/// with [`ErrorReason::CreditOverrun`], built by the mux slice.
///
/// The window is also bounded above (#8970): the relay backpressure
/// design requires hard finite per-stream and aggregate
/// per-connection limits, with credit returning only as bounded
/// buffer space is released. Each window therefore carries an
/// explicit `ceiling` set at construction from the effective
/// configuration — the design delegates the limit to configuration,
/// so no universal deployment limit is invented here. The built-in
/// constructors are fixed-size windows (ceiling = the configured
/// initial credit): a WINDOW_UPDATE can only return credit the
/// sender has actually consumed. A deployment that negotiates a
/// larger maximum constructs the window with that ceiling via
/// [`CreditWindow::new`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreditWindow {
    available: u64,
    ceiling: u64,
}

impl CreditWindow {
    /// A window holding `initial` credit that may never hold more
    /// than `ceiling`. A ceiling below the initial grant is raised
    /// to the initial grant: a window must always be able to hold
    /// the credit it was constructed with.
    pub fn new(initial: u64, ceiling: u64) -> Self {
        Self {
            available: initial,
            ceiling: ceiling.max(initial),
        }
    }

    pub fn per_stream() -> Self {
        Self::new(INITIAL_STREAM_CREDIT, INITIAL_STREAM_CREDIT)
    }

    pub fn per_connection() -> Self {
        Self::new(INITIAL_CONNECTION_CREDIT, INITIAL_CONNECTION_CREDIT)
    }

    pub fn available(&self) -> u64 {
        self.available
    }

    /// The finite maximum this window may ever hold.
    pub fn ceiling(&self) -> u64 {
        self.ceiling
    }

    /// Consume `bytes` of credit; `false` (and no state change) when the
    /// send would exceed the granted credit.
    pub fn try_consume(&mut self, bytes: u64) -> bool {
        if bytes > self.available {
            return false;
        }
        self.available -= bytes;
        true
    }

    /// Grant additional credit (a received WINDOW_UPDATE). A grant
    /// that would push the window past its ceiling — or overflow —
    /// is an explicit protocol/resource error (the mux slice maps it
    /// to [`ErrorReason::CreditOverrun`]): the window is left
    /// unchanged, never silently saturated (#8970).
    pub fn grant(&mut self, bytes: u64) -> Result<()> {
        let next = self.available.checked_add(bytes).with_context(|| {
            format!(
                "credit grant of {bytes} bytes overflows the window ({} available)",
                self.available
            )
        })?;
        if next > self.ceiling {
            bail!(
                "credit grant of {bytes} bytes exceeds the window ceiling ({} available, {} ceiling)",
                self.available,
                self.ceiling
            );
        }
        self.available = next;
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    fn frame_strategy() -> impl Strategy<Value = Frame> {
        (
            prop_oneof![
                Just(FrameType::Open),
                Just(FrameType::Data),
                Just(FrameType::Close),
                Just(FrameType::WindowUpdate),
                Just(FrameType::Ping),
                Just(FrameType::Pong),
                Just(FrameType::Resume),
                Just(FrameType::ResumeAck),
                Just(FrameType::Error),
            ],
            any::<u32>(),
            any::<u64>(),
            prop::collection::vec(any::<u8>(), 0..64),
        )
            .prop_map(|(ty, stream_id, seq, mut payload)| {
                // WINDOW_UPDATE payloads are a fixed 4-byte amount.
                if ty == FrameType::WindowUpdate {
                    payload.truncate(4);
                    payload.resize(4, 0);
                }
                Frame {
                    ty,
                    stream_id: if matches!(ty, FrameType::Open | FrameType::Data | FrameType::Close)
                    {
                        stream_id.max(1)
                    } else if ty == FrameType::WindowUpdate {
                        stream_id
                    } else {
                        0
                    },
                    seq: if ty == FrameType::Data { seq } else { 0 },
                    payload,
                }
            })
    }

    proptest! {
        /// Oracle: decode ∘ encode = identity for every valid frame.
        #[test]
        fn round_trip_is_identity(frame in frame_strategy()) {
            let encoded = frame.encode().unwrap();
            let (decoded, consumed) = Frame::decode_prefix(&encoded).unwrap().unwrap();
            prop_assert_eq!(consumed, encoded.len());
            prop_assert_eq!(decoded, frame);
        }

        /// Oracle: the streaming decoder agrees with one-shot decoding
        /// under any chunk split of the same byte stream.
        #[test]
        fn decoder_chunking_is_transparent(
            frames in prop::collection::vec(frame_strategy(), 1..8),
            split in 1usize..64,
        ) {
            let mut wire = Vec::new();
            for frame in &frames {
                wire.extend_from_slice(&frame.encode().unwrap());
            }
            let mut decoder = FrameDecoder::new();
            let mut decoded = Vec::new();
            for chunk in wire.chunks(split) {
                decoded.extend(decoder.feed(chunk).unwrap());
            }
            prop_assert_eq!(decoded, frames);
            prop_assert_eq!(decoder.pending_bytes(), 0);
        }

        /// Oracle: credit is never exceeded — any consume sequence the
        /// window accepts sums to at most initial + granted, and the
        /// window never holds more than its ceiling (#8970): a grant
        /// the window refuses changes neither the accounting nor the
        /// window state.
        #[test]
        fn credit_is_never_exceeded(
            ops in prop::collection::vec((any::<bool>(), 0u64..200_000), 0..32),
        ) {
            let mut window = CreditWindow::per_stream();
            let mut granted = INITIAL_STREAM_CREDIT;
            let mut consumed = 0u64;
            for (is_grant, amount) in ops {
                if is_grant {
                    if window.grant(amount).is_ok() {
                        granted += amount;
                    }
                } else if window.try_consume(amount) {
                    consumed += amount;
                }
                prop_assert!(consumed <= granted);
                prop_assert_eq!(window.available(), granted - consumed);
                prop_assert!(window.available() <= window.ceiling());
            }
        }
    }

    #[test]
    fn credit_grant_returns_consumed_credit_up_to_the_ceiling() {
        // #8970: normal credit return — consume, then a WINDOW_UPDATE
        // returns exactly the consumed space.
        let mut window = CreditWindow::per_stream();
        assert!(window.try_consume(1_000));
        window.grant(1_000).unwrap();
        assert_eq!(window.available(), INITIAL_STREAM_CREDIT);
        // An exact-ceiling grant on a drained window is accepted.
        let mut drained = CreditWindow::per_stream();
        assert!(drained.try_consume(INITIAL_STREAM_CREDIT));
        drained.grant(INITIAL_STREAM_CREDIT).unwrap();
        assert_eq!(drained.available(), drained.ceiling());
    }

    #[test]
    fn credit_grant_past_the_ceiling_is_refused_without_state_change() {
        // #8970: over-ceiling and repeated WINDOW_UPDATE grants are
        // explicit errors and never mutate the window (no silent
        // saturation toward u64::MAX).
        let mut window = CreditWindow::per_stream();
        let before = window;
        assert!(window.grant(1).is_err());
        assert_eq!(window, before);
        for _ in 0..4 {
            assert!(window.grant(u64::from(u32::MAX)).is_err());
        }
        assert_eq!(window, before);
        // Overflow itself is an error, not a saturation.
        assert!(window.grant(u64::MAX).is_err());
        assert_eq!(window, before);
        // A configured ceiling above the initial grant admits grants
        // up to that ceiling and no further.
        let mut configured = CreditWindow::new(100, 250);
        configured.grant(150).unwrap();
        assert_eq!(configured.available(), 250);
        assert!(configured.grant(1).is_err());
        assert_eq!(configured.available(), 250);
    }

    #[test]
    fn data_cap_boundary() {
        let at_cap = Frame::data(1, 0, vec![0; MAX_DATA_PAYLOAD]).unwrap();
        assert!(at_cap.encode().is_ok());
        assert!(Frame::data(1, 0, vec![0; MAX_DATA_PAYLOAD + 1]).is_err());
        // A wire frame whose DATA payload is over the cap is refused at
        // decode even though its body fits MAX_FRAME_BODY.
        let mut body = vec![FrameType::Data.to_u8()];
        body.extend_from_slice(&1u32.to_be_bytes());
        body.extend_from_slice(&0u64.to_be_bytes());
        body.extend_from_slice(&vec![0; MAX_DATA_PAYLOAD + 1]);
        let mut wire = Vec::new();
        wire.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
        wire.extend_from_slice(&body);
        assert!(Frame::decode_prefix(&wire).is_err());
    }

    #[test]
    fn frame_scope_and_sequence_semantics_are_validated_on_construction_and_wire() {
        assert!(Frame::data(0, 0, Vec::new()).is_err());
        assert!(Frame::new(FrameType::Ping, 1, 0, Vec::new()).is_err());
        assert!(Frame::new(FrameType::Data, 1, 1, Vec::new()).is_ok());
        assert!(Frame::new(FrameType::Close, 1, 1, Vec::new()).is_err());

        for ty in [
            FrameType::Ping,
            FrameType::Pong,
            FrameType::Resume,
            FrameType::ResumeAck,
            FrameType::Error,
        ] {
            assert!(Frame::new(ty, 0, 0, Vec::new()).is_ok(), "{ty:?}");
        }
        for ty in [FrameType::Open, FrameType::Data, FrameType::Close] {
            assert!(Frame::new(ty, 1, 0, Vec::new()).is_ok(), "{ty:?}");
        }
        assert!(Frame::new(FrameType::WindowUpdate, 0, 0, 4u32.to_be_bytes().to_vec()).is_ok());
        assert!(Frame::new(FrameType::WindowUpdate, 1, 0, 4u32.to_be_bytes().to_vec()).is_ok());
        assert!(Frame::new(FrameType::WindowUpdate, 0, 0, vec![0; 3]).is_err());

        let invalid_frames = [
            Frame {
                ty: FrameType::Data,
                stream_id: 0,
                seq: 0,
                payload: Vec::new(),
            },
            Frame {
                ty: FrameType::Close,
                stream_id: 1,
                seq: 1,
                payload: Vec::new(),
            },
            Frame {
                ty: FrameType::Ping,
                stream_id: 1,
                seq: 0,
                payload: Vec::new(),
            },
            Frame {
                ty: FrameType::WindowUpdate,
                stream_id: 0,
                seq: 0,
                payload: vec![0; 3],
            },
        ];
        for frame in invalid_frames {
            assert!(frame.encode().is_err(), "{frame:?}");
            let mut body = vec![frame.ty.to_u8()];
            body.extend_from_slice(&frame.stream_id.to_be_bytes());
            body.extend_from_slice(&frame.seq.to_be_bytes());
            body.extend_from_slice(&frame.payload);
            let mut wire = Vec::new();
            wire.extend_from_slice(&u32::try_from(body.len()).unwrap().to_be_bytes());
            wire.extend_from_slice(&body);
            assert!(Frame::decode_prefix(&wire).is_err(), "{frame:?}");
        }
    }

    #[test]
    fn oversized_length_prefix_is_refused_before_the_body() {
        let mut wire = Vec::new();
        wire.extend_from_slice(&u32::try_from(MAX_FRAME_BODY + 1).unwrap().to_be_bytes());
        assert!(Frame::decode_prefix(&wire).is_err());
    }

    #[test]
    fn incomplete_frame_waits_for_more_bytes() {
        let frame = Frame::data(3, 7, b"ciphertext".to_vec()).unwrap();
        let wire = frame.encode().unwrap();
        assert!(Frame::decode_prefix(&wire[..6]).unwrap().is_none());
        let mut decoder = FrameDecoder::new();
        assert!(decoder.feed(&wire[..6]).unwrap().is_empty());
        let completed = decoder.feed(&wire[6..]).unwrap();
        assert_eq!(completed, vec![frame]);
    }

    #[test]
    fn large_batch_of_small_frames_decodes_without_an_aggregate_feed_cap() {
        // Oracle: compare against the exact input frame sequence. The batch
        // is larger than the maximum retained single-frame wire size.
        let frame = Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap();
        let encoded = frame.encode().unwrap();
        let count = MAX_FRAME_WIRE_BYTES / encoded.len() + 1;
        let mut wire = Vec::with_capacity(count * encoded.len());
        for _ in 0..count {
            wire.extend_from_slice(&encoded);
        }

        let mut decoder = FrameDecoder::new();
        let decoded = decoder.feed(&wire).unwrap();

        assert_eq!(decoded, vec![frame; count]);
        assert_eq!(decoder.pending_bytes(), 0);
    }

    #[test]
    fn incomplete_frame_storage_is_bounded_and_oversized_prefix_is_rejected_early() {
        // An incomplete maximum-size body retains less than the maximum
        // legal wire size; no complete frame bytes are discarded or lost.
        let mut partial = Vec::with_capacity(MAX_FRAME_WIRE_BYTES - 1);
        partial.extend_from_slice(&u32::try_from(MAX_FRAME_BODY).unwrap().to_be_bytes());
        partial.resize(MAX_FRAME_WIRE_BYTES - 1, 0);
        let mut decoder = FrameDecoder::new();
        assert!(decoder.feed(&partial).unwrap().is_empty());
        assert_eq!(decoder.pending_bytes(), MAX_FRAME_WIRE_BYTES - 1);

        // The decoder reads only the missing prefix byte before rejecting;
        // the large unrelated suffix is never copied into retained storage.
        let mut oversized = Vec::with_capacity(1 + MAX_FRAME_WIRE_BYTES);
        oversized.push(0);
        oversized.extend_from_slice(&u32::try_from(MAX_FRAME_BODY + 1).unwrap().to_be_bytes()[1..]);
        oversized.resize(1 + MAX_FRAME_WIRE_BYTES, 0);
        let mut decoder = FrameDecoder::new();
        assert!(decoder.feed(&oversized[..3]).unwrap().is_empty());
        assert!(decoder.feed(&oversized[3..]).is_err());
        assert_eq!(decoder.pending_bytes(), 4);
    }

    #[test]
    fn control_frames_outrank_data() {
        let ping = Frame::new(FrameType::Ping, 0, 0, Vec::new()).unwrap();
        assert_eq!(
            ping.scheduling_class(SchedulingClass::Bulk),
            SchedulingClass::Control
        );
        let data = Frame::data(1, 0, vec![1]).unwrap();
        assert_eq!(
            data.scheduling_class(SchedulingClass::Bulk),
            SchedulingClass::Bulk
        );
        assert!(SchedulingClass::Control < SchedulingClass::Interactive);
        assert!(SchedulingClass::Interactive < SchedulingClass::Bulk);
    }

    #[test]
    fn error_reason_round_trip() {
        let frame = Frame::error(ErrorReason::CreditOverrun, "credit exceeded");
        assert_eq!(
            frame.error_reason().unwrap(),
            Some(ErrorReason::CreditOverrun)
        );
        let encoded = frame.encode().unwrap();
        let (decoded, _) = Frame::decode_prefix(&encoded).unwrap().unwrap();
        assert_eq!(
            decoded.error_reason().unwrap(),
            Some(ErrorReason::CreditOverrun)
        );
    }

    #[test]
    fn window_update_amount_is_big_endian_u32() {
        let frame = Frame::new(
            FrameType::WindowUpdate,
            5,
            0,
            4096u32.to_be_bytes().to_vec(),
        )
        .unwrap();
        assert_eq!(frame.window_update_amount().unwrap(), Some(4096));
    }

    #[test]
    fn stream_id_parity() {
        assert!(stream_id_is_initiator(1));
        assert!(!stream_id_is_initiator(2));
        assert!(!stream_id_is_initiator(0));
    }
}
