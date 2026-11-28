//! The `fleet id` conversion + serving edge, build slice 5 of
//! design #10391 (pass 2, §5 slice 5; issue #15399): threshold-
//! triggered postcard→CBOR/MessagePack conversion (§4.4) and the
//! `fleet id serve-ws` WebSocket serving socket (§4.5).
//!
//! Design §4.4/§4.5 contract implemented here:
//!
//! - **Conversion conditions (any one triggers):** the consumer is
//!   a browser or other non-Rust WS/WTransport client; OR the
//!   payload exceeds the postcard comfortable bound (default
//!   threshold [`CONVERSION_THRESHOLD`] = 1 MiB per value); OR the
//!   exchange needs incremental/streaming delivery. Conversion is a
//!   fleet-edge function only — the iroh wire stays postcard.
//! - **Variants by name, never by position:** conversion pivots
//!   through `serde_json::Value`, whose externally-tagged enum form
//!   carries the variant NAME (`"List"`, `{"Find": {...}}`), so a
//!   reordered-variant peer (the §4.4 skew case) decodes the
//!   converted form correctly where positional postcard silently
//!   corrupts (slice 3 proved the positional corruption).
//! - **Envelope (§4.5):** each WS message is one CBOR/MessagePack
//!   map `{ v: 1, seq, kind: req|resp|chunk|error, alpn, payload,
//!   digest? }`. Neither field names nor the shape are forked here;
//!   a change is a joint version bump (`fleet-id-cbor-v2`).
//! - **Chunking:** a converted payload larger than the threshold
//!   is split into [`CHUNK_SIZE`] (256 KiB) chunk envelopes with
//!   consecutive `seq` and a per-chunk BLAKE3 `digest`; a final
//!   empty chunk carries the BLAKE3 digest of the whole reassembled
//!   payload as its `digest`, so integrity is end-to-end, verified
//!   per chunk and in full at reassembly.
//! - **Auth (§4.5):** the socket binds localhost by default;
//!   non-local binding without a session credential is refused
//!   before the listener is created (hard error, §4.7). The
//!   credential is the NodeId-derived session token
//!   ([`session_token`]); when a token is configured it is required
//!   as `Authorization: Bearer <token>` on the handshake, on any
//!   bind.
//! - **Negotiation:** the client offers `Sec-WebSocket-Protocol:
//!   fleet-id-cbor-v1` (preferred) or `fleet-id-msgpack-v1`; the
//!   server answers with the first offered protocol it speaks and
//!   refuses the handshake when neither is offered.
//!
//! **Session-event `kind` decision (this slice's open question,
//! recorded on #14229):** pass-3 §4.2 wraps session *events* in
//! this envelope, but neither design names an event kind. Decision:
//! events ride as `kind: resp` ([`SESSION_EVENT_KIND`]). `resp` is
//! the only server→client value kind; `chunk` is reserved for
//! digest-bearing fragments of a chunked transfer and `error` for
//! failures, so an event — a complete value delivered
//! server→client — is a response-class message whose `seq` orders
//! it in the server→client sequence space. No new kind value and
//! no new envelope fields: the envelope is not forked.
//!
//! The bridge between the socket and the ID side is the
//! [`Bridge`] seam: it receives the converted payload bytes and
//! returns converted response bytes. The typed conversion helpers
//! ([`postcard_to_cbor`], [`cbor_to_postcard`], and the MessagePack
//! pair) are how a bridge crosses between the converted form and
//! the postcard wire form for a concrete value type; the live iroh
//! dial plugs into the same seam (slices 3–4 landed their layers
//! ahead of their live verbs the same way). The CLI's built-in
//! bridge is the fixture echo bridge ([`echo_bridge`]).

use std::collections::BTreeMap;
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;

use base64::Engine as _;
use sha1::{Digest as _, Sha1};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

/// Subprotocol negotiating the CBOR envelope form (§4.5).
pub const CBOR_PROTOCOL: &str = "fleet-id-cbor-v1";
/// Subprotocol negotiating the MessagePack envelope form (§4.5).
pub const MSGPACK_PROTOCOL: &str = "fleet-id-msgpack-v1";

/// The envelope version this slice speaks (`v: 1`, §4.5).
pub const ENVELOPE_VERSION: u64 = 1;

/// Default conversion threshold (§4.4): a value larger than 1 MiB
/// leaves postcard's comfortable bound and is converted + chunked.
pub const CONVERSION_THRESHOLD: usize = 1024 * 1024;

/// Chunk size for converted transfers (§4.4): 256 KiB per chunk,
/// each chunk its own WS message with its own sequence number.
pub const CHUNK_SIZE: usize = 256 * 1024;

/// The `kind` session events take in this envelope — see the
/// module-level decision record. Recorded on #14229.
pub const SESSION_EVENT_KIND: EnvelopeKind = EnvelopeKind::Resp;

/// Largest single WS message the socket accepts. Chunk envelopes
/// are ~256 KiB and single envelopes stay near the 1 MiB threshold,
/// so 8 MiB is generous headroom, not a transfer path.
const MAX_MESSAGE_BYTES: usize = 8 * 1024 * 1024;

/// Default bind for `fleet id serve-ws`: localhost only (§4.5).
pub const DEFAULT_BIND: &str = "127.0.0.1:9473";

/// Exit code: usage / argument errors.
pub const EXIT_USAGE: i32 = 2;
/// Exit code: non-local serve attempted without credentials (§4.7).
pub const EXIT_NO_CREDENTIALS: i32 = 3;
/// Exit code: the listener could not be created or the serve loop
/// failed after binding.
pub const EXIT_SERVE: i32 = 4;

/// Which converted wire form a connection negotiated (§4.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WireFormat {
    /// CBOR (RFC 8949), the preferred converted form.
    Cbor,
    /// MessagePack, accepted where the consumer demands it.
    Msgpack,
}

impl WireFormat {
    /// The `Sec-WebSocket-Protocol` token for this format.
    pub fn protocol(self) -> &'static str {
        match self {
            WireFormat::Cbor => CBOR_PROTOCOL,
            WireFormat::Msgpack => MSGPACK_PROTOCOL,
        }
    }

    /// The format a subprotocol token names, if this slice speaks
    /// it.
    pub fn from_protocol(token: &str) -> Option<Self> {
        match token {
            CBOR_PROTOCOL => Some(WireFormat::Cbor),
            MSGPACK_PROTOCOL => Some(WireFormat::Msgpack),
            _ => None,
        }
    }
}

/// §4.4 conversion conditions: any one trigger converts. The
/// threshold is a per-call input so callers can tune it (§4.4:
/// "default threshold 1 MiB per value — tunable per call").
pub fn conversion_triggered(
    consumer_is_browser: bool,
    payload_len: usize,
    streaming: bool,
) -> bool {
    consumer_is_browser || streaming || payload_len > CONVERSION_THRESHOLD
}

/// Whether a converted payload of this size travels chunked
/// (§4.4 wire-efficiency note) rather than as one envelope.
pub fn needs_chunking(encoded_len: usize) -> bool {
    encoded_len > CONVERSION_THRESHOLD
}

// ---------------------------------------------------------------------------
// Converted-form value model + CBOR / MessagePack codecs
// ---------------------------------------------------------------------------

/// The converted-form value tree both codecs share. Deliberately
/// the `serde_json::Value` shape plus a byte-string leaf, which
/// only the envelope layer uses (payload / digest fields); the
/// typed conversion layer never produces byte strings.
#[derive(Debug, Clone, PartialEq)]
enum CodecValue {
    Null,
    Bool(bool),
    U64(u64),
    I64(i64),
    F64(f64),
    Str(String),
    Bytes(Vec<u8>),
    Seq(Vec<CodecValue>),
    Map(Vec<(String, CodecValue)>),
}

/// Why a conversion or codec operation failed. Every variant is a
/// hard error: no best-effort decode, no silent fallback (§4.7).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConvertError {
    /// Postcard rejected the bytes (strict decode, slice 3 rules).
    Postcard(String),
    /// The CBOR/MessagePack bytes were malformed or trailed.
    Codec(String),
    /// The JSON pivot rejected the value or the target type.
    Json(String),
    /// A chunk sequence was broken (kind, alpn, seq, or digest).
    Chunk(String),
    /// The envelope itself was malformed (version, kind, fields).
    Envelope(String),
}

impl std::fmt::Display for ConvertError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConvertError::Postcard(msg) => write!(f, "postcard conversion failed: {msg}"),
            ConvertError::Codec(msg) => write!(f, "converted-form codec failed: {msg}"),
            ConvertError::Json(msg) => write!(f, "conversion pivot failed: {msg}"),
            ConvertError::Chunk(msg) => write!(f, "chunked transfer broken: {msg}"),
            ConvertError::Envelope(msg) => write!(f, "envelope malformed: {msg}"),
        }
    }
}

impl std::error::Error for ConvertError {}

fn json_to_codec(value: &serde_json::Value) -> CodecValue {
    match value {
        serde_json::Value::Null => CodecValue::Null,
        serde_json::Value::Bool(b) => CodecValue::Bool(*b),
        serde_json::Value::Number(n) => {
            if let Some(u) = n.as_u64() {
                CodecValue::U64(u)
            } else if let Some(i) = n.as_i64() {
                CodecValue::I64(i)
            } else if let Some(fl) = n.as_f64() {
                CodecValue::F64(fl)
            } else {
                CodecValue::Null
            }
        }
        serde_json::Value::String(s) => CodecValue::Str(s.clone()),
        serde_json::Value::Array(items) => {
            CodecValue::Seq(items.iter().map(json_to_codec).collect())
        }
        serde_json::Value::Object(map) => CodecValue::Map(
            map.iter()
                .map(|(k, v)| (k.clone(), json_to_codec(v)))
                .collect(),
        ),
    }
}

fn codec_to_json(value: &CodecValue) -> Result<serde_json::Value, ConvertError> {
    Ok(match value {
        CodecValue::Null => serde_json::Value::Null,
        CodecValue::Bool(b) => serde_json::Value::Bool(*b),
        CodecValue::U64(u) => serde_json::Value::Number((*u).into()),
        CodecValue::I64(i) => serde_json::Value::Number((*i).into()),
        CodecValue::F64(fl) => serde_json::Value::Number(
            serde_json::Number::from_f64(*fl)
                .ok_or_else(|| ConvertError::Codec(format!("non-finite float {fl}")))?,
        ),
        CodecValue::Str(s) => serde_json::Value::String(s.clone()),
        CodecValue::Bytes(_) => {
            return Err(ConvertError::Codec(
                "byte string has no value-layer form (envelope fields only)".to_string(),
            ));
        }
        CodecValue::Seq(items) => {
            serde_json::Value::Array(items.iter().map(codec_to_json).collect::<Result<_, _>>()?)
        }
        CodecValue::Map(entries) => serde_json::Value::Object(
            entries
                .iter()
                .map(|(k, v)| Ok((k.clone(), codec_to_json(v)?)))
                .collect::<Result<_, ConvertError>>()?,
        ),
    })
}

