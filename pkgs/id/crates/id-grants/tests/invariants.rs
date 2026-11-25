//! The #5799 record's seven implementation invariants, and the #5802
//! public-subject rules, as tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeSet;

use id_grants::{
    AccessRequest, Decision, Delegation, Duration, Grant, GrantError, GrantId, GrantSet, Operation,
    Requester, Scope, Subject,
};

const NOW: u64 = 1_800_000_000;

fn id(s: &str) -> GrantId {
    GrantId(s.to_owned())
}

fn grant(name: &str, subject: Subject, scopes: &[Scope], ops: &[Operation]) -> Grant {
    Grant {
        id: id(name),
        issuer: "owner".to_owned(),
        subject,
        scopes: scopes.iter().cloned().collect(),
        operations: ops.iter().copied().collect(),
        duration: Duration::Standing,
        delegation: Delegation::default(),
        parent: None,
    }
}

fn alice() -> Requester {
    Requester {
        identity: "alice".to_owned(),
        groups: BTreeSet::from(["editors".to_owned()]),
    }
}

fn bob() -> Requester {
    Requester {
        identity: "bob".to_owned(),
        groups: BTreeSet::new(),
    }
}

fn object(name: &str) -> Scope {
    Scope::Object(name.to_owned())
}

fn collection(name: &str) -> Scope {
    Scope::Collection(name.to_owned())
}

fn ask<'a>(
    set: &GrantSet,
    who: Option<&'a Requester>,
    obj: &'a str,
    coll: Option<&'a str>,
    op: Operation,
    now: u64,
) -> Decision {
    set.authorize(&AccessRequest {
        requester: who,
        object: obj,
        collection: coll,
        operation: op,
        now,
    })
}

fn alice_reads_note() -> GrantSet {
    let mut set = GrantSet::new();
    set.insert_root(grant(
        "g1",
        Subject::Identity("alice".to_owned()),
        &[object("note-1")],
        &[Operation::Read],
    ))
    .unwrap();
    set
}

// Invariant 1: subject, scope, operation, and lifetime all have to match.
#[test]
fn access_needs_matching_subject_scope_operation_and_lifetime() {
    let set = alice_reads_note();
    let a = alice();
    let b = bob();
    assert_eq!(
        ask(&set, Some(&a), "note-1", None, Operation::Read, NOW),
        Decision::Allow(id("g1"))
    );
    assert_eq!(
        ask(&set, Some(&b), "note-1", None, Operation::Read, NOW),
        Decision::Deny
    );
    assert_eq!(
        ask(&set, None, "note-1", None, Operation::Read, NOW),
        Decision::Deny
    );
    assert_eq!(
        ask(&set, Some(&a), "note-2", None, Operation::Read, NOW),
        Decision::Deny
    );
    assert_eq!(
        ask(&set, Some(&a), "note-1", None, Operation::Write, NOW),
        Decision::Deny
    );
}

#[test]
fn group_grants_cover_members_only() {
    let mut set = GrantSet::new();
    set.insert_root(grant(
        "g",
        Subject::Group("editors".to_owned()),
        &[object("doc")],
        &[Operation::Read],
    ))
    .unwrap();
    assert_eq!(
        ask(&set, Some(&alice()), "doc", None, Operation::Read, NOW),
        Decision::Allow(id("g"))
    );
    assert_eq!(
        ask(&set, Some(&bob()), "doc", None, Operation::Read, NOW),
        Decision::Deny
    );
}

// Invariant 2: expiry is checked on every access.
#[test]
fn expiry_is_checked_on_each_access_and_is_inclusive() {
    let mut set = GrantSet::new();
    let mut g = grant(
        "g",
        Subject::Identity("alice".to_owned()),
        &[object("o")],
        &[Operation::Read, Operation::Subscribe],
    );
    g.duration = Duration::ExpiresAt(NOW);
    set.insert_root(g).unwrap();
    let a = alice();
    assert_eq!(
        ask(&set, Some(&a), "o", None, Operation::Read, NOW),
        Decision::Allow(id("g"))
    );
    assert_eq!(
        ask(&set, Some(&a), "o", None, Operation::Subscribe, NOW + 1),
        Decision::Deny
    );
}

#[test]
fn a_missing_duration_does_not_deserialize() {
    let json_without_duration = r#"{
        "id": "g", "issuer": "owner", "subject": {"identity": "alice"},
        "scopes": [{"object": "o"}], "operations": ["read"]
    }"#;
    assert!(serde_json_like_parse(json_without_duration).is_err());
    let with =
        json_without_duration.replace(r#""operations""#, r#""duration": "standing", "operations""#);
    assert!(serde_json_like_parse(&with).is_ok());
}

