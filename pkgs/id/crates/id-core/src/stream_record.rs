//! Addressable live streams: the signed stream descriptor, chained
//! signed chunk records and the finite lifecycle state machine (slice
//! A9123-stream-records, issue #10246; parent #9123; design
//! `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/versioned-design.md`
//! section "#9123 — addressable live streams", A-9123.1, statements
//! S4.74–S4.82).
//!
//! A [`StreamDescriptor`] is stored at an array address
//! ([`StreamDescriptor::address`], over the A9118 array codec) and
//! carries `{stream_id, namespace_id, media_type, creator, created_at,
//! state, current_seq, final_ref?, generation}`. States are `open`,
//! `paused`, `finalizing`, `completed`, `aborted`, `expired`; the last
//! three are terminal and exactly one terminal record ever lands.
//!
//! Chunks are immutable content-addressed objects. Each signed
//! [`ChunkRecord`] binds the stream ID, the tenant scope (the
//! authenticated scope context of the #9083 tenancy contract — consumed
//! here, not defined), its sequence, the previous record ID, the
//! payload ID, media metadata and the signer. The payload is bound by
//! its content identity ([`ContentId`], which carries the hash
//! profile): A9114's `PayloadId` has no wrap constructor, so the chunk
//! carries `PayloadId::content_id` and [`ChunkRecord::unsigned`]
//! accepts a `PayloadId` directly. A record's ID is the raw-BLAKE3-256
//! hash of its complete canonical signed encoding, computed after
//! signing and never written into the bytes it hashes — the
//! envelope-ID discipline of A9114.
//!
//! Ordering is strict: no unordered "last write wins". A chunk lands
//! only at exactly `current_seq` with `prev_record_id` equal to the
//! chain head. Re-delivering an already-accepted chunk is idempotent
//! only for the same `(stream, sequence, record ID)`; a differing
//! record at a covered sequence is conflict/equivocation evidence
//! ([`StreamError::SequenceConflict`]). Multiwriter streams name a
//! namespace-configured sequencer authority in the machine; without
//! one, only the creator may sign chunks.
//!
//! Completion CASes `open -> finalizing -> completed` with a signed
//! [`TerminalRecord`] carrying the final content reference. Aborts and
//! expiry are also signed terminal records and never claim final
//! content. A finished stream resolves via the typed
//! [`Resolution::Completed`]; following it is client-selected, never
//! an implicit redirect.
//!
//! The crate-root export belongs to slice 9110-02; until that lands
//! the integration test includes this module and its dependencies by
//! path.

use std::fmt;

use ed25519_dalek::{Signature, Signer as _, SigningKey, Verifier as _, VerifyingKey};

use crate::array_address::ArrayAddress;
use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile, IdShapeError};
use crate::signed_envelope::{MAX_LABEL_BYTES, PayloadId};

/// The stream record format version this module writes and reads.
pub const STREAM_FORMAT_VERSION: u64 = 1;

/// Fixed prefix of every chunk signature preimage, ending in a NUL so
/// no domain text can extend it.
pub const CHUNK_DOMAIN_PREFIX: &[u8] = b"id/stream-chunk/v1\0";

/// Fixed prefix of every terminal-record signature preimage.
pub const TERMINAL_DOMAIN_PREFIX: &[u8] = b"id/stream-terminal/v1\0";

/// Slot prefix of a stream descriptor's array address.
pub const STREAM_ADDRESS_TAG: &[u8] = b"id-stream";

/// Identity of a stream: a profiled content ID over the genesis
/// descriptor fields, so the ID commits to namespace, creator and
/// creation time without carrying them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StreamId(ContentId);

impl StreamId {
    /// Derives the stream ID for a genesis descriptor.
    ///
    /// # Errors
    ///
    /// The hash profile's own rejection (never for raw BLAKE3).
    pub fn derive(
        namespace_id: &NamespaceId,
        media_type: &str,
        creator: &[u8; DIGEST_BYTES],
        created_at: u64,
    ) -> Result<Self, StreamError> {
        let genesis = map(vec![
            ("v", Value::UInt(STREAM_FORMAT_VERSION)),
            (
                "namespace_id",
                Value::Bytes(namespace_id.as_bytes().to_vec()),
            ),
            ("media_type", text(media_type)),
            ("creator", Value::Bytes(creator.to_vec())),
            ("created_at", Value::UInt(created_at)),
        ]);
        let bytes = encode_canonical(&genesis)?;
        Ok(Self(ContentId::of_bytes(
            HashProfile::RawBlake3_256,
            &bytes,
        )?))
    }

    /// Wraps a stream ID read from a signed reference.
    #[must_use]
    pub const fn from_content_id(id: ContentId) -> Self {
        Self(id)
    }

    /// The underlying content ID.
    #[must_use]
    pub const fn content_id(&self) -> &ContentId {
        &self.0
    }
}

/// Identity of one signed chunk record: the raw-BLAKE3-256 content ID
/// of the record's complete canonical signed encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkRecordId(ContentId);

impl ChunkRecordId {
    /// Wraps a record ID read from a signed reference.
    #[must_use]
    pub const fn from_content_id(id: ContentId) -> Self {
        Self(id)
    }

    /// The underlying content ID.
    #[must_use]
    pub const fn content_id(&self) -> &ContentId {
        &self.0
    }
}

/// Identity of one signed terminal record, computed like a chunk
/// record ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TerminalRecordId(ContentId);

impl TerminalRecordId {
    /// Wraps a record ID read from a signed reference.
    #[must_use]
    pub const fn from_content_id(id: ContentId) -> Self {
        Self(id)
    }

