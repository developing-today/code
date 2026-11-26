//! The blob plane's central rows (code#13318, design §8 to §10).
//!
//! Three pieces of the object-plane lifecycle are transactions on the
//! shared central primary, so they are ordered against each other per
//! partition:
//!
//! - **Publication mark.** [`mark_published`] records a verified object as
//!   `published` in `kv_blobs`. It is the service's verification step; only
//!   it writes that table, and only in that state.
//! - **Publish-CAS.** [`publish_cas`] is the key-value CAS for a value that
//!   is a `kv:ref-v1` reference, plus the reference registration in
//!   `kv_blob_refs`, in one transaction. It is decided only while the blob
//!   is published, so a CAS landing after a fenced delete records nothing
//!   and returns [`PublishCas::NotPublished`]: the writer stages the bytes
//!   again (a re-upload, not data loss).
//! - **Fenced delete.** [`fenced_delete`] re-verifies, inside the delete
//!   transaction, that no root names the content, then removes the row
//!   under the partition's next epoch and writes the audit row.
//!
//! [`roots`] loads a partition's GC roots as a [`crate::plane::Roots`], and
//! [`gc_pass`] runs one mark, clear and delete pass over a partition, with
//! candidacy kept in `kv_blob_candidates` so grace survives between passes.

use std::collections::BTreeSet;

use crate::blob::{KvBlobRefV1, Partition};
use crate::central::{self, CentralError, Intent, Op, Pipeline, Receipt, Result, Statement, Value};
use crate::plane::{Deletion, Roots, Sweep};

/// Record a verified object as published in its partition. Publishing the
/// same content again keeps the first row. While a purge holds the content
/// nothing is recorded: [`BLOB_PUBLISHED_READ`] then finds no row.
pub(crate) const BLOB_PUBLISH: &str = "INSERT INTO kv_blobs (
        jurisdiction, tier, content_id, bytes_len, locator, state, published_at_ms)
    SELECT ?1, ?2, ?3, ?4, ?5, 'published', ?6
    WHERE NOT EXISTS (SELECT 1 FROM kv_blob_purges AS purge
        WHERE purge.jurisdiction = ?1 AND purge.tier = ?2 AND purge.content_id = ?3)
    ON CONFLICT (jurisdiction, tier, content_id) DO NOTHING";

/// The published row for one content id, if there is one.
pub(crate) const BLOB_PUBLISHED_READ: &str = "SELECT bytes_len FROM kv_blobs
    WHERE jurisdiction = ?1 AND tier = ?2 AND content_id = ?3";

/// Claim up to ?5 (none when NULL) deleted content ids whose bytes are not purged yet, under
/// attempt ?6 at ?3. Content a published row names again is skipped (its
/// bytes are live), and so is content another purge claimed at or after
/// ?4; an older claim is taken over.
pub(crate) const BLOB_PURGE_CLAIM: &str = "INSERT OR REPLACE INTO kv_blob_purges (
        jurisdiction, tier, content_id, attempt, claimed_ms)
    SELECT DISTINCT deletion.jurisdiction, deletion.tier, deletion.content_id, ?6, ?3
    FROM kv_blob_deletions AS deletion
    WHERE deletion.jurisdiction = ?1 AND deletion.tier = ?2
        AND deletion.purged_at_ms IS NULL
        AND NOT EXISTS (SELECT 1 FROM kv_blobs AS blob
            WHERE blob.jurisdiction = ?1 AND blob.tier = ?2
                AND blob.content_id = deletion.content_id)
        AND NOT EXISTS (SELECT 1 FROM kv_blob_purges AS purge
            WHERE purge.jurisdiction = ?1 AND purge.tier = ?2
                AND purge.content_id = deletion.content_id AND purge.claimed_ms >= ?4)
    ORDER BY deletion.content_id
    LIMIT IFNULL(?5, 0)
    RETURNING content_id";

/// The bytes are gone: mark every unpurged deletion of the content, only
/// under this attempt's claim.
pub(crate) const BLOB_PURGED_MARK: &str = "UPDATE kv_blob_deletions SET purged_at_ms = ?4
    WHERE jurisdiction = ?1 AND tier = ?2 AND content_id = ?3 AND purged_at_ms IS NULL
        AND EXISTS (SELECT 1 FROM kv_blob_purges AS purge
            WHERE purge.jurisdiction = ?1 AND purge.tier = ?2
                AND purge.content_id = ?3 AND purge.attempt = ?5)";

/// Drop this attempt's claim, after a purge or instead of one.
pub(crate) const BLOB_PURGE_RELEASE: &str = "DELETE FROM kv_blob_purges
    WHERE jurisdiction = ?1 AND tier = ?2 AND content_id = ?3 AND attempt = ?5";

