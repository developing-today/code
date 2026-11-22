//! Second step of the #9106 walking skeleton (#12232): a signed record
//! written on node A reaches node B, is read there from its `id://`
//! address, and a conflicting write on B is reported on both nodes.
//!
//! Both halves of the substrate are used through their crates, not by
//! `#[path]`: part A from `id_core` (signed graph records, `id://`
//! content addresses, profile dispatch) and part B from `id_sync`
//! (ingest gate, conflict store, sync scope, anti-entropy). The first
//! tests run between two in-process [`Replica`]s and the link between
//! them is [`reconcile`]; the last runs the same exchange between
//! disk-backed nodes ([`StoredReplica`]) that are dropped and
//! reopened between steps, and restart with what they had. Nothing
//! here opens a socket: carrying the same exchange over the real
//! transport is the next slice.
//!
//! # How a part A record becomes a part B version
//!
//! The two halves were built separately and each has its own envelope.
//! This test fixes the join (decision recorded on #12232; undo by
//! changing `publish` below and that comment):
//!
//! - **Body.** A version's body is the canonical bytes of the part A
//!   signed record envelope, carried unchanged.
//! - **Event id.** The version's event id is the digest of the record's
//!   envelope ID. The same 32 bytes are the digest in the record's
//!   `id://…/v1/content/raw-blake3-256.<digest>` address, so an address
//!   names a version directly and a reader needs no second lookup.
//! - **Object.** The object is the record's logical graph. Two records
//!   that follow the same record in one graph are two heads, which is
//!   the conflict the store reports.
//! - **Parents.** A version's parents are the event ids of the records
//!   it follows.
//!
//! Oracles: differential (what B reads equals what A signed; both nodes
//! report the same conflict key, versions and base), metamorphic (the outcome does not depend on
//! which side starts the exchange or on how often it is repeated), and
//! a model of the heads kept here as plain sets.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use ed25519_dalek::SigningKey;
use id_core::graph::record::{GraphRecord, OpenedRecord, open};
use id_core::graph::term::{Term, Triple};
use id_core::hash_profile::ProfileDispatch;
use id_core::id_shape::{ContentId, HashProfile};
use id_core::id_uri_format::resolve_context;
use id_core::id_uri_parse::{IdUriV1, UriTarget};
use id_core::signed_envelope::{Envelope as RecordEnvelope, LogicalGraphId};
use id_sync::federation::record_join::JoinRefusal;
use id_sync::federation::record_store::{DiskReplica, StoredReplica};
use id_sync::federation::slices::b07_conflict_report::{
    ConflictStore, IngestError, IngestOutcome, ObjectId, ReportStatus, RetentionPolicy,
    VersionHeader, conflict_key,
};
use id_sync::federation::slices::b08_sync_scope::SyncScopeConfig;
use id_sync::federation::slices::b09_anti_entropy::{Replica, reconcile};
use id_sync::federation::slices::b32_ingest_gate::{
    Envelope, EventId, FreshnessWindow, IngressPath, RejectReason, SignerAuthorization,
    sign_envelope,
};

const START: u64 = 10_000;
const SCOPE: &str = "notes";
const TRUST_DOMAIN: &str = "home";
const NAMESPACE: &str = "notes";
const CONTEXT: &str = "namespace/notes";
const GRAPH: [u8; 32] = [0x11; 32];
const WRITERS: [(&str, u8); 2] = [("alice", 7), ("bob", 9)];
const ALICE: usize = 0;
const BOB: usize = 1;

fn key(writer: usize) -> SigningKey {
    SigningKey::from_bytes(&[WRITERS[writer].1; 32])
}

fn conflict_store() -> ConflictStore {
    let signers = WRITERS
        .iter()
        .enumerate()
        .map(|(writer, (identity, _))| SignerAuthorization {
            identity: (*identity).to_owned(),
            verifying_key: key(writer).verifying_key().to_bytes(),
            scopes: [SCOPE.to_owned()].into_iter().collect(),
        })
        .collect();
    let freshness = FreshnessWindow {
        max_age: 1_000_000,
        max_future_skew: 30,
    };
    ConflictStore::new(signers, freshness, RetentionPolicy { retain_for: 3_600 })
}

fn node(name: &str) -> Replica {
    Replica::new(name, TRUST_DOMAIN, conflict_store())
}