    /// The underlying content ID.
    #[must_use]
    pub const fn content_id(&self) -> &ContentId {
        &self.0
    }
}

/// A namespace (space) address: 32 owner-assigned bytes, stable
/// across every record written into that namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NamespaceId([u8; DIGEST_BYTES]);

impl NamespaceId {
    /// A namespace address from its assigned bytes.
    #[must_use]
    pub const fn new(bytes: [u8; DIGEST_BYTES]) -> Self {
        Self(bytes)
    }

    /// The address bytes.
    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; DIGEST_BYTES] {
        &self.0
    }
}

/// The finite lifecycle states of a stream. `Completed`, `Aborted`
/// and `Expired` are terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum StreamState {
    /// Accepting chunks.
    Open,
    /// Suspended by the creator; chunks are rejected until resumed.
    Paused,
    /// Completion CAS in progress; no further chunks.
    Finalizing,
    /// Terminal: completed with a final content reference.
    Completed,
    /// Terminal: aborted; claims no final content.
    Aborted,
    /// Terminal: expired; claims no final content.
    Expired,
}

impl StreamState {
    /// The state token used in canonical encodings.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Open => "open",
            Self::Paused => "paused",
            Self::Finalizing => "finalizing",
            Self::Completed => "completed",
            Self::Aborted => "aborted",
            Self::Expired => "expired",
        }
    }

    /// Reads a state token.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "open" => Some(Self::Open),
            "paused" => Some(Self::Paused),
            "finalizing" => Some(Self::Finalizing),
            "completed" => Some(Self::Completed),
            "aborted" => Some(Self::Aborted),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }

    /// Whether this state is terminal (no outgoing transitions).
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::Aborted | Self::Expired)
    }
}

/// The signed stream descriptor (A-9123.1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamDescriptor {
    /// Stream identity.
    pub stream_id: StreamId,
    /// Namespace the stream lives in.
    pub namespace_id: NamespaceId,
    /// Media type shared by the stream's chunks.
    pub media_type: String,
    /// Creator's Ed25519 public key.
    pub creator: [u8; DIGEST_BYTES],
    /// Creation time (logical or Unix seconds; the chain never orders
    /// by it — resume cursors are sequence + record ID).
    pub created_at: u64,
    /// Current lifecycle state.
    pub state: StreamState,
    /// Number of chunks accepted so far; the next chunk's sequence.
    pub current_seq: u64,
    /// Final content/manifest reference; set only by completion.
    pub final_ref: Option<ContentId>,
    /// Descriptor generation; bumped on every state transition.
    pub generation: u64,
}

impl StreamDescriptor {
    /// A fresh descriptor in the `open` state.
    ///
    /// # Errors
    ///
    /// [`StreamError::Malformed`] when the media type is empty or over
    /// the label bound, or the stream ID derivation fails.
    pub fn open(
        namespace_id: NamespaceId,
        media_type: &str,
        creator: [u8; DIGEST_BYTES],
        created_at: u64,
    ) -> Result<Self, StreamError> {
        check_label("media_type", media_type)?;
        let stream_id = StreamId::derive(&namespace_id, media_type, &creator, created_at)?;
        Ok(Self {
            stream_id,
            namespace_id,
            media_type: media_type.to_owned(),
            creator,
            created_at,
            state: StreamState::Open,
            current_seq: 0,
            final_ref: None,
            generation: 0,
        })
    }

    /// The array address this descriptor is stored at: the stream tag
    /// slot followed by the stream ID digest slot.
    #[must_use]
    pub fn address(&self) -> ArrayAddress {
        ArrayAddress::new(vec![
            Some(STREAM_ADDRESS_TAG.to_vec()),
            Some(self.stream_id.content_id().digest().to_vec()),
        ])
    }

    /// The descriptor as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`StreamError::Malformed`] on an out-of-bound label.
    pub fn to_value(&self) -> Result<Value, StreamError> {
        check_label("media_type", &self.media_type)?;
        let mut entries = vec![
            ("v", Value::UInt(STREAM_FORMAT_VERSION)),
            ("stream_id", content_id_value(self.stream_id.content_id())),
            (
                "namespace_id",
                Value::Bytes(self.namespace_id.as_bytes().to_vec()),
            ),
            ("media_type", text(&self.media_type)),
            ("creator", Value::Bytes(self.creator.to_vec())),
            ("created_at", Value::UInt(self.created_at)),
            ("state", text(self.state.token())),
            ("current_seq", Value::UInt(self.current_seq)),
            ("generation", Value::UInt(self.generation)),
        ];
        if let Some(final_ref) = &self.final_ref {
            entries.push(("final_ref", content_id_value(final_ref)));
        }
        Ok(map(entries))
    }