/// Register the reference a publish-CAS wrote, for either outcome: a
/// committed head and a proposed conflict body are both roots.
pub(crate) const BLOB_REF_INSERT: &str = "INSERT OR IGNORE INTO kv_blob_refs (
        jurisdiction, tier, content_id, writer, host_incarnation, outbox_id,
        scope_id, registry_id, namespace, key, registered_at_ms)
    SELECT ?14, ?15, ?16, ?8, ?9, ?10, ?1, ?2, ?3, ?4, ?13
    FROM idem_receipts AS receipt
    WHERE receipt.writer = ?8 AND receipt.host_incarnation = ?9
        AND receipt.outbox_id = ?10 AND receipt.attempt = ?12";

/// Every content id a root names in one partition.
pub(crate) const BLOB_ROOTS_READ: &str = "SELECT DISTINCT content_id FROM kv_blob_refs
    WHERE jurisdiction = ?1 AND tier = ?2 ORDER BY content_id";

/// Every published object in one partition.
pub(crate) const BLOBS_READ: &str =
    "SELECT content_id, bytes_len, locator, published_at_ms FROM kv_blobs
    WHERE jurisdiction = ?1 AND tier = ?2 ORDER BY content_id";

/// The fence: an audit row under the partition's next epoch, written only
/// when the object is published and no root names it.
pub(crate) const BLOB_DELETION_INSERT: &str = "INSERT INTO kv_blob_deletions (
        jurisdiction, tier, epoch, content_id, bytes_len,
        candidate_since_ms, deleted_at_ms, attempt)
    SELECT ?1, ?2,
        COALESCE((SELECT MAX(epoch) FROM kv_blob_deletions
            WHERE jurisdiction = ?1 AND tier = ?2), 0) + 1,
        blob.content_id, blob.bytes_len, ?4, ?5, ?6
    FROM kv_blobs AS blob
    WHERE blob.jurisdiction = ?1 AND blob.tier = ?2 AND blob.content_id = ?3
        AND NOT EXISTS (SELECT 1 FROM kv_blob_refs AS root
            WHERE root.jurisdiction = ?1 AND root.tier = ?2 AND root.content_id = ?3)";

/// The delete itself, only under this attempt's fence row.
pub(crate) const BLOB_DELETE: &str = "DELETE FROM kv_blobs
    WHERE jurisdiction = ?1 AND tier = ?2 AND content_id = ?3
        AND EXISTS (SELECT 1 FROM kv_blob_deletions AS fence
            WHERE fence.jurisdiction = ?1 AND fence.tier = ?2
                AND fence.content_id = ?3 AND fence.attempt = ?6)";

/// This attempt's fence row, if the delete happened.
pub(crate) const BLOB_DELETION_READ: &str = "SELECT epoch, deleted_at_ms FROM kv_blob_deletions
    WHERE jurisdiction = ?1 AND tier = ?2 AND content_id = ?3 AND attempt = ?6";

/// Drop candidacy for content a root names again, or that is gone.
pub(crate) const BLOB_CANDIDATES_CLEAR: &str = "DELETE FROM kv_blob_candidates
    WHERE jurisdiction = ?1 AND tier = ?2
        AND (EXISTS (SELECT 1 FROM kv_blob_refs AS root
                WHERE root.jurisdiction = ?1 AND root.tier = ?2
                    AND root.content_id = kv_blob_candidates.content_id)
            OR NOT EXISTS (SELECT 1 FROM kv_blobs AS blob
                WHERE blob.jurisdiction = ?1 AND blob.tier = ?2
                    AND blob.content_id = kv_blob_candidates.content_id))
    RETURNING content_id";

/// Mark every published, unrooted object not yet a candidate, as of ?3.
pub(crate) const BLOB_CANDIDATES_MARK: &str = "INSERT OR IGNORE INTO kv_blob_candidates (
        jurisdiction, tier, content_id, since_ms)
    SELECT blob.jurisdiction, blob.tier, blob.content_id, ?3
    FROM kv_blobs AS blob
    WHERE blob.jurisdiction = ?1 AND blob.tier = ?2
        AND NOT EXISTS (SELECT 1 FROM kv_blob_refs AS root
            WHERE root.jurisdiction = ?1 AND root.tier = ?2
                AND root.content_id = blob.content_id)
    RETURNING content_id";

/// Candidates whose grace ran out by ?3 (candidacy began at or before it).
pub(crate) const BLOB_CANDIDATES_DUE: &str = "SELECT content_id, since_ms FROM kv_blob_candidates
    WHERE jurisdiction = ?1 AND tier = ?2 AND since_ms <= ?3 ORDER BY content_id";

/// A fenced delete drops the candidacy, only under this attempt's fence.
pub(crate) const BLOB_CANDIDATE_DROP: &str = "DELETE FROM kv_blob_candidates
    WHERE jurisdiction = ?1 AND tier = ?2 AND content_id = ?3
        AND EXISTS (SELECT 1 FROM kv_blob_deletions AS fence
            WHERE fence.jurisdiction = ?1 AND fence.tier = ?2
                AND fence.content_id = ?3 AND fence.attempt = ?6)";