fn serde_json_like_parse(json: &str) -> Result<Grant, serde_json::Error> {
    serde_json::from_str(json)
}

// Invariant 3: a collection grant cannot escape its collection.
#[test]
fn collection_grant_covers_its_members_and_nothing_else() {
    let mut set = GrantSet::new();
    set.insert_root(grant(
        "g",
        Subject::Identity("alice".to_owned()),
        &[collection("recipes")],
        &[Operation::Read],
    ))
    .unwrap();
    let a = alice();
    assert_eq!(
        ask(
            &set,
            Some(&a),
            "added-later",
            Some("recipes"),
            Operation::Read,
            NOW
        ),
        Decision::Allow(id("g"))
    );
    assert_eq!(
        ask(&set, Some(&a), "x", Some("finances"), Operation::Read, NOW),
        Decision::Deny
    );
    assert_eq!(
        ask(&set, Some(&a), "recipes", None, Operation::Read, NOW),
        Decision::Deny,
        "an object named like the collection is not in it"
    );
}

// Invariant 4: subscribe requires read; write and append are explicit.
#[test]
fn operations_are_explicit_and_subscribe_requires_read() {
    let mut g = grant(
        "g",
        Subject::Identity("alice".to_owned()),
        &[object("o")],
        &[Operation::Subscribe],
    );
    assert_eq!(g.validate(), Err(GrantError::SubscribeWithoutRead));
    g.operations = BTreeSet::new();
    assert_eq!(g.validate(), Err(GrantError::EmptyOperations));

    let mut set = GrantSet::new();
    set.insert_root(grant(
        "g",
        Subject::Identity("alice".to_owned()),
        &[object("o")],
        &[Operation::Read, Operation::Subscribe, Operation::Append],
    ))
    .unwrap();
    let a = alice();
    assert_eq!(
        ask(&set, Some(&a), "o", None, Operation::Append, NOW),
        Decision::Allow(id("g"))
    );
    assert_eq!(
        ask(&set, Some(&a), "o", None, Operation::Write, NOW),
        Decision::Deny,
        "append never implies write"
    );
}

#[test]
fn creation_checks_refuse_empty_and_inconsistent_grants() {
    let base = grant(
        "g",
        Subject::Identity("alice".to_owned()),
        &[object("o")],
        &[Operation::Read],
    );
    let mut g = base.clone();
    g.scopes.clear();
    assert_eq!(g.validate(), Err(GrantError::EmptyScope));
    let mut g = base.clone();
    g.subject = Subject::Identity(String::new());
    assert_eq!(g.validate(), Err(GrantError::EmptyName));
    let mut g = base.clone();
    g.delegation.may_redelegate = true;
    assert_eq!(g.validate(), Err(GrantError::RedelegateWithoutDelegate));
    let mut g = base.clone();
    g.duration = Duration::SnapshotOnce;
    g.operations.insert(Operation::Subscribe);
    assert_eq!(g.validate(), Err(GrantError::SnapshotOnceNotReadOnly));
    let mut g = base;
    g.parent = Some(id("p"));
    assert_eq!(
        GrantSet::new().insert_root(g),
        Err(GrantError::UnknownParent)
    );
}

#[test]
fn snapshot_once_serves_one_snapshot() {
    let mut set = GrantSet::new();
    let mut g = grant(
        "g",
        Subject::Identity("alice".to_owned()),
        &[object("o")],
        &[Operation::Read],
    );
    g.duration = Duration::SnapshotOnce;
    set.insert_root(g).unwrap();
    let a = alice();
    assert_eq!(
        ask(&set, Some(&a), "o", None, Operation::Read, NOW),
        Decision::Allow(id("g"))
    );
    assert!(set.consume_snapshot(&id("g")));
    assert_eq!(
        ask(&set, Some(&a), "o", None, Operation::Read, NOW),
        Decision::Deny
    );
}

fn delegable_set() -> GrantSet {
    let mut set = GrantSet::new();
    let mut parent = grant(
        "parent",
        Subject::Identity("alice".to_owned()),
        &[object("a"), object("b")],
        &[Operation::Read, Operation::Subscribe],
    );
    parent.duration = Duration::ExpiresAt(NOW + 100);
    parent.delegation = Delegation {
        may_delegate: true,
        may_redelegate: false,
    };
    set.insert_root(parent).unwrap();
    set
}