    /// `canonical_cbor(StreamDescriptor)`.
    ///
    /// # Errors
    ///
    /// As [`StreamDescriptor::to_value`], plus the encoder's rejection.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, StreamError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a descriptor from canonical bytes.
    ///
    /// # Errors
    ///
    /// [`StreamError`] for malformed bytes, an unknown field, a
    /// missing or mistyped field, or an unknown state token.
    pub fn decode(bytes: &[u8]) -> Result<Self, StreamError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// Reads a descriptor from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// As [`StreamDescriptor::decode`].
    pub fn from_value(value: &Value) -> Result<Self, StreamError> {
        let fields = Fields::new(
            value,
            &[
                "v",
                "stream_id",
                "namespace_id",
                "media_type",
                "creator",
                "created_at",
                "state",
                "current_seq",
                "generation",
                "final_ref",
            ],
        )?;
        check_version(fields.uint("v")?)?;
        let state_token = fields.text("state")?;
        let state = StreamState::from_token(&state_token).ok_or_else(|| {
            StreamError::malformed(format!("unknown stream state {state_token:?}"))
        })?;
        let final_ref = fields
            .optional("final_ref")
            .map(|value| content_id_from_value(value))
            .transpose()?;
        if state == StreamState::Completed && final_ref.is_none() {
            return Err(StreamError::malformed(
                "a completed descriptor must carry its final reference",
            ));
        }
        if state != StreamState::Completed && final_ref.is_some() {
            return Err(StreamError::malformed(
                "only a completed descriptor may carry a final reference",
            ));
        }
        Ok(Self {
            stream_id: StreamId::from_content_id(content_id_from_value(
                fields.required("stream_id")?,
            )?),
            namespace_id: NamespaceId::new(fields.digest("namespace_id")?),
            media_type: fields.text("media_type")?,
            creator: fields.digest("creator")?,
            created_at: fields.uint("created_at")?,
            state,
            current_seq: fields.uint("current_seq")?,
            final_ref,
            generation: fields.uint("generation")?,
        })
    }
}

/// One signed chunk record. Immutable once signed; its ID covers the
/// complete signed encoding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkRecord {
    /// The stream this chunk belongs to.
    pub stream_id: StreamId,
    /// The authenticated tenant scope the chunk was written under.
    pub tenant_scope: Vec<u8>,
    /// Position in the chain; the genesis chunk is sequence 0.
    pub sequence: u64,
    /// The previous chunk's record ID; `None` at sequence 0.
    pub prev_record_id: Option<ChunkRecordId>,
    /// Content identity of the chunk payload (profile included).
    pub payload_id: ContentId,
    /// Media metadata: the chunk's media type.
    pub media_type: String,
    /// Signer's Ed25519 public key.
    pub signer: [u8; DIGEST_BYTES],
    /// Ed25519 signature over [`ChunkRecord::signing_preimage`].
    pub signature: [u8; 64],
}

impl ChunkRecord {
    /// An unsigned chunk record binding `payload`'s ID. Sign it with
    /// [`ChunkRecord::sign`].
    ///
    /// # Errors
    ///
    /// [`StreamError::Malformed`] on an empty or over-bound tenant
    /// scope or media type.
    pub fn unsigned(
        stream_id: StreamId,
        tenant_scope: Vec<u8>,
        sequence: u64,
        prev_record_id: Option<ChunkRecordId>,
        payload: &PayloadId,
        media_type: &str,
    ) -> Result<Self, StreamError> {
        check_scope(&tenant_scope)?;
        check_label("media_type", media_type)?;
        Ok(Self {
            stream_id,
            tenant_scope,
            sequence,
            prev_record_id,
            payload_id: *payload.content_id(),
            media_type: media_type.to_owned(),
            signer: [0u8; DIGEST_BYTES],
            signature: [0u8; 64],
        })
    }

    /// Signs the record with `key`, setting the signer to the key's
    /// public half.
    ///
    /// # Errors
    ///
    /// [`StreamError`] when the record cannot be canonically encoded.
    pub fn sign(mut self, key: &SigningKey) -> Result<Self, StreamError> {
        self.signer = key.verifying_key().to_bytes();
        let preimage = self.signing_preimage()?;
        self.signature = key.sign(&preimage).to_bytes();
        Ok(self)
    }

    /// The signed body as a canonical CBOR value (every field except
    /// the signature).
    ///
    /// # Errors
    ///
    /// [`StreamError::Malformed`] on out-of-bound labels.
    pub fn body_value(&self) -> Result<Value, StreamError> {
        check_scope(&self.tenant_scope)?;
        check_label("media_type", &self.media_type)?;
        let mut entries = vec![
            ("v", Value::UInt(STREAM_FORMAT_VERSION)),
            ("stream_id", content_id_value(self.stream_id.content_id())),
            ("tenant_scope", Value::Bytes(self.tenant_scope.clone())),
            ("sequence", Value::UInt(self.sequence)),
            ("payload_id", content_id_value(&self.payload_id)),
            ("media_type", text(&self.media_type)),
            ("signer", Value::Bytes(self.signer.to_vec())),
        ];
        if let Some(prev) = &self.prev_record_id {
            entries.push(("prev_record_id", content_id_value(prev.content_id())));
        }
        Ok(map(entries))
    }

    /// The complete signed record as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// As [`ChunkRecord::body_value`].
    pub fn to_value(&self) -> Result<Value, StreamError> {
        let Value::Map(mut entries) = self.body_value()? else {
            return Err(StreamError::malformed("chunk body is not a map"));
        };
        entries.push((text("signature"), Value::Bytes(self.signature.to_vec())));
        Ok(Value::Map(entries))
    }

