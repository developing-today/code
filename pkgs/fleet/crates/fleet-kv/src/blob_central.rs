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
//! [`roots`] loads a partition's GC roots as a [`crate::plane::Roots`].

use std::collections::BTreeSet;

use crate::blob::{KvBlobRefV1, Partition};
use crate::central::{self, CentralError, Intent, Op, Pipeline, Receipt, Result, Statement, Value};
use crate::plane::{Deletion, Roots};

/// Record a verified object as published in its partition. Publishing the
/// same content again keeps the first row.
pub(crate) const BLOB_PUBLISH: &str = "INSERT INTO kv_blobs (
        jurisdiction, tier, content_id, bytes_len, locator, state, published_at_ms)
    VALUES (?1, ?2, ?3, ?4, ?5, 'published', ?6)
    ON CONFLICT (jurisdiction, tier, content_id) DO NOTHING";

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

/// Mark `reference`'s object published in its partition.
pub fn mark_published(primary: &dyn Pipeline, reference: &KvBlobRefV1) -> Result<()> {
    let [jurisdiction, tier] = partition_args(&reference.partition);
    let args = [
        jurisdiction,
        tier,
        central::text(&reference.digest),
        central::integer(reference.bytes_len),
        central::text(&reference.locator),
        Value::Integer(central::now_ms()),
    ];
    primary.transaction(&[Statement::numbered(BLOB_PUBLISH, &args)])?;
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
        ] {
            assert!(crate::catalog::find(sql).is_some(), "{sql}");
        }
    }
}
