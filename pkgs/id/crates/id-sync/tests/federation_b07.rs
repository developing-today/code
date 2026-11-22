//! B07 invariant test: the first-class conflict report (#10248,
//! statements S1.34-S1.38).
//!
//! Oracle: **invariant** — a conflict report is persisted before any
//! resolution, and an unrelated object can continue sync. The
//! reference is an independent state machine in this file: it keeps
//! each object's versions, computes heads by brute force (versions no
//! other version names as a parent) and the common base as the
//! childless members of the shared ancestry, and predicts every
//! outcome from the rules in the slice's contract. Payloads are
//! encoded here from the documented wire layout, and body digests are
//! computed with `sha2` directly. After a random stream the run is
//! replayed one object at a time into fresh stores, which must reach
//! the same outcomes and reports: a conflict on one object never
//! changes another.
//!
//! Both slice modules are loaded by path: the federation roots record
//! them without declaring them (slice 9110-02), and B07 reaches B32 as
//! `super::b32_ingest_gate`.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../src/federation/slices/b32_ingest_gate.rs"]
mod b32_ingest_gate;

#[path = "../src/federation/slices/b07_conflict_report.rs"]
mod b07;

use std::collections::{BTreeMap, BTreeSet};

use b07::{
    AuditEntry, ConflictStore, IngestError, IngestOutcome, ObjectId, ReportId, ReportStatus,
    RetentionPolicy, VersionHeader,
};
use b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, IngressPath, RejectReason, SignerAuthorization,
    sign_envelope,
};
use ed25519_dalek::SigningKey;
use proptest::prelude::*;
use sha2::{Digest, Sha256};

const NOW: u64 = 10_000;
const FRESHNESS: FreshnessWindow = FreshnessWindow {
    max_age: 600,
    max_future_skew: 30,
};
const RETENTION: RetentionPolicy = RetentionPolicy { retain_for: 3_600 };
const SIGNERS: [(&str, u8); 3] = [("alice", 7), ("bob", 9), ("carol", 11)];

fn key(seed: u8) -> SigningKey {
    SigningKey::from_bytes(&[seed; 32])
}

fn signers() -> Vec<SignerAuthorization> {
    SIGNERS
        .iter()
        .map(|(identity, seed)| SignerAuthorization {
            identity: (*identity).to_owned(),
            verifying_key: key(*seed).verifying_key().to_bytes(),
            scopes: std::iter::once("sync".to_owned()).collect(),
        })
        .collect()
}

fn store() -> ConflictStore {
    ConflictStore::new(signers(), FRESHNESS, RETENTION)
}

fn sha(bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hasher.finalize().into()
}

const fn object(n: u8) -> ObjectId {
    [0xA0 | n; 32]
}

fn event(n: u32) -> EventId {
    let mut id = [0_u8; 32];
    id[..4].copy_from_slice(&n.to_be_bytes());
    id[31] = 0xEE;
    id
}

/// The wire layout from the slice contract, written independently.
fn payload(
    object_id: ObjectId,
    resolves: Option<ReportId>,
    parents: &[EventId],
    body: &[u8],
) -> Vec<u8> {
    let mut sorted = parents.to_vec();
    sorted.sort_unstable();
    let mut out = b"id-sync/b07-version/v1".to_vec();
    out.extend_from_slice(&object_id);
    match resolves {
        None => out.push(0),
        Some(id) => {
            out.push(1);
            out.extend_from_slice(&id);
        }
    }
    out.extend_from_slice(&u32::try_from(sorted.len()).unwrap().to_be_bytes());
    for parent in sorted {
        out.extend_from_slice(&parent);
    }
    out.extend_from_slice(body);
    out
}

/// The conflict key from the slice contract, computed independently.
fn key_of(ids: &[EventId]) -> [u8; 32] {
    let distinct: BTreeSet<&EventId> = ids.iter().collect();
    let mut out = b"id-sync/b07-conflict-key/v1".to_vec();
    out.extend_from_slice(&u64::try_from(distinct.len()).unwrap().to_be_bytes());
    for id in distinct {
        out.extend_from_slice(id);
    }
    sha(&out)
}

fn envelope(
    signer: usize,
    event_id: EventId,
    object_id: ObjectId,
    resolves: Option<ReportId>,
    parents: &[EventId],
    body: &[u8],
) -> Envelope {
    let (identity, seed) = SIGNERS[signer];
    sign_envelope(
        &key(seed),
        identity,
        "sync",
        &payload(object_id, resolves, parents, body),
        event_id,
        NOW,
    )
}