    /// `canonical_cbor(signed ChunkRecord)`.
    ///
    /// # Errors
    ///
    /// As [`ChunkRecord::body_value`], plus the encoder's rejection.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, StreamError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a chunk record from canonical bytes.
    ///
    /// # Errors
    ///
    /// [`StreamError`] for malformed bytes, an unknown field, or a
    /// missing or mistyped field.
    pub fn decode(bytes: &[u8]) -> Result<Self, StreamError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// Reads a chunk record from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// As [`ChunkRecord::decode`].
    pub fn from_value(value: &Value) -> Result<Self, StreamError> {
        let fields = Fields::new(
            value,
            &[
                "v",
                "stream_id",
                "tenant_scope",
                "sequence",
                "prev_record_id",
                "payload_id",
                "media_type",
                "signer",
                "signature",
            ],
        )?;
        check_version(fields.uint("v")?)?;
        let signature = fields.bytes("signature")?;
        let signature: [u8; 64] = signature
            .try_into()
            .map_err(|_| StreamError::malformed("signature must be 64 bytes"))?;
        Ok(Self {
            stream_id: StreamId::from_content_id(content_id_from_value(
                fields.required("stream_id")?,
            )?),
            tenant_scope: fields.bytes("tenant_scope")?,
            sequence: fields.uint("sequence")?,
            prev_record_id: fields
                .optional("prev_record_id")
                .map(|value| content_id_from_value(value).map(ChunkRecordId::from_content_id))
                .transpose()?,
            payload_id: content_id_from_value(fields.required("payload_id")?)?,
            media_type: fields.text("media_type")?,
            signer: fields.digest("signer")?,
            signature,
        })
    }

    /// The bytes the signature covers: the chunk domain prefix, then
    /// `canonical_cbor(body)`.
    ///
    /// # Errors
    ///
    /// As [`ChunkRecord::body_value`], plus the encoder's rejection.
    pub fn signing_preimage(&self) -> Result<Vec<u8>, StreamError> {
        let mut preimage = CHUNK_DOMAIN_PREFIX.to_vec();
        preimage.extend_from_slice(&encode_canonical(&self.body_value()?)?);
        Ok(preimage)
    }

    /// This record's ID: the raw-BLAKE3-256 content ID of the complete
    /// signed encoding.
    ///
    /// # Errors
    ///
    /// As [`ChunkRecord::canonical_bytes`].
    pub fn record_id(&self) -> Result<ChunkRecordId, StreamError> {
        Ok(ChunkRecordId(ContentId::of_bytes(
            HashProfile::RawBlake3_256,
            &self.canonical_bytes()?,
        )?))
    }

    /// Verifies the signature against the record's signer key.
    ///
    /// # Errors
    ///
    /// [`StreamError::BadSignature`] when the key or signature does
    /// not verify; [`StreamError`] when the body cannot be encoded.
    pub fn verify_signature(&self) -> Result<(), StreamError> {
        let key = VerifyingKey::from_bytes(&self.signer).map_err(|_| StreamError::BadSignature)?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.signing_preimage()?, &signature)
            .map_err(|_| StreamError::BadSignature)
    }
}

/// Which terminal transition a [`TerminalRecord`] performs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminalKind {
    /// `finalizing -> completed`; carries the final reference.
    Completed,
    /// `open|paused|finalizing -> aborted`; claims no final content.
    Aborted,
    /// `open|paused -> expired`; claims no final content.
    Expired,
}

impl TerminalKind {
    /// The kind token used in canonical encodings.
    #[must_use]
    pub const fn token(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::Aborted => "aborted",
            Self::Expired => "expired",
        }
    }

    /// Reads a kind token.
    #[must_use]
    pub fn from_token(token: &str) -> Option<Self> {
        match token {
            "completed" => Some(Self::Completed),
            "aborted" => Some(Self::Aborted),
            "expired" => Some(Self::Expired),
            _ => None,
        }
    }

    /// The lifecycle state this kind lands the stream in.
    #[must_use]
    pub const fn lands_in(self) -> StreamState {
        match self {
            Self::Completed => StreamState::Completed,
            Self::Aborted => StreamState::Aborted,
            Self::Expired => StreamState::Expired,
        }
    }
}

/// A signed terminal record: the one and only record that may close a
/// stream. Only a completion carries a final reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalRecord {
    /// Which terminal transition this record performs.
    pub kind: TerminalKind,
    /// The stream this record closes.
    pub stream_id: StreamId,
    /// Final content/manifest reference; present only for completion.
    pub final_ref: Option<ContentId>,
    /// Signer's Ed25519 public key.
    pub signer: [u8; DIGEST_BYTES],
    /// Ed25519 signature over the terminal signing preimage.
    pub signature: [u8; 64],
}

impl TerminalRecord {
    /// An unsigned terminal record. Sign it with
    /// [`TerminalRecord::sign`].
    ///
    /// # Errors
    ///
    /// [`StreamError::FinalRefMismatch`] when a completion has no
    /// final reference or a non-completion claims one.
    pub fn unsigned(
        kind: TerminalKind,
        stream_id: StreamId,
        final_ref: Option<ContentId>,
    ) -> Result<Self, StreamError> {
        check_final_ref(kind, &final_ref)?;
        Ok(Self {
            kind,
            stream_id,
            final_ref,
            signer: [0u8; DIGEST_BYTES],
            signature: [0u8; 64],
        })
    }

    /// Signs the record with `key`.
    ///
    /// # Errors
    ///
    /// [`StreamError`] when the record cannot be canonically encoded.
    pub fn sign(mut self, key: &SigningKey) -> Result<Self, StreamError> {
        self.signer = key.verifying_key().to_bytes();
        let preimage = self.signing_preimage()?;
        self.signature = key.sign(&preimage).to_bytes();
        Ok(self)
    }

    /// The signed body as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`StreamError::FinalRefMismatch`] on a final-reference shape
    /// the kind forbids.
    pub fn body_value(&self) -> Result<Value, StreamError> {
        check_final_ref(self.kind, &self.final_ref)?;
        let mut entries = vec![
            ("v", Value::UInt(STREAM_FORMAT_VERSION)),
            ("kind", text(self.kind.token())),
            ("stream_id", content_id_value(self.stream_id.content_id())),
            ("signer", Value::Bytes(self.signer.to_vec())),
        ];
        if let Some(final_ref) = &self.final_ref {
            entries.push(("final_ref", content_id_value(final_ref)));
        }
        Ok(map(entries))
    }