/// A byte cursor shared by both decoders.
struct Reader<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn new(buf: &'a [u8]) -> Self {
        Reader { buf, pos: 0 }
    }

    fn byte(&mut self) -> Result<u8, ConvertError> {
        let b = self
            .buf
            .get(self.pos)
            .copied()
            .ok_or_else(|| ConvertError::Codec("unexpected end of input".to_string()))?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], ConvertError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| ConvertError::Codec("length overflows the input".to_string()))?;
        let slice = self
            .buf
            .get(self.pos..end)
            .ok_or_else(|| ConvertError::Codec("declared length exceeds the input".to_string()))?;
        self.pos = end;
        Ok(slice)
    }

    fn uint(&mut self, n: usize) -> Result<u64, ConvertError> {
        let bytes = self.take(n)?;
        let mut out: u64 = 0;
        for b in bytes {
            out = (out << 8) | u64::from(*b);
        }
        Ok(out)
    }

    fn finish<T>(self, value: T) -> Result<T, ConvertError> {
        if self.pos != self.buf.len() {
            return Err(ConvertError::Codec(format!(
                "{} trailing byte(s) after the value",
                self.buf.len() - self.pos
            )));
        }
        Ok(value)
    }
}

const MAX_DEPTH: usize = 64;

// --- CBOR (RFC 8949) ---

fn cbor_write_head(out: &mut Vec<u8>, major: u8, arg: u64) {
    let mt = major << 5;
    if arg < 24 {
        out.push(mt | arg as u8);
    } else if arg <= u64::from(u8::MAX) {
        out.push(mt | 24);
        out.push(arg as u8);
    } else if arg <= u64::from(u16::MAX) {
        out.push(mt | 25);
        out.extend_from_slice(&(arg as u16).to_be_bytes());
    } else if arg <= u64::from(u32::MAX) {
        out.push(mt | 26);
        out.extend_from_slice(&(arg as u32).to_be_bytes());
    } else {
        out.push(mt | 27);
        out.extend_from_slice(&arg.to_be_bytes());
    }
}

fn cbor_encode(value: &CodecValue, out: &mut Vec<u8>) {
    match value {
        CodecValue::Null => out.push(0xf6),
        CodecValue::Bool(false) => out.push(0xf4),
        CodecValue::Bool(true) => out.push(0xf5),
        CodecValue::U64(u) => cbor_write_head(out, 0, *u),
        CodecValue::I64(i) if *i >= 0 => cbor_write_head(out, 0, *i as u64),
        CodecValue::I64(i) => cbor_write_head(out, 1, (-1i128 - i128::from(*i)) as u64),
        CodecValue::F64(fl) => {
            out.push(0xfb);
            out.extend_from_slice(&fl.to_be_bytes());
        }
        CodecValue::Str(s) => {
            cbor_write_head(out, 3, s.len() as u64);
            out.extend_from_slice(s.as_bytes());
        }
        CodecValue::Bytes(b) => {
            cbor_write_head(out, 2, b.len() as u64);
            out.extend_from_slice(b);
        }
        CodecValue::Seq(items) => {
            cbor_write_head(out, 4, items.len() as u64);
            for item in items {
                cbor_encode(item, out);
            }
        }
        CodecValue::Map(entries) => {
            cbor_write_head(out, 5, entries.len() as u64);
            for (k, v) in entries {
                cbor_encode(&CodecValue::Str(k.clone()), out);
                cbor_encode(v, out);
            }
        }
    }
}

fn cbor_read_arg(r: &mut Reader<'_>, info: u8) -> Result<u64, ConvertError> {
    match info {
        0..=23 => Ok(u64::from(info)),
        24 => Ok(u64::from(r.byte()?)),
        25 => r.uint(2),
        26 => r.uint(4),
        27 => r.uint(8),
        _ => Err(ConvertError::Codec(format!(
            "reserved/indefinite CBOR additional info {info}"
        ))),
    }
}

fn f16_to_f64(h: u16) -> f64 {
    let sign = u64::from(h >> 15) << 63;
    let exp = i32::from((h >> 10) & 0x1f);
    let frac = u64::from(h & 0x3ff);
    let bits = if exp == 0 {
        if frac == 0 {
            sign
        } else {
            let mut e: i32 = -14;
            let mut fr = frac;
            while fr & 0x400 == 0 {
                fr <<= 1;
                e -= 1;
            }
            sign | (((e + 1023) as u64) << 52) | ((fr & 0x3ff) << 42)
        }
    } else if exp == 31 {
        sign | (0x7ffu64 << 52) | (frac << 42)
    } else {
        sign | (((exp - 15 + 1023) as u64) << 52) | (frac << 42)
    };
    f64::from_bits(bits)
}

fn cbor_decode(r: &mut Reader<'_>, depth: usize) -> Result<CodecValue, ConvertError> {
    if depth > MAX_DEPTH {
        return Err(ConvertError::Codec("nesting too deep".to_string()));
    }
    let head = r.byte()?;
    let major = head >> 5;
    let info = head & 0x1f;
    Ok(match major {
        0 => CodecValue::U64(cbor_read_arg(r, info)?),
        1 => {
            let arg = cbor_read_arg(r, info)?;
            let value = -1i128 - i128::from(arg);
            CodecValue::I64(
                i64::try_from(value).map_err(|_| {
                    ConvertError::Codec("negative integer underflows i64".to_string())
                })?,
            )
        }
        2 => {
            let len = usize::try_from(cbor_read_arg(r, info)?)
                .map_err(|_| ConvertError::Codec("byte string too long".to_string()))?;
            CodecValue::Bytes(r.take(len)?.to_vec())
        }
        3 => {
            let len = usize::try_from(cbor_read_arg(r, info)?)
                .map_err(|_| ConvertError::Codec("text string too long".to_string()))?;
            let bytes = r.take(len)?;
            CodecValue::Str(
                String::from_utf8(bytes.to_vec())
                    .map_err(|_| ConvertError::Codec("text string is not UTF-8".to_string()))?,
            )
        }
        4 => {
            let len = usize::try_from(cbor_read_arg(r, info)?)
                .map_err(|_| ConvertError::Codec("array too long".to_string()))?;
            let mut items = Vec::with_capacity(len.min(4096));
            for _ in 0..len {
                items.push(cbor_decode(r, depth + 1)?);
            }
            CodecValue::Seq(items)
        }
        5 => {
            let len = usize::try_from(cbor_read_arg(r, info)?)
                .map_err(|_| ConvertError::Codec("map too long".to_string()))?;
            let mut entries = Vec::with_capacity(len.min(4096));
            for _ in 0..len {
                let key = match cbor_decode(r, depth + 1)? {
                    CodecValue::Str(s) => s,
                    _ => {
                        return Err(ConvertError::Codec(
                            "map key is not a text string".to_string(),
                        ));
                    }
                };
                entries.push((key, cbor_decode(r, depth + 1)?));
            }
            CodecValue::Map(entries)
        }
        7 => match info {
            20 => CodecValue::Bool(false),
            21 => CodecValue::Bool(true),
            22 => CodecValue::Null,
            25 => CodecValue::F64(f16_to_f64(r.uint(2)? as u16)),
            26 => CodecValue::F64(f64::from(f32::from_bits(r.uint(4)? as u32))),
            27 => CodecValue::F64(f64::from_bits(r.uint(8)?)),
            _ => {
                return Err(ConvertError::Codec(format!(
                    "unsupported CBOR simple/float info {info}"
                )));
            }
        },
        _ => unreachable!("major type is 3 bits"),
    })
}

// --- MessagePack ---

fn msgpack_encode(value: &CodecValue, out: &mut Vec<u8>) {
    match value {
        CodecValue::Null => out.push(0xc0),
        CodecValue::Bool(false) => out.push(0xc2),
        CodecValue::Bool(true) => out.push(0xc3),
        CodecValue::U64(u) => {
            if *u <= 0x7f {
                out.push(*u as u8);
            } else if *u <= u64::from(u8::MAX) {
                out.push(0xcc);
                out.push(*u as u8);
            } else if *u <= u64::from(u16::MAX) {
                out.push(0xcd);
                out.extend_from_slice(&(*u as u16).to_be_bytes());
            } else if *u <= u64::from(u32::MAX) {
                out.push(0xce);
                out.extend_from_slice(&(*u as u32).to_be_bytes());
            } else {
                out.push(0xcf);
                out.extend_from_slice(&u.to_be_bytes());
            }
        }
        CodecValue::I64(i) => {
            if *i >= 0 {
                msgpack_encode(&CodecValue::U64(*i as u64), out);
            } else if *i >= -32 {
                out.push((i128::from(*i) + 256) as u8);
            } else if *i >= i64::from(i8::MIN) {
                out.push(0xd0);
                out.push(*i as u8);
            } else if *i >= i64::from(i16::MIN) {
                out.push(0xd1);
                out.extend_from_slice(&(*i as i16).to_be_bytes());
            } else if *i >= i64::from(i32::MIN) {
                out.push(0xd2);
                out.extend_from_slice(&(*i as i32).to_be_bytes());
            } else {
                out.push(0xd3);
                out.extend_from_slice(&i.to_be_bytes());
            }
        }
        CodecValue::F64(fl) => {
            out.push(0xcb);
            out.extend_from_slice(&fl.to_be_bytes());
        }
        CodecValue::Str(s) => {
            let len = s.len();
            if len <= 31 {
                out.push(0xa0 | len as u8);
            } else if len <= usize::from(u8::MAX) {
                out.push(0xd9);
                out.push(len as u8);
            } else if len <= usize::from(u16::MAX) {
                out.push(0xda);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                out.push(0xdb);
                out.extend_from_slice(&(len as u32).to_be_bytes());
            }
            out.extend_from_slice(s.as_bytes());
        }
        CodecValue::Bytes(b) => {
            let len = b.len();
            if len <= usize::from(u8::MAX) {
                out.push(0xc4);
                out.push(len as u8);
            } else if len <= usize::from(u16::MAX) {
                out.push(0xc5);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                out.push(0xc6);
                out.extend_from_slice(&(len as u32).to_be_bytes());
            }
            out.extend_from_slice(b);
        }
        CodecValue::Seq(items) => {
            let len = items.len();
            if len <= 15 {
                out.push(0x90 | len as u8);
            } else if len <= usize::from(u16::MAX) {
                out.push(0xdc);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                out.push(0xdd);
                out.extend_from_slice(&(len as u32).to_be_bytes());
            }
            for item in items {
                msgpack_encode(item, out);
            }
        }
        CodecValue::Map(entries) => {
            let len = entries.len();
            if len <= 15 {
                out.push(0x80 | len as u8);
            } else if len <= usize::from(u16::MAX) {
                out.push(0xde);
                out.extend_from_slice(&(len as u16).to_be_bytes());
            } else {
                out.push(0xdf);
                out.extend_from_slice(&(len as u32).to_be_bytes());
            }
            for (k, v) in entries {
                msgpack_encode(&CodecValue::Str(k.clone()), out);
                msgpack_encode(v, out);
            }
        }
    }
}