/// The reference: each object's versions and their parents.
#[derive(Default)]
struct Reference {
    objects: BTreeMap<ObjectId, BTreeMap<EventId, Vec<EventId>>>,
}

impl Reference {
    fn heads(&self, object_id: &ObjectId) -> BTreeSet<EventId> {
        let Some(versions) = self.objects.get(object_id) else {
            return BTreeSet::new();
        };
        let named: BTreeSet<&EventId> = versions.values().flatten().collect();
        versions
            .keys()
            .filter(|v| !named.contains(v))
            .copied()
            .collect()
    }

    fn ancestry(&self, object_id: &ObjectId, from: EventId) -> BTreeSet<EventId> {
        let versions = &self.objects[object_id];
        let mut out = BTreeSet::from([from]);
        loop {
            let grown: BTreeSet<EventId> = out
                .iter()
                .flat_map(|v| versions[v].iter().copied())
                .chain(out.iter().copied())
                .collect();
            if grown.len() == out.len() {
                return out;
            }
            out = grown;
        }
    }

    /// Shared ancestry of the heads, minus anything another shared
    /// version names as a parent.
    fn base(&self, object_id: &ObjectId) -> Vec<EventId> {
        let heads = self.heads(object_id);
        let mut shared: Option<BTreeSet<EventId>> = None;
        for head in &heads {
            let ancestry = self.ancestry(object_id, *head);
            shared = Some(match shared {
                None => ancestry,
                Some(acc) => &acc & &ancestry,
            });
        }
        let shared = shared.unwrap_or_default();
        let versions = &self.objects[object_id];
        let named: BTreeSet<EventId> = shared
            .iter()
            .flat_map(|v| versions[v].iter().copied())
            .collect();
        shared.difference(&named).copied().collect()
    }
}

/// One generated step.
#[derive(Debug, Clone, Copy)]
struct Step {
    object: u8,
    signer: usize,
    parents: u8,
    resolves: u8,
    path: usize,
    body: u8,
}

fn step_strategy() -> impl Strategy<Value = Step> {
    (
        0_u8..3,
        0..SIGNERS.len(),
        0_u8..6,
        0_u8..4,
        0..5_usize,
        any::<u8>(),
    )
        .prop_map(|(object, signer, parents, resolves, path, body)| Step {
            object,
            signer,
            parents,
            resolves,
            path,
            body,
        })
}

