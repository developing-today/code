//! The #5802 publish and unpublish rules (D1, D2.3, D5; invariants 2,
//! 3, 6 and 7), as tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::BTreeMap;

use id_grants::{
    AccessRequest, ClassLevel, Decision, DiscoveryMode, Duration, GrantError, GrantSet, Operation,
    Publications, PublishError, PublishRequest, Requester, Scope, ShareId, UNLISTED_LABEL,
    UNPUBLISH_NOTICE,
};

const NOW: u64 = 1_800_000_000;

fn share(byte: u8) -> ShareId {
    ShareId::mint([byte; 32])
}

fn request(scope: &[(&str, u8)]) -> PublishRequest {
    PublishRequest {
        scope: scope
            .iter()
            .map(|(id, level)| (Scope::Object((*id).to_owned()), ClassLevel(*level)))
            .collect::<BTreeMap<_, _>>(),
        subscribe: false,
        duration: Duration::Standing,
        discovery: DiscoveryMode::default(),
        ceiling: ClassLevel(1),
        owner_ceiling: ClassLevel(2),
        title: "Recipes".to_owned(),
        description: "My recipes".to_owned(),
        now: NOW,
    }
}

fn anyone_reads(grants: &GrantSet, object: &str, now: u64) -> bool {
    let stranger = Requester {
        identity: "stranger".to_owned(),
        ..Requester::default()
    };
    [None, Some(&stranger)].into_iter().all(|who| {
        matches!(
            grants.authorize(&AccessRequest {
                requester: who,
                object,
                collection: None,
                operation: Operation::Read,
                now,
            }),
            Decision::Allow(_)
        )
    })
}

#[test]
fn share_ids_are_hex_of_the_entropy_and_not_derived_from_objects() {
    let id = share(0xab);
    assert_eq!(id.as_str().len(), 64);
    assert!(id.as_str().chars().all(|c| c == 'a' || c == 'b'));
    assert_ne!(share(1), share(2));
}

#[test]
fn publish_creates_grant_record_and_manifest_together() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    let record = book
        .publish(&mut grants, share(1), request(&[("note-1", 0)]))
        .unwrap()
        .clone();
    assert_eq!(record.operations.len(), 1);
    assert!(record.operations.contains(&Operation::Read));
    assert_eq!(record.discovery, DiscoveryMode::Unlisted);
    let grant = grants.get(&record.grant).unwrap();
    assert_eq!(grant.issuer, "owner");
    assert!(!grant.delegation.may_delegate);
    let manifest = book.resolve(&share(1), &grants, NOW).unwrap();
    assert_eq!(manifest.owner, "owner");
    assert_eq!(manifest.scope, record.scope);
    assert!(anyone_reads(&grants, "note-1", NOW));
    assert!(!anyone_reads(&grants, "note-2", NOW));
}

#[test]
fn subscribe_is_the_only_extra_operation() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    let mut req = request(&[("note-1", 0)]);
    req.subscribe = true;
    let record = book.publish(&mut grants, share(1), req).unwrap();
    assert!(record.operations.contains(&Operation::Subscribe));
    assert!(!record.operations.contains(&Operation::Write));
    assert!(!record.operations.contains(&Operation::Append));
}

#[test]
fn a_refused_publish_changes_nothing() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");

    assert_eq!(
        book.publish(&mut grants, share(1), request(&[])),
        Err(PublishError::EmptyScope)
    );
    let mut high = request(&[("note-1", 0)]);
    high.ceiling = ClassLevel(3);
    assert_eq!(
        book.publish(&mut grants, share(1), high),
        Err(PublishError::CeilingAboveOwner)
    );
    assert_eq!(
        book.publish(
            &mut grants,
            share(1),
            request(&[("note-1", 0), ("secret", 2)])
        ),
        Err(PublishError::AboveCeiling(Scope::Object(
            "secret".to_owned()
        )))
    );
    // A snapshot_once share with subscribe is refused by the grants model.
    let mut snap = request(&[("note-1", 0)]);
    snap.duration = Duration::SnapshotOnce;
    snap.subscribe = true;
    assert_eq!(
        book.publish(&mut grants, share(1), snap),
        Err(PublishError::Grant(GrantError::SnapshotOnceNotReadOnly))
    );

    assert_eq!(grants.grants().count(), 0);
    assert_eq!(book.audit().count(), 0);
    assert_eq!(book.resolve(&share(1), &grants, NOW), None);
    // The identifier was never used, so it can still be published.
    assert!(
        book.publish(&mut grants, share(1), request(&[("note-1", 0)]))
            .is_ok()
    );
}