fn child(name: &str, parent: &str) -> Grant {
    Grant {
        id: id(name),
        issuer: "alice".to_owned(),
        subject: Subject::Identity("bob".to_owned()),
        scopes: BTreeSet::from([object("a")]),
        operations: BTreeSet::from([Operation::Read]),
        duration: Duration::ExpiresAt(NOW + 50),
        delegation: Delegation::default(),
        parent: Some(id(parent)),
    }
}

// Invariant 5: delegated grants are attenuated; parent revocation cascades.
#[test]
fn delegation_must_attenuate() {
    let a = alice();
    let mut set = delegable_set();
    set.insert_delegated(child("c", "parent"), &a, NOW).unwrap();

    let mut wider = child("c2", "parent");
    wider.scopes.insert(object("z"));
    assert_eq!(
        set.clone().insert_delegated(wider, &a, NOW),
        Err(GrantError::WiderScope)
    );
    let mut more_ops = child("c2", "parent");
    more_ops.operations.insert(Operation::Write);
    assert_eq!(
        set.clone().insert_delegated(more_ops, &a, NOW),
        Err(GrantError::ExtraOperation(Operation::Write))
    );
    let mut later = child("c2", "parent");
    later.duration = Duration::ExpiresAt(NOW + 101);
    assert_eq!(
        set.clone().insert_delegated(later, &a, NOW),
        Err(GrantError::LaterExpiry)
    );
    let mut standing = child("c2", "parent");
    standing.duration = Duration::Standing;
    assert_eq!(
        set.clone().insert_delegated(standing, &a, NOW),
        Err(GrantError::LaterExpiry)
    );
    let mut redelegating = child("c2", "parent");
    redelegating.delegation.may_delegate = true;
    assert_eq!(
        set.clone().insert_delegated(redelegating, &a, NOW),
        Err(GrantError::RedelegationNotAllowed)
    );
    let mut wrong_issuer = child("c2", "parent");
    wrong_issuer.issuer = "mallory".to_owned();
    assert_eq!(
        set.clone().insert_delegated(wrong_issuer, &a, NOW),
        Err(GrantError::IssuerNotDelegator)
    );
    assert_eq!(
        set.clone()
            .insert_delegated(child("c2", "parent"), &bob(), NOW),
        Err(GrantError::DelegatorNotSubject)
    );
    assert_eq!(
        set.clone().insert_delegated(child("c", "parent"), &a, NOW),
        Err(GrantError::DuplicateId)
    );
}

#[test]
fn delegation_is_off_by_default() {
    let mut set = alice_reads_note();
    let mut c = child("c", "g1");
    c.scopes = BTreeSet::from([object("note-1")]);
    assert_eq!(
        set.insert_delegated(c, &alice(), NOW),
        Err(GrantError::ParentNotDelegable)
    );
}

#[test]
fn revoking_a_parent_cascades_to_its_children() {
    let a = alice();
    let b = bob();
    let mut set = delegable_set();
    set.insert_delegated(child("c", "parent"), &a, NOW).unwrap();
    assert_eq!(
        ask(&set, Some(&b), "a", None, Operation::Read, NOW),
        Decision::Allow(id("c"))
    );
    assert!(set.revoke(&id("parent")));
    assert_eq!(
        ask(&set, Some(&b), "a", None, Operation::Read, NOW),
        Decision::Deny
    );
    assert_eq!(
        set.insert_delegated(child("c3", "parent"), &a, NOW),
        Err(GrantError::UnknownParent),
        "a revoked parent cannot delegate"
    );
}

#[test]
fn a_parent_expiring_ends_its_children() {
    let a = alice();
    let b = bob();
    let mut set = delegable_set();
    let mut snapshot = child("c", "parent");
    snapshot.duration = Duration::SnapshotOnce;
    set.insert_delegated(snapshot, &a, NOW).unwrap();
    assert_eq!(
        ask(&set, Some(&b), "a", None, Operation::Read, NOW + 101),
        Decision::Deny
    );
}

// Invariant 6: an unknown resource and an ungranted one look the same.
#[test]
fn deny_carries_nothing_that_separates_absent_from_ungranted() {
    let set = alice_reads_note();
    let b = bob();
    let ungranted = ask(&set, Some(&b), "note-1", None, Operation::Read, NOW);
    let absent = ask(&set, Some(&b), "no-such-note", None, Operation::Read, NOW);
    assert_eq!(ungranted, absent);
    assert_eq!(format!("{ungranted:?}"), format!("{absent:?}"));
}