    /// The complete signed record as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// As [`TerminalRecord::body_value`].
    pub fn to_value(&self) -> Result<Value, StreamError> {
        let Value::Map(mut entries) = self.body_value()? else {
            return Err(StreamError::malformed("terminal body is not a map"));
        };
        entries.push((text("signature"), Value::Bytes(self.signature.to_vec())));
        Ok(Value::Map(entries))
    }

    /// `canonical_cbor(signed TerminalRecord)`.
    ///
    /// # Errors
    ///
    /// As [`TerminalRecord::body_value`], plus the encoder's rejection.
    pub fn canonical_bytes(&self) -> Result<Vec<u8>, StreamError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a terminal record from canonical bytes.
    ///
    /// # Errors
    ///
    /// [`StreamError`] for malformed bytes, an unknown field, a
    /// missing or mistyped field, or a forbidden final reference.
    pub fn decode(bytes: &[u8]) -> Result<Self, StreamError> {
        Self::from_value(&decode_canonical(bytes)?)
    }

    /// Reads a terminal record from a decoded canonical value.
    ///
    /// # Errors
    ///
    /// As [`TerminalRecord::decode`].
    pub fn from_value(value: &Value) -> Result<Self, StreamError> {
        let fields = Fields::new(
            value,
            &["v", "kind", "stream_id", "final_ref", "signer", "signature"],
        )?;
        check_version(fields.uint("v")?)?;
        let kind_token = fields.text("kind")?;
        let kind = TerminalKind::from_token(&kind_token).ok_or_else(|| {
            StreamError::malformed(format!("unknown terminal kind {kind_token:?}"))
        })?;
        let signature = fields.bytes("signature")?;
        let signature: [u8; 64] = signature
            .try_into()
            .map_err(|_| StreamError::malformed("signature must be 64 bytes"))?;
        let record = Self {
            kind,
            stream_id: StreamId::from_content_id(content_id_from_value(
                fields.required("stream_id")?,
            )?),
            final_ref: fields
                .optional("final_ref")
                .map(|value| content_id_from_value(value))
                .transpose()?,
            signer: fields.digest("signer")?,
            signature,
        };
        check_final_ref(record.kind, &record.final_ref)?;
        Ok(record)
    }

    /// The bytes the signature covers: the terminal domain prefix,
    /// then `canonical_cbor(body)`.
    ///
    /// # Errors
    ///
    /// As [`TerminalRecord::body_value`], plus the encoder's rejection.
    pub fn signing_preimage(&self) -> Result<Vec<u8>, StreamError> {
        let mut preimage = TERMINAL_DOMAIN_PREFIX.to_vec();
        preimage.extend_from_slice(&encode_canonical(&self.body_value()?)?);
        Ok(preimage)
    }

    /// This record's ID, computed like a chunk record ID.
    ///
    /// # Errors
    ///
    /// As [`TerminalRecord::canonical_bytes`].
    pub fn record_id(&self) -> Result<TerminalRecordId, StreamError> {
        Ok(TerminalRecordId(ContentId::of_bytes(
            HashProfile::RawBlake3_256,
            &self.canonical_bytes()?,
        )?))
    }

    /// Verifies the signature against the record's signer key.
    ///
    /// # Errors
    ///
    /// [`StreamError::BadSignature`] when verification fails.
    pub fn verify_signature(&self) -> Result<(), StreamError> {
        let key = VerifyingKey::from_bytes(&self.signer).map_err(|_| StreamError::BadSignature)?;
        let signature = Signature::from_bytes(&self.signature);
        key.verify(&self.signing_preimage()?, &signature)
            .map_err(|_| StreamError::BadSignature)
    }
}

/// The outcome of an append that did not fail.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppendOutcome {
    /// The chunk extended the chain.
    Appended,
    /// The exact chunk was already accepted; nothing changed.
    Duplicate,
}

/// How a stream resolves for a reader.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Not completed: the live state (including aborted/expired).
    Live {
        /// The current lifecycle state.
        state: StreamState,
    },
    /// Completed: the typed final reference. Following it is
    /// client-selected, never an implicit redirect.
    Completed {
        /// The final content/manifest reference.
        final_id: ContentId,
    },
}

/// The finite lifecycle state machine for one stream. Holds the
/// descriptor, the accepted chunk chain (record IDs, in order) and
/// the terminal record once one lands.
#[derive(Debug, Clone)]
pub struct StreamLifecycle {
    descriptor: StreamDescriptor,
    tenant_scope: Vec<u8>,
    sequencer: Option<[u8; DIGEST_BYTES]>,
    chain: Vec<ChunkRecordId>,
    terminal: Option<TerminalRecord>,
}

impl StreamLifecycle {
    /// Opens the machine over a fresh `open` descriptor.
    /// `tenant_scope` is the authenticated scope context every chunk
    /// must bind; `sequencer` is the namespace-configured multiwriter
    /// authority, when one is configured.
    ///
    /// # Errors
    ///
    /// [`StreamError::Malformed`] on an out-of-bound tenant scope;
    /// [`StreamError::BadTransition`] when the descriptor is not a
    /// fresh open descriptor.
    pub fn new(
        descriptor: StreamDescriptor,
        tenant_scope: Vec<u8>,
        sequencer: Option<[u8; DIGEST_BYTES]>,
    ) -> Result<Self, StreamError> {
        check_scope(&tenant_scope)?;
        if descriptor.state != StreamState::Open
            || descriptor.current_seq != 0
            || descriptor.final_ref.is_some()
        {
            return Err(StreamError::BadTransition {
                from: descriptor.state,
                action: "open",
            });
        }
        Ok(Self {
            descriptor,
            tenant_scope,
            sequencer,
            chain: Vec::new(),
            terminal: None,
        })
    }

