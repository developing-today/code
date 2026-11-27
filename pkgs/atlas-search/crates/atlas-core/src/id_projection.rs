//! Atlas → ID projector for ingest attestations and batch runs (#11183).
//!
//! This is the §9.2 slice of the publication design
//! (`doc/2026-11-09T07-20-00Z_design_atlas_id_publication/`): §2.4
//! (one `attestation.claim` per batch) with the §8.1 mapping the design
//! leaves to this slice — the #6045 statement bytes embedded verbatim
//! as the claim body, the statement hash as the claim's subject, no
//! transcoding, so Atlas and ID attestations stay byte-comparable (the
//! default §8.1 records, adopted here; a v2 mapping would mint new
//! claim names per §7, never redefine these).
//!
//! Input is the landed statement shape
//! ([`BatchStatement`](crate::attestation::BatchStatement), #13766):
//! [`project_batch`] maps one sealed statement to one claim, and
//! [`project_run`] maps an ordered run of statements while verifying
//! the `prior_attestation` batch chain (#6045 §5) — a retry chains to
//! the rejection it retries, so rejected statements are links, not
//! gaps. Scope is the mapping only: no registry table, no gate
//! wiring, no event signing. The emitted [`ProjectedClaim`] is the
//! unsigned claim content a later publisher signs; every claim carries
//! the projection version (§7), and any claim recomputes from its
//! statement rows alone (§6.4) — the tests name that recomputation as
//! their oracle.
//!
//! Refused batches publish too (§2.4): a rejection is a provenance
//! fact, and the refusing policy + reason ride in the claim body as
//! the statement's `outcome` + `classification` (scanner, verdict,
//! evidence). The projector never pre-filters for consumers (§4): a
//! rejected or quarantined batch projects exactly like a published
//! one.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::attestation::{BatchOutcome, BatchStatement};
use crate::error::{Error, Result};

/// The ID event kind carrying a projected batch attestation (§2.4).
pub const CLAIM_KIND: &str = "attestation.claim";

/// The projection version every emitted claim carries (§7). Format
/// evolution appends new claim names; it never redefines this
/// mapping's meaning.
pub const PROJECTION_VERSION: &str = "v1";

/// One projected `attestation.claim`: the unsigned claim content for a
/// single sealed batch statement (§2.4, §8.1 default).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectedClaim {
    /// Always [`CLAIM_KIND`].
    pub kind: String,
    /// The batch statement's envelope digest (`attestation_digest`,
    /// #6045 §2): `sha256` over the canonical statement bytes,
    /// recomputed here, never copied from the statement (the statement
    /// never carries it).
    pub subject: String,
    /// The statement's canonical bytes verbatim — no transcoding
    /// (§8.1 default).
    pub body: Vec<u8>,
    /// Always [`PROJECTION_VERSION`] (§7).
    pub projection_version: String,
    /// The batch's fate, copied from the statement for consumers that
    /// filter without decoding the body.
    pub outcome: BatchOutcome,
    /// The batch id, copied from the statement for run-log indexing.
    pub batch_id: String,
    /// The chain link this batch claims, copied from the statement.
    pub prior_attestation: Option<String>,
}

/// One projected run: the claims for an ordered run of batch
/// statements plus the batch-chain head (§6.1 replay pin).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectedRun {
    /// One claim per input statement, in input order (§2.4).
    pub claims: Vec<ProjectedClaim>,
    /// The last claim's subject — the chain head a replayer pins
    /// (§6.1). `None` for an empty run.
    pub batch_chain_head: Option<String>,
}

/// Immutable Atlas and publisher inputs needed to reproduce one
/// projection run (§6.1).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRunPins {
    /// Maximum ingested `version_seq` per file at the snapshot boundary.
    pub atlas_snapshot_marker: BTreeMap<String, u64>,
    /// Batch-chain head at the start of this run, if the run continues a
    /// previously published chain.
    pub batch_chain_head: Option<String>,
    /// Stable public key identifier used to attribute emitted claims.
    pub publisher_key_id: String,
    /// Projection mapping version selected for this run.
    pub projection_version: String,
}

