//! Draft / published journal facts per program version (#13742;
//! creator cross-world v2 outline,
//! `doc/2026-11-23T21-00-00Z_design_id6309_creator_cross_world_v2_outline.md`
//! sections 2 and 6 item 2).
//!
//! Each program version a world holds has exactly two states, recorded
//! as a durable fact in the world that owns it:
//!
//! - **draft** — the version exists locally and has not been offered
//!   for publication;
//! - **published** — a signed bundle for exactly this version exists
//!   and was offered for publication under #6278. The fact is *keyed*
//!   to the #6278 record: it carries the bundle's
//!   [`BundleRecord`] and only validates while that record verifies
//!   and names this program version's identity. "Published" does not
//!   mean a hosted gallery entry exists (outline section 1).
//!
//! There is deliberately no third state. Scheduling is a property of
//! an announcement (slice 3, #13743), never of the program, so a
//! schedule cannot change what a program *is* (outline section 2).
//!
//! The fact is carried as a world record (see [`crate::world_records`]):
//! one record per program version under [`RECORD_KEY_PREFIX`], its
//! value the fact's JSON encoding. Records are a projection of world
//! state, so the fact replicates and rebuilds exactly like every other
//! record, and a peer with no network reads the same state as a hosted
//! node (outline section 3).
//!
//! # Migration note (pre-fact worlds)
//!
//! Worlds and programs that predate this fact have no
//! `program-publish/` records. Absence is meaningful and safe: a
//! program version with no fact reads as **draft**, the state every
//! pre-fact version was in fact in — nothing was ever offered for
//! publication on its behalf, and draft is the only reading that
//! cannot overclaim. No backfill is written: absence stays absence
//! until an explicit state is recorded, reads never mutate the
//! records, and no pre-existing record is touched, so the migration
//! loses no data by construction. The receipt-side scan
//! ([`crate::world_records::program_publish_facts`]) applies the same
//! rule to replicated records.

use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};

use crate::program_bundle::{BundleRecord, ProgramIdentity};
use crate::world::{MAX_RECORD_VALUE_BYTES, Records};

/// The record-key prefix every publish-state fact lives under. One
/// record per program version: the key is this prefix followed by the
/// version's D1 identity (author, escaped name, version, content
/// hash), so facts enumerate by prefix and never collide across
/// versions.
pub const RECORD_KEY_PREFIX: &str = "program-publish/";

/// The two states a program version can be in. There is no third:
/// scheduling lives on the announcement record, not here.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProgramPublishState {
    /// Exists locally; not offered for publication.
    Draft,
    /// A signed bundle for this version was offered for publication
    /// under #6278, keyed to the record the fact carries.
    Published,
}

/// One program version's draft / published fact.
///
/// A `published` fact carries the #6278 publication record it is
/// keyed to ([`BundleRecord`]: the signed manifest and its bundle
/// signature, design D6). The record is the authority for the
/// published claim — per outline section 4 question 5, "published at"
/// comes from the signed bundle's publication record, not the local
/// clock — so the fact stores no timestamp of its own.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProgramPublishFact {
    /// The program version this fact is about (D1 identity).
    pub program: ProgramIdentity,
    /// The version's current state.
    pub state: ProgramPublishState,
    /// The #6278 record the published state is keyed to. Present if
    /// and only if `state` is [`ProgramPublishState::Published`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub publication: Option<BundleRecord>,
}

impl ProgramPublishFact {
    /// A draft fact for `program`: no bundle has been offered for
    /// publication.
    #[must_use]
    pub fn draft(program: ProgramIdentity) -> Self {
        Self {
            program,
            state: ProgramPublishState::Draft,
            publication: None,
        }
    }

    /// A published fact for `program`, keyed to the #6278 record that
    /// publication produced.
    ///
    /// # Errors
    ///
    /// The record fails verification, or it names a different program
    /// version than `program` — a published fact may only ever be
    /// keyed to its own version's record.
    pub fn published(program: ProgramIdentity, record: BundleRecord) -> Result<Self> {
        let fact = Self {
            program,
            state: ProgramPublishState::Published,
            publication: Some(record),
        };
        fact.validate()?;
        Ok(fact)
    }

