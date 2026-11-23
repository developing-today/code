//! One in-toto-shaped attestation statement per ingest batch (#6045).
//!
//! This is the **attestation-marked claim shape** the publication design
//! leaves open in §8.1 (exact body bytes of `attestation.claim`): the
//! statement bytes are embedded verbatim as the claim body, with the
//! statement hash as the claim's subject — no transcoding, so Atlas and
//! ID attestations are byte-comparable. It is also the shape the
//! attested-only consumer view preset keys on (#13766): a published
//! version is attested exactly when the pinned snapshot contains a
//! `tag.set` claim named `atlas.wasGeneratedBy` whose subject is that
//! version's document and whose value names an `attestation.claim` event
//! whose body is one of these statements (predicate decision recorded
//! on #13766 under FDR-0013; a rejected batch writes no corpus rows, so
//! no published version can carry a rejected batch's attestation).
//!
//! Field-for-field this is #6045 §2: the in-toto Statement v1 envelope
//! (`_type`, `predicateType`), `subject[]` = the chunk/span rows the
//! batch wrote, `materials[]` = the source bytes the batch read with
//! verbatim #6044 marks, `predicate.builder` = ingester name + version +
//! code rev, `predicate.batch` = batch identity + outcome,
//! `predicate.classification` = tier + scan verdict, and
//! `predicate.prior_attestation` = the previous batch's envelope digest
//! for the same source, or null. The envelope digest is the
//! attestation's primary key (`attestation_digest`): `sha256` over the
//! canonical statement bytes, per #6045 §2 — never stored in the
//! statement itself, always recomputed. The DSSE envelope
//! (`payloadType: application/vnd.in-toto+json`, `signatures: []`) is
//! adopted with empty signatures: signing is optional for this system
//! (#6045 §6) and a future signer fills the reserved array without a
//! schema change.
//!
//! Scope is deliberately the shape only. No registry table, no gate
//! wiring, no emission point lives here: the `ingest_attestations`
//! table (#6045 §3) and the five #6057 stage hooks (#6045 §4) are later
//! slices, and the projector's batch-statement → claim mapping
//! (#11183) consumes [`BatchStatement::canonical_bytes`] as its body
//! bytes. A statement exists only sealed — `finished_at` and `outcome`
//! are required — because #6045 §4 seals exactly once, at publish or
//! at rejection, and a rejected batch seals with an empty `subject[]`.

use serde::{Deserialize, Serialize};

use crate::classify::{canonical_json, sha256_hex};
use crate::error::{Error, Result};
use crate::shards::Tier;

/// The in-toto Statement v1 type URI, recorded verbatim on every
/// attestation (#6045 §2).
pub const STATEMENT_TYPE: &str = "https://in-toto.io/Statement/v1";

/// The Atlas predicate type for an ingest batch, versioned: a future
/// predicate change mints `v2`, never edits `v1` rows (#6045 §2).
pub const PREDICATE_TYPE: &str = "https://atlas.local/ingestBatch/v1";

/// The DSSE envelope payload type for a statement (#6045 §2).
pub const ENVELOPE_PAYLOAD_TYPE: &str = "application/vnd.in-toto+json";

/// Whether a sealed batch published its rows or was refused: the
/// `outcome` of `predicate.batch` (#6045 §2). A rejected batch never
/// touches the corpus, so its statement carries an empty `subject[]`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BatchOutcome {
    Published,
    Rejected,
    Quarantined,
}

impl BatchOutcome {
    /// The outcome as the statement spells it.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            BatchOutcome::Published => "published",
            BatchOutcome::Rejected => "rejected",
            BatchOutcome::Quarantined => "quarantined",
        }
    }
}

/// A `sha256` digest object: the only digest shape a statement uses
/// (#6045 §2 — sha256 everywhere, no negotiation).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestSet {
    /// Lowercase hex SHA-256.
    pub sha256: String,
}

impl DigestSet {
    /// A digest object for `digest`, refusing anything that is not a
    /// lowercase hex SHA-256.
    ///
    /// # Errors
    ///
    /// [`Error::Fatal`] when `digest` is not 64 lowercase hex characters.
    pub fn new(digest: &str) -> Result<Self> {
        if !is_sha256_hex(digest) {
            return Err(refuse(format!(
                "attestation: digest {digest:?} is not a lowercase hex SHA-256"
            )));
        }
        Ok(Self {
            sha256: digest.to_owned(),
        })
    }
}