/// Run a stream through the store and the reference in lockstep.
/// Returns, per object, the envelopes presented and the outcomes seen,
/// for the per-object replay.
#[allow(clippy::type_complexity)]
fn run(
    steps: &[Step],
) -> BTreeMap<ObjectId, Vec<(IngressPath, Envelope, Result<IngestOutcome, IngestError>)>> {
    let mut store = store();
    let mut reference = Reference::default();
    let mut log: BTreeMap<ObjectId, Vec<_>> = BTreeMap::new();
    for (n, step) in steps.iter().enumerate() {
        let object_id = object(step.object);
        let event_id = event(u32::try_from(n).unwrap());
        let path = IngressPath::ALL[step.path];
        let heads: Vec<EventId> = reference.heads(&object_id).into_iter().collect();
        let all: Vec<EventId> = reference
            .objects
            .get(&object_id)
            .map(|v| v.keys().copied().collect())
            .unwrap_or_default();
        let parents: Vec<EventId> = match step.parents {
            0 => heads.clone(),
            1 => heads.iter().take(1).copied().collect(),
            2 => Vec::new(),
            3 => all
                .get(usize::from(step.body) % all.len().max(1))
                .copied()
                .into_iter()
                .collect(),
            4 => heads.iter().rev().take(2).copied().collect(),
            // A version and a head: concurrent or not, by chance.
            _ => all
                .get(usize::from(step.body) % all.len().max(1))
                .copied()
                .into_iter()
                .chain(heads.first().copied())
                .collect(),
        };
        let parents: Vec<EventId> = parents
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let open = store.open_report(&object_id);
        let resolves = match step.resolves {
            1 => Some(key_of(&parents)),
            2 => Some(key_of(&heads)),
            3 => Some([0x5A; 32]),
            _ => None,
        };
        let body = [step.body, step.object];
        let env = envelope(step.signer, event_id, object_id, resolves, &parents, &body);

        // Predict.
        let conflicted = heads.len() > 1;
        let merge_key = key_of(&parents);
        let concurrent = parents.iter().all(|p| {
            let ancestry = reference.ancestry(&object_id, *p);
            parents.iter().all(|q| q == p || !ancestry.contains(q))
        });
        let expect_err = match (parents.len() >= 2, resolves) {
            (true, Some(r)) if r == merge_key && !concurrent => Some(IngestError::RedundantParents),
            (true, Some(r)) if r == merge_key => None,
            (true, None) => Some(IngestError::ResolutionRequired { key: merge_key }),
            (_, Some(r)) => Some(IngestError::NotAResolution { named: r }),
            (false, None) => None,
        };
        assert_eq!(conflicted, open.is_some(), "open report iff diverged");

        let audit_before = store.audit().len();
        let got = store.ingest(path, &env, "peer-a", NOW);
        log.entry(object_id)
            .or_default()
            .push((path, env.clone(), got.clone()));

        if let Some(err) = expect_err {
            assert_eq!(got, Err(err));
            assert_eq!(store.audit().len(), audit_before, "a refusal left a trace");
            assert!(store.version(&event_id).is_none());
            continue;
        }
        reference
            .objects
            .entry(object_id)
            .or_default()
            .insert(event_id, {
                let mut p = parents.clone();
                p.sort_unstable();
                p
            });
        let new_heads = reference.heads(&object_id);
        let outcome = got.unwrap();

        // The stored version carries what was signed and who signed it.
        let stored = store.version(&event_id).unwrap();
        assert_eq!(stored.signer, SIGNERS[step.signer].0);
        assert_eq!(
            stored.verifying_key,
            key(SIGNERS[step.signer].1).verifying_key().to_bytes()
        );
        assert_eq!(stored.path, path);
        assert_eq!(stored.body_digest, sha(&body));
        assert_eq!(stored.resolves, resolves);

        match (open, new_heads.len()) {
            (None, 1) => assert_eq!(outcome, IngestOutcome::Applied),
            (Some(o), 1) => {
                assert_eq!(outcome, IngestOutcome::Resolved(o));
                assert_eq!(
                    store.report(&o).unwrap().status,
                    ReportStatus::Resolved {
                        by: event_id,
                        at: NOW
                    }
                );
            }
            (previous, _) => {
                let IngestOutcome::Conflict(id) = outcome else {
                    panic!("divergence produced {outcome:?}");
                };
                // The report is the first output of the step.
                assert_eq!(
                    store.audit().last(),
                    Some(&AuditEntry::ReportCreated {
                        object_id,
                        report_id: id,
                        supersedes: previous,
                    })
                );
                if let Some(previous) = previous {
                    assert_ne!(previous, id);
                    assert_eq!(
                        store.report(&previous).unwrap().status,
                        ReportStatus::Superseded { by: id }
                    );
                }
                let stored = store.report(&id).unwrap();
                assert_eq!(stored.status, ReportStatus::Open);
                let report = &stored.report;
                assert_eq!(report.report_id(), id, "report is addressable by its id");
                assert_eq!(report.object_id, object_id);
                let versions: BTreeSet<EventId> =
                    report.versions.iter().map(|v| v.event_id).collect();
                assert_eq!(versions, new_heads);
                assert_eq!(
                    report.resolution_parents(),
                    new_heads.iter().copied().collect::<Vec<_>>()
                );
                assert_eq!(report.base, reference.base(&object_id));
                assert_eq!(report.scopes, vec!["sync".to_owned()]);
                assert_eq!(report.policy_digest, store.policy_digest());
                assert_eq!(report.detected_at, NOW);
                assert_eq!(report.expires_at, NOW + RETENTION.retain_for);
                assert_eq!(report.supersedes, previous);
                for version in &report.versions {
                    assert_eq!(Some(version), store.version(&version.event_id));
                }
            }
        }

        // Every object matches the reference after every step.
        for object_id in reference.objects.keys() {
            let heads = reference.heads(object_id);
            assert_eq!(
                store.heads(object_id),
                heads.iter().copied().collect::<Vec<_>>()
            );
            assert_eq!(store.open_report(object_id).is_some(), heads.len() > 1);
        }
    }
    check_audit_order(&store);
    log
}