    /// The current descriptor.
    #[must_use]
    pub const fn descriptor(&self) -> &StreamDescriptor {
        &self.descriptor
    }

    /// The current lifecycle state.
    #[must_use]
    pub const fn state(&self) -> StreamState {
        self.descriptor.state
    }

    /// The accepted chunk chain, in order.
    #[must_use]
    pub fn chain(&self) -> &[ChunkRecordId] {
        &self.chain
    }

    /// The chain head: the resume cursor's record ID.
    #[must_use]
    pub fn last_record_id(&self) -> Option<ChunkRecordId> {
        self.chain.last().copied()
    }

    /// The terminal record, once one has landed.
    #[must_use]
    pub const fn terminal_record(&self) -> Option<&TerminalRecord> {
        self.terminal.as_ref()
    }

    /// Resolves the stream for a reader.
    #[must_use]
    pub fn resolve(&self) -> Resolution {
        match (self.descriptor.state, self.descriptor.final_ref) {
            (StreamState::Completed, Some(final_id)) => Resolution::Completed { final_id },
            (state, _) => Resolution::Live { state },
        }
    }

    /// Appends one signed chunk record.
    ///
    /// # Errors
    ///
    /// In check order: [`StreamError::StreamMismatch`],
    /// [`StreamError::SequenceConflict`] or a duplicate outcome for an
    /// already-covered sequence, [`StreamError::Terminal`],
    /// [`StreamError::NotOpen`], [`StreamError::ScopeMismatch`],
    /// [`StreamError::UnauthorizedSigner`],
    /// [`StreamError::BadSignature`], [`StreamError::SequenceGap`],
    /// [`StreamError::ChainBreak`].
    pub fn append_chunk(&mut self, record: &ChunkRecord) -> Result<AppendOutcome, StreamError> {
        if record.stream_id != self.descriptor.stream_id {
            return Err(StreamError::StreamMismatch);
        }
        let record_id = record.record_id()?;
        if record.sequence < self.descriptor.current_seq {
            let covered = self
                .chain
                .get(usize::try_from(record.sequence).unwrap_or(usize::MAX))
                .copied();
            return if covered == Some(record_id) {
                Ok(AppendOutcome::Duplicate)
            } else {
                Err(StreamError::SequenceConflict {
                    sequence: record.sequence,
                })
            };
        }
        if self.terminal.is_some() {
            return Err(StreamError::Terminal);
        }
        if self.descriptor.state != StreamState::Open {
            return Err(StreamError::NotOpen {
                state: self.descriptor.state,
            });
        }
        if record.tenant_scope != self.tenant_scope {
            return Err(StreamError::ScopeMismatch);
        }
        if !self.is_authorized(&record.signer) {
            return Err(StreamError::UnauthorizedSigner);
        }
        record.verify_signature()?;
        if record.sequence != self.descriptor.current_seq {
            return Err(StreamError::SequenceGap {
                expected: self.descriptor.current_seq,
                found: record.sequence,
            });
        }
        if record.prev_record_id != self.last_record_id() {
            return Err(StreamError::ChainBreak);
        }
        self.chain.push(record_id);
        self.descriptor.current_seq += 1;
        Ok(AppendOutcome::Appended)
    }

    /// `open -> paused`.
    ///
    /// # Errors
    ///
    /// [`StreamError::Terminal`] once closed;
    /// [`StreamError::BadTransition`] from any other state.
    pub fn pause(&mut self) -> Result<(), StreamError> {
        self.transition(StreamState::Open, StreamState::Paused, "pause")
    }

    /// `paused -> open`.
    ///
    /// # Errors
    ///
    /// As [`StreamLifecycle::pause`].
    pub fn resume(&mut self) -> Result<(), StreamError> {
        self.transition(StreamState::Paused, StreamState::Open, "resume")
    }

    /// The completion CAS, first half: `open -> finalizing`. From
    /// `paused` the stream must be resumed first.
    ///
    /// # Errors
    ///
    /// As [`StreamLifecycle::pause`].
    pub fn begin_finalize(&mut self) -> Result<(), StreamError> {
        self.transition(StreamState::Open, StreamState::Finalizing, "finalize")
    }

    /// The completion CAS, second half: `finalizing -> completed`,
    /// landing the signed completion record and its final reference.
    ///
    /// # Errors
    ///
    /// [`StreamError::Terminal`] once closed;
    /// [`StreamError::BadTransition`] unless finalizing;
    /// [`StreamError::StreamMismatch`],
    /// [`StreamError::UnauthorizedSigner`] or
    /// [`StreamError::BadSignature`] for a record that does not
    /// verify; [`StreamError::FinalRefMismatch`] for a record that is
    /// not a completion.
    pub fn complete(&mut self, record: TerminalRecord) -> Result<(), StreamError> {
        self.apply_terminal(record, TerminalKind::Completed, &[StreamState::Finalizing])
    }

    /// `open|paused|finalizing -> aborted` via a signed abort record.
    ///
    /// # Errors
    ///
    /// As [`StreamLifecycle::complete`].
    pub fn abort(&mut self, record: TerminalRecord) -> Result<(), StreamError> {
        self.apply_terminal(
            record,
            TerminalKind::Aborted,
            &[
                StreamState::Open,
                StreamState::Paused,
                StreamState::Finalizing,
            ],
        )
    }