/// Open the disk-backed node called `name` on `dir`, fresh or
/// restarted: what it had is replayed from its directory.
async fn disk_node(dir: &std::path::Path, name: &str) -> DiskReplica {
    StoredReplica::open_dir(dir, Replica::new(name, TRUST_DOMAIN, conflict_store()))
        .await
        .unwrap()
}

fn link() -> SyncScopeConfig {
    SyncScopeConfig::new(SCOPE, TRUST_DOMAIN)
}

fn note(text: &[u8]) -> GraphRecord {
    GraphRecord::new(
        LogicalGraphId::new(GRAPH),
        vec![Triple::new(
            Term::typed(1, b"urn:note:1".to_vec()),
            Term::typed(2, b"body".to_vec()),
            Term::typed(3, text.to_vec()),
        )],
    )
}

/// A record a writer put on a node: what was signed, its event id and
/// the address a reader on `resolver` would be given.
struct Written {
    signed: OpenedRecord,
    event: EventId,
}

impl Written {
    fn address(&self, resolver: &str) -> String {
        IdUriV1::new(
            resolver,
            NAMESPACE,
            UriTarget::Content(*self.signed.id.content_id()),
        )
        .unwrap()
        .format()
    }
}

/// Sign `record` as a part A record, wrap it as a part B version that
/// follows `parents`, and produce it on `node`.
fn publish(
    node: &mut Replica,
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> (Written, IngestOutcome) {
    let identity = WRITERS[writer].0;
    let signed = record.sign(CONTEXT, identity, &key(writer)).unwrap();
    let event: EventId = *signed.id.content_id().digest();
    let object: ObjectId = *record.graph.as_bytes();
    let header = VersionHeader {
        object_id: object,
        resolves: (parents.len() >= 2).then(|| conflict_key(parents)),
        parents: parents.to_vec(),
        body: signed.envelope.canonical_bytes().unwrap(),
    };
    let version = sign_envelope(&key(writer), identity, SCOPE, &header.encode(), event, now);
    let outcome = node.produce(&version, now).unwrap();
    (Written { signed, event }, outcome)
}

/// `publish`, for a disk-backed node: sign `record`, wrap it as a
/// version that follows `parents`, and produce it on `node`, which
/// persists it before returning.
async fn publish_disk(
    node: &mut DiskReplica,
    writer: usize,
    record: &GraphRecord,
    parents: &[EventId],
    now: u64,
) -> (Written, IngestOutcome) {
    let identity = WRITERS[writer].0;
    let signed = record.sign(CONTEXT, identity, &key(writer)).unwrap();
    let event: EventId = *signed.id.content_id().digest();
    let object: ObjectId = *record.graph.as_bytes();
    let header = VersionHeader {
        object_id: object,
        resolves: (parents.len() >= 2).then(|| conflict_key(parents)),
        parents: parents.to_vec(),
        body: signed.envelope.canonical_bytes().unwrap(),
    };
    let version = sign_envelope(&key(writer), identity, SCOPE, &header.encode(), event, now);
    let outcome = node.produce(&version, now).await.unwrap();
    (Written { signed, event }, outcome)
}

/// Why a node could not serve a record for an address.
#[derive(Debug, PartialEq, Eq)]
enum Unreadable {
    /// The node holds no version under the address's digest.
    NotHeld,
    /// The version's body is not the bytes the address names.
    BodyIsNotTheNamedBytes,
    /// The body is the named bytes, and they are not a valid signed record.
    NotASignedRecord,
    /// The record is valid, and the version that carries it says
    /// something else about it.
    JoinBroken(&'static str),
}

/// Read a record on `node` given only its address text: the address
/// names the version, the node's own copy supplies the bytes, and the
/// bytes are checked against the address before they are opened. The
/// gate admits a version without reading its body, so every part of the
/// join is checked here, on the way out.
fn read(node: &Replica, address: &str) -> Result<OpenedRecord, Unreadable> {
    let dispatch = ProfileDispatch::default();
    let uri = IdUriV1::parse(address).unwrap();
    resolve_context(uri.resolver_hint(), node.name()).unwrap();
    let UriTarget::Content(named) = uri.target() else {
        panic!("{address} does not name a record");
    };
    let version = node.envelope(named.digest()).ok_or(Unreadable::NotHeld)?;
    let header = VersionHeader::decode(&version.payload).unwrap();
    dispatch
        .verify_bytes(named, &header.body)
        .map_err(|_| Unreadable::BodyIsNotTheNamedBytes)?;
    let envelope =
        RecordEnvelope::decode(&header.body).map_err(|_| Unreadable::NotASignedRecord)?;
    let opened = open(envelope, &dispatch).map_err(|_| Unreadable::NotASignedRecord)?;
    if opened.id.content_id() != named {
        return Err(Unreadable::JoinBroken("event id"));
    }
    if &header.object_id != opened.record.graph.as_bytes() {
        return Err(Unreadable::JoinBroken("object"));
    }
    if version.verifying_key != opened.envelope.body.key {
        return Err(Unreadable::JoinBroken("signer"));
    }
    Ok(opened)
}

/// The address of the version `event` as a reader on `resolver` writes it.
fn address_of(event: &EventId, resolver: &str) -> String {
    IdUriV1::new(
        resolver,
        NAMESPACE,
        UriTarget::Content(ContentId::new(HashProfile::RawBlake3_256, *event)),
    )
    .unwrap()
    .format()
}

fn heads(node: &Replica) -> BTreeSet<EventId> {
    node.store().heads(&GRAPH).into_iter().collect()
}

#[test]
fn a_record_written_on_one_node_is_read_from_its_address_on_the_other() {
    let mut a = node("node-a");
    let mut b = node("node-b");

    let (base, outcome) = publish(&mut a, ALICE, &note(b"first draft"), &[], START);
    assert_eq!(outcome, IngestOutcome::Applied);

    // Before the exchange, B does not have it: the address resolves to
    // nothing there, and it is not read from anywhere else.
    assert_eq!(read(&b, &base.address("node-b")), Err(Unreadable::NotHeld));

    let exchange = reconcile(&mut a, &mut b, &link(), START + 1);
    assert_eq!(exchange.forward.offer.send, vec![base.event]);

    // After it, B reads exactly what Alice signed on A, from the address.
    let on_b = read(&b, &base.address("node-b")).unwrap();
    assert_eq!(on_b, base.signed);
    assert_eq!(on_b.envelope.body.signer_id, "alice");
    assert_eq!(
        on_b.assertions().next().unwrap().triple,
        &note(b"first draft").triples[0]
    );
    assert_eq!(read(&a, &base.address("node-a")).unwrap(), base.signed);
    assert_eq!(heads(&a), BTreeSet::from([base.event]));
    assert_eq!(heads(&b), BTreeSet::from([base.event]));

    // Repeating the exchange, in either direction, changes nothing.
    assert!(reconcile(&mut a, &mut b, &link(), START + 2).is_quiet());
    assert!(reconcile(&mut b, &mut a, &link(), START + 3).is_quiet());
}

#[test]
fn a_conflicting_write_is_reported_on_both_nodes_and_nothing_is_merged() {
    for a_starts in [true, false] {
        let mut a = node("node-a");
        let mut b = node("node-b");
        let (base, _) = publish(&mut a, ALICE, &note(b"first draft"), &[], START);
        reconcile(&mut a, &mut b, &link(), START + 1);

        // Both writers follow the same record, each on their own node.
        let (from_a, on_a) = publish(
            &mut a,
            ALICE,
            &note(b"alice's edit"),
            &[base.event],
            START + 2,
        );
        let (from_b, on_b) = publish(&mut b, BOB, &note(b"bob's edit"), &[base.event], START + 2);
        // Locally each write is a plain successor: neither node can know.
        assert_eq!(on_a, IngestOutcome::Applied);
        assert_eq!(on_b, IngestOutcome::Applied);
        assert_ne!(from_a.event, from_b.event);

        // The outcome must not depend on which side starts the exchange.
        if a_starts {
            reconcile(&mut a, &mut b, &link(), START + 3);
        } else {
            reconcile(&mut b, &mut a, &link(), START + 3);
        }

        // Model: both edits are heads on both nodes; the base is not.
        let expected = BTreeSet::from([from_a.event, from_b.event]);
        let key = conflict_key(&[from_a.event, from_b.event]);
        let mut report_ids = Vec::new();
        for (side, name) in [(&a, "node-a"), (&b, "node-b")] {
            assert_eq!(heads(side), expected, "{name} heads");
            let id = side
                .store()
                .open_report(&GRAPH)
                .unwrap_or_else(|| panic!("{name} holds no open report"));
            let stored = side.store().report(&id).unwrap();
            assert_eq!(stored.report.conflict_key(), key, "{name} conflict key");
            assert_eq!(stored.report.object_id, GRAPH);
            assert_eq!(stored.report.base, vec![base.event], "{name} base");
            let named: BTreeSet<EventId> =
                stored.report.versions.iter().map(|v| v.event_id).collect();
            assert_eq!(named, expected, "{name} report versions");
            let signers: BTreeSet<&str> = stored
                .report
                .versions
                .iter()
                .map(|v| v.signer.as_str())
                .collect();
            assert_eq!(signers, BTreeSet::from(["alice", "bob"]));
            report_ids.push(id);

            // Nothing was merged or dropped: both edits, and the record
            // they follow, are each still readable from their address.
            assert_eq!(read(side, &from_a.address(name)).unwrap(), from_a.signed);
            assert_eq!(read(side, &from_b.address(name)).unwrap(), from_b.signed);
            assert_eq!(read(side, &base.address(name)).unwrap(), base.signed);
        }
        // The conflict key is the identity the two nodes share. A report's
        // own ID also covers how each version arrived and when the node
        // noticed, so each node's report is its own record of the conflict.
        let (report_a, report_b) = (report_ids[0], report_ids[1]);

        // Another exchange neither clears the report nor invents a merge.
        assert!(reconcile(&mut a, &mut b, &link(), START + 4).is_quiet());
        assert!(a.store().open_report(&GRAPH).is_some());
        assert!(b.store().open_report(&GRAPH).is_some());

        // A writer resolves it with a record that follows both edits;
        // the next exchange closes the report on the other node too.
        let (merged, outcome) = publish(
            &mut b,
            BOB,
            &note(b"both edits, merged by bob"),
            &[from_a.event, from_b.event],
            START + 5,
        );
        assert_eq!(outcome, IngestOutcome::Resolved(report_b));
        let exchange = reconcile(&mut a, &mut b, &link(), START + 6);
        assert_eq!(
            exchange.backward.outcomes,
            vec![(merged.event, Ok(IngestOutcome::Resolved(report_a)))]
        );
        for (side, name) in [(&a, "node-a"), (&b, "node-b")] {
            assert_eq!(heads(side), BTreeSet::from([merged.event]), "{name}");
            assert!(side.store().open_report(&GRAPH).is_none(), "{name}");
            assert_eq!(read(side, &merged.address(name)).unwrap(), merged.signed);
        }
        assert!(reconcile(&mut a, &mut b, &link(), START + 7).is_quiet());
    }
}

#[test]
fn a_record_from_a_writer_the_node_does_not_know_is_not_admitted() {
    // The gate is not bypassed by the join: a well-formed, correctly
    // signed part A record from a key outside the node's signer table
    // is refused at produce, and its address resolves to nothing.
    let mut a = node("node-a");
    let stranger = SigningKey::from_bytes(&[42; 32]);
    let record = note(b"uninvited");
    let signed = record.sign(CONTEXT, "mallory", &stranger).unwrap();
    let event: EventId = *signed.id.content_id().digest();
    let header = VersionHeader {
        object_id: GRAPH,
        resolves: None,
        parents: vec![],
        body: signed.envelope.canonical_bytes().unwrap(),
    };
    let version = sign_envelope(&stranger, "mallory", SCOPE, &header.encode(), event, START);
    assert_eq!(
        a.produce(&version, START),
        Err(IngestError::Rejected(RejectReason::UnknownSigner))
    );
    assert_eq!(
        read(&a, &address_of(&event, "node-a")),
        Err(Unreadable::NotHeld)
    );
    assert!(heads(&a).is_empty());

    // A known writer's key under another known writer's name is refused
    // too: Bob cannot publish as Alice.
    let as_alice = sign_envelope(&key(BOB), "alice", SCOPE, &header.encode(), event, START);
    assert_eq!(
        a.produce(&as_alice, START),
        Err(IngestError::Rejected(RejectReason::SignerMismatch {
            claimed: "alice".to_owned(),
            bound_to: "bob".to_owned(),
        }))
    );
    assert!(heads(&a).is_empty());
}

/// A broken join is refused the same way however it arrives: at
/// `produce` on its writer's own node and at `receive` from a peer,
/// with its class, and it leaves no trace in either node's
/// inventory, so a peer that somehow holds one cannot pass it on
/// and no reader is ever served it.
fn assert_refused(version: &Envelope, class: JoinRefusal, event: &EventId) {
    let mut a = node("node-a");
    assert_eq!(
        a.produce(version, START),
        Err(IngestError::JoinRefused(class)),
        "produce"
    );
    assert!(!a.inventory().contains(event), "produce: inventoried");
    assert_eq!(
        read(&a, &address_of(event, "node-a")),
        Err(Unreadable::NotHeld)
    );

    let mut b = node("node-b");
    assert_eq!(
        b.receive(IngressPath::Internal, version, "node-a", START),
        Err(IngestError::JoinRefused(class)),
        "receive"
    );
    assert!(!b.inventory().contains(event), "receive: inventoried");
    assert!(b.refused().contains_key(event), "receive: not recorded");
}

#[test]
fn a_version_whose_body_breaks_the_join_is_refused_at_admission() {
    // The gate checks the version's own signature and never opens
    // the body, so without the admission check (#12279) an authorized
    // writer could get a version admitted whose body is not the
    // record its event id promises, and it would be refused only on
    // the way out. Each case below is refused on the way in instead,
    // with the class that names how the join breaks.
    let wrap = |writer: usize, event: EventId, object: ObjectId, body: Vec<u8>| {
        let header = VersionHeader {
            object_id: object,
            resolves: None,
            parents: vec![],
            body,
        };
        sign_envelope(
            &key(writer),
            WRITERS[writer].0,
            SCOPE,
            &header.encode(),
            event,
            START,
        )
    };
    let digest = |bytes: &[u8]| -> EventId { *blake3::hash(bytes).as_bytes() };

    // 1. A record whose signature was changed, re-addressed to match.
    let honest = note(b"alice wrote this")
        .sign(CONTEXT, "alice", &key(ALICE))
        .unwrap();
    let mut forged = honest.envelope.clone();
    forged.signature[0] ^= 0x01;
    let forged_bytes = forged.canonical_bytes().unwrap();
    let event = digest(&forged_bytes);
    let version = wrap(ALICE, event, GRAPH, forged_bytes);
    assert_refused(&version, JoinRefusal::BodyNotValidRecord, &event);

    // 2. Alice's valid record, republished by Bob under his own key.
    let bytes = honest.envelope.canonical_bytes().unwrap();
    let event = digest(&bytes);
    assert_eq!(event, *honest.id.content_id().digest());
    let version = wrap(BOB, event, GRAPH, bytes.clone());
    assert_refused(&version, JoinRefusal::SignerMismatch, &event);

    // 3. A valid record filed under another object.
    let version = wrap(ALICE, event, [0x22; 32], bytes.clone());
    assert_refused(&version, JoinRefusal::ObjectMismatch, &event);

    // 4. A valid record under an event id that is not its digest:
    // neither the record's own address nor the id it was filed
    // under finds anything, because it was never admitted.
    let elsewhere: EventId = [0x33; 32];
    let version = wrap(ALICE, elsewhere, GRAPH, bytes.clone());
    assert_refused(&version, JoinRefusal::EventIdMismatch, &elsewhere);
    let a = node("node-a");
    assert_eq!(
        read(&a, &address_of(&event, "node-a")),
        Err(Unreadable::NotHeld)
    );

    // 5. Bytes that are no envelope at all, correctly addressed.
    let junk = b"not an envelope".to_vec();
    let event = digest(&junk);
    let version = wrap(ALICE, event, GRAPH, junk);
    assert_refused(&version, JoinRefusal::BodyNotEnvelope, &event);

    // Control: the same wrapper with nothing broken is served.
    let mut a = node("node-a");
    let event = *honest.id.content_id().digest();
    a.produce(&wrap(ALICE, event, GRAPH, bytes), START).unwrap();
    assert_eq!(read(&a, &address_of(&event, "node-a")), Ok(honest));
}

#[test]
fn a_node_restarts_with_what_it_had() {
    // The same exchange as the in-process tests, between disk-backed
    // nodes that are dropped and reopened between steps. A reopened
    // node is the dropped node: same inventory, same heads, same
    // reports, and it serves the same records from their addresses
    // with no exchange in between. Restarting between any two steps
    // changes no outcome (the metamorphic oracle of the tests above,
    // with a restart inserted).
    let runtime = tokio::runtime::Runtime::new().unwrap();
    runtime.block_on(async {
        let dir_a = tempfile::tempdir().unwrap();
        let dir_b = tempfile::tempdir().unwrap();
        let mut a = disk_node(dir_a.path(), "node-a").await;
        let mut b = disk_node(dir_b.path(), "node-b").await;

        // Write on A, exchange, read on B from the address.
        let (base, outcome) = publish_disk(&mut a, ALICE, &note(b"first draft"), &[], START).await;
        assert_eq!(outcome, IngestOutcome::Applied);
        let exchange = a.reconcile_with(&mut b, &link(), START + 1).await.unwrap();
        assert_eq!(exchange.forward.offer.send, vec![base.event]);
        assert_eq!(
            read(b.replica(), &base.address("node-b")).unwrap(),
            base.signed
        );

        // B restarts with what it had: no exchange, and the record
        // is still read from its address. (Shutdown is what a
        // process exit does to the blob store; drop alone leaves
        // its database locked.)
        let inventory_before = b.replica().inventory();
        b.shutdown().await.unwrap();
        drop(b);
        let mut b = disk_node(dir_b.path(), "node-b").await;
        assert_eq!(b.replica().inventory(), inventory_before);
        assert_eq!(heads(b.replica()), BTreeSet::from([base.event]));
        assert_eq!(
            read(b.replica(), &base.address("node-b")).unwrap(),
            base.signed
        );

        // Conflicting writes on both nodes, exchanged: both report.
        let (from_a, _) = publish_disk(
            &mut a,
            ALICE,
            &note(b"alice's edit"),
            &[base.event],
            START + 2,
        )
        .await;
        let (from_b, _) =
            publish_disk(&mut b, BOB, &note(b"bob's edit"), &[base.event], START + 2).await;
        a.reconcile_with(&mut b, &link(), START + 3).await.unwrap();
        let expected = BTreeSet::from([from_a.event, from_b.event]);
        let key = conflict_key(&[from_a.event, from_b.event]);
        let report_a = a.store().open_report(&GRAPH).unwrap();
        let report_b = b.store().open_report(&GRAPH).unwrap();
        assert_eq!(
            a.store().report(&report_a).unwrap().report.conflict_key(),
            key
        );
        assert_eq!(
            b.store().report(&report_b).unwrap().report.conflict_key(),
            key
        );

        // Both restart: the open report is still on both, under the
        // same id, naming the same versions and base, and both edits
        // still read from their addresses.
        a.shutdown().await.unwrap();
        b.shutdown().await.unwrap();
        drop(a);
        drop(b);
        let mut a = disk_node(dir_a.path(), "node-a").await;
        let mut b = disk_node(dir_b.path(), "node-b").await;
        for (side, name, report_id) in [(&a, "node-a", report_a), (&b, "node-b", report_b)] {
            assert_eq!(side.store().open_report(&GRAPH), Some(report_id), "{name}");
            let stored = side.store().report(&report_id).unwrap();
            assert_eq!(stored.report.conflict_key(), key, "{name}");
            let named: BTreeSet<EventId> =
                stored.report.versions.iter().map(|v| v.event_id).collect();
            assert_eq!(named, expected, "{name}");
            assert_eq!(stored.report.base, vec![base.event], "{name}");
            assert_eq!(
                read(side.replica(), &from_a.address(name)).unwrap(),
                from_a.signed
            );
            assert_eq!(
                read(side.replica(), &from_b.address(name)).unwrap(),
                from_b.signed
            );
            assert_eq!(
                read(side.replica(), &base.address(name)).unwrap(),
                base.signed
            );
        }

        // A resolution written after the restart closes the report
        // on both nodes, and the closure survives a further restart.
        let (merged, outcome) = publish_disk(
            &mut b,
            BOB,
            &note(b"both edits, merged by bob"),
            &[from_a.event, from_b.event],
            START + 5,
        )
        .await;
        assert_eq!(outcome, IngestOutcome::Resolved(report_b));
        a.reconcile_with(&mut b, &link(), START + 6).await.unwrap();
        assert!(a.store().open_report(&GRAPH).is_none());
        a.shutdown().await.unwrap();
        drop(a);
        let a = disk_node(dir_a.path(), "node-a").await;
        assert!(a.store().open_report(&GRAPH).is_none());
        assert_eq!(heads(a.replica()), BTreeSet::from([merged.event]));
        assert_eq!(
            read(a.replica(), &merged.address("node-a")).unwrap(),
            merged.signed
        );
        assert_eq!(
            a.store().report(&report_a).unwrap().status,
            ReportStatus::Resolved {
                by: merged.event,
                at: START + 6,
            }
        );
    });
}