// Invariant 7: revocation means future denial.
#[test]
fn revocation_denies_from_the_next_check() {
    let mut set = alice_reads_note();
    let a = alice();
    assert!(matches!(
        ask(&set, Some(&a), "note-1", None, Operation::Read, NOW),
        Decision::Allow(_)
    ));
    assert!(set.revoke(&id("g1")));
    assert_eq!(
        ask(&set, Some(&a), "note-1", None, Operation::Read, NOW),
        Decision::Deny
    );
    assert!(!set.revoke(&id("unknown")));
}

// #5802 D1 and D3: the public audience.
#[test]
fn public_grants_are_read_only_and_never_delegable() {
    let mut g = grant(
        "p",
        Subject::Public,
        &[collection("site")],
        &[Operation::Read],
    );
    for op in [Operation::Write, Operation::Append] {
        let mut bad = g.clone();
        bad.operations.insert(op);
        assert_eq!(bad.validate(), Err(GrantError::PublicOperation(op)));
    }
    g.delegation.may_delegate = true;
    assert_eq!(g.validate(), Err(GrantError::PublicDelegation));
}

#[test]
fn public_grants_cover_anonymous_requesters() {
    let mut set = GrantSet::new();
    set.insert_root(grant(
        "p",
        Subject::Public,
        &[collection("site")],
        &[Operation::Read, Operation::Subscribe],
    ))
    .unwrap();
    assert_eq!(
        ask(&set, None, "page", Some("site"), Operation::Subscribe, NOW),
        Decision::Allow(id("p"))
    );
    assert_eq!(
        ask(
            &set,
            Some(&bob()),
            "page",
            Some("site"),
            Operation::Read,
            NOW
        ),
        Decision::Allow(id("p"))
    );
}

#[test]
fn a_public_grant_never_borrows_a_private_grants_scope() {
    // Alice holds a private write grant on one page; the public read grant
    // covers the whole site. Neither widens the other: anonymous readers
    // still cannot write, and Alice's write is allowed only by her own
    // grant.
    let mut set = GrantSet::new();
    set.insert_root(grant(
        "p",
        Subject::Public,
        &[collection("site")],
        &[Operation::Read],
    ))
    .unwrap();
    set.insert_root(grant(
        "w",
        Subject::Identity("alice".to_owned()),
        &[object("page")],
        &[Operation::Read, Operation::Write],
    ))
    .unwrap();
    assert_eq!(
        ask(&set, None, "page", Some("site"), Operation::Write, NOW),
        Decision::Deny
    );
    assert_eq!(
        ask(
            &set,
            Some(&alice()),
            "page",
            Some("site"),
            Operation::Write,
            NOW
        ),
        Decision::Allow(id("w"))
    );
    assert_eq!(
        ask(
            &set,
            Some(&alice()),
            "other",
            Some("site"),
            Operation::Write,
            NOW
        ),
        Decision::Deny
    );
}

// A host checks grants against the resource owner: only grants rooted in
// a grant the owner issued count, delegated ones included.
#[test]
fn rooted_check_counts_only_the_owners_lineage() {
    let a = alice();
    let b = bob();
    let mut set = delegable_set();
    set.insert_delegated(child("c", "parent"), &a, NOW).unwrap();
    let request = AccessRequest {
        requester: Some(&b),
        object: "a",
        collection: None,
        operation: Operation::Read,
        now: NOW,
    };
    assert_eq!(
        set.authorize_rooted_in(&request, "owner"),
        Decision::Allow(id("c"))
    );
    assert_eq!(set.authorize_rooted_in(&request, "mallory"), Decision::Deny);

    // A root grant someone else issued never counts for the owner.
    let mut stray = GrantSet::new();
    let mut foreign = grant(
        "g9",
        Subject::Identity("bob".to_owned()),
        &[object("a")],
        &[Operation::Read],
    );
    foreign.issuer = "mallory".to_owned();
    stray.insert_root(foreign).unwrap();
    assert_eq!(stray.authorize_rooted_in(&request, "owner"), Decision::Deny);
    assert_eq!(stray.authorize(&request), Decision::Allow(id("g9")));

    // Revoking the owner's root still cascades under the rooted check.
    set.revoke(&id("parent"));
    assert_eq!(set.authorize_rooted_in(&request, "owner"), Decision::Deny);
}