    /// `open|paused -> expired` via a signed expiry record.
    ///
    /// # Errors
    ///
    /// As [`StreamLifecycle::complete`].
    pub fn expire(&mut self, record: TerminalRecord) -> Result<(), StreamError> {
        self.apply_terminal(
            record,
            TerminalKind::Expired,
            &[StreamState::Open, StreamState::Paused],
        )
    }

    fn is_authorized(&self, signer: &[u8; DIGEST_BYTES]) -> bool {
        *signer == self.descriptor.creator || self.sequencer == Some(*signer)
    }

    fn transition(
        &mut self,
        from: StreamState,
        to: StreamState,
        action: &'static str,
    ) -> Result<(), StreamError> {
        if self.terminal.is_some() {
            return Err(StreamError::Terminal);
        }
        if self.descriptor.state != from {
            return Err(StreamError::BadTransition {
                from: self.descriptor.state,
                action,
            });
        }
        self.descriptor.state = to;
        self.descriptor.generation += 1;
        Ok(())
    }

    fn apply_terminal(
        &mut self,
        record: TerminalRecord,
        expected_kind: TerminalKind,
        allowed_from: &[StreamState],
    ) -> Result<(), StreamError> {
        if self.terminal.is_some() {
            return Err(StreamError::Terminal);
        }
        if !allowed_from.contains(&self.descriptor.state) {
            return Err(StreamError::BadTransition {
                from: self.descriptor.state,
                action: expected_kind.token(),
            });
        }
        if record.kind != expected_kind {
            return Err(StreamError::malformed(
                "terminal record kind does not match the attempted transition",
            ));
        }
        if record.stream_id != self.descriptor.stream_id {
            return Err(StreamError::StreamMismatch);
        }
        check_final_ref(record.kind, &record.final_ref)?;
        if !self.is_authorized(&record.signer) {
            return Err(StreamError::UnauthorizedSigner);
        }
        record.verify_signature()?;
        if record.kind == TerminalKind::Completed {
            self.descriptor.final_ref = record.final_ref;
        }
        self.descriptor.state = record.kind.lands_in();
        self.descriptor.generation += 1;
        self.terminal = Some(record);
        Ok(())
    }
}

/// Why a stream operation was rejected. Every variant has a stable
/// failure class ([`StreamError::class`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamError {
    /// The bytes or values were malformed.
    Malformed(String),
    /// A canonical-CBOR rejection.
    Cbor(CborError),
    /// An ID-shape rejection.
    Shape(IdShapeError),
    /// The stream is already terminal; exactly one terminal record
    /// ever lands.
    Terminal,
    /// Chunks are accepted only in the `open` state.
    NotOpen {
        /// The state the stream is in.
        state: StreamState,
    },
    /// The transition is not in the lifecycle.
    BadTransition {
        /// The state the stream is in.
        from: StreamState,
        /// The attempted action.
        action: &'static str,
    },
    /// The record names a different stream.
    StreamMismatch,
    /// The record's tenant scope is not the stream's scope.
    ScopeMismatch,
    /// The signer is neither the creator nor the configured
    /// sequencer authority.
    UnauthorizedSigner,
    /// The signature does not verify.
    BadSignature,
    /// The sequence skips ahead of the chain head.
    SequenceGap {
        /// The sequence the chain expects.
        expected: u64,
        /// The sequence the record carried.
        found: u64,
    },
    /// A differing record at an already-covered sequence:
    /// conflict/equivocation evidence.
    SequenceConflict {
        /// The covered sequence.
        sequence: u64,
    },
    /// The previous-record link does not match the chain head.
    ChainBreak,
    /// A terminal record's final reference does not fit its kind.
    FinalRefMismatch,
}

impl StreamError {
    /// A malformed-input error with a human-readable detail.
    pub fn malformed(detail: impl Into<String>) -> Self {
        Self::Malformed(detail.into())
    }

    /// The stable failure class: `malformed`, `terminal`, `not_open`,
    /// `bad_transition`, `stream_mismatch`, `scope_mismatch`,
    /// `unauthorized_signer`, `bad_signature`, `sequence_gap`,
    /// `sequence_conflict`, `chain_break` or `final_ref_mismatch`.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Malformed(_) | Self::Cbor(_) | Self::Shape(_) => "malformed",
            Self::Terminal => "terminal",
            Self::NotOpen { .. } => "not_open",
            Self::BadTransition { .. } => "bad_transition",
            Self::StreamMismatch => "stream_mismatch",
            Self::ScopeMismatch => "scope_mismatch",
            Self::UnauthorizedSigner => "unauthorized_signer",
            Self::BadSignature => "bad_signature",
            Self::SequenceGap { .. } => "sequence_gap",
            Self::SequenceConflict { .. } => "sequence_conflict",
            Self::ChainBreak => "chain_break",
            Self::FinalRefMismatch => "final_ref_mismatch",
        }
    }
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Malformed(detail) => write!(f, "malformed stream record: {detail}"),
            Self::Cbor(error) => write!(f, "malformed stream record: {error}"),
            Self::Shape(error) => write!(f, "malformed stream record: {error}"),
            Self::Terminal => write!(f, "stream is already terminal"),
            Self::NotOpen { state } => write!(f, "stream is {}, not open", state.token()),
            Self::BadTransition { from, action } => {
                write!(f, "cannot {action} a {} stream", from.token())
            }
            Self::StreamMismatch => write!(f, "record names a different stream"),
            Self::ScopeMismatch => write!(f, "record binds a different tenant scope"),
            Self::UnauthorizedSigner => write!(f, "signer is not authorized for this stream"),
            Self::BadSignature => write!(f, "signature does not verify"),
            Self::SequenceGap { expected, found } => {
                write!(f, "sequence gap: expected {expected}, found {found}")
            }
            Self::SequenceConflict { sequence } => {
                write!(f, "conflicting record at covered sequence {sequence}")
            }
            Self::ChainBreak => write!(f, "previous record link does not match the chain head"),
            Self::FinalRefMismatch => {
                write!(f, "terminal record final reference does not fit its kind")
            }
        }
    }
}