fn msgpack_decode(r: &mut Reader<'_>, depth: usize) -> Result<CodecValue, ConvertError> {
    if depth > MAX_DEPTH {
        return Err(ConvertError::Codec("nesting too deep".to_string()));
    }
    let head = r.byte()?;
    let str_len = |r: &mut Reader<'_>, n: usize| -> Result<String, ConvertError> {
        let bytes = r.take(n)?;
        String::from_utf8(bytes.to_vec())
            .map_err(|_| ConvertError::Codec("msgpack str is not UTF-8".to_string()))
    };
    Ok(match head {
        0x00..=0x7f => CodecValue::U64(u64::from(head)),
        0xe0..=0xff => CodecValue::I64(i64::from(head as i8)),
        0x80..=0x8f => {
            let mut entries = Vec::new();
            for _ in 0..(head & 0x0f) {
                let key = match msgpack_decode(r, depth + 1)? {
                    CodecValue::Str(s) => s,
                    _ => {
                        return Err(ConvertError::Codec(
                            "msgpack map key is not a str".to_string(),
                        ));
                    }
                };
                entries.push((key, msgpack_decode(r, depth + 1)?));
            }
            CodecValue::Map(entries)
        }
        0x90..=0x9f => {
            let mut items = Vec::new();
            for _ in 0..(head & 0x0f) {
                items.push(msgpack_decode(r, depth + 1)?);
            }
            CodecValue::Seq(items)
        }
        0xa0..=0xbf => CodecValue::Str(str_len(r, usize::from(head & 0x1f))?),
        0xc0 => CodecValue::Null,
        0xc2 => CodecValue::Bool(false),
        0xc3 => CodecValue::Bool(true),
        0xc4 => {
            let len = usize::from(r.byte()?);
            CodecValue::Bytes(r.take(len)?.to_vec())
        }
        0xc5 => {
            let len = usize::try_from(r.uint(2)?).unwrap_or(usize::MAX);
            CodecValue::Bytes(r.take(len)?.to_vec())
        }
        0xc6 => {
            let len = usize::try_from(r.uint(4)?).unwrap_or(usize::MAX);
            CodecValue::Bytes(r.take(len)?.to_vec())
        }
        0xca => CodecValue::F64(f64::from(f32::from_bits(r.uint(4)? as u32))),
        0xcb => CodecValue::F64(f64::from_bits(r.uint(8)?)),
        0xcc => CodecValue::U64(u64::from(r.byte()?)),
        0xcd => CodecValue::U64(r.uint(2)?),
        0xce => CodecValue::U64(r.uint(4)?),
        0xcf => CodecValue::U64(r.uint(8)?),
        0xd0 => CodecValue::I64(i64::from(r.byte()? as i8)),
        0xd1 => CodecValue::I64(i64::from(r.uint(2)? as u16 as i16)),
        0xd2 => CodecValue::I64(i64::from(r.uint(4)? as u32 as i32)),
        0xd3 => CodecValue::I64(r.uint(8)? as i64),
        0xd9 => {
            let len = usize::from(r.byte()?);
            CodecValue::Str(str_len(r, len)?)
        }
        0xda => {
            let len = usize::try_from(r.uint(2)?).unwrap_or(usize::MAX);
            CodecValue::Str(str_len(r, len)?)
        }
        0xdb => {
            let len = usize::try_from(r.uint(4)?).unwrap_or(usize::MAX);
            CodecValue::Str(str_len(r, len)?)
        }
        0xdc => {
            let len = r.uint(2)?;
            let mut items = Vec::new();
            for _ in 0..len {
                items.push(msgpack_decode(r, depth + 1)?);
            }
            CodecValue::Seq(items)
        }
        0xdd => {
            let len = r.uint(4)?;
            let mut items = Vec::new();
            for _ in 0..len {
                items.push(msgpack_decode(r, depth + 1)?);
            }
            CodecValue::Seq(items)
        }
        0xde => {
            let len = r.uint(2)?;
            let mut entries = Vec::new();
            for _ in 0..len {
                let key = match msgpack_decode(r, depth + 1)? {
                    CodecValue::Str(s) => s,
                    _ => {
                        return Err(ConvertError::Codec(
                            "msgpack map key is not a str".to_string(),
                        ));
                    }
                };
                entries.push((key, msgpack_decode(r, depth + 1)?));
            }
            CodecValue::Map(entries)
        }
        0xdf => {
            let len = r.uint(4)?;
            let mut entries = Vec::new();
            for _ in 0..len {
                let key = match msgpack_decode(r, depth + 1)? {
                    CodecValue::Str(s) => s,
                    _ => {
                        return Err(ConvertError::Codec(
                            "msgpack map key is not a str".to_string(),
                        ));
                    }
                };
                entries.push((key, msgpack_decode(r, depth + 1)?));
            }
            CodecValue::Map(entries)
        }
        _ => {
            return Err(ConvertError::Codec(format!(
                "unsupported msgpack marker {head:#04x}"
            )));
        }
    })
}

fn encode_value(value: &CodecValue, format: WireFormat) -> Vec<u8> {
    let mut out = Vec::new();
    match format {
        WireFormat::Cbor => cbor_encode(value, &mut out),
        WireFormat::Msgpack => msgpack_encode(value, &mut out),
    }
    out
}

fn decode_value(bytes: &[u8], format: WireFormat) -> Result<CodecValue, ConvertError> {
    let mut reader = Reader::new(bytes);
    let value = match format {
        WireFormat::Cbor => cbor_decode(&mut reader, 0)?,
        WireFormat::Msgpack => msgpack_decode(&mut reader, 0)?,
    };
    reader.finish(value)
}

// ---------------------------------------------------------------------------
// Typed conversion (§4.4): postcard <-> converted form, variants by name
// ---------------------------------------------------------------------------

/// Convert one postcard-encoded value (bare body, slice-3 strict
/// rules) to its CBOR form, enum variants encoded by NAME.
pub fn postcard_to_cbor<T>(postcard_bytes: &[u8]) -> Result<Vec<u8>, ConvertError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let value: T = crate::id_postcard::decode_strict(postcard_bytes)
        .map_err(|e| ConvertError::Postcard(e.to_string()))?;
    let json = serde_json::to_value(&value).map_err(|e| ConvertError::Json(e.to_string()))?;
    Ok(encode_value(&json_to_codec(&json), WireFormat::Cbor))
}

/// Convert one CBOR value back to its postcard body. The target
/// type's variants are matched by NAME, so bytes produced from a
/// reordered-variant peer still land on the right variant.
pub fn cbor_to_postcard<T>(cbor_bytes: &[u8]) -> Result<Vec<u8>, ConvertError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let json = codec_to_json(&decode_value(cbor_bytes, WireFormat::Cbor)?)?;
    let value: T = serde_json::from_value(json).map_err(|e| ConvertError::Json(e.to_string()))?;
    postcard::to_allocvec(&value).map_err(|e| ConvertError::Postcard(e.to_string()))
}

/// MessagePack twin of [`postcard_to_cbor`].
pub fn postcard_to_msgpack<T>(postcard_bytes: &[u8]) -> Result<Vec<u8>, ConvertError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let value: T = crate::id_postcard::decode_strict(postcard_bytes)
        .map_err(|e| ConvertError::Postcard(e.to_string()))?;
    let json = serde_json::to_value(&value).map_err(|e| ConvertError::Json(e.to_string()))?;
    Ok(encode_value(&json_to_codec(&json), WireFormat::Msgpack))
}

/// MessagePack twin of [`cbor_to_postcard`].
pub fn msgpack_to_postcard<T>(msgpack_bytes: &[u8]) -> Result<Vec<u8>, ConvertError>
where
    T: serde::de::DeserializeOwned + serde::Serialize,
{
    let json = codec_to_json(&decode_value(msgpack_bytes, WireFormat::Msgpack)?)?;
    let value: T = serde_json::from_value(json).map_err(|e| ConvertError::Json(e.to_string()))?;
    postcard::to_allocvec(&value).map_err(|e| ConvertError::Postcard(e.to_string()))
}

// ---------------------------------------------------------------------------
// The §4.5 envelope
// ---------------------------------------------------------------------------

/// The envelope's `kind` (§4.5). The four values are the design's;
/// adding one is an envelope change both passes must name.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeKind {
    /// Client→server request.
    Req,
    /// Server→client response — and, by this slice's recorded
    /// decision, server→client session events
    /// ([`SESSION_EVENT_KIND`]).
    Resp,
    /// One fragment of a chunked transfer (digest-bearing).
    Chunk,
    /// A failure; the payload is a UTF-8 diagnostic.
    Error,
}

impl EnvelopeKind {
    /// The wire string for this kind.
    pub fn as_str(self) -> &'static str {
        match self {
            EnvelopeKind::Req => "req",
            EnvelopeKind::Resp => "resp",
            EnvelopeKind::Chunk => "chunk",
            EnvelopeKind::Error => "error",
        }
    }

    /// The kind a wire string names, if it is one of the four.
    pub fn from_wire(s: &str) -> Option<Self> {
        match s {
            "req" => Some(EnvelopeKind::Req),
            "resp" => Some(EnvelopeKind::Resp),
            "chunk" => Some(EnvelopeKind::Chunk),
            "error" => Some(EnvelopeKind::Error),
            _ => None,
        }
    }
}

