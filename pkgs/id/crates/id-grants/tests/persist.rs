//! The stored form of a grant set (#14790 slice 2b): a round trip keeps
//! every decision, and a load re-checks what it reads.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use id_grants::{
    AccessRequest, Decision, Delegation, Duration, Grant, GrantError, GrantId, GrantSet,
    GrantSetSnapshot, LoadError, Operation, Requester, SNAPSHOT_FORMAT, Scope, Subject,
};

const NOW: u64 = 1_800_000_000;

fn id(s: &str) -> GrantId {
    GrantId(s.to_owned())
}

fn root(name: &str, subject: Subject) -> Grant {
    Grant {
        id: id(name),
        issuer: "owner".to_owned(),
        subject,
        scopes: BTreeSet::from([Scope::Object("a".to_owned()), Scope::Object("b".to_owned())]),
        operations: BTreeSet::from([Operation::Read, Operation::Subscribe]),
        duration: Duration::ExpiresAt(NOW + 100),
        delegation: Delegation {
            may_delegate: true,
            may_redelegate: false,
        },
        parent: None,
    }
}

fn child(name: &str, parent: &str, issuer: &str) -> Grant {
    Grant {
        id: id(name),
        issuer: issuer.to_owned(),
        subject: Subject::Identity("bob".to_owned()),
        scopes: BTreeSet::from([Scope::Object("a".to_owned())]),
        operations: BTreeSet::from([Operation::Read]),
        duration: Duration::ExpiresAt(NOW + 50),
        delegation: Delegation::default(),
        parent: Some(id(parent)),
    }
}

fn who(identity: &str, groups: &[&str]) -> Requester {
    Requester {
        identity: identity.to_owned(),
        groups: groups.iter().map(|g| (*g).to_owned()).collect(),
    }
}

/// A set with an identity root, a group root, a delegated child of
/// each, a revoked grant and a served snapshot.
fn populated() -> GrantSet {
    let mut set = GrantSet::new();
    set.insert_root(root("alice-root", Subject::Identity("alice".to_owned())))
        .unwrap();
    set.insert_root(root("team-root", Subject::Group("team".to_owned())))
        .unwrap();
    set.insert_delegated(child("c1", "alice-root", "alice"), &who("alice", &[]), NOW)
        .unwrap();
    set.insert_delegated(
        child("c2", "team-root", "carol"),
        &who("carol", &["team"]),
        NOW,
    )
    .unwrap();
    let mut snap = root("snap", Subject::Identity("dave".to_owned()));
    snap.operations = BTreeSet::from([Operation::Read]);
    snap.duration = Duration::SnapshotOnce;
    snap.delegation = Delegation::default();
    set.insert_root(snap).unwrap();
    set.consume_snapshot(&id("snap"));
    let mut gone = root("gone", Subject::Identity("erin".to_owned()));
    gone.delegation = Delegation::default();
    set.insert_root(gone).unwrap();
    set.revoke(&id("gone"));
    set
}

fn reads(set: &GrantSet, requester: &Requester, object: &str) -> Decision {
    set.authorize(&AccessRequest {
        requester: Some(requester),
        object,
        collection: None,
        operation: Operation::Read,
        now: NOW,
    })
}

#[test]
fn a_round_trip_keeps_every_grant_and_decision() {
    let set = populated();
    let snapshot = set.to_snapshot();
    assert_eq!(snapshot.format, SNAPSHOT_FORMAT);
    // Parents come before children.
    let position = |name: &str| {
        snapshot
            .grants
            .iter()
            .position(|g| g.id == id(name))
            .unwrap()
    };
    assert!(position("alice-root") < position("c1"));
    assert!(position("team-root") < position("c2"));

    let json = serde_json::to_string(&snapshot).unwrap();
    let loaded = GrantSet::from_snapshot(serde_json::from_str(&json).unwrap()).unwrap();
    assert_eq!(loaded, set);
    for requester in [
        who("bob", &[]),
        who("alice", &[]),
        who("dave", &[]),
        who("erin", &[]),
    ] {
        for object in ["a", "b"] {
            assert_eq!(
                reads(&loaded, &requester, object),
                reads(&set, &requester, object)
            );
        }
    }
    assert_eq!(reads(&loaded, &who("dave", &[]), "a"), Decision::Deny);
    assert_eq!(reads(&loaded, &who("erin", &[]), "a"), Decision::Deny);
}