/// Report before resolution: every resolution, and every version that
/// names a report, comes after that report's creation.
fn check_audit_order(store: &ConflictStore) {
    let mut created = BTreeSet::new();
    let audit = store.audit();
    for (i, entry) in audit.iter().enumerate() {
        match entry {
            AuditEntry::ReportCreated {
                report_id,
                supersedes,
                ..
            } => {
                if let Some(previous) = supersedes {
                    assert!(created.contains(previous));
                }
                created.insert(*report_id);
            }
            AuditEntry::Resolved { report_id, .. } => assert!(created.contains(report_id)),
            AuditEntry::Recorded {
                object_id,
                event_id,
            } => {
                let version = store.version(event_id).unwrap();
                if let Some(named) = version.resolves {
                    // A merge names exactly its parents, comes after a
                    // report on its object, and changes the report.
                    assert!(version.parents.len() >= 2);
                    assert_eq!(key_of(&version.parents), named);
                    assert!(
                        audit[..i].iter().any(|e| matches!(
                            e,
                            AuditEntry::ReportCreated { object_id: o, .. } if o == object_id
                        )),
                        "merge before any report on its object"
                    );
                    assert!(matches!(
                        audit.get(i + 1),
                        Some(AuditEntry::Resolved { .. } | AuditEntry::ReportCreated { .. })
                    ));
                }
            }
            AuditEntry::Held { .. } | AuditEntry::Refused { .. } => {}
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(96))]

    #[test]
    fn conflicts_are_reported_before_resolution_and_stay_per_object(
        steps in proptest::collection::vec(step_strategy(), 1..40),
    ) {
        let log = run(&steps);
        // Unrelated sync: each object alone reaches the same outcomes
        // and the same reports as it did beside the others.
        for entries in log.values() {
            let mut alone = store();
            for (path, env, outcome) in entries {
                assert_eq!(&alone.ingest(*path, env, "peer-a", NOW), outcome);
            }
        }
    }
}

#[test]
fn divergence_reports_versions_signers_sources_and_disputed_data() {
    let mut store = store();
    let doc = object(1);
    let root = envelope(0, event(1), doc, None, &[], b"v0");
    assert_eq!(
        store.ingest(IngressPath::Local, &root, "self", NOW),
        Ok(IngestOutcome::Applied)
    );
    let ours = envelope(0, event(2), doc, None, &[event(1)], b"ours");
    let theirs = envelope(1, event(3), doc, None, &[event(1)], b"theirs");
    assert_eq!(
        store.ingest(IngressPath::Local, &ours, "self", NOW),
        Ok(IngestOutcome::Applied)
    );
    let Ok(IngestOutcome::Conflict(id)) = store.ingest(IngressPath::Remote, &theirs, "peer-b", NOW)
    else {
        panic!("no report");
    };
    let report = &store.report(&id).unwrap().report;
    assert_eq!(report.base, vec![event(1)]);
    let sides: Vec<(&str, &str, IngressPath, [u8; 32], u64)> = report
        .versions
        .iter()
        .map(|v| {
            (
                v.signer.as_str(),
                v.source.as_str(),
                v.path,
                v.body_digest,
                v.body_len,
            )
        })
        .collect();
    assert_eq!(
        sides,
        vec![
            ("alice", "self", IngressPath::Local, sha(b"ours"), 4),
            ("bob", "peer-b", IngressPath::Remote, sha(b"theirs"), 6),
        ]
    );

    // A merge that does not name the conflict is refused, and so is one
    // that names some other conflict.
    let key = store.report(&id).unwrap().report.conflict_key();
    assert_eq!(key, key_of(&[event(3), event(2)]));
    let silent = envelope(2, event(4), doc, None, &[event(2), event(3)], b"merged");
    assert_eq!(
        store.ingest(IngressPath::Remote, &silent, "peer-c", NOW),
        Err(IngestError::ResolutionRequired { key })
    );
    let wrong = envelope(
        2,
        event(5),
        doc,
        Some([1; 32]),
        &[event(2), event(3)],
        b"merged",
    );
    assert_eq!(
        store.ingest(IngressPath::Remote, &wrong, "peer-c", NOW),
        Err(IngestError::NotAResolution { named: [1; 32] })
    );
    assert_eq!(store.open_report(&doc), Some(id));

    // A merge of a version and its own ancestor resolves nothing.
    let redundant = envelope(
        2,
        event(7),
        doc,
        Some(key_of(&[event(1), event(2)])),
        &[event(1), event(2)],
        b"merged",
    );
    assert_eq!(
        store.ingest(IngressPath::Remote, &redundant, "peer-c", NOW),
        Err(IngestError::RedundantParents)
    );

    let resolution = envelope(
        2,
        event(6),
        doc,
        Some(key),
        &[event(2), event(3)],
        b"merged",
    );
    assert_eq!(
        store.ingest(IngressPath::Remote, &resolution, "peer-c", NOW),
        Ok(IngestOutcome::Resolved(id))
    );
    assert_eq!(store.heads(&doc), vec![event(6)]);
    assert_eq!(store.open_report(&doc), None);
    // The resolved report stays addressable with its evidence.
    assert_eq!(store.report(&id).unwrap().report.versions.len(), 2);
}