impl std::error::Error for StreamError {}

impl From<CborError> for StreamError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<IdShapeError> for StreamError {
    fn from(error: IdShapeError) -> Self {
        Self::Shape(error)
    }
}

fn check_label(field: &'static str, value: &str) -> Result<(), StreamError> {
    if value.is_empty() || value.len() > MAX_LABEL_BYTES {
        return Err(StreamError::malformed(format!(
            "{field} must be 1..={MAX_LABEL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn check_scope(scope: &[u8]) -> Result<(), StreamError> {
    if scope.is_empty() || scope.len() > MAX_LABEL_BYTES {
        return Err(StreamError::malformed(format!(
            "tenant scope must be 1..={MAX_LABEL_BYTES} bytes"
        )));
    }
    Ok(())
}

fn check_final_ref(kind: TerminalKind, final_ref: &Option<ContentId>) -> Result<(), StreamError> {
    match (kind, final_ref) {
        (TerminalKind::Completed, Some(_))
        | (TerminalKind::Aborted | TerminalKind::Expired, None) => Ok(()),
        _ => Err(StreamError::FinalRefMismatch),
    }
}

fn check_version(version: u64) -> Result<(), StreamError> {
    if version != STREAM_FORMAT_VERSION {
        return Err(StreamError::malformed(format!(
            "unsupported stream format version {version}"
        )));
    }
    Ok(())
}

fn text(value: &str) -> Value {
    Value::Text(value.to_owned())
}

fn map(entries: Vec<(&str, Value)>) -> Value {
    Value::Map(
        entries
            .into_iter()
            .map(|(key, value)| (text(key), value))
            .collect(),
    )
}

fn content_id_value(id: &ContentId) -> Value {
    Value::Array(vec![
        text(id.profile().token()),
        Value::Bytes(id.digest().to_vec()),
    ])
}

fn content_id_from_value(value: &Value) -> Result<ContentId, StreamError> {
    let Value::Array(parts) = value else {
        return Err(StreamError::malformed("content ID must be an array"));
    };
    let [Value::Text(token), Value::Bytes(digest)] = parts.as_slice() else {
        return Err(StreamError::malformed(
            "content ID must be [profile, digest]",
        ));
    };
    let profile = HashProfile::from_token(token)
        .ok_or_else(|| StreamError::malformed(format!("unknown hash profile {token:?}")))?;
    let digest: [u8; DIGEST_BYTES] = digest
        .clone()
        .try_into()
        .map_err(|_| StreamError::malformed("content ID digest must be 32 bytes"))?;
    Ok(ContentId::new(profile, digest))
}

/// A decoded canonical map with critical-field discipline: every
/// field of a V1 record is critical, so an unknown field is a
/// rejection, never silently dropped.
struct Fields<'a> {
    entries: &'a [(Value, Value)],
}

impl<'a> Fields<'a> {
    fn new(value: &'a Value, known: &[&str]) -> Result<Self, StreamError> {
        let Value::Map(entries) = value else {
            return Err(StreamError::malformed("record must be a map"));
        };
        for (key, _) in entries {
            let Value::Text(key) = key else {
                return Err(StreamError::malformed("record keys must be text"));
            };
            if !known.contains(&key.as_str()) {
                return Err(StreamError::malformed(format!(
                    "unknown critical field {key:?}"
                )));
            }
        }
        Ok(Self { entries })
    }

    fn required(&self, field: &str) -> Result<&'a Value, StreamError> {
        self.optional(field)
            .ok_or_else(|| StreamError::malformed(format!("missing field {field:?}")))
    }

    fn optional(&self, field: &str) -> Option<&'a Value> {
        self.entries.iter().find_map(|(key, value)| match key {
            Value::Text(name) if name == field => Some(value),
            _ => None,
        })
    }

    fn text(&self, field: &str) -> Result<String, StreamError> {
        match self.required(field)? {
            Value::Text(value) => Ok(value.clone()),
            _ => Err(StreamError::malformed(format!(
                "field {field:?} must be text"
            ))),
        }
    }

    fn bytes(&self, field: &str) -> Result<Vec<u8>, StreamError> {
        match self.required(field)? {
            Value::Bytes(value) => Ok(value.clone()),
            _ => Err(StreamError::malformed(format!(
                "field {field:?} must be bytes"
            ))),
        }
    }

    fn digest(&self, field: &str) -> Result<[u8; DIGEST_BYTES], StreamError> {
        self.bytes(field)?
            .try_into()
            .map_err(|_| StreamError::malformed(format!("field {field:?} must be 32 bytes")))
    }

    fn uint(&self, field: &str) -> Result<u64, StreamError> {
        match self.required(field)? {
            Value::UInt(value) => Ok(*value),
            _ => Err(StreamError::malformed(format!(
                "field {field:?} must be a uint"
            ))),
        }
    }
}