/// One §4.5 envelope: `{ v: 1, seq, kind, alpn, payload, digest? }`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Sequence number within the sender's sequence space.
    /// Responses carry the sequence of the request they answer;
    /// chunks increment from the transfer's base sequence.
    pub seq: u64,
    /// What this message is.
    pub kind: EnvelopeKind,
    /// The ID protocol the payload came from (e.g. `/iroh-meta/3`).
    pub alpn: String,
    /// The converted-form payload (or one chunk of it).
    pub payload: Vec<u8>,
    /// BLAKE3 digest: per-chunk on chunk envelopes, the whole
    /// payload's digest on the terminating chunk; absent on
    /// single-envelope transfers.
    pub digest: Option<[u8; 32]>,
}

impl Envelope {
    /// A single-envelope request/response/error.
    pub fn single(kind: EnvelopeKind, seq: u64, alpn: &str, payload: Vec<u8>) -> Self {
        Envelope {
            seq,
            kind,
            alpn: alpn.to_string(),
            payload,
            digest: None,
        }
    }

    /// Encode this envelope in the negotiated format.
    pub fn encode(&self, format: WireFormat) -> Vec<u8> {
        let mut entries = vec![
            ("v".to_string(), CodecValue::U64(ENVELOPE_VERSION)),
            ("seq".to_string(), CodecValue::U64(self.seq)),
            (
                "kind".to_string(),
                CodecValue::Str(self.kind.as_str().to_string()),
            ),
            ("alpn".to_string(), CodecValue::Str(self.alpn.clone())),
            (
                "payload".to_string(),
                CodecValue::Bytes(self.payload.clone()),
            ),
        ];
        if let Some(digest) = &self.digest {
            entries.push(("digest".to_string(), CodecValue::Bytes(digest.to_vec())));
        }
        encode_value(&CodecValue::Map(entries), format)
    }

    /// Decode one envelope, strictly: version must be 1, kind must
    /// be one of the four, every field must be present and typed.
    pub fn decode(bytes: &[u8], format: WireFormat) -> Result<Self, ConvertError> {
        let entries = match decode_value(bytes, format)? {
            CodecValue::Map(entries) => entries,
            _ => return Err(ConvertError::Envelope("envelope is not a map".to_string())),
        };
        let fields: BTreeMap<String, CodecValue> = entries.into_iter().collect();
        let version = match fields.get("v") {
            Some(CodecValue::U64(v)) => *v,
            _ => return Err(ConvertError::Envelope("missing/invalid v".to_string())),
        };
        if version != ENVELOPE_VERSION {
            return Err(ConvertError::Envelope(format!(
                "unsupported envelope version {version} (this slice speaks v{ENVELOPE_VERSION}; a change is a joint version bump)"
            )));
        }
        let seq = match fields.get("seq") {
            Some(CodecValue::U64(s)) => *s,
            _ => return Err(ConvertError::Envelope("missing/invalid seq".to_string())),
        };
        let kind = match fields.get("kind") {
            Some(CodecValue::Str(s)) => EnvelopeKind::from_wire(s)
                .ok_or_else(|| ConvertError::Envelope(format!("unknown envelope kind {s:?}")))?,
            _ => return Err(ConvertError::Envelope("missing/invalid kind".to_string())),
        };
        let alpn = match fields.get("alpn") {
            Some(CodecValue::Str(s)) => s.clone(),
            _ => return Err(ConvertError::Envelope("missing/invalid alpn".to_string())),
        };
        let payload = match fields.get("payload") {
            Some(CodecValue::Bytes(b)) => b.clone(),
            _ => {
                return Err(ConvertError::Envelope(
                    "missing/invalid payload".to_string(),
                ));
            }
        };
        let digest = match fields.get("digest") {
            None => None,
            Some(CodecValue::Bytes(b)) if b.len() == 32 => {
                let mut digest = [0u8; 32];
                digest.copy_from_slice(b);
                Some(digest)
            }
            Some(_) => {
                return Err(ConvertError::Envelope(
                    "digest is not a 32-byte string".to_string(),
                ));
            }
        };
        Ok(Envelope {
            seq,
            kind,
            alpn,
            payload,
            digest,
        })
    }
}

/// The envelopes carrying one payload in one direction: a single
/// envelope when it fits under the conversion threshold, otherwise
/// the chunked form ([`chunk_envelopes`]).
pub fn envelopes_for_payload(
    kind: EnvelopeKind,
    alpn: &str,
    seq: u64,
    payload: &[u8],
) -> Vec<Envelope> {
    if needs_chunking(payload.len()) {
        chunk_envelopes(alpn, seq, payload)
    } else {
        vec![Envelope::single(kind, seq, alpn, payload.to_vec())]
    }
}

/// Split a payload into chunk envelopes (§4.4/§4.5): consecutive
/// sequences from `base_seq`, each chunk's `digest` the BLAKE3 of
/// that chunk, and a terminating empty chunk whose `digest` is the
/// BLAKE3 of the whole payload.
pub fn chunk_envelopes(alpn: &str, base_seq: u64, payload: &[u8]) -> Vec<Envelope> {
    let mut out = Vec::new();
    let mut seq = base_seq;
    for chunk in payload.chunks(CHUNK_SIZE) {
        out.push(Envelope {
            seq,
            kind: EnvelopeKind::Chunk,
            alpn: alpn.to_string(),
            payload: chunk.to_vec(),
            digest: Some(*blake3::hash(chunk).as_bytes()),
        });
        seq += 1;
    }
    out.push(Envelope {
        seq,
        kind: EnvelopeKind::Chunk,
        alpn: alpn.to_string(),
        payload: Vec::new(),
        digest: Some(*blake3::hash(payload).as_bytes()),
    });
    out
}

/// Reassembles one chunked transfer, verifying every chunk's
/// digest on arrival, the sequence's contiguity, and the whole
/// payload's digest at the terminator. Any break is a hard error.
#[derive(Debug)]
pub struct ChunkReassembler {
    alpn: String,
    base_seq: Option<u64>,
    next_seq: u64,
    buf: Vec<u8>,
    finished: bool,
}

impl ChunkReassembler {
    /// A reassembler for one transfer on the given ALPN.
    pub fn new(alpn: &str) -> Self {
        ChunkReassembler {
            alpn: alpn.to_string(),
            base_seq: None,
            next_seq: 0,
            buf: Vec::new(),
            finished: false,
        }
    }

    /// The sequence the transfer started at, once the first chunk
    /// has arrived (responses answer at this sequence).
    pub fn base_seq(&self) -> Option<u64> {
        self.base_seq
    }

    /// Feed one envelope. Returns the reassembled payload exactly
    /// once, when the terminating chunk verifies.
    pub fn push(&mut self, env: &Envelope) -> Result<Option<Vec<u8>>, ConvertError> {
        if self.finished {
            return Err(ConvertError::Chunk(
                "chunk after the terminating chunk".to_string(),
            ));
        }
        if env.kind != EnvelopeKind::Chunk {
            return Err(ConvertError::Chunk(format!(
                "expected a chunk envelope, got kind {:?}",
                env.kind
            )));
        }
        if env.alpn != self.alpn {
            return Err(ConvertError::Chunk(format!(
                "chunk alpn {:?} does not match transfer alpn {:?}",
                env.alpn, self.alpn
            )));
        }
        let digest = env
            .digest
            .ok_or_else(|| ConvertError::Chunk("chunk envelope carries no digest".to_string()))?;
        match self.base_seq {
            None => {
                self.base_seq = Some(env.seq);
                self.next_seq = env.seq;
            }
            Some(_) => {}
        }
        if env.seq != self.next_seq {
            return Err(ConvertError::Chunk(format!(
                "chunk sequence gap: expected seq {}, got {}",
                self.next_seq, env.seq
            )));
        }
        self.next_seq += 1;
        if env.payload.is_empty() {
            // Terminating chunk: its digest covers the whole
            // reassembled payload.
            if *blake3::hash(&self.buf).as_bytes() != digest {
                return Err(ConvertError::Chunk(
                    "whole-payload digest mismatch at the terminating chunk".to_string(),
                ));
            }
            self.finished = true;
            return Ok(Some(std::mem::take(&mut self.buf)));
        }
        if *blake3::hash(&env.payload).as_bytes() != digest {
            return Err(ConvertError::Chunk(format!(
                "chunk digest mismatch at seq {}",
                env.seq
            )));
        }
        self.buf.extend_from_slice(&env.payload);
        Ok(None)
    }
}

// ---------------------------------------------------------------------------
// Serving: configuration, credential, bind rules (§4.5)
// ---------------------------------------------------------------------------

/// The session credential for non-local serving (§4.5): the bearer
/// token derived from the session's NodeId — BLAKE3 of the NodeId's
/// hex form, itself hex-encoded. The design leaves the exact
/// construction to the pass-1 credential source; this is the
/// construction this slice fixes, and both sides derive it the
/// same way from the slice-1 stamp.
pub fn session_token(node_id_hex: &str) -> String {
    let digest = blake3::hash(node_id_hex.as_bytes());
    let mut out = String::with_capacity(64);
    for byte in digest.as_bytes() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Serving configuration for one `serve-ws` socket.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// The address to bind. Loopback by default (§4.5).
    pub bind: SocketAddr,
    /// The session credential, when one is configured. Required
    /// for any non-loopback bind; enforced on the handshake
    /// whenever present.
    pub token: Option<String>,
}

impl ServeConfig {
    /// A localhost config with no credential.
    pub fn localhost(port: u16) -> Self {
        ServeConfig {
            bind: SocketAddr::from(([127, 0, 0, 1], port)),
            token: None,
        }
    }

    /// The §4.5 bind rule, checked before any listener exists:
    /// non-local binding without credentials is refused.
    pub fn validate(&self) -> Result<(), ServeError> {
        if !self.bind.ip().is_loopback() && self.token.is_none() {
            return Err(ServeError::NonLocalWithoutCredentials(self.bind));
        }
        Ok(())
    }
}

/// Why serving failed. The variants are the §4.7 hard errors:
/// distinct causes, never a silent fallback.
#[derive(Debug)]
pub enum ServeError {
    /// Non-local bind attempted with no session credential.
    NonLocalWithoutCredentials(SocketAddr),
    /// The listener could not be created.
    Bind(String),
    /// The transport or handshake failed.
    Transport(String),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::NonLocalWithoutCredentials(addr) => write!(
                f,
                "refusing non-local serve on {addr} without credentials: pass --token or --id-secret (design #10391 §4.5: no anonymous non-local serving)"
            ),
            ServeError::Bind(msg) => write!(f, "cannot bind serve-ws listener: {msg}"),
            ServeError::Transport(msg) => write!(f, "serve-ws transport failed: {msg}"),
        }
    }
}