/// A published object as central records it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Published {
    pub content_id: String,
    pub bytes_len: u64,
    pub locator: String,
    pub published_at_ms: i64,
}

/// How a publish-CAS ended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PublishCas {
    /// The CAS was decided and the reference registered as a root.
    Decided(Receipt),
    /// The blob is not published in its partition (never marked, or
    /// deleted under a fence): nothing was written. Stage it again.
    NotPublished,
}

/// The GC roots of one partition, as loaded from central.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct CentralRoots {
    pub partition: Option<Partition>,
    pub content_ids: BTreeSet<String>,
}

impl Roots for CentralRoots {
    fn is_referenced(&self, partition: &Partition, digest: &str) -> bool {
        self.partition.as_ref() == Some(partition) && self.content_ids.contains(digest)
    }
}

fn partition_args(partition: &Partition) -> [Value; 2] {
    [
        central::text(&partition.jurisdiction),
        central::text(&partition.tier),
    ]
}

/// Mark `reference`'s object published in its partition. `false` when a
/// purge of the same content holds it: nothing was recorded, so stage the
/// bytes again once the purge is done.
pub fn mark_published(primary: &dyn Pipeline, reference: &KvBlobRefV1) -> Result<bool> {
    let [jurisdiction, tier] = partition_args(&reference.partition);
    let args = [
        jurisdiction,
        tier,
        central::text(&reference.digest),
        central::integer(reference.bytes_len),
        central::text(&reference.locator),
        Value::Integer(central::now_ms()),
    ];
    let results = primary.transaction(&[
        Statement::numbered(BLOB_PUBLISH, &args),
        Statement::numbered(BLOB_PUBLISHED_READ, &args),
    ])?;
    Ok(results.last().is_some_and(|rows| !rows.rows.is_empty()))
}

/// How long a purge claim holds before another purge may take it over.
pub const PURGE_LEASE_MS: i64 = 60 * 60 * 1000;

/// The content ids one purge claimed, under its attempt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PurgeClaim {
    pub partition: Partition,
    pub attempt: String,
    pub content_ids: Vec<String>,
}