/// One chunk/span row the batch wrote: the statement's `subject[]`
/// entry (#6045 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectEntry {
    /// The row's stable locator (`<shard>:<chunk_id>` once promoted;
    /// `<staging_unit_id>:<item_id>:<idx>` pre-promotion).
    pub uri: String,
    /// The row's content hash (#3125 CAS hash).
    pub digest: DigestSet,
    /// The span locators from #3125's occurrence model.
    pub annotations: SubjectAnnotations,
}

/// The annotations on a subject row: its span locators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubjectAnnotations {
    /// Span locators (line numbers / dates / spans for extracts).
    pub spans: Vec<String>,
}

/// One source the batch read: the statement's `materials[]` entry
/// (#6045 §2). The `uri` is the source-mark string verbatim in #6044's
/// grammar, copied never reinterpreted; the content hash rides beside
/// it so the material stays resolvable even if the source is later lost.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MaterialEntry {
    /// The source-mark string, verbatim.
    pub uri: String,
    /// Hash of the source bytes as ingested, before chunking.
    pub digest: DigestSet,
}

/// The ingester that ran the batch: `predicate.builder` (#6045 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Builder {
    /// Ingester name, from #6019's controlled vocabulary.
    pub id: String,
    /// Ingester version + repo git hash, per the fleet's versioning ruling.
    pub version: BuilderVersion,
}

/// The ingester build behind a batch.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuilderVersion {
    /// Binary/tool version.
    pub ingester_version: String,
    /// `git rev-parse HEAD` of the checkout that performed the ingest.
    pub code_rev: String,
}

/// Batch identity: `predicate.batch` (#6045 §2). Joins to #5669's
/// `staging_units` on `staging_unit_id` and to the #6057 gate run on
/// `gate_run_id`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Batch {
    /// The batch's id.
    pub batch_id: String,
    /// The staging unit the batch ran in.
    pub staging_unit_id: String,
    /// The gate run that judged the batch.
    pub gate_run_id: String,
    /// When the batch entered the gate (ISO timestamp).
    pub started_at: String,
    /// When the batch sealed, at publish or at rejection (ISO timestamp).
    pub finished_at: String,
    /// What the batch's fate was.
    pub outcome: BatchOutcome,
    /// Hashes of subject candidates dropped as duplicates at the dedup
    /// stage (#6045 §4): they never reach `subject[]`.
    pub dedup_dropped: Vec<String>,
}

/// The classification scan's verdict: `predicate.classification`
/// (#6045 §2). For a rejected batch this is where the refusing verdict
/// lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Classification {
    /// The three-level core tier, as assigned by the #6057 classify stage.
    #[serde(with = "tier_serde")]
    pub tier: Tier,
    /// The classification machinery that produced the verdict.
    pub scanner_name: String,
    /// The scanner's version.
    pub scanner_version: String,
    /// The scan's verdict.
    pub scan_verdict: String,
    /// A content hash or pointer to the scan output, not the output itself.
    pub scan_evidence: String,
}

/// The statement's predicate: everything the in-toto predicate type
/// carries for an ingest batch (#6045 §2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchPredicate {
    /// The ingester that ran the batch.
    pub builder: Builder,
    /// The batch's identity and fate.
    pub batch: Batch,
    /// The classification tier + scan results.
    pub classification: Classification,
    /// The envelope digest of the prior batch's attestation for the same
    /// source, or null for a first ingest (#6045 §5). A rejected batch
    /// counts as a link: a retry chains to the rejection.
    pub prior_attestation: Option<String>,
}

/// One in-toto-shaped attestation statement for an ingest batch
/// (#6045 §2). The statement's envelope digest
/// ([`BatchStatement::envelope_digest`]) is its primary key; the
/// statement itself never carries it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BatchStatement {
    /// The in-toto Statement v1 type URI; always [`STATEMENT_TYPE`].
    #[serde(rename = "_type")]
    pub statement_type: String,
    /// The Atlas predicate type; always [`PREDICATE_TYPE`].
    #[serde(rename = "predicateType")]
    pub predicate_type: String,
    /// The chunk/span rows the batch wrote. Empty for a pure rejection.
    pub subject: Vec<SubjectEntry>,
    /// The source bytes the batch read.
    pub materials: Vec<MaterialEntry>,
    /// The batch predicate.
    pub predicate: BatchPredicate,
}