/// Replayable record of a projector run. The caller persists this value
/// atomically with its publication unit; without the pins, output alone
/// is not sufficient evidence for replay (§6.1, §6.4).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectionRunLog {
    /// Stable identifier assigned by the caller before projection.
    pub run_id: String,
    /// Complete input pins used for the run.
    pub pins: ProjectionRunPins,
    /// Emissions in source order; a row's document and relations must be
    /// committed as one unit by the publisher (§6.4).
    pub claims: Vec<ProjectedClaim>,
    /// Resulting chain head, distinct from the input head in `pins`.
    pub batch_chain_head: Option<String>,
}

fn refuse(msg: impl std::fmt::Display) -> Error {
    Error::fatal(msg.to_string())
}

/// Maps one sealed batch statement to its `attestation.claim` (§2.4,
/// §8.1 default): subject = recomputed envelope digest, body =
/// canonical statement bytes verbatim.
///
/// # Errors
///
/// [`Error::Fatal`] when the statement fails
/// [`BatchStatement::validate`] (via
/// [`BatchStatement::canonical_bytes`]).
pub fn project_batch(statement: &BatchStatement) -> Result<ProjectedClaim> {
    let body = statement.canonical_bytes()?;
    let subject = statement.envelope_digest()?;
    Ok(ProjectedClaim {
        kind: CLAIM_KIND.to_owned(),
        subject,
        body,
        projection_version: PROJECTION_VERSION.to_owned(),
        outcome: statement.predicate.batch.outcome,
        batch_id: statement.predicate.batch.batch_id.clone(),
        prior_attestation: statement.predicate.prior_attestation.clone(),
    })
}

/// Maps an ordered run of batch statements to their claims, verifying
/// the `prior_attestation` batch chain across the run (#6045 §5):
/// every statement after the first must name the previous
/// statement's envelope digest. The first statement may start a chain
/// (`None`) or continue one sealed in an earlier run (`Some` digest
/// from outside this run — well-formedness is enforced by
/// [`BatchStatement::validate`]; cross-run history is the replayer's
/// pin, §6.1, not this run's to re-derive).
///
/// Fails closed: a broken link refuses the whole run, never a prefix.
///
/// # Errors
///
/// [`Error::Fatal`] when any statement is invalid or any chain link
/// does not name the previous statement's digest.
pub fn project_run(statements: &[BatchStatement]) -> Result<ProjectedRun> {
    let mut claims = Vec::with_capacity(statements.len());
    let mut previous_digest: Option<String> = None;
    for statement in statements {
        let claim = project_batch(statement)?;
        if let Some(expected) = &previous_digest {
            if claim.prior_attestation.as_deref() != Some(expected.as_str()) {
                return Err(refuse(format!(
                    "id_projection: batch {:?} breaks the batch chain: prior_attestation {:?} does not name the previous batch digest {expected:?}",
                    claim.batch_id, claim.prior_attestation,
                )));
            }
        }
        previous_digest = Some(claim.subject.clone());
        claims.push(claim);
    }
    let batch_chain_head = claims.last().map(|claim| claim.subject.clone());
    Ok(ProjectedRun {
        claims,
        batch_chain_head,
    })
}