#[test]
fn grants_load_in_any_order() {
    let mut snapshot = populated().to_snapshot();
    snapshot.grants.reverse();
    assert_eq!(GrantSet::from_snapshot(snapshot).unwrap(), populated());
}

#[test]
fn an_edited_child_wider_than_its_parent_does_not_load() {
    let mut snapshot = populated().to_snapshot();
    let c1 = snapshot
        .grants
        .iter_mut()
        .find(|g| g.id == id("c1"))
        .unwrap();
    c1.operations.insert(Operation::Write);
    assert_eq!(
        GrantSet::from_snapshot(snapshot),
        Err(LoadError::Invalid(
            id("c1"),
            GrantError::ExtraOperation(Operation::Write)
        ))
    );
}

#[test]
fn an_edited_public_grant_with_write_does_not_load() {
    let mut snapshot = GrantSet::new().to_snapshot();
    let mut public = root("pub", Subject::Public);
    public.delegation = Delegation::default();
    public.operations.insert(Operation::Write);
    snapshot.grants.push(public);
    assert_eq!(
        GrantSet::from_snapshot(snapshot),
        Err(LoadError::Invalid(
            id("pub"),
            GrantError::PublicOperation(Operation::Write)
        ))
    );
}

#[test]
fn a_child_issued_by_someone_outside_the_parent_does_not_load() {
    let mut snapshot = populated().to_snapshot();
    let c1 = snapshot
        .grants
        .iter_mut()
        .find(|g| g.id == id("c1"))
        .unwrap();
    c1.issuer = "mallory".to_owned();
    assert_eq!(
        GrantSet::from_snapshot(snapshot),
        Err(LoadError::Invalid(
            id("c1"),
            GrantError::DelegatorNotSubject
        ))
    );
}

#[test]
fn missing_parents_loops_and_duplicates_do_not_load() {
    let mut orphan = GrantSet::new().to_snapshot();
    orphan.grants.push(child("c1", "nowhere", "alice"));
    assert_eq!(
        GrantSet::from_snapshot(orphan),
        Err(LoadError::UnknownParent(id("c1")))
    );

    let mut looped = GrantSet::new().to_snapshot();
    looped.grants.push(child("x", "y", "bob"));
    looped.grants.push(child("y", "x", "bob"));
    assert!(matches!(
        GrantSet::from_snapshot(looped),
        Err(LoadError::UnknownParent(_))
    ));

    let mut twice = populated().to_snapshot();
    let first = twice.grants[0].clone();
    twice.grants.push(first.clone());
    assert_eq!(
        GrantSet::from_snapshot(twice),
        Err(LoadError::DuplicateId(first.id))
    );
}

#[test]
fn revocations_and_served_snapshots_must_name_known_grants() {
    let mut unknown = populated().to_snapshot();
    unknown.revoked.insert(id("ghost"));
    assert_eq!(
        GrantSet::from_snapshot(unknown),
        Err(LoadError::UnknownId(id("ghost")))
    );

    let mut not_snapshot = populated().to_snapshot();
    not_snapshot.consumed.insert(id("alice-root"));
    assert_eq!(
        GrantSet::from_snapshot(not_snapshot),
        Err(LoadError::NotSnapshotOnce(id("alice-root")))
    );
}

#[test]
fn an_unknown_format_does_not_load() {
    let snapshot = GrantSetSnapshot {
        format: SNAPSHOT_FORMAT + 1,
        grants: Vec::new(),
        revoked: BTreeSet::new(),
        consumed: BTreeSet::new(),
    };
    assert_eq!(
        GrantSet::from_snapshot(snapshot),
        Err(LoadError::UnknownFormat(SNAPSHOT_FORMAT + 1))
    );
}

#[test]
fn an_expired_grant_loads_but_still_allows_nothing() {
    let mut set = GrantSet::new();
    let mut short = root("short", Subject::Identity("bob".to_owned()));
    short.duration = Duration::ExpiresAt(NOW - 1);
    short.delegation = Delegation::default();
    set.insert_root(short).unwrap();
    let loaded = GrantSet::from_snapshot(set.to_snapshot()).unwrap();
    assert_eq!(reads(&loaded, &who("bob", &[]), "a"), Decision::Deny);
}