/// The DSSE envelope carrying a statement (#6045 §2): `payloadType`,
/// `payload` = the statement, `signatures` empty in v1.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatementEnvelope {
    /// Always [`ENVELOPE_PAYLOAD_TYPE`].
    #[serde(rename = "payloadType")]
    pub payload_type: String,
    /// The statement.
    pub payload: BatchStatement,
    /// Empty today; a future signer fills it without a schema change.
    pub signatures: Vec<EnvelopeSignature>,
}

/// One DSSE envelope signature. Unused in v1; the shape is reserved so
/// signing needs no migration (#6045 §6).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EnvelopeSignature {
    /// The signing key's id.
    #[serde(rename = "keyid")]
    pub key_id: String,
    /// The signature bytes, base64.
    pub sig: String,
}

/// Serde for [`Tier`] as its lowercase name (`public` / `internal` /
/// `secret`): the tier has no serde impl of its own, and the statement
/// must spell the tier the way #6045 §2 does.
mod tier_serde {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    use super::Tier;

    pub fn serialize<S>(tier: &Tier, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        tier.name().serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Tier, D::Error>
    where
        D: Deserializer<'de>,
    {
        let name = String::deserialize(deserializer)?;
        Tier::parse(&name).ok_or_else(|| serde::de::Error::custom(format!("unknown tier {name:?}")))
    }
}

fn refuse(msg: impl std::fmt::Display) -> Error {
    Error::fatal(msg.to_string())
}

fn is_sha256_hex(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

fn non_empty(field: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return Err(refuse(format!("attestation: {field} must not be empty")));
    }
    Ok(())
}

impl BatchStatement {
    /// A statement for `subject` and `materials` under `predicate`,
    /// with the §2 type URIs filled in.
    #[must_use]
    pub fn new(
        subject: Vec<SubjectEntry>,
        materials: Vec<MaterialEntry>,
        predicate: BatchPredicate,
    ) -> Self {
        Self {
            statement_type: STATEMENT_TYPE.to_owned(),
            predicate_type: PREDICATE_TYPE.to_owned(),
            subject,
            materials,
            predicate,
        }
    }

    /// The statement's field checks: the §2 type URIs verbatim, every
    /// digest a lowercase hex SHA-256, every id and timestamp present.
    /// Called by [`BatchStatement::canonical_bytes`], so bytes are only
    /// ever produced for a well-formed statement.
    ///
    /// # Errors
    ///
    /// [`Error::Fatal`] naming the first field that fails a check.
    pub fn validate(&self) -> Result<()> {
        if self.statement_type != STATEMENT_TYPE {
            return Err(refuse(format!(
                "attestation: _type must be {STATEMENT_TYPE:?}"
            )));
        }
        if self.predicate_type != PREDICATE_TYPE {
            return Err(refuse(format!(
                "attestation: predicateType must be {PREDICATE_TYPE:?}"
            )));
        }
        for entry in &self.subject {
            non_empty("subject uri", &entry.uri)?;
            if !is_sha256_hex(&entry.digest.sha256) {
                return Err(refuse(format!(
                    "attestation: subject digest {:?} is not a lowercase hex SHA-256",
                    entry.digest.sha256
                )));
            }
        }
        for entry in &self.materials {
            non_empty("material uri", &entry.uri)?;
            if !is_sha256_hex(&entry.digest.sha256) {
                return Err(refuse(format!(
                    "attestation: material digest {:?} is not a lowercase hex SHA-256",
                    entry.digest.sha256
                )));
            }
        }
        let predicate = &self.predicate;
        non_empty("builder id", &predicate.builder.id)?;
        non_empty(
            "builder ingester_version",
            &predicate.builder.version.ingester_version,
        )?;
        non_empty("builder code_rev", &predicate.builder.version.code_rev)?;
        non_empty("batch id", &predicate.batch.batch_id)?;
        non_empty("staging unit id", &predicate.batch.staging_unit_id)?;
        non_empty("gate run id", &predicate.batch.gate_run_id)?;
        non_empty("batch started_at", &predicate.batch.started_at)?;
        non_empty("batch finished_at", &predicate.batch.finished_at)?;
        for digest in &predicate.batch.dedup_dropped {
            if !is_sha256_hex(digest) {
                return Err(refuse(format!(
                    "attestation: dedup_dropped digest {digest:?} is not a lowercase hex SHA-256"
                )));
            }
        }
        non_empty("scanner name", &predicate.classification.scanner_name)?;
        non_empty("scanner version", &predicate.classification.scanner_version)?;
        non_empty("scan verdict", &predicate.classification.scan_verdict)?;
        non_empty("scan evidence", &predicate.classification.scan_evidence)?;
        if let Some(prior) = &predicate.prior_attestation {
            if !is_sha256_hex(prior) {
                return Err(refuse(format!(
                    "attestation: prior_attestation {prior:?} is not a lowercase hex SHA-256"
                )));
            }
        }
        Ok(())
    }