    /// Check the fact's internal consistency: the state and the
    /// record agree, and a carried record verifies against its own
    /// signature and names this program version.
    ///
    /// # Errors
    ///
    /// A draft carrying a record, a published fact without one, a
    /// record that does not verify, or a record naming another
    /// program version.
    pub fn validate(&self) -> Result<()> {
        match (&self.state, &self.publication) {
            (ProgramPublishState::Draft, Some(_)) => {
                bail!("a draft fact carries no publication record")
            }
            (ProgramPublishState::Draft, None) => Ok(()),
            (ProgramPublishState::Published, None) => {
                bail!("a published fact must carry its #6278 publication record")
            }
            (ProgramPublishState::Published, Some(record)) => {
                let manifest = record
                    .verify()
                    .context("the publication record does not verify")?;
                ensure!(
                    manifest.identity() == self.program,
                    "the publication record names {:?}, not this fact's program {:?}",
                    manifest.identity(),
                    self.program
                );
                Ok(())
            }
        }
    }
}

/// Escape a program name for the record key: `%` first, then `/`, so
/// the key's four identity segments stay unambiguous for any name the
/// bundle checks accept.
fn escape_name(name: &str) -> String {
    name.replace('%', "%25").replace('/', "%2F")
}

/// The world-record key carrying `program`'s publish-state fact.
#[must_use]
pub fn record_key(program: &ProgramIdentity) -> String {
    format!(
        "{RECORD_KEY_PREFIX}{}/{}/{}/{}",
        program.author,
        escape_name(&program.name),
        program.version,
        program.content_hash
    )
}

/// Parse and validate one fact from its record value.
///
/// # Errors
///
/// The value is not a well-formed fact, or it fails
/// [`ProgramPublishFact::validate`].
pub fn parse_fact(value: &serde_json::Value) -> Result<ProgramPublishFact> {
    let fact: ProgramPublishFact = serde_json::from_value(value.clone())
        .context("the publish-state record value is not a well-formed fact")?;
    fact.validate()?;
    Ok(fact)
}

/// Record `fact` in `records`, replacing any fact for the same
/// program version. This is the write side; the world program's
/// projection republishes the records from state as usual.
///
/// # Errors
///
/// The fact fails validation, or its encoding exceeds the record
/// value cap (a manifest near the bundle format's own limit cannot
/// ride in a record; the refusal names the sizes).
pub fn record_fact(records: &mut Records, fact: &ProgramPublishFact) -> Result<()> {
    fact.validate()?;
    let value = serde_json::to_value(fact).context("encoding the publish-state fact")?;
    let size = serde_json::to_vec(&value).map_or(usize::MAX, |v| v.len());
    ensure!(
        size <= MAX_RECORD_VALUE_BYTES,
        "the publish-state fact for {:?} encodes to {size} bytes; \
         the record value limit is {MAX_RECORD_VALUE_BYTES}",
        fact.program
    );
    records.insert(record_key(&fact.program), value);
    Ok(())
}

/// The fact recorded for `program`, if any.
///
/// # Errors
///
/// A record exists under the version's key but is malformed, fails
/// validation, or names a different program version (a key collision
/// must surface, never silently read as another version's state).
pub fn fact_in(records: &Records, program: &ProgramIdentity) -> Result<Option<ProgramPublishFact>> {
    let Some(value) = records.get(&record_key(program)) else {
        return Ok(None);
    };
    let fact = parse_fact(value)?;
    ensure!(
        fact.program == *program,
        "the publish-state record under {:?}'s key names {:?}",
        program,
        fact.program
    );
    Ok(Some(fact))
}