impl std::error::Error for ServeError {}

/// The ID-side bridge: converted payload bytes in, converted
/// response bytes out, per ALPN and negotiated format. See the
/// module docs for how the typed conversion helpers plug in here.
pub type BridgeFuture = Pin<Box<dyn Future<Output = Result<Vec<u8>, String>> + Send>>;
pub type Bridge = Arc<dyn Fn(&str, WireFormat, &[u8]) -> BridgeFuture + Send + Sync>;

/// The CLI's built-in bridge: a fixture echo at the converted
/// layer (the socket, envelope, negotiation, chunking, and auth
/// are fully live; the ID exchange itself is the bridge seam).
pub fn echo_bridge() -> Bridge {
    Arc::new(|_alpn: &str, _format: WireFormat, payload: &[u8]| {
        let payload = payload.to_vec();
        Box::pin(async move { Ok(payload) })
    })
}

// ---------------------------------------------------------------------------
// WebSocket transport (RFC 6455, binary frames)
// ---------------------------------------------------------------------------

const WS_GUID: &str = "258EAFA5-E914-47DA-95CA-C5AB0DC85B11";

/// The `Sec-WebSocket-Accept` value for a client key (RFC 6455
/// §4.2.2): base64 of SHA-1(key + GUID).
pub fn ws_accept_key(client_key: &str) -> String {
    let mut hasher = Sha1::new();
    hasher.update(client_key.as_bytes());
    hasher.update(WS_GUID.as_bytes());
    let digest = hasher.finalize();
    base64::engine::general_purpose::STANDARD.encode(digest)
}

/// Read the HTTP handshake head (request line + headers) one byte
/// at a time up to the blank line, so no frame bytes are consumed.
async fn read_http_head(stream: &mut TcpStream) -> Result<String, ServeError> {
    let mut head = Vec::with_capacity(1024);
    let mut window = [0u8; 4];
    loop {
        let mut byte = [0u8; 1];
        stream
            .read_exact(&mut byte)
            .await
            .map_err(|e| ServeError::Transport(format!("handshake read: {e}")))?;
        head.push(byte[0]);
        window.copy_within(1.., 0);
        window[3] = byte[0];
        if &window == b"\r\n\r\n" {
            break;
        }
        if head.len() > 16 * 1024 {
            return Err(ServeError::Transport(
                "handshake head exceeds 16 KiB".to_string(),
            ));
        }
    }
    String::from_utf8(head)
        .map_err(|_| ServeError::Transport("handshake head is not UTF-8".to_string()))
}

async fn write_http_response(
    stream: &mut TcpStream,
    status: &str,
    extra_headers: &str,
) -> Result<(), ServeError> {
    let response = format!("HTTP/1.1 {status}\r\nConnection: close\r\n{extra_headers}\r\n");
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| ServeError::Transport(format!("handshake write: {e}")))
}

/// Run the server side of the handshake: negotiate the envelope
/// format from `Sec-WebSocket-Protocol`, enforce the configured
/// credential, and answer 101. Any refusal is written as its HTTP
/// status before the error returns.
async fn ws_handshake(
    stream: &mut TcpStream,
    config: &ServeConfig,
) -> Result<WireFormat, ServeError> {
    let head = read_http_head(stream).await?;
    let mut lines = head.lines();
    let request_line = lines.next().unwrap_or("");
    if !request_line.starts_with("GET ") {
        write_http_response(stream, "400 Bad Request", "").await?;
        return Err(ServeError::Transport(format!(
            "handshake is not a GET upgrade: {request_line:?}"
        )));
    }
    let mut headers: BTreeMap<String, String> = BTreeMap::new();
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            headers.insert(name.trim().to_ascii_lowercase(), value.trim().to_string());
        }
    }
    if let Some(token) = &config.token {
        let expected = format!("Bearer {token}");
        if headers.get("authorization").map(|s| s.as_str()) != Some(expected.as_str()) {
            write_http_response(stream, "401 Unauthorized", "").await?;
            return Err(ServeError::Transport(
                "serve-ws handshake refused: missing or wrong session credential".to_string(),
            ));
        }
    }
    let key = match headers.get("sec-websocket-key") {
        Some(k) if !k.is_empty() => k.clone(),
        _ => {
            write_http_response(stream, "400 Bad Request", "").await?;
            return Err(ServeError::Transport(
                "handshake carries no Sec-WebSocket-Key".to_string(),
            ));
        }
    };
    let offered = headers
        .get("sec-websocket-protocol")
        .cloned()
        .unwrap_or_default();
    let negotiated = offered
        .split(',')
        .map(|t| t.trim())
        .find_map(WireFormat::from_protocol);
    let format = match negotiated {
        Some(format) => format,
        None => {
            write_http_response(stream, "400 Bad Request", "").await?;
            return Err(ServeError::Transport(format!(
                "handshake offers no supported subprotocol (want {CBOR_PROTOCOL} or {MSGPACK_PROTOCOL}, got {offered:?})"
            )));
        }
    };
    let accept = ws_accept_key(&key);
    let response = format!(
        "HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\nSec-WebSocket-Protocol: {}\r\n\r\n",
        format.protocol()
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|e| ServeError::Transport(format!("handshake write: {e}")))?;
    Ok(format)
}

/// Write one unmasked binary frame (server→client form).
async fn ws_write_frame(
    stream: &mut TcpStream,
    opcode: u8,
    payload: &[u8],
) -> Result<(), ServeError> {
    let mut head = Vec::with_capacity(10);
    head.push(0x80 | opcode);
    if payload.len() < 126 {
        head.push(payload.len() as u8);
    } else if payload.len() <= usize::from(u16::MAX) {
        head.push(126);
        head.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    } else {
        head.push(127);
        head.extend_from_slice(&(payload.len() as u64).to_be_bytes());
    }
    stream
        .write_all(&head)
        .await
        .map_err(|e| ServeError::Transport(format!("frame write: {e}")))?;
    stream
        .write_all(payload)
        .await
        .map_err(|e| ServeError::Transport(format!("frame write: {e}")))
}

/// Read one complete binary message, answering pings and
/// reassembling fragments. `Ok(None)` is a clean close.
async fn ws_read_message(stream: &mut TcpStream) -> Result<Option<Vec<u8>>, ServeError> {
    let mut message: Vec<u8> = Vec::new();
    loop {
        let mut head = [0u8; 2];
        stream
            .read_exact(&mut head)
            .await
            .map_err(|e| ServeError::Transport(format!("frame read: {e}")))?;
        let fin = head[0] & 0x80 != 0;
        let opcode = head[0] & 0x0f;
        let masked = head[1] & 0x80 != 0;
        let mut len = u64::from(head[1] & 0x7f);
        if len == 126 {
            let mut ext = [0u8; 2];
            stream
                .read_exact(&mut ext)
                .await
                .map_err(|e| ServeError::Transport(format!("frame read: {e}")))?;
            len = u64::from(u16::from_be_bytes(ext));
        } else if len == 127 {
            let mut ext = [0u8; 8];
            stream
                .read_exact(&mut ext)
                .await
                .map_err(|e| ServeError::Transport(format!("frame read: {e}")))?;
            len = u64::from_be_bytes(ext);
        }
        if len > MAX_MESSAGE_BYTES as u64 {
            return Err(ServeError::Transport(format!(
                "frame of {len} bytes exceeds the {MAX_MESSAGE_BYTES}-byte message cap"
            )));
        }
        let mask_key = if masked {
            let mut key = [0u8; 4];
            stream
                .read_exact(&mut key)
                .await
                .map_err(|e| ServeError::Transport(format!("frame read: {e}")))?;
            Some(key)
        } else {
            None
        };
        let mut payload = vec![0u8; len as usize];
        stream
            .read_exact(&mut payload)
            .await
            .map_err(|e| ServeError::Transport(format!("frame read: {e}")))?;
        if let Some(key) = mask_key {
            for (i, byte) in payload.iter_mut().enumerate() {
                *byte ^= key[i % 4];
            }
        }
        match opcode {
            0x8 => {
                let _ = ws_write_frame(stream, 0x8, &[]).await;
                return Ok(None);
            }
            0x9 => {
                ws_write_frame(stream, 0xA, &payload).await?;
            }
            0xA => {}
            0x0 | 0x2 => {
                message.extend_from_slice(&payload);
                if message.len() > MAX_MESSAGE_BYTES {
                    return Err(ServeError::Transport(
                        "fragmented message exceeds the message cap".to_string(),
                    ));
                }
                if fin {
                    return Ok(Some(message));
                }
            }
            other => {
                return Err(ServeError::Transport(format!(
                    "unsupported websocket opcode {other} (binary only)"
                )));
            }
        }
    }
}

// ---------------------------------------------------------------------------
// The serve loop
// ---------------------------------------------------------------------------

/// Serve one connection: handshake, then exchange envelopes until
/// the client closes. Bridge failures answer with an `error`
/// envelope; malformed envelopes do the same and the connection
/// stays up for the next exchange.
async fn handle_connection(
    mut stream: TcpStream,
    config: Arc<ServeConfig>,
    bridge: Bridge,
) -> Result<(), ServeError> {
    let format = ws_handshake(&mut stream, &config).await?;
    let mut reassembler: Option<ChunkReassembler> = None;
    while let Some(message) = ws_read_message(&mut stream).await? {
        let envelope = match Envelope::decode(&message, format) {
            Ok(envelope) => envelope,
            Err(e) => {
                let error =
                    Envelope::single(EnvelopeKind::Error, 0, "", format!("{e}").into_bytes());
                ws_write_frame(&mut stream, 0x2, &error.encode(format)).await?;
                continue;
            }
        };
        let (alpn, seq, payload) = match envelope.kind {
            EnvelopeKind::Req => (
                envelope.alpn.clone(),
                envelope.seq,
                envelope.payload.clone(),
            ),
            EnvelopeKind::Chunk => {
                let active =
                    reassembler.get_or_insert_with(|| ChunkReassembler::new(&envelope.alpn));
                match active.push(&envelope) {
                    Ok(Some(payload)) => {
                        let base = active.base_seq().unwrap_or(envelope.seq);
                        let alpn = envelope.alpn.clone();
                        reassembler = None;
                        (alpn, base, payload)
                    }
                    Ok(None) => continue,
                    Err(e) => {
                        reassembler = None;
                        let error = Envelope::single(
                            EnvelopeKind::Error,
                            envelope.seq,
                            &envelope.alpn,
                            format!("{e}").into_bytes(),
                        );
                        ws_write_frame(&mut stream, 0x2, &error.encode(format)).await?;
                        continue;
                    }
                }
            }
            EnvelopeKind::Resp | EnvelopeKind::Error => {
                let error = Envelope::single(
                    EnvelopeKind::Error,
                    envelope.seq,
                    &envelope.alpn,
                    format!(
                        "unexpected {:?} envelope from a client (clients send req/chunk)",
                        envelope.kind
                    )
                    .into_bytes(),
                );
                ws_write_frame(&mut stream, 0x2, &error.encode(format)).await?;
                continue;
            }
        };
        match bridge(&alpn, format, &payload).await {
            Ok(response) => {
                for out in envelopes_for_payload(EnvelopeKind::Resp, &alpn, seq, &response) {
                    ws_write_frame(&mut stream, 0x2, &out.encode(format)).await?;
                }
            }
            Err(msg) => {
                let error = Envelope::single(EnvelopeKind::Error, seq, &alpn, msg.into_bytes());
                ws_write_frame(&mut stream, 0x2, &error.encode(format)).await?;
            }
        }
    }
    Ok(())
}