    /// The canonical statement bytes: the statement as JSON with object
    /// keys sorted at every depth, so the bytes — and therefore the
    /// envelope digest — never depend on field order (#6045 §2: JCS /
    /// OLPC canonical form, fixed at v1).
    ///
    /// # Errors
    ///
    /// [`Error::Fatal`] when the statement fails [`BatchStatement::validate`].
    pub fn canonical_bytes(&self) -> Result<Vec<u8>> {
        self.validate()?;
        let value = serde_json::to_value(self)
            .map_err(|err| refuse(format!("attestation: statement would not serialize: {err}")))?;
        Ok(canonical_json(&value).into_bytes())
    }

    /// The envelope digest: `sha256` over the canonical statement bytes
    /// (#6045 §2). This is the attestation's primary key
    /// (`attestation_digest`) and the digest a later statement names in
    /// `prior_attestation`.
    ///
    /// # Errors
    ///
    /// As [`BatchStatement::canonical_bytes`].
    pub fn envelope_digest(&self) -> Result<String> {
        Ok(sha256_hex(&self.canonical_bytes()?))
    }

    /// The DSSE envelope carrying this statement, with empty signatures
    /// (v1 is unsigned, #6045 §6).
    #[must_use]
    pub fn to_envelope(&self) -> StatementEnvelope {
        StatementEnvelope {
            payload_type: ENVELOPE_PAYLOAD_TYPE.to_owned(),
            payload: self.clone(),
            signatures: Vec::new(),
        }
    }

    /// Decodes a statement from canonical bytes. Only canonical bytes
    /// decode: the decoded statement is re-canonicalized and must
    /// reproduce the input byte for byte, so a reordered, pretty-printed,
    /// or duplicate-key spelling is refused rather than silently accepted.
    ///
    /// # Errors
    ///
    /// [`Error::Fatal`] when the bytes are not JSON, do not decode as a
    /// statement, fail validation, or are not the statement's canonical
    /// spelling.
    pub fn from_canonical_bytes(bytes: &[u8]) -> Result<Self> {
        let value: serde_json::Value = serde_json::from_slice(bytes)
            .map_err(|err| refuse(format!("attestation: statement bytes are not JSON: {err}")))?;
        let statement: BatchStatement = serde_json::from_value(value)
            .map_err(|err| refuse(format!("attestation: statement bytes do not decode: {err}")))?;
        statement.validate()?;
        if statement.canonical_bytes()? != bytes {
            return Err(refuse(
                "attestation: statement bytes are not the canonical spelling",
            ));
        }
        Ok(statement)
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::shards::Tier;

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn predicate(outcome: BatchOutcome) -> BatchPredicate {
        BatchPredicate {
            builder: Builder {
                id: "atlas-ingest".to_owned(),
                version: BuilderVersion {
                    ingester_version: "0.1.0".to_owned(),
                    code_rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                },
            },
            batch: Batch {
                batch_id: "batch-1".to_owned(),
                staging_unit_id: "unit-1".to_owned(),
                gate_run_id: "gate-1".to_owned(),
                started_at: "2026-11-23T00:00:00Z".to_owned(),
                finished_at: "2026-11-23T00:01:00Z".to_owned(),
                outcome,
                dedup_dropped: Vec::new(),
            },
            classification: Classification {
                tier: Tier::Public,
                scanner_name: "atlas-classify".to_owned(),
                scanner_version: "0.1.0".to_owned(),
                scan_verdict: "pass".to_owned(),
                scan_evidence: digest(0xaa),
            },
            prior_attestation: None,
        }
    }

    fn subject(uri: &str, byte: u8) -> SubjectEntry {
        SubjectEntry {
            uri: uri.to_owned(),
            digest: DigestSet {
                sha256: digest(byte),
            },
            annotations: SubjectAnnotations {
                spans: vec!["L1-L9".to_owned()],
            },
        }
    }

    fn material(uri: &str, byte: u8) -> MaterialEntry {
        MaterialEntry {
            uri: uri.to_owned(),
            digest: DigestSet {
                sha256: digest(byte),
            },
        }
    }

    /// #6045 §2: the statement round-trips through its canonical bytes,
    /// and the envelope digest is the sha256 of those bytes.
    #[test]
    fn statement_round_trips_and_digest_is_sha256_of_canonical_bytes() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![material("repo@abc:path/to/file", 0x22)],
            predicate(BatchOutcome::Published),
        );
        let bytes = statement.canonical_bytes().unwrap();
        assert_eq!(
            BatchStatement::from_canonical_bytes(&bytes).unwrap(),
            statement
        );
        let statement_digest = statement.envelope_digest().unwrap();
        assert_eq!(statement_digest.len(), 64);
        assert!(statement_digest.bytes().all(|b| b.is_ascii_hexdigit()));
        assert_eq!(statement_digest, sha256_hex(&bytes));
        // The digest is deterministic: same statement, same bytes, same digest.
        assert_eq!(statement.envelope_digest().unwrap(), statement_digest);
        assert_eq!(statement.canonical_bytes().unwrap(), bytes);
    }