/// `program`'s current state in `records`.
///
/// The migration default: a version with no recorded fact — every
/// pre-fact world, and any version nobody has stated a state for —
/// reads as [`ProgramPublishState::Draft`]. See the module's
/// migration note.
///
/// # Errors
///
/// A recorded fact for this version is malformed or invalid
/// ([`fact_in`]).
pub fn state_in(records: &Records, program: &ProgramIdentity) -> Result<ProgramPublishState> {
    Ok(fact_in(records, program)?.map_or(ProgramPublishState::Draft, |fact| fact.state))
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use ed25519_dalek::SigningKey;

    use super::*;
    use crate::program_bundle::{ProgramSpec, export};

    fn author() -> SigningKey {
        SigningKey::from_bytes(&[7_u8; 32])
    }

    fn bundle(name: &str, version: u64, payload: &[u8]) -> crate::program_bundle::ProgramBundle {
        export(
            payload.to_vec(),
            ProgramSpec {
                name,
                version,
                licence: None,
                description: None,
                categories: &[],
            },
            &author(),
        )
        .unwrap()
    }

    #[test]
    fn draft_is_recorded_with_no_published_bundle() {
        // Failing-first case 1: a draft is a complete fact on its own;
        // no bundle, no record, nothing published anywhere.
        let program = bundle("quiz", 1, b"quiz payload v1").identity();
        let mut records = Records::new();
        record_fact(&mut records, &ProgramPublishFact::draft(program.clone())).unwrap();

        let fact = fact_in(&records, &program).unwrap().unwrap();
        assert_eq!(fact.state, ProgramPublishState::Draft);
        assert_eq!(fact.publication, None);
        assert_eq!(
            state_in(&records, &program).unwrap(),
            ProgramPublishState::Draft
        );

        // The fact is per version: the next version has no fact yet
        // and reads as draft by the same rule, not as a copy of v1's.
        let v2 = ProgramIdentity {
            version: 2,
            ..program.clone()
        };
        assert_eq!(fact_in(&records, &v2).unwrap(), None);
        assert_eq!(state_in(&records, &v2).unwrap(), ProgramPublishState::Draft);
    }

    #[test]
    fn published_is_recorded_once_the_6278_record_exists() {
        // Failing-first case 2: publication happens under #6278 and
        // produces the bundle record; only then can a published fact
        // be recorded, keyed to that record.
        let bundle = bundle("quiz", 1, b"quiz payload v1");
        let program = bundle.identity();
        let record = bundle.record();

        let fact = ProgramPublishFact::published(program.clone(), record.clone()).unwrap();
        let mut records = Records::new();
        record_fact(&mut records, &fact).unwrap();

        let read = fact_in(&records, &program).unwrap().unwrap();
        assert_eq!(read.state, ProgramPublishState::Published);
        assert_eq!(read.publication, Some(record));
        assert_eq!(
            state_in(&records, &program).unwrap(),
            ProgramPublishState::Published
        );
    }

    #[test]
    fn published_refuses_a_record_for_another_program_or_a_forgery() {
        let quiz = bundle("quiz", 1, b"quiz payload v1");
        let other = bundle("other", 1, b"other payload");
        // A real record, but for a different program version.
        let err = ProgramPublishFact::published(quiz.identity(), other.record()).unwrap_err();
        assert!(err.to_string().contains("names"), "{err:#}");
        // This program's record with a tampered signature.
        let mut forged = quiz.record();
        forged.bundle_signature = "00".repeat(64);
        let err = ProgramPublishFact::published(quiz.identity(), forged).unwrap_err();
        assert!(err.to_string().contains("does not verify"), "{err:#}");
        // A draft carrying a record is not a state either.
        let mut draft = ProgramPublishFact::draft(quiz.identity());
        draft.publication = Some(quiz.record());
        assert!(draft.validate().is_err());
    }

    #[test]
    fn pre_fact_world_reads_as_draft_and_loses_nothing() {
        // Migration case: a world whose records predate this fact.
        // Every program reads as draft, and the read changes nothing.
        let mut records = Records::new();
        records.insert("count".to_owned(), serde_json::json!(3));
        records.insert("owner/name".to_owned(), serde_json::json!("quiz hall"));
        let before = records.clone();

        let program = bundle("quiz", 1, b"quiz payload v1").identity();
        assert_eq!(fact_in(&records, &program).unwrap(), None);
        assert_eq!(
            state_in(&records, &program).unwrap(),
            ProgramPublishState::Draft
        );
        assert_eq!(
            crate::world_records::program_publish_facts(&records)
                .unwrap()
                .len(),
            0
        );
        assert_eq!(records, before, "reading state must not mutate the records");
    }

    #[test]
    fn recorded_facts_enumerate_through_the_records_scan() {
        let draft_program = bundle("drafty", 3, b"draft payload").identity();
        let published_bundle = bundle("quiz", 1, b"quiz payload v1");
        let mut records = Records::new();
        record_fact(
            &mut records,
            &ProgramPublishFact::draft(draft_program.clone()),
        )
        .unwrap();
        record_fact(
            &mut records,
            &ProgramPublishFact::published(published_bundle.identity(), published_bundle.record())
                .unwrap(),
        )
        .unwrap();
        records.insert("count".to_owned(), serde_json::json!(3));

        let facts = crate::world_records::program_publish_facts(&records).unwrap();
        assert_eq!(facts.len(), 2);
        assert!(
            facts
                .iter()
                .any(|f| f.program == draft_program && f.state == ProgramPublishState::Draft)
        );
        assert!(
            facts
                .iter()
                .any(|f| f.program == published_bundle.identity()
                    && f.state == ProgramPublishState::Published)
        );
    }
}