/// Bind and serve forever. The bind rule is validated before the
/// listener exists (§4.5).
pub async fn serve(config: ServeConfig, bridge: Bridge) -> Result<(), ServeError> {
    config.validate()?;
    let listener = TcpListener::bind(config.bind)
        .await
        .map_err(|e| ServeError::Bind(format!("{}: {e}", config.bind)))?;
    eprintln!(
        "fleet id serve-ws listening on {} (protocols: {CBOR_PROTOCOL}, {MSGPACK_PROTOCOL})",
        config.bind
    );
    serve_on(listener, config, bridge).await
}

/// Serve on an already-bound listener (the shape tests use to
/// learn the bound address; callers validate the config first).
pub async fn serve_on(
    listener: TcpListener,
    config: ServeConfig,
    bridge: Bridge,
) -> Result<(), ServeError> {
    let config = Arc::new(config);
    loop {
        let (stream, _peer) = listener
            .accept()
            .await
            .map_err(|e| ServeError::Transport(format!("accept: {e}")))?;
        let config = Arc::clone(&config);
        let bridge = Arc::clone(&bridge);
        tokio::spawn(async move {
            let _ = handle_connection(stream, config, bridge).await;
        });
    }
}

// ---------------------------------------------------------------------------
// CLI: `fleet id serve-ws`
// ---------------------------------------------------------------------------

const USAGE: &str = "\
usage: fleet id serve-ws [--bind <addr>] [--token <bearer> | --id-secret <hex>]

