//! Canonical signed graph records (slice A-GRAPH-9125-01, #10196;
//! design part A §3.1, §3.2 and §4.2, statements S5.1–S5.7, S5.11).
//!
//! A graph record is the signed payload of one write into a logical
//! graph: a schema version, the stable [`LogicalGraphId`], zero or more
//! [`Triple`]s and an optional `source_record`, the already identified
//! evidence the triples were taken from. It travels as the payload of a
//! deterministic-CBOR [`Envelope`] (the new signed-record profile; legacy
//! artifact JSON is a separate format and is untouched).
//!
//! # Payload
//!
//! `canonical_cbor({"v": 1, "graph": bstr(32), "triples": [triple...],
//! "source_record"?: [2, profile, digest]})`. The definite-length
//! `triples` array header is the record's triple count. Every field is
//! critical: an unknown key, a missing required key, another schema
//! version or a non-canonical encoding rejects. Terms keep their bytes
//! exactly; nothing here normalizes them.
//!
//! # Identities
//!
//! - [`PayloadId`]: the profile hash of the raw payload bytes above.
//! - The envelope [`Body`] commits that payload ID, the payload length,
//!   the domain [`GRAPH_RECORD_DOMAIN`], the caller's context and the
//!   graph, and is signed with the domain-prefixed preimage. It holds
//!   neither its signature nor its own envelope ID.
//! - [`EnvelopeId`]: the profile hash of the complete canonical envelope,
//!   signature included. This is the record's identity, and an
//!   assertion is addressed as `(record EnvelopeId, triple index)`, a
//!   stable position that resumable readers can seek to without
//!   materializing earlier records.
//! - [`LogicalGraphId`] is assigned, not derived from content, so it
//!   stays put across records.
//!
//! No ID appears in its own preimage: a record whose `source_record` or
//! any reference term names the record's own envelope or its own
//! payload is rejected as [`RecordError::SelfReference`].
//!
//! Module wiring into `graph/mod.rs` and the crate root belongs to slice
//! 9110-09; until then the integration test includes this file by path.

use std::fmt;

use ed25519_dalek::SigningKey;

use super::term::{ObjectRef, Term, TermError, Triple};
use crate::canonical_cbor::{CborError, Value, decode_canonical, encode_canonical};
use crate::hash_profile::ProfileDispatch;
use crate::id_shape::{ContentId, DIGEST_BYTES, HashProfile};
use crate::signed_envelope::{
    Body, Envelope, EnvelopeError, EnvelopeId, LogicalGraphId, PayloadId, SUITE_ED25519,
};

/// The payload schema version this reader writes and accepts.
pub const SCHEMA_VERSION: u64 = 1;

/// The signing domain of a graph record.
pub const GRAPH_RECORD_DOMAIN: &str = "id/graph-record";

/// The profile record identities are computed under.
pub const RECORD_ID_PROFILE: HashProfile = HashProfile::RawBlake3_256;

/// Upper bound on triples in one record, checked before decoding them.
pub const MAX_TRIPLES: usize = 4096;

const FIELDS: [&str; 4] = ["v", "graph", "triples", "source_record"];

/// One signed write into a logical graph.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GraphRecord {
    /// The graph the triples are written into.
    pub graph: LogicalGraphId,
    /// The triples, in signed order.
    pub triples: Vec<Triple>,
    /// The evidence these triples were taken from, if any.
    pub source_record: Option<EnvelopeId>,
}

/// A record whose envelope verified and whose payload decoded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpenedRecord {
    /// The record's identity: its envelope ID under
    /// [`RECORD_ID_PROFILE`].
    pub id: EnvelopeId,
    /// The decoded payload.
    pub record: GraphRecord,
    /// The verified envelope, kept byte for byte.
    pub envelope: Envelope,
}

/// One triple of a record, with its provenance.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assertion<'a> {
    /// The record the triple was signed in.
    pub record: EnvelopeId,
    /// The triple's position in that record.
    pub index: u32,
    /// The graph it belongs to.
    pub graph: LogicalGraphId,
    /// The triple itself.
    pub triple: &'a Triple,
    /// The evidence the record names, if any.
    pub source_record: Option<EnvelopeId>,
}

/// Why a graph record was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecordError {
    /// The payload is not canonical CBOR, or has an unknown field.
    Cbor(CborError),
    /// A term or triple was rejected.
    Term(TermError),
    /// The envelope was rejected.
    Envelope(EnvelopeError),
    /// A field is missing or has the wrong type or size.
    Field {
        /// The field name.
        field: &'static str,
        /// What was wrong.
        problem: &'static str,
    },
    /// The payload names a schema version this reader does not know.
    UnsupportedSchema(u64),
    /// The record holds more than [`MAX_TRIPLES`] triples.
    TooManyTriples(usize),
    /// The envelope was signed for another domain.
    WrongDomain(String),
    /// The envelope body and the payload name different graphs.
    GraphMismatch,
    /// The record references its own envelope or its own payload.
    SelfReference,
}