#[test]
fn a_conflicted_object_does_not_block_another() {
    let mut store = store();
    let (a, b) = (object(1), object(2));
    store
        .ingest(
            IngressPath::Remote,
            &envelope(0, event(1), a, None, &[], b"a1"),
            "p",
            NOW,
        )
        .unwrap();
    let report = store
        .ingest(
            IngressPath::Remote,
            &envelope(1, event(2), a, None, &[], b"a2"),
            "p",
            NOW,
        )
        .unwrap();
    assert!(matches!(report, IngestOutcome::Conflict(_)));
    let mut prev = Vec::new();
    for n in 10..15 {
        let env = envelope(2, event(n), b, None, &prev, &[u8::try_from(n).unwrap()]);
        assert_eq!(
            store.ingest(IngressPath::Remote, &env, "p", NOW),
            Ok(IngestOutcome::Applied)
        );
        prev = vec![event(n)];
    }
    assert_eq!(store.heads(&b), vec![event(14)]);
    assert_eq!(store.heads(&a).len(), 2);
}

#[test]
fn versions_whose_parents_are_missing_are_held_then_applied() {
    let mut store = store();
    let doc = object(1);
    let child = envelope(0, event(2), doc, None, &[event(1)], b"child");
    assert_eq!(
        store.ingest(IngressPath::Remote, &child, "p", NOW),
        Ok(IngestOutcome::Held)
    );
    assert!(store.is_held(&event(2)));
    let parent = envelope(0, event(1), doc, None, &[], b"parent");
    assert_eq!(
        store.ingest(IngressPath::Remote, &parent, "p", NOW),
        Ok(IngestOutcome::Applied)
    );
    assert!(!store.is_held(&event(2)));
    assert_eq!(store.heads(&doc), vec![event(2)]);
    assert_eq!(
        store.audit(),
        &[
            AuditEntry::Held {
                object_id: doc,
                event_id: event(2)
            },
            AuditEntry::Recorded {
                object_id: doc,
                event_id: event(1)
            },
            AuditEntry::Recorded {
                object_id: doc,
                event_id: event(2)
            },
        ]
    );
}

#[test]
fn gate_and_shape_refusals_store_nothing() {
    let mut store = store();
    let doc = object(1);
    let mut unsigned = envelope(0, event(1), doc, None, &[], b"x");
    unsigned.signature = None;
    assert_eq!(
        store.ingest(IngressPath::Local, &unsigned, "self", NOW),
        Err(IngestError::Rejected(RejectReason::Unsigned))
    );
    assert_eq!(store.gate().admitted_count(), 0);
    assert!(!store.gate().is_admitted(&event(1)));
    let garbage = sign_envelope(&key(7), "alice", "sync", b"not a version", event(2), NOW);
    assert_eq!(
        store.ingest(IngressPath::Local, &garbage, "self", NOW),
        Err(IngestError::MalformedVersion)
    );
    // Parents out of order are not canonical.
    let mut bytes = payload(doc, None, &[event(3), event(4)], b"x");
    let start = b"id-sync/b07-version/v1".len() + 32 + 1 + 4;
    let (left, right) = bytes[start..start + 64].split_at_mut(32);
    left.swap_with_slice(right);
    assert_eq!(VersionHeader::decode(&bytes), None);
    let self_parent = envelope(0, event(5), doc, None, &[event(5)], b"x");
    assert_eq!(
        store.ingest(IngressPath::Local, &self_parent, "self", NOW),
        Err(IngestError::MalformedVersion)
    );

    store
        .ingest(
            IngressPath::Local,
            &envelope(0, event(6), object(2), None, &[], b"y"),
            "self",
            NOW,
        )
        .unwrap();
    let foreign = envelope(0, event(7), doc, None, &[event(6)], b"x");
    assert_eq!(
        store.ingest(IngressPath::Local, &foreign, "self", NOW),
        Err(IngestError::ForeignParent(event(6)))
    );
    assert!(store.heads(&doc).is_empty());
}