/// Projects and records a run together with every input needed to replay
/// it (§6.1). The first statement must continue exactly the pinned batch
/// chain head; otherwise the snapshot and batch inputs do not describe a
/// coherent run.
pub fn project_run_with_pins(
    run_id: impl Into<String>,
    statements: &[BatchStatement],
    pins: ProjectionRunPins,
) -> Result<ProjectionRunLog> {
    let run_id = run_id.into();
    if run_id.trim().is_empty() {
        return Err(refuse("id_projection: run id must not be empty"));
    }
    if pins.publisher_key_id.trim().is_empty() {
        return Err(refuse("id_projection: publisher key id must not be empty"));
    }
    if pins.projection_version != PROJECTION_VERSION {
        return Err(refuse(format!(
            "id_projection: unsupported projection version {:?}",
            pins.projection_version
        )));
    }
    if statements
        .first()
        .and_then(|statement| statement.predicate.prior_attestation.as_deref())
        != pins.batch_chain_head.as_deref()
    {
        return Err(refuse(
            "id_projection: first batch does not continue the pinned batch-chain head",
        ));
    }

    let projected = project_run(statements)?;
    Ok(ProjectionRunLog {
        run_id,
        pins,
        claims: projected.claims,
        batch_chain_head: projected.batch_chain_head,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::attestation::{
        Batch, BatchPredicate, Builder, BuilderVersion, Classification, DigestSet, MaterialEntry,
        SubjectAnnotations, SubjectEntry,
    };
    use crate::classify::sha256_hex;
    use crate::shards::Tier;

    fn digest(byte: u8) -> String {
        format!("{byte:02x}").repeat(32)
    }

    fn predicate(batch_id: &str, outcome: BatchOutcome) -> BatchPredicate {
        BatchPredicate {
            builder: Builder {
                id: "atlas-ingest".to_owned(),
                version: BuilderVersion {
                    ingester_version: "0.1.0".to_owned(),
                    code_rev: "0123456789abcdef0123456789abcdef01234567".to_owned(),
                },
            },
            batch: Batch {
                batch_id: batch_id.to_owned(),
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

    fn refused_predicate(batch_id: &str) -> BatchPredicate {
        let mut predicate = predicate(batch_id, BatchOutcome::Rejected);
        predicate.classification.scan_verdict = "refused:policy-violation".to_owned();
        predicate.classification.scan_evidence = digest(0xbb);
        predicate
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

    /// Oracle: the claim recomputes from the Atlas rows — the body
    /// decodes to the input statement and the subject is the sha256 of
    /// the body bytes.
    fn assert_recomputes(claim: &ProjectedClaim, statement: &BatchStatement) {
        assert_eq!(claim.kind, CLAIM_KIND);
        assert_eq!(claim.projection_version, PROJECTION_VERSION);
        assert_eq!(
            BatchStatement::from_canonical_bytes(&claim.body).unwrap(),
            *statement,
            "claim body must decode to the input statement rows"
        );
        assert_eq!(
            claim.subject,
            sha256_hex(&claim.body),
            "claim subject must recompute as sha256 of the body bytes"
        );
        assert_eq!(claim.subject, statement.envelope_digest().unwrap());
        assert_eq!(claim.batch_id, statement.predicate.batch.batch_id);
        assert_eq!(
            claim.prior_attestation,
            statement.predicate.prior_attestation,
        );
    }

    /// §2.4 + §8.1: one published batch projects to one claim whose
    /// subject is the batch digest and whose body is the statement
    /// bytes verbatim.
    #[test]
    fn published_batch_projects_one_claim_per_digest() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![material("repo@abc:path/to/file", 0x22)],
            predicate("batch-1", BatchOutcome::Published),
        );
        let claim = project_batch(&statement).unwrap();
        assert_eq!(claim.outcome, BatchOutcome::Published);
        assert_recomputes(&claim, &statement);
        assert_eq!(claim.body, statement.canonical_bytes().unwrap());
    }

    /// §2.4 refusals: a rejected batch (empty subject, refusing policy
    /// + reason in classification) publishes like any batch — the
    /// policy and reason ride in the claim body, nothing is dropped.
    #[test]
    fn refusal_publishes_with_policy_and_reason_in_body() {
        let statement = BatchStatement::new(
            vec![],
            vec![material("repo@abc:path", 0x22)],
            refused_predicate("batch-rejected-1"),
        );
        let claim = project_batch(&statement).unwrap();
        assert_eq!(claim.outcome, BatchOutcome::Rejected);
        assert_recomputes(&claim, &statement);
        let decoded = BatchStatement::from_canonical_bytes(&claim.body).unwrap();
        assert!(
            decoded.subject.is_empty(),
            "a rejected batch seals with an empty subject"
        );
        assert_eq!(
            decoded.predicate.classification.scan_verdict,
            "refused:policy-violation"
        );
        assert_eq!(decoded.predicate.classification.scan_evidence, digest(0xbb));
    }

    /// Quarantine is a third outcome, not a second rejection: it
    /// projects with its own outcome preserved.
    #[test]
    fn quarantine_projects_with_its_outcome_preserved() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-9", 0x99)],
            vec![material("repo@abc:path", 0x22)],
            predicate("batch-quarantined-1", BatchOutcome::Quarantined),
        );
        let claim = project_batch(&statement).unwrap();
        assert_eq!(claim.outcome, BatchOutcome::Quarantined);
        assert_recomputes(&claim, &statement);
    }

    fn chained(statement: BatchStatement, prior: &str) -> BatchStatement {
        let mut next = statement;
        next.predicate.prior_attestation = Some(prior.to_owned());
        next
    }

    /// §2.4 + #6045 §5: a run projects one claim per statement in
    /// order, the chain links verify (a retry chains to the rejection
    /// it retries), and the run reports the chain head.
    #[test]
    fn batch_chain_tracks_across_runs_including_rejections() {
        let first = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![material("repo@abc:path", 0x22)],
            predicate("batch-1", BatchOutcome::Published),
        );
        let first_digest = first.envelope_digest().unwrap();
        let rejection = chained(
            BatchStatement::new(
                vec![],
                vec![material("repo@abc:path", 0x22)],
                refused_predicate("batch-2"),
            ),
            &first_digest,
        );
        let rejection_digest = rejection.envelope_digest().unwrap();
        let retry = chained(
            BatchStatement::new(
                vec![subject("shard-0:chunk-2", 0x33)],
                vec![material("repo@abc:path", 0x22)],
                predicate("batch-3", BatchOutcome::Published),
            ),
            &rejection_digest,
        );

        let run = project_run(&[first.clone(), rejection.clone(), retry.clone()]).unwrap();
        assert_eq!(run.claims.len(), 3);
        assert_recomputes(&run.claims[0], &first);
        assert_recomputes(&run.claims[1], &rejection);
        assert_recomputes(&run.claims[2], &retry);
        assert_eq!(run.claims[1].outcome, BatchOutcome::Rejected);
        assert_eq!(
            run.claims[1].prior_attestation.as_deref(),
            Some(first_digest.as_str())
        );
        assert_eq!(
            run.claims[2].prior_attestation.as_deref(),
            Some(rejection_digest.as_str()),
            "a retry chains to the rejection it retries"
        );
        assert_eq!(
            run.batch_chain_head.as_deref(),
            Some(run.claims[2].subject.as_str())
        );
    }

    /// A run may continue a chain sealed earlier: the head statement's
    /// prior names a digest outside this run, and the run still
    /// projects.
    #[test]
    fn run_head_may_continue_an_earlier_chain() {
        let earlier = BatchStatement::new(
            vec![subject("shard-0:chunk-0", 0x00)],
            vec![],
            predicate("batch-0", BatchOutcome::Published),
        );
        let earlier_digest = earlier.envelope_digest().unwrap();
        let continued = chained(
            BatchStatement::new(
                vec![subject("shard-0:chunk-1", 0x11)],
                vec![],
                predicate("batch-1", BatchOutcome::Published),
            ),
            &earlier_digest,
        );
        let run = project_run(std::slice::from_ref(&continued)).unwrap();
        assert_eq!(run.claims.len(), 1);
        assert_recomputes(&run.claims[0], &continued);
        assert_eq!(
            run.batch_chain_head.as_deref(),
            Some(run.claims[0].subject.as_str())
        );
    }

    /// Fails closed: a link that does not name the previous batch's
    /// digest refuses the whole run, never a prefix.
    #[test]
    fn broken_chain_refuses_the_whole_run() {
        let first = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![],
            predicate("batch-1", BatchOutcome::Published),
        );
        let mut impostor = predicate("batch-2", BatchOutcome::Published);
        impostor.prior_attestation = Some(digest(0xff));
        let second = BatchStatement::new(vec![], vec![], impostor);
        assert!(project_run(&[first, second]).is_err());
    }

    /// An empty run projects to no claims and no head.
    #[test]
    fn empty_run_projects_to_no_claims() {
        let run = project_run(&[]).unwrap();
        assert!(run.claims.is_empty());
        assert_eq!(run.batch_chain_head, None);
    }

    /// Oracle: a logged run preserves every replay input pin alongside
    /// the exact deterministic emissions, so serialization carries the
    /// same inputs a verifier needs to reproduce the run.
    #[test]
    fn logged_run_persists_replay_pins_and_emissions() {
        let statement = BatchStatement::new(
            vec![subject("shard-0:chunk-1", 0x11)],
            vec![material("repo@abc:path", 0x22)],
            predicate("batch-1", BatchOutcome::Published),
        );
        let pins = ProjectionRunPins {
            atlas_snapshot_marker: BTreeMap::from([("file-1".to_owned(), 7)]),
            batch_chain_head: None,
            publisher_key_id: "publisher-key-1".to_owned(),
            projection_version: PROJECTION_VERSION.to_owned(),
        };

        let log = project_run_with_pins("run-1", &[statement.clone()], pins.clone()).unwrap();
        let encoded = serde_json::to_vec(&log).unwrap();
        let decoded: ProjectionRunLog = serde_json::from_slice(&encoded).unwrap();

        assert_eq!(decoded.run_id, "run-1");
        assert_eq!(decoded.pins, pins);
        assert_eq!(decoded.claims, project_run(&[statement]).unwrap().claims);
        assert_eq!(
            decoded.batch_chain_head,
            decoded.claims.last().map(|claim| claim.subject.clone())
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]

        /// Oracle: any chained run of valid statements projects 1:1 —
        /// every claim recomputes from its statement rows and every
        /// link names the previous digest.
        #[test]
        fn chained_run_projects_one_claim_per_statement(
            outcomes in prop::collection::vec(
                prop::sample::select(vec![
                    BatchOutcome::Published,
                    BatchOutcome::Rejected,
                    BatchOutcome::Quarantined,
                ]),
                1..5,
            ),
        ) {
            let mut statements = Vec::with_capacity(outcomes.len());
            let mut prior: Option<String> = None;
            for (n, outcome) in outcomes.iter().enumerate() {
                let mut predicate =
                    predicate(&format!("batch-{n}"), *outcome);
                predicate.prior_attestation = prior.clone();
                let statement = BatchStatement::new(
                    vec![SubjectEntry {
                        uri: format!("shard-0:chunk-{n}"),
                        digest: DigestSet {
                            sha256: format!("{n:064x}"),
                        },
                        annotations: SubjectAnnotations { spans: vec![] },
                    }],
                    vec![],
                    predicate,
                );
                prior = Some(statement.envelope_digest().unwrap());
                statements.push(statement);
            }
            let run = project_run(&statements).unwrap();
            prop_assert_eq!(run.claims.len(), statements.len());
            for (claim, statement) in run.claims.iter().zip(statements.iter()) {
                prop_assert_eq!(
                    &BatchStatement::from_canonical_bytes(&claim.body).unwrap(),
                    statement
                );
                prop_assert_eq!(claim.subject.as_str(), sha256_hex(&claim.body));
                prop_assert_eq!(claim.subject.as_str(), statement.envelope_digest().unwrap());
            }
            prop_assert_eq!(
                run.batch_chain_head,
                run.claims.last().map(|claim| claim.subject.clone())
            );
        }
    }
}