impl RecordError {
    /// A stable failure class: `malformed`, `unsupported`,
    /// `resource_limit`, `self_reference`, or the envelope's own class.
    #[must_use]
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Cbor(_) | Self::Field { .. } | Self::WrongDomain(_) | Self::GraphMismatch => {
                "malformed"
            }
            Self::Term(error) => error.class(),
            Self::Envelope(error) => error.class(),
            Self::UnsupportedSchema(_) => "unsupported",
            Self::TooManyTriples(_) => "resource_limit",
            Self::SelfReference => "self_reference",
        }
    }
}

impl fmt::Display for RecordError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Cbor(error) => write!(f, "graph record encoding: {error}"),
            Self::Term(error) => write!(f, "graph record term: {error}"),
            Self::Envelope(error) => write!(f, "graph record envelope: {error}"),
            Self::Field { field, problem } => write!(f, "field {field}: {problem}"),
            Self::UnsupportedSchema(version) => {
                write!(f, "unsupported graph record schema {version}")
            }
            Self::TooManyTriples(count) => {
                write!(f, "{count} triples, over the {MAX_TRIPLES}-triple limit")
            }
            Self::WrongDomain(domain) => write!(f, "signed for domain {domain:?}"),
            Self::GraphMismatch => write!(f, "envelope and payload name different graphs"),
            Self::SelfReference => write!(f, "record references its own identity"),
        }
    }
}

impl std::error::Error for RecordError {}

impl From<CborError> for RecordError {
    fn from(error: CborError) -> Self {
        Self::Cbor(error)
    }
}

impl From<TermError> for RecordError {
    fn from(error: TermError) -> Self {
        Self::Term(error)
    }
}

impl From<EnvelopeError> for RecordError {
    fn from(error: EnvelopeError) -> Self {
        Self::Envelope(error)
    }
}

const fn field(field: &'static str, problem: &'static str) -> RecordError {
    RecordError::Field { field, problem }
}

fn lookup<'a>(entries: &'a [(Value, Value)], name: &str) -> Option<&'a Value> {
    entries
        .iter()
        .find(|(key, _)| matches!(key, Value::Text(key) if key == name))
        .map(|(_, value)| value)
}

impl GraphRecord {
    /// A record of `triples` in `graph` with no source.
    #[must_use]
    pub const fn new(graph: LogicalGraphId, triples: Vec<Triple>) -> Self {
        Self {
            graph,
            triples,
            source_record: None,
        }
    }