Conversion + WebSocket serving (design #10391 §4.5, slice 5): serves
the { v, seq, kind, alpn, payload, digest? } envelope over WebSocket
in CBOR (fleet-id-cbor-v1) or MessagePack (fleet-id-msgpack-v1),
negotiated per connection. Binds localhost by default; a non-local
bind requires a session credential (--token, or --id-secret to
derive the NodeId session token from a slice-1 session key), which
clients then present as 'Authorization: Bearer <token>'.

The built-in bridge is the fixture echo bridge; the live iroh dial
plugs into the same bridge seam.

exit codes: 0 ok, 2 usage, 3 non-local serve without credentials,
4 bind/serve failure.";

/// `fleet id serve-ws` — see [`USAGE`]. Returns a process exit
/// code; on success it serves until the process is stopped.
pub fn run(args: &[String]) -> i32 {
    let mut rest = args.iter();
    match rest.next().map(|s| s.as_str()) {
        Some("serve-ws") => {}
        _ => {
            eprintln!("error: id_serve::run routed a non serve-ws command\n\n{USAGE}");
            return EXIT_USAGE;
        }
    }
    let mut bind: SocketAddr = DEFAULT_BIND
        .parse()
        .expect("the default bind literal parses");
    let mut token: Option<String> = None;
    let mut id_secret: Option<String> = None;
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--help" | "-h" | "help" => {
                println!("{USAGE}");
                return 0;
            }
            "--bind" => match rest.next() {
                Some(value) => match value.parse::<SocketAddr>() {
                    Ok(addr) => bind = addr,
                    Err(e) => {
                        eprintln!("error: --bind value {value:?} is not an address: {e}");
                        return EXIT_USAGE;
                    }
                },
                None => {
                    eprintln!("error: --bind needs an address");
                    return EXIT_USAGE;
                }
            },
            "--token" => match rest.next() {
                Some(value) => token = Some(value.clone()),
                None => {
                    eprintln!("error: --token needs a value");
                    return EXIT_USAGE;
                }
            },
            "--id-secret" => match rest.next() {
                Some(value) => id_secret = Some(value.clone()),
                None => {
                    eprintln!("error: --id-secret needs a 64-hex-character value");
                    return EXIT_USAGE;
                }
            },
            other => {
                eprintln!("error: unknown argument '{other}'\n\n{USAGE}");
                return EXIT_USAGE;
            }
        }
    }
    if token.is_some() && id_secret.is_some() {
        eprintln!("error: pass only one of --token and --id-secret\n\n{USAGE}");
        return EXIT_USAGE;
    }
    if let Some(hex) = &id_secret {
        match crate::id_key::SessionKey::from_secret_hex(hex) {
            Ok(key) => token = Some(session_token(&key.node_id_hex())),
            Err(e) => {
                eprintln!("error: --id-secret rejected: {e}");
                return EXIT_USAGE;
            }
        }
    }
    let config = ServeConfig { bind, token };
    if let Err(e) = config.validate() {
        eprintln!("error: {e}");
        return EXIT_NO_CREDENTIALS;
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            eprintln!("error: cannot start tokio runtime: {e}");
            return EXIT_SERVE;
        }
    };
    match runtime.block_on(serve(config, echo_bridge())) {
        Ok(()) => 0,
        Err(ServeError::NonLocalWithoutCredentials(_)) => EXIT_NO_CREDENTIALS,
        Err(e) => {
            eprintln!("error: {e}");
            EXIT_SERVE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};
    use std::time::Duration;

    /// Stand-in for the meta protocol's value shapes, mirroring
    /// slice 3's fixture: enough to prove conversion + serving
    /// without linking `id-sync`.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum Probe {
        List,
        Find { query: String, prefer_name: bool },
        GetTags { subject: Option<Vec<u8>> },
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum ProbeResp {
        Entries(Vec<String>),
        Tags(Vec<(Vec<u8>, Vec<u8>)>),
    }

    /// Same-shape swap, opposite order (slice 3's fixture): the
    /// skew case §4.4's by-name conversion exists for.
    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum OrderA {
        List,
        Ping,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    enum OrderB {
        Ping,
        List,
    }

    #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
    struct Blob {
        data: String,
    }

    #[test]
    fn codec_golden_bytes() {
        let value = CodecValue::Map(vec![("a".to_string(), CodecValue::U64(1))]);
        assert_eq!(
            encode_value(&value, WireFormat::Cbor),
            vec![0xa1, 0x61, 0x61, 0x01]
        );
        assert_eq!(
            encode_value(&value, WireFormat::Msgpack),
            vec![0x81, 0xa1, 0x61, 0x01]
        );
        // CBOR half-float 1.0 (0xf9 0x3c00) decodes.
        assert_eq!(
            decode_value(&[0xf9, 0x3c, 0x00], WireFormat::Cbor).unwrap(),
            CodecValue::F64(1.0)
        );
        // Trailing bytes are refused by both codecs.
        assert!(decode_value(&[0xa1, 0x61, 0x61, 0x01, 0x00], WireFormat::Cbor).is_err());
        assert!(decode_value(&[0x81, 0xa1, 0x61, 0x01, 0xc0], WireFormat::Msgpack).is_err());
    }

    #[test]
    fn envelope_round_trips_both_formats() {
        for format in [WireFormat::Cbor, WireFormat::Msgpack] {
            let plain = Envelope::single(EnvelopeKind::Req, 7, "/iroh-meta/3", b"payload".to_vec());
            let back = Envelope::decode(&plain.encode(format), format).unwrap();
            assert_eq!(back, plain);
            let digested = Envelope {
                seq: 8,
                kind: EnvelopeKind::Chunk,
                alpn: "/iroh-meta/3".to_string(),
                payload: vec![1, 2, 3],
                digest: Some([9u8; 32]),
            };
            let back = Envelope::decode(&digested.encode(format), format).unwrap();
            assert_eq!(back, digested);
        }
    }

    #[test]
    fn envelope_rejects_wrong_version_and_kind() {
        let mut entries = vec![
            ("v".to_string(), CodecValue::U64(2)),
            ("seq".to_string(), CodecValue::U64(0)),
            ("kind".to_string(), CodecValue::Str("req".to_string())),
            (
                "alpn".to_string(),
                CodecValue::Str("/iroh-meta/3".to_string()),
            ),
            ("payload".to_string(), CodecValue::Bytes(vec![])),
        ];
        let bytes = encode_value(&CodecValue::Map(entries.clone()), WireFormat::Cbor);
        assert!(matches!(
            Envelope::decode(&bytes, WireFormat::Cbor),
            Err(ConvertError::Envelope(_))
        ));
        entries[0].1 = CodecValue::U64(1);
        entries[2].1 = CodecValue::Str("event".to_string());
        let bytes = encode_value(&CodecValue::Map(entries), WireFormat::Cbor);
        assert!(matches!(
            Envelope::decode(&bytes, WireFormat::Cbor),
            Err(ConvertError::Envelope(_))
        ));
    }

    #[test]
    fn session_events_ride_as_resp_by_decision() {
        // The §4.5 open question, decided in this slice and
        // recorded on #14229: no new kind, no new fields.
        assert_eq!(SESSION_EVENT_KIND, EnvelopeKind::Resp);
        assert_eq!(SESSION_EVENT_KIND.as_str(), "resp");
    }

    #[test]
    fn golden_session_event_envelope_round_trip() {
        // Golden conformance for #14229 §6.6 / #16326: a pass-3
        // §4.2 session event, serialized exactly as the REST side's
        // socket carries it (`socket_event` in id-web's
        // sessions.rs: `{"id": seq, "event": name, "data": data}`,
        // where `data` is `RunEventData::to_json`'s object), crosses
        // the fleet §4.5 socket path and back byte-identical.
        //
        // Oracle: the §4.5 framing bytes as landed in this module
        // (`Envelope::encode`/`decode`). The four events are one
        // whole session log in order: the opening `app` session
        // event, a utf8 `out` chunk, a base64 `err` chunk (bytes
        // 0xff 0x00 are not valid UTF-8), and the closing `exit`.
        //
        // Neither design assigns session events their own ALPN;
        // the envelope carries the caller's ALPN opaquely, so this
        // test pins pass-through with a representative value and
        // assigns none.
        const GOLDEN_ALPN: &str = "/id-session/1";
        let golden_events: [(u64, &str); 4] = [
            (
                1,
                r#"{"id":1,"event":"app","data":{"session":"ses_16326_golden"}}"#,
            ),
            (
                2,
                r#"{"id":2,"event":"out","data":{"encoding":"utf8","data":"hello\n"}}"#,
            ),
            (
                3,
                r#"{"id":3,"event":"err","data":{"encoding":"base64","data":"/wA="}}"#,
            ),
            (
                4,
                r#"{"id":4,"event":"exit","data":{"exit_code":0,"signal":null,"timed_out":false,"stdout_truncated":false,"stderr_truncated":false}}"#,
            ),
        ];
        for format in [WireFormat::Cbor, WireFormat::Msgpack] {
            for (seq, event_json) in golden_events {
                let payload = event_json.as_bytes().to_vec();
                let sent = Envelope::single(SESSION_EVENT_KIND, seq, GOLDEN_ALPN, payload.clone());
                let wire = sent.encode(format);

                // The wire form is exactly the §4.5 map — the five
                // named fields, no added/renamed/forked fields.
                let CodecValue::Map(entries) = decode_value(&wire, format).unwrap() else {
                    panic!("envelope wire form is not a map");
                };
                let field_names: Vec<&str> =
                    entries.iter().map(|(name, _)| name.as_str()).collect();
                assert_eq!(field_names, ["v", "seq", "kind", "alpn", "payload"]);
                let fields: BTreeMap<String, CodecValue> = entries.into_iter().collect();
                assert_eq!(fields["v"], CodecValue::U64(ENVELOPE_VERSION));
                assert_eq!(fields["seq"], CodecValue::U64(seq));
                assert_eq!(fields["kind"], CodecValue::Str("resp".to_string()));
                assert_eq!(fields["payload"], CodecValue::Bytes(payload.clone()));

                // Through the socket path and back: the decoded
                // envelope is the sent one, its payload is the
                // REST-side event bytes unchanged, and re-encoding
                // the decoded envelope reproduces the exact wire
                // bytes (byte-identical in both directions).
                let received = Envelope::decode(&wire, format).unwrap();
                assert_eq!(received, sent);
                assert_eq!(received.kind, SESSION_EVENT_KIND);
                assert_eq!(received.seq, seq);
                assert_eq!(received.payload, payload);
                assert_eq!(received.encode(format), wire);
            }
        }
    }

    #[test]
    fn golden_session_event_chunked_round_trip() {
        // The streaming leg of the same #14229 §6.6 golden check:
        // a session `out` event larger than the conversion
        // threshold crosses as chunk envelopes and reassembles to
        // the exact REST-side event bytes, digests verified by the
        // reassembler.
        let big_data = "x".repeat(CONVERSION_THRESHOLD + 4096);
        let event_json =
            format!(r#"{{"id":2,"event":"out","data":{{"encoding":"utf8","data":"{big_data}"}}}}"#);
        let payload = event_json.into_bytes();
        assert!(needs_chunking(payload.len()));
        let envelopes = envelopes_for_payload(SESSION_EVENT_KIND, "/id-session/1", 2, &payload);
        assert!(envelopes.len() > 1);
        let mut reassembler = ChunkReassembler::new("/id-session/1");
        let mut reassembled = None;
        for env in &envelopes {
            assert_eq!(env.kind, EnvelopeKind::Chunk);
            if let Some(done) = reassembler.push(env).unwrap() {
                reassembled = Some(done);
            }
        }
        assert_eq!(reassembled.unwrap(), payload);
    }

    #[test]
    fn conversion_round_trip_and_variant_by_name_skew() {
        for value in [
            Probe::List,
            Probe::Find {
                query: "config".to_string(),
                prefer_name: true,
            },
            Probe::GetTags {
                subject: Some(b"report.txt".to_vec()),
            },
        ] {
            let postcard = postcard::to_allocvec(&value).unwrap();
            let cbor = postcard_to_cbor::<Probe>(&postcard).unwrap();
            assert_eq!(cbor_to_postcard::<Probe>(&cbor).unwrap(), postcard);
            let msgpack = postcard_to_msgpack::<Probe>(&postcard).unwrap();
            assert_eq!(msgpack_to_postcard::<Probe>(&msgpack).unwrap(), postcard);
        }
        // Skew fixture: OrderA::List is postcard byte [0], which
        // positional decode reads as OrderB::Ping (slice 3 proved
        // the corruption). Through by-name conversion the same
        // value lands as OrderB::List — the right variant.
        let postcard = postcard::to_allocvec(&OrderA::List).unwrap();
        let as_b: OrderB = postcard::from_bytes(&postcard).unwrap();
        assert_eq!(as_b, OrderB::Ping, "positional decode corrupts (control)");
        let cbor = postcard_to_cbor::<OrderA>(&postcard).unwrap();
        let fixed = cbor_to_postcard::<OrderB>(&cbor).unwrap();
        let as_b_fixed: OrderB = crate::id_postcard::decode_strict(&fixed).unwrap();
        assert_eq!(as_b_fixed, OrderB::List, "by-name conversion survives skew");
        let msgpack = postcard_to_msgpack::<OrderA>(&postcard).unwrap();
        let fixed = msgpack_to_postcard::<OrderB>(&msgpack).unwrap();
        let as_b_fixed: OrderB = crate::id_postcard::decode_strict(&fixed).unwrap();
        assert_eq!(as_b_fixed, OrderB::List);
    }

    #[test]
    fn conversion_trigger_conditions_match_design() {
        // §4.4: browser consumer, OR over-threshold, OR streaming.
        assert!(conversion_triggered(true, 3, false));
        assert!(conversion_triggered(false, CONVERSION_THRESHOLD + 1, false));
        assert!(conversion_triggered(false, 3, true));
        assert!(!conversion_triggered(false, 3, false));
        assert!(!conversion_triggered(false, CONVERSION_THRESHOLD, false));
        assert!(needs_chunking(CONVERSION_THRESHOLD + 1));
        assert!(!needs_chunking(CONVERSION_THRESHOLD));
    }

    #[test]
    fn chunking_over_1mib_verifies_per_chunk_and_whole_digests() {
        // >1 MiB deterministic payload (the acceptance shape).
        let payload: Vec<u8> = (0..(CONVERSION_THRESHOLD + 12345))
            .map(|i| (i % 251) as u8)
            .collect();
        let envelopes = chunk_envelopes("/iroh-meta/3", 40, &payload);
        let data_chunks = payload.len().div_ceil(CHUNK_SIZE);
        assert_eq!(envelopes.len(), data_chunks + 1, "chunks + terminator");
        let mut reassembler = ChunkReassembler::new("/iroh-meta/3");
        let mut completed = None;
        for env in &envelopes {
            if let Some(done) = reassembler.push(env).unwrap() {
                completed = Some(done);
            }
        }
        assert_eq!(reassembler.base_seq(), Some(40));
        assert_eq!(completed.expect("terminator completes"), payload);

        // A tampered chunk fails its per-chunk digest.
        let mut tampered = chunk_envelopes("/iroh-meta/3", 0, &payload);
        tampered[1].payload[0] ^= 0xff;
        let mut reassembler = ChunkReassembler::new("/iroh-meta/3");
        reassembler.push(&tampered[0]).unwrap();
        assert!(matches!(
            reassembler.push(&tampered[1]),
            Err(ConvertError::Chunk(_))
        ));

        // A skipped sequence fails contiguity.
        let envelopes = chunk_envelopes("/iroh-meta/3", 0, &payload);
        let mut reassembler = ChunkReassembler::new("/iroh-meta/3");
        reassembler.push(&envelopes[0]).unwrap();
        assert!(matches!(
            reassembler.push(&envelopes[2]),
            Err(ConvertError::Chunk(_))
        ));

        // A tampered terminator fails the whole-payload digest.
        let mut envelopes = chunk_envelopes("/iroh-meta/3", 0, &payload);
        let last = envelopes.len() - 1;
        envelopes[last].digest = Some([0u8; 32]);
        let mut reassembler = ChunkReassembler::new("/iroh-meta/3");
        let mut result = Ok(None);
        for env in &envelopes {
            result = reassembler.push(env);
            if result.is_err() {
                break;
            }
        }
        assert!(matches!(result, Err(ConvertError::Chunk(_))));
    }

    #[test]
    fn bind_rules_refuse_anonymous_non_local() {
        assert!(ServeConfig::localhost(0).validate().is_ok());
        let non_local = ServeConfig {
            bind: "0.0.0.0:9473".parse().unwrap(),
            token: None,
        };
        assert!(matches!(
            non_local.validate(),
            Err(ServeError::NonLocalWithoutCredentials(_))
        ));
        let credentialed = ServeConfig {
            bind: "0.0.0.0:9473".parse().unwrap(),
            token: Some("tok".to_string()),
        };
        assert!(credentialed.validate().is_ok());
        // The token construction is deterministic per NodeId and
        // differs across NodeIds.
        assert_eq!(
            session_token("ab".repeat(32).as_str()),
            session_token("ab".repeat(32).as_str())
        );
        assert_ne!(
            session_token("ab".repeat(32).as_str()),
            session_token("cd".repeat(32).as_str())
        );
    }

    #[test]
    fn ws_accept_key_matches_rfc6455_example() {
        // RFC 6455 §1.3 worked example.
        assert_eq!(
            ws_accept_key("dGhlIHNhbXBsZSBub25jZQ=="),
            "s3pPLMBiTxaQ9kYGzzhZRbK+xOo="
        );
    }

    // --- Live socket fixtures -------------------------------------

    /// The postcard-side fixture responder: strict-decodes a
    /// Probe and answers as the test ID node would.
    fn probe_responder(postcard_req: &[u8]) -> Result<Vec<u8>, String> {
        let req: Probe =
            crate::id_postcard::decode_strict(postcard_req).map_err(|e| e.to_string())?;
        let resp = match req {
            Probe::List => ProbeResp::Entries(vec!["alpha".to_string(), "beta".to_string()]),
            Probe::Find { query, .. } => ProbeResp::Entries(vec![query]),
            Probe::GetTags { subject } => {
                ProbeResp::Tags(vec![(subject.unwrap_or_default(), b"seen".to_vec())])
            }
        };
        postcard::to_allocvec(&resp).map_err(|e| e.to_string())
    }

    /// A bridge that crosses converted→postcard, runs the fixture
    /// responder, and crosses back — the §4.5 edge shape.
    fn probe_bridge() -> Bridge {
        Arc::new(|_alpn: &str, format: WireFormat, payload: &[u8]| {
            let payload = payload.to_vec();
            Box::pin(async move {
                let postcard_req = match format {
                    WireFormat::Cbor => cbor_to_postcard::<Probe>(&payload),
                    WireFormat::Msgpack => msgpack_to_postcard::<Probe>(&payload),
                }
                .map_err(|e| e.to_string())?;
                let postcard_resp = probe_responder(&postcard_req)?;
                match format {
                    WireFormat::Cbor => postcard_to_cbor::<ProbeResp>(&postcard_resp),
                    WireFormat::Msgpack => postcard_to_msgpack::<ProbeResp>(&postcard_resp),
                }
                .map_err(|e| e.to_string())
            })
        })
    }

    fn blob_bridge() -> Bridge {
        Arc::new(|_alpn: &str, format: WireFormat, payload: &[u8]| {
            let payload = payload.to_vec();
            Box::pin(async move {
                let postcard_req = match format {
                    WireFormat::Cbor => cbor_to_postcard::<Blob>(&payload),
                    WireFormat::Msgpack => msgpack_to_postcard::<Blob>(&payload),
                }
                .map_err(|e| e.to_string())?;
                let blob: Blob =
                    crate::id_postcard::decode_strict(&postcard_req).map_err(|e| e.to_string())?;
                let postcard_resp = postcard::to_allocvec(&blob).map_err(|e| e.to_string())?;
                match format {
                    WireFormat::Cbor => postcard_to_cbor::<Blob>(&postcard_resp),
                    WireFormat::Msgpack => postcard_to_msgpack::<Blob>(&postcard_resp),
                }
                .map_err(|e| e.to_string())
            })
        })
    }

    async fn start_server(config: ServeConfig, bridge: Bridge) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let _ = serve_on(listener, config, bridge).await;
        });
        addr
    }

    /// A hand-rolled WebSocket client speaking only the envelope
    /// in the negotiated converted form — the browser-fixture
    /// shape: no postcard knowledge anywhere in it.
    struct TestClient {
        stream: TcpStream,
        format: WireFormat,
    }

    async fn read_head(stream: &mut TcpStream) -> String {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        loop {
            stream.read_exact(&mut byte).await.unwrap();
            head.push(byte[0]);
            if head.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        String::from_utf8(head).unwrap()
    }

    impl TestClient {
        async fn connect(
            addr: SocketAddr,
            protocol: &str,
            token: Option<&str>,
        ) -> Result<Self, String> {
            let mut stream = TcpStream::connect(addr).await.map_err(|e| e.to_string())?;
            let auth = token
                .map(|t| format!("Authorization: Bearer {t}\r\n"))
                .unwrap_or_default();
            let request = format!(
                "GET / HTTP/1.1\r\nHost: {addr}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\nSec-WebSocket-Protocol: {protocol}\r\n{auth}\r\n"
            );
            stream
                .write_all(request.as_bytes())
                .await
                .map_err(|e| e.to_string())?;
            let head = read_head(&mut stream).await;
            if !head.starts_with("HTTP/1.1 101") {
                return Err(head.lines().next().unwrap_or("").to_string());
            }
            let format = WireFormat::from_protocol(protocol)
                .ok_or_else(|| "test bug: unsupported protocol".to_string())?;
            Ok(TestClient { stream, format })
        }

        async fn send(&mut self, payload: &[u8]) {
            let mask = [0x12u8, 0x34, 0x56, 0x78];
            let mut head = vec![0x82u8];
            if payload.len() < 126 {
                head.push(0x80 | payload.len() as u8);
            } else if payload.len() <= usize::from(u16::MAX) {
                head.push(0x80 | 126);
                head.extend_from_slice(&(payload.len() as u16).to_be_bytes());
            } else {
                head.push(0x80 | 127);
                head.extend_from_slice(&(payload.len() as u64).to_be_bytes());
            }
            head.extend_from_slice(&mask);
            self.stream.write_all(&head).await.unwrap();
            let masked: Vec<u8> = payload
                .iter()
                .enumerate()
                .map(|(i, b)| b ^ mask[i % 4])
                .collect();
            self.stream.write_all(&masked).await.unwrap();
        }

        async fn recv(&mut self) -> Vec<u8> {
            let mut head = [0u8; 2];
            self.stream.read_exact(&mut head).await.unwrap();
            let opcode = head[0] & 0x0f;
            let mut len = u64::from(head[1] & 0x7f);
            if len == 126 {
                let mut ext = [0u8; 2];
                self.stream.read_exact(&mut ext).await.unwrap();
                len = u64::from(u16::from_be_bytes(ext));
            } else if len == 127 {
                let mut ext = [0u8; 8];
                self.stream.read_exact(&mut ext).await.unwrap();
                len = u64::from_be_bytes(ext);
            }
            assert_ne!(opcode, 0x8, "server closed the connection");
            let mut payload = vec![0u8; len as usize];
            self.stream.read_exact(&mut payload).await.unwrap();
            payload
        }

        /// Send one logical payload (single or chunked envelopes)
        /// and read the whole response payload back, verifying
        /// chunk digests through the reassembler.
        async fn exchange(&mut self, alpn: &str, payload: &[u8]) -> Vec<u8> {
            for env in envelopes_for_payload(EnvelopeKind::Req, alpn, 5, payload) {
                self.send(&env.encode(self.format)).await;
            }
            let mut reassembler: Option<ChunkReassembler> = None;
            loop {
                let message = self.recv().await;
                let env = Envelope::decode(&message, self.format).unwrap();
                match env.kind {
                    EnvelopeKind::Resp => return env.payload,
                    EnvelopeKind::Chunk => {
                        let active = reassembler.get_or_insert_with(|| ChunkReassembler::new(alpn));
                        if let Some(done) = active.push(&env).unwrap() {
                            return done;
                        }
                    }
                    other => panic!("unexpected response kind {other:?}"),
                }
            }
        }
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn websocket_value_exchange_with_no_postcard_client() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let addr = start_server(ServeConfig::localhost(0), probe_bridge()).await;
            let mut client = TestClient::connect(addr, CBOR_PROTOCOL, None)
                .await
                .expect("handshake negotiates cbor");
            // The client builds its request as a plain JSON-shaped
            // value encoded as CBOR — no postcard anywhere.
            let request_value = serde_json::json!({
                "Find": { "query": "config", "prefer_name": true }
            });
            let request_bytes = encode_value(&json_to_codec(&request_value), WireFormat::Cbor);
            let response_bytes = client.exchange("/iroh-meta/3", &request_bytes).await;
            let response_value =
                codec_to_json(&decode_value(&response_bytes, WireFormat::Cbor).unwrap()).unwrap();
            assert_eq!(response_value, serde_json::json!({ "Entries": ["config"] }));

            // MessagePack negotiates and exchanges the same way.
            let mut client = TestClient::connect(addr, MSGPACK_PROTOCOL, None)
                .await
                .expect("handshake negotiates msgpack");
            let request_bytes = encode_value(&json_to_codec(&request_value), WireFormat::Msgpack);
            let response_bytes = client.exchange("/iroh-meta/3", &request_bytes).await;
            let response_value =
                codec_to_json(&decode_value(&response_bytes, WireFormat::Msgpack).unwrap())
                    .unwrap();
            assert_eq!(response_value, serde_json::json!({ "Entries": ["config"] }));
        })
        .await
        .expect("value exchange completes within timeout");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn websocket_chunked_transfer_over_1mib() {
        tokio::time::timeout(Duration::from_secs(30), async {
            let addr = start_server(ServeConfig::localhost(0), blob_bridge()).await;
            let mut client = TestClient::connect(addr, CBOR_PROTOCOL, None)
                .await
                .expect("handshake");
            let data = "x".repeat(CONVERSION_THRESHOLD + 4096);
            let request_value = serde_json::json!({ "data": data });
            let request_bytes = encode_value(&json_to_codec(&request_value), WireFormat::Cbor);
            assert!(needs_chunking(request_bytes.len()));
            // exchange() chunks the request, and reassembles the
            // chunked response verifying every digest.
            let response_bytes = client.exchange("/iroh-meta/3", &request_bytes).await;
            let response_value =
                codec_to_json(&decode_value(&response_bytes, WireFormat::Cbor).unwrap()).unwrap();
            assert_eq!(response_value, request_value);
        })
        .await
        .expect("chunked transfer completes within timeout");
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn websocket_rejects_unknown_protocol_and_missing_credential() {
        tokio::time::timeout(Duration::from_secs(20), async {
            let addr = start_server(ServeConfig::localhost(0), probe_bridge()).await;
            let refused = TestClient::connect(addr, "fleet-id-xml-v9", None).await;
            match refused {
                Err(head) => {
                    assert!(head.contains("400"), "expected a 400 refusal, got: {head}")
                }
                Ok(_) => panic!("unknown subprotocol was accepted"),
            }

            let token = session_token(&"ab".repeat(32));
            let config = ServeConfig {
                bind: "127.0.0.1:0".parse().unwrap(),
                token: Some(token.clone()),
            };
            let addr = start_server(config, probe_bridge()).await;
            let refused = TestClient::connect(addr, CBOR_PROTOCOL, None).await;
            match refused {
                Err(head) => {
                    assert!(head.contains("401"), "expected a 401 refusal, got: {head}")
                }
                Ok(_) => panic!("missing credential was accepted"),
            }
            let refused = TestClient::connect(addr, CBOR_PROTOCOL, Some("wrong")).await;
            match refused {
                Err(head) => {
                    assert!(head.contains("401"), "expected a 401 refusal, got: {head}")
                }
                Ok(_) => panic!("wrong credential was accepted"),
            }
            let mut client = TestClient::connect(addr, CBOR_PROTOCOL, Some(&token))
                .await
                .expect("the session credential is accepted");
            let request_value = serde_json::json!("List");
            let request_bytes = encode_value(&json_to_codec(&request_value), WireFormat::Cbor);
            let response_bytes = client.exchange("/iroh-meta/3", &request_bytes).await;
            let response_value =
                codec_to_json(&decode_value(&response_bytes, WireFormat::Cbor).unwrap()).unwrap();
            assert_eq!(
                response_value,
                serde_json::json!({ "Entries": ["alpha", "beta"] })
            );
        })
        .await
        .expect("refusal checks complete within timeout");
    }
}