    /// #6045 §2: the digest never depends on field order — a statement
    /// re-serialized from a reordered JSON object has identical bytes.
    #[test]
    fn digest_is_stable_under_key_reordering() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![material("repo@abc:path/to/file", 0x22)],
            predicate(BatchOutcome::Published),
        );
        let bytes = statement.canonical_bytes().unwrap();
        let reordered: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        // Rebuild the object with its keys in reverse order, then
        // re-canonicalize: the bytes must be identical.
        let reversed = match reordered {
            serde_json::Value::Object(map) => {
                let mut pairs: Vec<(String, serde_json::Value)> = map.into_iter().collect();
                pairs.reverse();
                serde_json::Value::Object(pairs.into_iter().collect())
            }
            other => other,
        };
        assert_eq!(canonical_json(&reversed).into_bytes(), bytes);
    }

    /// Only canonical bytes decode: pretty-printed and duplicate-key
    /// spellings of the same statement are refused.
    #[test]
    fn non_canonical_bytes_are_refused() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![],
            predicate(BatchOutcome::Published),
        );
        let bytes = statement.canonical_bytes().unwrap();
        let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        let pretty = serde_json::to_string_pretty(&value).unwrap();
        assert!(
            BatchStatement::from_canonical_bytes(pretty.as_bytes()).is_err(),
            "pretty-printed bytes must not decode"
        );
        // A duplicate key collapses on parse, so the re-canonicalized
        // bytes differ from the input and the input is refused.
        let mut duplicate = bytes.clone();
        let insert_at = duplicate
            .iter()
            .position(|b| *b == b',')
            .expect("canonical bytes contain a comma");
        let extra = format!(r#""_type":"{STATEMENT_TYPE}","#);
        duplicate.splice(insert_at + 1..insert_at + 1, extra.bytes());
        assert!(
            BatchStatement::from_canonical_bytes(&duplicate).is_err(),
            "duplicate-key bytes must not decode"
        );
    }

    /// #6045 §4: a rejected batch seals with an empty subject and still
    /// round-trips; its digest differs from the published twin, so the
    /// outcome is part of what the digest binds.
    #[test]
    fn rejected_batch_seals_with_empty_subject() {
        let rejected = BatchStatement::new(
            vec![],
            vec![material("repo@abc:path", 0x22)],
            predicate(BatchOutcome::Rejected),
        );
        let bytes = rejected.canonical_bytes().unwrap();
        assert_eq!(
            BatchStatement::from_canonical_bytes(&bytes).unwrap(),
            rejected
        );
        let published = BatchStatement::new(
            vec![],
            vec![material("repo@abc:path", 0x22)],
            predicate(BatchOutcome::Published),
        );
        assert_ne!(
            rejected.envelope_digest().unwrap(),
            published.envelope_digest().unwrap()
        );
    }

    /// #6045 §5: the chain link is the prior batch's envelope digest, or
    /// null; anything else is refused before bytes are produced.
    #[test]
    fn prior_attestation_must_be_a_digest() {
        let first = BatchStatement::new(vec![], vec![], predicate(BatchOutcome::Published));
        assert_eq!(first.predicate.prior_attestation, None);
        let first_digest = first.envelope_digest().unwrap();

        let mut next_predicate = predicate(BatchOutcome::Published);
        next_predicate.prior_attestation = Some(first_digest.clone());
        let chained = BatchStatement::new(vec![], vec![], next_predicate);
        assert_eq!(
            chained.predicate.prior_attestation.as_deref(),
            Some(first_digest.as_str())
        );
        assert!(chained.canonical_bytes().is_ok());

        let mut broken_predicate = predicate(BatchOutcome::Published);
        broken_predicate.prior_attestation = Some("not-a-digest".to_owned());
        let broken = BatchStatement::new(vec![], vec![], broken_predicate);
        assert!(broken.canonical_bytes().is_err());
        assert!(broken.envelope_digest().is_err());
    }

    /// #6045 §6: the v1 envelope is unsigned — empty signatures, the
    /// in-toto payload type, and the statement as payload.
    #[test]
    fn envelope_is_unsigned_v1() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![],
            predicate(BatchOutcome::Published),
        );
        let envelope = statement.to_envelope();
        assert_eq!(envelope.payload_type, ENVELOPE_PAYLOAD_TYPE);
        assert!(envelope.signatures.is_empty());
        assert_eq!(envelope.payload, statement);
        let json = serde_json::to_value(&envelope).unwrap();
        assert_eq!(json["payloadType"], ENVELOPE_PAYLOAD_TYPE);
        assert_eq!(json["signatures"], serde_json::Value::Array(vec![]));
    }

    /// Validation fails closed: a wrong type URI or a malformed digest
    /// produces no bytes.
    #[test]
    fn validation_fails_closed() {
        let mut statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![],
            predicate(BatchOutcome::Published),
        );
        statement.statement_type = "https://example.invalid/Statement/v9".to_owned();
        assert!(statement.canonical_bytes().is_err());

        let mut statement = BatchStatement::new(
            vec![SubjectEntry {
                uri: "shard-0:chunk-1".to_owned(),
                digest: DigestSet {
                    sha256: "xyz".to_owned(),
                },
                annotations: SubjectAnnotations { spans: vec![] },
            }],
            vec![],
            predicate(BatchOutcome::Published),
        );
        assert!(statement.canonical_bytes().is_err());
        statement.subject[0].digest.sha256 = "A".repeat(64);
        assert!(
            statement.canonical_bytes().is_err(),
            "uppercase hex is not a lowercase digest"
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Oracle: canonical round-trip + digest determinism hold for
        /// any subject/material rows the emitter could produce.
        #[test]
        fn canonical_round_trip_holds_for_any_rows(
            rows in prop::collection::vec((0u8..8, 0u64..1000), 0..4),
        ) {
            let subject: Vec<SubjectEntry> = rows
                .iter()
                .map(|(shard, n)| SubjectEntry {
                    uri: format!("shard-{shard}:chunk-{n}"),
                    digest: DigestSet { sha256: format!("{n:064x}") },
                    annotations: SubjectAnnotations {
                        spans: vec![format!("L{n}")],
                    },
                })
                .collect();
            let materials: Vec<MaterialEntry> = rows
                .iter()
                .map(|(shard, n)| MaterialEntry {
                    uri: format!("repo@abc:shard-{shard}/file-{n}"),
                    digest: DigestSet { sha256: format!("{n:064x}") },
                })
                .collect();
            let statement = BatchStatement::new(subject, materials, predicate(BatchOutcome::Published));
            let bytes = statement.canonical_bytes().unwrap();
            prop_assert_eq!(BatchStatement::from_canonical_bytes(&bytes).unwrap(), statement);
            prop_assert_eq!(statement.envelope_digest().unwrap(), sha256_hex(&bytes));
        }
    }
}