    /// Every object the record references: its source and each
    /// reference term, in signed order.
    pub fn references(&self) -> impl Iterator<Item = ObjectRef> + '_ {
        self.source_record
            .map(ObjectRef::Envelope)
            .into_iter()
            .chain(
                self.triples
                    .iter()
                    .flat_map(Triple::terms)
                    .filter_map(Term::object_ref)
                    .copied(),
            )
    }

    /// The payload as a canonical CBOR value.
    ///
    /// # Errors
    ///
    /// [`RecordError::TooManyTriples`], or a term rejection.
    pub fn to_value(&self) -> Result<Value, RecordError> {
        if self.triples.len() > MAX_TRIPLES {
            return Err(RecordError::TooManyTriples(self.triples.len()));
        }
        let triples = self
            .triples
            .iter()
            .map(Triple::to_value)
            .collect::<Result<Vec<_>, _>>()?;
        // Keys in canonical order: shorter first, then bytewise.
        let mut entries = vec![
            (Value::Text("v".to_owned()), Value::UInt(SCHEMA_VERSION)),
            (
                Value::Text("graph".to_owned()),
                Value::Bytes(self.graph.as_bytes().to_vec()),
            ),
            (Value::Text("triples".to_owned()), Value::Array(triples)),
        ];
        if let Some(source) = self.source_record {
            entries.push((
                Value::Text("source_record".to_owned()),
                Term::Object(ObjectRef::Envelope(source)).to_value()?,
            ));
        }
        Ok(Value::Map(entries))
    }

    /// The exact payload bytes the envelope carries.
    ///
    /// # Errors
    ///
    /// As [`GraphRecord::to_value`], or an encoding error.
    pub fn payload_bytes(&self) -> Result<Vec<u8>, RecordError> {
        Ok(encode_canonical(&self.to_value()?)?)
    }

    /// Reads a record from its exact payload bytes.
    ///
    /// # Errors
    ///
    /// [`RecordError`] for non-canonical bytes, an unknown or missing
    /// field, another schema version, too many triples or a bad term.
    pub fn from_payload(bytes: &[u8]) -> Result<Self, RecordError> {
        let value = decode_canonical(bytes)?;
        let Value::Map(entries) = &value else {
            return Err(field("record", "must be a map"));
        };
        for (key, _) in entries {
            let Value::Text(name) = key else {
                return Err(field("record", "keys must be text"));
            };
            if !FIELDS.contains(&name.as_str()) {
                return Err(RecordError::Cbor(CborError::UnknownCriticalField {
                    field: name.clone(),
                }));
            }
        }
        match lookup(entries, "v") {
            Some(Value::UInt(SCHEMA_VERSION)) => {}
            Some(Value::UInt(other)) => return Err(RecordError::UnsupportedSchema(*other)),
            Some(_) => return Err(field("v", "must be an unsigned integer")),
            None => return Err(field("v", "is missing")),
        }
        let graph = match lookup(entries, "graph") {
            Some(Value::Bytes(bytes)) => <[u8; DIGEST_BYTES]>::try_from(bytes.as_slice())
                .map(LogicalGraphId::new)
                .map_err(|_error| field("graph", "must be 32 bytes"))?,
            Some(_) => return Err(field("graph", "must be bytes")),
            None => return Err(field("graph", "is missing")),
        };
        let triples = match lookup(entries, "triples") {
            Some(Value::Array(items)) if items.len() > MAX_TRIPLES => {
                return Err(RecordError::TooManyTriples(items.len()));
            }
            Some(Value::Array(items)) => items
                .iter()
                .map(Triple::from_value)
                .collect::<Result<Vec<_>, _>>()?,
            Some(_) => return Err(field("triples", "must be an array")),
            None => return Err(field("triples", "is missing")),
        };
        let source_record = match lookup(entries, "source_record") {
            None => None,
            Some(value) => match Term::from_value(value)? {
                Term::Object(ObjectRef::Envelope(id)) => Some(id),
                _ => return Err(field("source_record", "must be an envelope reference")),
            },
        };
        Ok(Self {
            graph,
            triples,
            source_record,
        })
    }

    /// Rejects a record that names `own` envelope or the payload it is
    /// carried as. An ID never appears in its own preimage.
    ///
    /// # Errors
    ///
    /// [`RecordError::SelfReference`].
    pub fn check_not_self_referential(
        &self,
        own: &EnvelopeId,
        payload: &[u8],
    ) -> Result<(), RecordError> {
        // Hash the payload once per profile, not once per reference.
        let own_payload: Vec<ContentId> = HashProfile::ALL
            .iter()
            .filter_map(|profile| ContentId::of_bytes(*profile, payload).ok())
            .collect();
        for reference in self.references() {
            let id = match reference {
                ObjectRef::Envelope(id) => *id.content_id(),
                ObjectRef::Content(id) => id,
            };
            if id == *own.content_id() || own_payload.contains(&id) {
                return Err(RecordError::SelfReference);
            }
        }
        Ok(())
    }

    /// Signs the record as a new envelope under [`GRAPH_RECORD_DOMAIN`].
    ///
    /// # Errors
    ///
    /// A payload or envelope rejection, or
    /// [`RecordError::SelfReference`].
    pub fn sign(
        &self,
        context: &str,
        signer_id: &str,
        key: &SigningKey,
    ) -> Result<OpenedRecord, RecordError> {
        let payload = self.payload_bytes()?;
        let body = Body {
            domain: GRAPH_RECORD_DOMAIN.to_owned(),
            context: context.to_owned(),
            subject: None,
            graph: Some(self.graph),
            predecessor: None,
            signer_id: signer_id.to_owned(),
            key: key.verifying_key().to_bytes(),
            suite: SUITE_ED25519.to_owned(),
            payload_id: PayloadId::of(RECORD_ID_PROFILE, &payload).map_err(EnvelopeError::from)?,
            payload_length: payload.len() as u64,
        };
        let envelope = Envelope::sign(body, payload, key)?;
        let id = envelope.id(RECORD_ID_PROFILE)?;
        self.check_not_self_referential(&id, &envelope.payload)?;
        Ok(OpenedRecord {
            id,
            record: self.clone(),
            envelope,
        })
    }
}

/// Verifies `envelope` as a graph record and decodes its payload.
///
/// # Errors
///
/// [`RecordError`]: a wrong domain, an envelope that does not verify,
/// a bad payload, a graph that differs between body and payload, or a
/// self-reference.
pub fn open(envelope: Envelope, dispatch: &ProfileDispatch) -> Result<OpenedRecord, RecordError> {
    if envelope.body.domain != GRAPH_RECORD_DOMAIN {
        return Err(RecordError::WrongDomain(envelope.body.domain));
    }
    envelope.verify(dispatch)?;
    let record = GraphRecord::from_payload(&envelope.payload)?;
    if envelope.body.graph != Some(record.graph) {
        return Err(RecordError::GraphMismatch);
    }
    let id = envelope.id(RECORD_ID_PROFILE)?;
    record.check_not_self_referential(&id, &envelope.payload)?;
    Ok(OpenedRecord {
        id,
        record,
        envelope,
    })
}

impl OpenedRecord {
    /// The record's triples with their provenance, in signed order.
    pub fn assertions(&self) -> impl Iterator<Item = Assertion<'_>> + '_ {
        self.record
            .triples
            .iter()
            .zip(0_u32..)
            .map(|(triple, index)| Assertion {
                record: self.id,
                index,
                graph: self.record.graph,
                triple,
                source_record: self.record.source_record,
            })
    }
}