/// Claim up to `limit` deleted content ids of `partition` whose bytes are
/// still to be removed. Until the claim is finished or released, no upload
/// may publish the same content again.
pub fn purge_claim(
    primary: &dyn Pipeline,
    partition: &Partition,
    now_ms: i64,
    limit: u32,
) -> Result<PurgeClaim> {
    let [jurisdiction, tier] = partition_args(partition);
    let attempt = central::attempt_nonce();
    let args = [
        jurisdiction,
        tier,
        Value::Integer(now_ms),
        Value::Integer(now_ms.saturating_sub(PURGE_LEASE_MS)),
        Value::Integer(i64::from(limit)),
        central::text(&attempt),
    ];
    let results = primary.transaction(&[Statement::numbered(BLOB_PURGE_CLAIM, &args)])?;
    let content_ids = results
        .first()
        .map(|rows| {
            rows.rows
                .iter()
                .filter_map(|row| row.first().and_then(Value::as_str).map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    Ok(PurgeClaim {
        partition: partition.clone(),
        attempt,
        content_ids,
    })
}

/// Finish one claimed content id: when `purged`, its deletions are marked
/// purged; either way the claim is released.
pub fn purge_finish(
    primary: &dyn Pipeline,
    claim: &PurgeClaim,
    content_id: &str,
    purged: bool,
    now_ms: i64,
) -> Result<()> {
    let [jurisdiction, tier] = partition_args(&claim.partition);
    let args = [
        jurisdiction,
        tier,
        central::text(content_id),
        Value::Integer(now_ms),
        central::text(&claim.attempt),
    ];
    let mut statements = Vec::with_capacity(2);
    if purged {
        statements.push(Statement::numbered(BLOB_PURGED_MARK, &args));
    }
    statements.push(Statement::numbered(BLOB_PURGE_RELEASE, &args));
    primary.transaction(&statements)?;
    Ok(())
}

/// The statements of one publish-CAS attempt: the CAS, gated on the blob
/// being published, plus the root registration, receipt read last.
pub fn publish_cas_statements(
    intent: &Intent,
    reference: &KvBlobRefV1,
    attempt: &str,
) -> Vec<Statement> {
    let mut args = central::cas_args(intent, attempt);
    args.extend(partition_args(&reference.partition));
    args.push(central::text(&reference.digest));
    vec![
        Statement::numbered(central::BLOB_RECEIPT_INSERT, &args),
        Statement::numbered(central::BLOB_CONFLICT_INSERT, &args),
        Statement::numbered(central::BLOB_VERSION_INSERT, &args),
        Statement::numbered(central::HEAD_UPSERT, &args),
        Statement::numbered(BLOB_REF_INSERT, &args),
        Statement::numbered(central::RECEIPT_READ, &args),
    ]
}

/// One publish-CAS: `intent` must set the encoded `reference` as its value.
pub fn publish_cas(
    primary: &dyn Pipeline,
    intent: &Intent,
    reference: &KvBlobRefV1,
) -> Result<PublishCas> {
    central::validate(&intent.address)?;
    let Op::Set(value) = &intent.op else {
        return Err(CentralError::Malformed("publish_cas_not_a_reference"));
    };
    if KvBlobRefV1::decode(value).ok().as_ref() != Some(reference) {
        return Err(CentralError::Malformed("publish_cas_not_a_reference"));
    }
    let attempt = central::attempt_nonce();
    let statements = publish_cas_statements(intent, reference, &attempt);
    Ok(
        match central::decide(primary, intent, &statements, &attempt)? {
            Some(receipt) => PublishCas::Decided(receipt),
            None => PublishCas::NotPublished,
        },
    )
}

/// [`publish_cas`], retried with backoff while the primary is unavailable,
/// as [`central::cas_with_retry`] retries a plain CAS. Each attempt carries
/// a fresh nonce under the same idempotency key, so a retry after a lost
/// answer replays the decided receipt.
pub fn publish_cas_with_retry(
    primary: &dyn Pipeline,
    intent: &Intent,
    reference: &KvBlobRefV1,
    sleep: &mut dyn FnMut(std::time::Duration),
) -> Result<PublishCas> {
    central::with_retry(intent, sleep, || publish_cas(primary, intent, reference))
}

/// Load the GC roots of `partition`.
pub fn roots(primary: &dyn Pipeline, partition: &Partition) -> Result<CentralRoots> {
    let rows = central::read(primary, BLOB_ROOTS_READ, &partition_args(partition))?;
    Ok(CentralRoots {
        partition: Some(partition.clone()),
        content_ids: rows
            .iter()
            .filter_map(|row| row.first().map(central::string))
            .collect(),
    })
}

/// Every published object in `partition`.
pub fn published(primary: &dyn Pipeline, partition: &Partition) -> Result<Vec<Published>> {
    let rows = central::read(primary, BLOBS_READ, &partition_args(partition))?;
    Ok(rows
        .iter()
        .map(|row| {
            let field = |index: usize| row.get(index).cloned().unwrap_or(Value::Null);
            Published {
                content_id: central::string(&field(0)),
                bytes_len: central::unsigned(&field(1)),
                locator: central::string(&field(2)),
                published_at_ms: field(3).as_i64().unwrap_or(0),
            }
        })
        .collect())
}

/// Delete `content_id` from `partition` under a new fence epoch, after
/// re-verifying in the same transaction that no root names it. `None` when
/// a root appeared (candidacy is cleared) or the object is already gone.
pub fn fenced_delete(
    primary: &dyn Pipeline,
    partition: &Partition,
    content_id: &str,
    candidate_since_ms: i64,
) -> Result<Option<Deletion>> {
    let [jurisdiction, tier] = partition_args(partition);
    let attempt = central::attempt_nonce();
    let args = [
        jurisdiction,
        tier,
        central::text(content_id),
        Value::Integer(candidate_since_ms),
        Value::Integer(central::now_ms()),
        central::text(&attempt),
    ];
    let results = primary.transaction(&[
        Statement::numbered(BLOB_DELETION_INSERT, &args),
        Statement::numbered(BLOB_DELETE, &args),
        Statement::numbered(BLOB_CANDIDATE_DROP, &args),
        Statement::numbered(BLOB_DELETION_READ, &args),
    ])?;
    let Some(row) = results.last().and_then(|rows| rows.rows.first()) else {
        return Ok(None);
    };
    let field = |index: usize| row.get(index).cloned().unwrap_or(Value::Null);
    Ok(Some(Deletion {
        partition: partition.clone(),
        digest: content_id.to_string(),
        epoch: central::unsigned(&field(0)),
        candidate_since_ms,
        reverified_at_ms: field(1).as_i64().unwrap_or(0),
    }))
}

/// One GC pass over `partition` at `now_ms`, with `grace_ms` of grace.
///
/// First, in one transaction, candidacy is cleared for content a root names
/// again (or that is gone), and every published object no root names
/// becomes a candidate as of `now_ms`. Then each candidate whose grace has
/// run out is fenced-deleted; [`fenced_delete`] re-verifies the roots in its
/// own transaction, so a reference that lands between the two steps keeps
/// the object (and the next pass clears its candidacy). Candidacy lives on
/// the primary, so grace counts from the first pass that saw the object
/// unrooted, whichever process ran it.
pub fn gc_pass(
    primary: &dyn Pipeline,
    partition: &Partition,
    now_ms: i64,
    grace_ms: i64,
) -> Result<Sweep> {
    let [jurisdiction, tier] = partition_args(partition);
    let marked_at = [jurisdiction.clone(), tier.clone(), Value::Integer(now_ms)];
    let results = primary.transaction(&[
        Statement::numbered(BLOB_CANDIDATES_CLEAR, &marked_at),
        Statement::numbered(BLOB_CANDIDATES_MARK, &marked_at),
    ])?;
    let count = |index: usize| results.get(index).map_or(0, |rows| rows.rows.len());
    let mut sweep = Sweep {
        marked: count(1),
        cleared: count(0),
        deleted: Vec::new(),
    };
    let due = central::read(
        primary,
        BLOB_CANDIDATES_DUE,
        &[
            jurisdiction,
            tier,
            Value::Integer(now_ms.saturating_sub(grace_ms)),
        ],
    )?;
    for row in due {
        let content_id = row.first().map(central::string).unwrap_or_default();
        let since_ms = row.get(1).and_then(Value::as_i64).unwrap_or(now_ms);
        if let Some(deletion) = fenced_delete(primary, partition, &content_id, since_ms)? {
            sweep.deleted.push(deletion);
        }
    }
    Ok(sweep)
}

#[cfg(test)]
#[allow(clippy::expect_used, clippy::unwrap_used)]
mod tests {
    use super::*;
    use crate::blob;
    use crate::central::{Address, IdempotencyKey, Outcome, SqlitePrimary};

    fn primary() -> SqlitePrimary {
        let primary = SqlitePrimary::in_memory().unwrap();
        central::migrate(&primary).unwrap();
        primary
    }

    fn eu() -> Partition {
        Partition::new("eu", "standard")
    }

    fn reference(bytes: &[u8]) -> KvBlobRefV1 {
        let digest = blob::digest(&mut &bytes[..]).unwrap();
        KvBlobRefV1 {
            digest: digest.digest,
            bytes_len: digest.bytes_len,
            partition: eu(),
            locator: format!("r2:eu/standard/{}", bytes.len()),
        }
    }

    fn intent(key: &str, expected: u64, reference: &KvBlobRefV1, outbox: &str) -> Intent {
        Intent {
            address: Address::new("shared:team", "files", key),
            expected_version: expected,
            op: Op::Set(reference.encode()),
            idempotency: IdempotencyKey {
                writer: "host-a".into(),
                host_incarnation: "inc-1".into(),
                outbox_id: outbox.into(),
            },
        }
    }

    fn committed(outcome: PublishCas) -> u64 {
        match outcome {
            PublishCas::Decided(Receipt {
                outcome: Outcome::Committed { version },
                ..
            }) => version,
            other => panic!("expected a commit, got {other:?}"),
        }
    }

    #[test]
    fn blob_publish_cas_heads_carry_the_reference_envelope() {
        let primary = primary();
        let reference = reference(b"a large value");
        mark_published(&primary, &reference).unwrap();
        let first = intent("k", 0, &reference, "1");
        committed(publish_cas(&primary, &first, &reference).unwrap());
        let head = central::head(&primary, &first.address).unwrap().unwrap();
        assert_eq!(head.envelope, blob::REF_ENVELOPE);
        assert_eq!(
            head.envelope().unwrap(),
            blob::Envelope::Ref(reference.clone())
        );
        // A plain CAS over it is inline bytes again, even when those bytes
        // spell a reference: the envelope, not the bytes, decides.
        let plain = Intent {
            expected_version: 1,
            idempotency: IdempotencyKey {
                outbox_id: "2".into(),
                ..first.idempotency.clone()
            },
            ..first.clone()
        };
        central::cas(&primary, &plain).unwrap();
        let head = central::head(&primary, &first.address).unwrap().unwrap();
        assert_eq!(head.envelope, blob::RAW_ENVELOPE);
        assert_eq!(
            head.envelope().unwrap(),
            blob::Envelope::Raw(reference.encode())
        );
        // A lost publish-CAS keeps its body as a reference proposal.
        let stale = intent("k", 0, &reference, "3");
        let PublishCas::Decided(Receipt {
            outcome: Outcome::Conflict { conflict_id, .. },
            ..
        }) = publish_cas(&primary, &stale, &reference).unwrap()
        else {
            panic!("expected a conflict");
        };
        let rows = primary
            .transaction_unlisted(&[Statement::numbered(
                "SELECT proposed_envelope FROM kv_conflicts WHERE id = ?1",
                &[Value::Integer(conflict_id)],
            )])
            .unwrap();
        assert_eq!(rows[0].rows[0][0].as_str(), Some(blob::REF_ENVELOPE));
        // Every head of the scope reads with its envelope.
        let listed =
            central::heads(&primary, "shared:team", central::REGISTRY_ID, "files").unwrap();
        assert_eq!(listed[0].1.envelope, blob::RAW_ENVELOPE);
    }

    #[test]
    fn blob_publish_cas_needs_a_published_object() {
        let primary = primary();
        let reference = reference(b"a large value");
        let refused = publish_cas(&primary, &intent("k", 0, &reference, "1"), &reference).unwrap();
        assert_eq!(refused, PublishCas::NotPublished);
        assert!(
            central::head(&primary, &intent("k", 0, &reference, "1").address)
                .unwrap()
                .is_none()
        );
        assert!(roots(&primary, &eu()).unwrap().content_ids.is_empty());

        mark_published(&primary, &reference).unwrap();
        let version =
            committed(publish_cas(&primary, &intent("k", 0, &reference, "1"), &reference).unwrap());
        assert_eq!(version, 1);
        let head = central::head(&primary, &intent("k", 0, &reference, "1").address)
            .unwrap()
            .expect("head");
        assert_eq!(head.version, 1);
        assert!(
            roots(&primary, &eu())
                .unwrap()
                .is_referenced(&eu(), &reference.digest)
        );
        assert!(
            !roots(&primary, &eu())
                .unwrap()
                .is_referenced(&Partition::new("us", "standard"), &reference.digest)
        );
    }

    #[test]
    fn blob_publish_cas_refuses_a_value_that_is_not_the_reference() {
        let primary = primary();
        let reference = reference(b"bytes");
        let mut wrong = intent("k", 0, &reference, "1");
        wrong.op = Op::Set(b"raw".to_vec());
        let refused = publish_cas(&primary, &wrong, &reference).expect_err("raw value");
        assert_eq!(refused.code(), "publish_cas_not_a_reference");
        let other = self::reference(b"other bytes");
        let refused =
            publish_cas(&primary, &intent("k", 0, &other, "1"), &reference).expect_err("mismatch");
        assert_eq!(refused.code(), "publish_cas_not_a_reference");
    }

    #[test]
    fn blob_publish_cas_replays_and_conflicts_keep_their_roots() {
        let primary = primary();
        let first = reference(b"first");
        let second = reference(b"second");
        mark_published(&primary, &first).unwrap();
        mark_published(&primary, &second).unwrap();
        committed(publish_cas(&primary, &intent("k", 0, &first, "1"), &first).unwrap());
        // The same idempotency key again replays the recorded receipt.
        match publish_cas(&primary, &intent("k", 0, &first, "1"), &first).unwrap() {
            PublishCas::Decided(receipt) => assert!(receipt.replayed),
            other => panic!("expected a replay, got {other:?}"),
        }
        // A stale expected version conflicts; its proposed ref is a root.
        match publish_cas(&primary, &intent("k", 0, &second, "2"), &second).unwrap() {
            PublishCas::Decided(Receipt {
                outcome: Outcome::Conflict { actual, .. },
                ..
            }) => assert_eq!(actual, 1),
            other => panic!("expected a conflict, got {other:?}"),
        }
        let roots = roots(&primary, &eu()).unwrap();
        assert!(roots.is_referenced(&eu(), &first.digest));
        assert!(roots.is_referenced(&eu(), &second.digest));
        assert_eq!(published(&primary, &eu()).unwrap().len(), 2);
    }

    fn deleted_ids(sweep: &Sweep) -> Vec<String> {
        sweep
            .deleted
            .iter()
            .map(|deletion| deletion.digest.clone())
            .collect()
    }

    #[test]
    fn blob_gc_pass_waits_out_grace_then_fences_only_unrooted_objects() {
        let primary = primary();
        let kept = reference(b"kept");
        let orphan = reference(b"orphan");
        for reference in [&kept, &orphan] {
            mark_published(&primary, reference).unwrap();
        }
        committed(publish_cas(&primary, &intent("k", 0, &kept, "1"), &kept).unwrap());

        let first = gc_pass(&primary, &eu(), 100, 50).unwrap();
        assert_eq!((first.marked, first.cleared), (1, 0));
        assert!(first.deleted.is_empty(), "grace has not started to run");
        // Candidacy is kept: a later pass marks nothing new, and grace
        // still counts from the first pass.
        let early = gc_pass(&primary, &eu(), 149, 50).unwrap();
        assert_eq!((early.marked, early.cleared), (0, 0));
        assert!(early.deleted.is_empty(), "one millisecond short of grace");

        let due = gc_pass(&primary, &eu(), 150, 50).unwrap();
        assert_eq!(deleted_ids(&due), vec![orphan.digest.clone()]);
        assert_eq!(due.deleted[0].candidate_since_ms, 100);
        assert_eq!(due.deleted[0].epoch, 1);
        let left: Vec<_> = published(&primary, &eu())
            .unwrap()
            .into_iter()
            .map(|published| published.content_id)
            .collect();
        assert_eq!(left, vec![kept.digest.clone()]);
        // Nothing is left to do.
        let after = gc_pass(&primary, &eu(), 10_000, 50).unwrap();
        assert_eq!(
            (after.marked, after.cleared, after.deleted.len()),
            (0, 0, 0)
        );
    }

    #[test]
    fn blob_gc_pass_clears_candidacy_when_a_reference_lands_during_grace() {
        let primary = primary();
        let late = reference(b"referenced during grace");
        mark_published(&primary, &late).unwrap();
        assert_eq!(gc_pass(&primary, &eu(), 0, 50).unwrap().marked, 1);

        committed(publish_cas(&primary, &intent("late", 0, &late, "1"), &late).unwrap());
        let cleared = gc_pass(&primary, &eu(), 60, 50).unwrap();
        assert_eq!((cleared.marked, cleared.cleared), (0, 1));
        assert!(
            cleared.deleted.is_empty(),
            "a rooted object is never deleted"
        );
        assert_eq!(published(&primary, &eu()).unwrap().len(), 1);
    }

    #[test]
    fn blob_gc_pass_restarts_grace_for_content_published_again_after_a_delete() {
        let primary = primary();
        let churn = reference(b"published, deleted, published again");
        mark_published(&primary, &churn).unwrap();
        gc_pass(&primary, &eu(), 0, 50).unwrap();
        assert_eq!(gc_pass(&primary, &eu(), 50, 50).unwrap().deleted.len(), 1);

        // The fence dropped the candidacy, so the new object's grace
        // starts at the pass that first sees it, not at the old mark.
        mark_published(&primary, &churn).unwrap();
        let again = gc_pass(&primary, &eu(), 60, 50).unwrap();
        assert_eq!((again.marked, again.deleted.len()), (1, 0));
        assert!(
            gc_pass(&primary, &eu(), 109, 50)
                .unwrap()
                .deleted
                .is_empty()
        );
        let deleted = gc_pass(&primary, &eu(), 110, 50).unwrap();
        assert_eq!(deleted.deleted[0].candidate_since_ms, 60);
        assert_eq!(deleted.deleted[0].epoch, 2);
    }

    #[test]
    fn blob_fenced_delete_reverifies_and_raises_the_epoch() {
        let primary = primary();
        let kept = reference(b"kept");
        let orphan = reference(b"orphan");
        let other = reference(b"another orphan");
        for reference in [&kept, &orphan, &other] {
            mark_published(&primary, reference).unwrap();
        }
        committed(publish_cas(&primary, &intent("k", 0, &kept, "1"), &kept).unwrap());

        // A rooted object survives the fence: re-verify sees its root.
        assert_eq!(
            fenced_delete(&primary, &eu(), &kept.digest, 10).unwrap(),
            None
        );

        let deleted = fenced_delete(&primary, &eu(), &orphan.digest, 10)
            .unwrap()
            .expect("deleted");
        assert_eq!(deleted.epoch, 1);
        assert_eq!(deleted.candidate_since_ms, 10);
        let deleted = fenced_delete(&primary, &eu(), &other.digest, 20)
            .unwrap()
            .expect("deleted");
        assert_eq!(deleted.epoch, 2);
        // Deleting again finds nothing.
        assert_eq!(
            fenced_delete(&primary, &eu(), &orphan.digest, 30).unwrap(),
            None
        );
        let left: Vec<_> = published(&primary, &eu())
            .unwrap()
            .into_iter()
            .map(|published| published.content_id)
            .collect();
        assert_eq!(left, vec![kept.digest.clone()]);
        // Epochs are per partition.
        let us = KvBlobRefV1 {
            partition: Partition::new("us", "standard"),
            ..orphan.clone()
        };
        mark_published(&primary, &us).unwrap();
        let deleted = fenced_delete(&primary, &us.partition, &us.digest, 0)
            .unwrap()
            .expect("deleted");
        assert_eq!(deleted.epoch, 1);
    }

    #[test]
    fn blob_publish_cas_after_the_fence_records_nothing_until_restaged() {
        let primary = primary();
        let raced = reference(b"raced");
        mark_published(&primary, &raced).unwrap();
        fenced_delete(&primary, &eu(), &raced.digest, 0)
            .unwrap()
            .expect("deleted");
        let late = publish_cas(&primary, &intent("k", 0, &raced, "1"), &raced).unwrap();
        assert_eq!(late, PublishCas::NotPublished);
        assert!(
            !roots(&primary, &eu())
                .unwrap()
                .is_referenced(&eu(), &raced.digest)
        );
        // The writer stages again; the service marks it published again.
        mark_published(&primary, &raced).unwrap();
        let version =
            committed(publish_cas(&primary, &intent("k", 0, &raced, "1"), &raced).unwrap());
        assert_eq!(version, 1);
    }

    fn unpurged(primary: &SqlitePrimary) -> Vec<(String, u64)> {
        let rows = primary
            .transaction_unlisted(&[Statement::numbered(
                "SELECT content_id, epoch FROM kv_blob_deletions
                WHERE purged_at_ms IS NULL ORDER BY jurisdiction, tier, epoch",
                &[],
            )])
            .unwrap();
        rows[0]
            .rows
            .iter()
            .map(|row| {
                (
                    row[0].as_str().unwrap().to_string(),
                    central::unsigned(&row[1]),
                )
            })
            .collect()
    }

    #[test]
    fn blob_purge_claims_deleted_content_and_blocks_its_republication() {
        let primary = primary();
        let gone = reference(b"deleted, then purged");
        let live = reference(b"still published");
        assert!(mark_published(&primary, &gone).unwrap());
        assert!(mark_published(&primary, &live).unwrap());
        fenced_delete(&primary, &eu(), &gone.digest, 0)
            .unwrap()
            .expect("deleted");

        // Only deleted content is claimed: published bytes are never purged.
        let claim = purge_claim(&primary, &eu(), 100, 10).unwrap();
        assert_eq!(claim.content_ids, vec![gone.digest.clone()]);
        // A second purge inside the lease claims nothing.
        assert!(
            purge_claim(&primary, &eu(), 101, 10)
                .unwrap()
                .content_ids
                .is_empty()
        );
        // While the claim holds, the content cannot be published again.
        assert!(!mark_published(&primary, &gone).unwrap());

        purge_finish(&primary, &claim, &gone.digest, true, 120).unwrap();
        assert!(unpurged(&primary).is_empty());
        assert!(
            purge_claim(&primary, &eu(), 130, 10)
                .unwrap()
                .content_ids
                .is_empty()
        );
        // Once purged the content publishes again, and a later delete of it
        // is a new deletion to purge.
        assert!(mark_published(&primary, &gone).unwrap());
        assert!(
            purge_claim(&primary, &eu(), 140, 10)
                .unwrap()
                .content_ids
                .is_empty()
        );
        let again = fenced_delete(&primary, &eu(), &gone.digest, 150)
            .unwrap()
            .expect("deleted again");
        assert_eq!(unpurged(&primary), vec![(gone.digest.clone(), again.epoch)]);
        assert_eq!(
            purge_claim(&primary, &eu(), 160, 10).unwrap().content_ids,
            vec![gone.digest.clone()]
        );
    }

    #[test]
    fn blob_purge_release_keeps_the_deletion_and_a_stale_claim_is_taken_over() {
        let primary = primary();
        let orphans: Vec<_> = [&b"one"[..], b"two", b"three"]
            .into_iter()
            .map(reference)
            .collect();
        for orphan in &orphans {
            mark_published(&primary, orphan).unwrap();
            fenced_delete(&primary, &eu(), &orphan.digest, 0)
                .unwrap()
                .expect("deleted");
        }
        let mut digests: Vec<_> = orphans.iter().map(|o| o.digest.clone()).collect();
        digests.sort();

        // The limit bounds one claim; the next claim takes the rest.
        let first = purge_claim(&primary, &eu(), 0, 2).unwrap();
        assert_eq!(first.content_ids, digests[..2].to_vec());
        let second = purge_claim(&primary, &eu(), 1, 2).unwrap();
        assert_eq!(second.content_ids, digests[2..].to_vec());

        // Releasing without a purge keeps the deletion unpurged.
        purge_finish(&primary, &second, &digests[2], false, 2).unwrap();
        assert_eq!(unpurged(&primary).len(), 3);
        // Another attempt's finish does nothing to this claim.
        purge_finish(&primary, &second, &digests[0], true, 3).unwrap();
        assert_eq!(unpurged(&primary).len(), 3);

        // Past the lease, the first claim is taken over; its own finish then
        // marks nothing.
        let later = PURGE_LEASE_MS + 1;
        let taken = purge_claim(&primary, &eu(), later, 10).unwrap();
        assert_eq!(taken.content_ids, digests);
        purge_finish(&primary, &first, &digests[0], true, later).unwrap();
        assert_eq!(unpurged(&primary).len(), 3);
        for digest in &digests {
            purge_finish(&primary, &taken, digest, true, later).unwrap();
        }
        assert!(unpurged(&primary).is_empty());
    }

    #[test]
    fn blob_central_statements_are_in_the_catalog() {
        for sql in [
            central::BLOB_RECEIPT_INSERT,
            BLOB_PUBLISH,
            BLOB_REF_INSERT,
            BLOB_ROOTS_READ,
            BLOBS_READ,
            BLOB_DELETION_INSERT,
            BLOB_DELETE,
            BLOB_DELETION_READ,
            BLOB_PUBLISHED_READ,
            BLOB_PURGE_CLAIM,
            BLOB_PURGED_MARK,
            BLOB_PURGE_RELEASE,
        ] {
            assert!(crate::catalog::find(sql).is_some(), "{sql}");
        }
    }
}