#[test]
fn header_encoding_matches_the_documented_layout() {
    let header = VersionHeader {
        object_id: object(3),
        resolves: Some([9; 32]),
        parents: vec![event(8), event(2)],
        body: b"body".to_vec(),
    };
    let expected = payload(object(3), Some([9; 32]), &[event(8), event(2)], b"body");
    assert_eq!(header.encode(), expected);
    let decoded = VersionHeader::decode(&expected).unwrap();
    assert_eq!(decoded.parents, vec![event(2), event(8)]);
    assert_eq!(decoded.resolves, Some([9; 32]));
    assert_eq!(decoded.body, b"body");
    for cut in 0..expected.len() - 4 {
        assert_eq!(
            VersionHeader::decode(&expected[..cut]),
            None,
            "truncated at {cut}"
        );
    }
}

#[test]
fn retention_drops_only_closed_reports_after_expiry() {
    let mut store = store();
    let (a, b) = (object(1), object(2));
    let mut open_ids = Vec::new();
    for (n, doc) in [(1, a), (3, b)] {
        store
            .ingest(
                IngressPath::Remote,
                &envelope(0, event(n), doc, None, &[], b"1"),
                "p",
                NOW,
            )
            .unwrap();
        let IngestOutcome::Conflict(id) = store
            .ingest(
                IngressPath::Remote,
                &envelope(1, event(n + 1), doc, None, &[], b"2"),
                "p",
                NOW,
            )
            .unwrap()
        else {
            panic!("no conflict");
        };
        open_ids.push(id);
    }
    let resolve = envelope(
        2,
        event(9),
        a,
        Some(store.report(&open_ids[0]).unwrap().report.conflict_key()),
        &[event(1), event(2)],
        b"m",
    );
    store
        .ingest(IngressPath::Remote, &resolve, "p", NOW)
        .unwrap();

    let later = NOW + RETENTION.retain_for;
    assert_eq!(store.expire(later - 1), 0);
    assert_eq!(store.expire(later), 1);
    assert_eq!(store.reports().count(), 1);
    assert!(store.report(&open_ids[0]).is_none());
    assert_eq!(
        store.report(&open_ids[1]).unwrap().status,
        ReportStatus::Open
    );
    assert_eq!(store.expire(u64::MAX), 0, "open reports never expire");
}

#[test]
fn one_signed_resolution_resolves_the_same_conflict_on_every_replica() {
    let doc = object(4);
    let root = envelope(0, event(1), doc, None, &[], b"v0");
    let left = envelope(0, event(2), doc, None, &[event(1)], b"left");
    let right = envelope(1, event(3), doc, None, &[event(1)], b"right");
    let key = key_of(&[event(2), event(3)]);
    let resolution = envelope(2, event(4), doc, Some(key), &[event(2), event(3)], b"m");

    // Two replicas see the same versions in a different order, by
    // different paths and from different peers.
    let mut a = store();
    let mut b = store();
    for env in [&root, &left, &right] {
        a.ingest(IngressPath::Local, env, "self", NOW).unwrap();
    }
    for env in [&right, &root, &left] {
        b.ingest(IngressPath::Remote, env, "peer-a", NOW + 5)
            .unwrap();
    }
    let (ra, rb) = (a.open_report(&doc).unwrap(), b.open_report(&doc).unwrap());
    assert_ne!(ra, rb, "each replica keeps its own record");
    assert_eq!(a.report(&ra).unwrap().report.conflict_key(), key);
    assert_eq!(b.report(&rb).unwrap().report.conflict_key(), key);

    assert_eq!(
        a.ingest(IngressPath::Local, &resolution, "self", NOW),
        Ok(IngestOutcome::Resolved(ra))
    );
    assert_eq!(
        b.ingest(IngressPath::Remote, &resolution, "peer-a", NOW + 5),
        Ok(IngestOutcome::Resolved(rb))
    );
    assert_eq!(a.heads(&doc), b.heads(&doc));
}