#[test]
fn unpublish_revokes_all_three_and_resolves_like_nothing() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    let mut req = request(&[("note-1", 0)]);
    req.discovery = DiscoveryMode::Listed;
    book.publish(&mut grants, share(1), req).unwrap();
    assert_eq!(book.listing(&grants, NOW).len(), 1);

    assert!(book.unpublish(&mut grants, &share(1), NOW + 5));
    assert!(!anyone_reads(&grants, "note-1", NOW + 5));
    assert_eq!(
        book.listing(&grants, NOW + 5),
        [] as [id_grants::ListingEntry; 0]
    );
    // Unpublished and never-existed give the same answer.
    assert_eq!(
        book.resolve(&share(1), &grants, NOW + 5),
        book.resolve(&share(9), &grants, NOW + 5)
    );
    // A second unpublish is a no-op.
    assert!(!book.unpublish(&mut grants, &share(1), NOW + 6));

    // The record stays on the owner's audit surface, with its end.
    let audit: Vec<_> = book.audit().collect();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].unpublished_at, Some(NOW + 5));
}

#[test]
fn dead_links_stay_dead_and_republishing_mints_a_new_identifier() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    book.publish(&mut grants, share(1), request(&[("note-1", 0)]))
        .unwrap();
    book.unpublish(&mut grants, &share(1), NOW);
    assert_eq!(
        book.publish(&mut grants, share(1), request(&[("note-1", 0)])),
        Err(PublishError::IdentifierUsed)
    );
    // Purging the audit record does not free the identifier.
    assert!(book.purge(&share(1)));
    assert_eq!(book.audit().count(), 0);
    assert_eq!(
        book.publish(&mut grants, share(1), request(&[("note-1", 0)])),
        Err(PublishError::IdentifierUsed)
    );
    book.publish(&mut grants, share(2), request(&[("note-1", 0)]))
        .unwrap();
    assert!(anyone_reads(&grants, "note-1", NOW));
    assert_eq!(book.resolve(&share(1), &grants, NOW), None);
    assert!(book.resolve(&share(2), &grants, NOW).is_some());
}

#[test]
fn unlisted_shares_stay_off_the_listing() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    book.publish(&mut grants, share(1), request(&[("note-1", 0)]))
        .unwrap();
    assert_eq!(book.listing(&grants, NOW), [] as [id_grants::ListingEntry; 0]);
    assert!(book.resolve(&share(1), &grants, NOW).is_some());
}

#[test]
fn an_expired_share_resolves_like_nothing() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    let mut req = request(&[("note-1", 0)]);
    req.duration = Duration::ExpiresAt(NOW + 10);
    book.publish(&mut grants, share(1), req).unwrap();
    assert!(book.resolve(&share(1), &grants, NOW).is_some());
    assert_eq!(book.resolve(&share(1), &grants, NOW + 11), None);
}

#[test]
fn the_classification_gate_applies_at_each_emission() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    let record = book
        .publish(&mut grants, share(1), request(&[("note-1", 1)]))
        .unwrap();
    assert!(record.may_emit(ClassLevel(0)));
    assert!(record.may_emit(ClassLevel(1)));
    assert!(!record.may_emit(ClassLevel(2)));
}

#[test]
fn the_publish_copy_states_the_boundary() {
    assert_eq!(
        UNPUBLISH_NOTICE,
        "Unpublishing stops new access through ID. It cannot take back copies anyone already made."
    );
    assert_eq!(UNLISTED_LABEL, "anyone with the link");
}
