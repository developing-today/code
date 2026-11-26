//! The owner's public listing (#5802 D2 and invariant 3; #14782), as
//! tests.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::collections::{BTreeMap, BTreeSet};

use id_grants::{
    ClassLevel, Delegation, DiscoveryMode, Duration, Grant, GrantId, GrantSet, ListingEntry,
    Operation, Publications, PublishRequest, Scope, ShareId, Subject,
};

const NOW: u64 = 1_800_000_000;

fn share(byte: u8) -> ShareId {
    ShareId::mint([byte; 32])
}

fn request(object: &str, discovery: DiscoveryMode, title: &str) -> PublishRequest {
    PublishRequest {
        scope: BTreeMap::from([(Scope::Object(object.to_owned()), ClassLevel(0))]),
        subscribe: false,
        duration: Duration::Standing,
        discovery,
        ceiling: ClassLevel(0),
        owner_ceiling: ClassLevel(0),
        title: title.to_owned(),
        description: format!("about {title}"),
        now: NOW,
    }
}

/// An owner with two listed shares, one unlisted share, and a private
/// grant to a friend.
fn owner_with_shares() -> (Publications, GrantSet) {
    let mut grants = GrantSet::new();
    grants
        .insert_root(Grant {
            id: GrantId("private".to_owned()),
            issuer: "owner".to_owned(),
            subject: Subject::Identity("friend".to_owned()),
            scopes: BTreeSet::from([Scope::Object("diary".to_owned())]),
            operations: BTreeSet::from([Operation::Read]),
            duration: Duration::Standing,
            delegation: Delegation::default(),
            parent: None,
        })
        .unwrap();
    let mut book = Publications::new("owner");
    book.publish(
        &mut grants,
        share(1),
        request("recipes", DiscoveryMode::Listed, "Recipes"),
    )
    .unwrap();
    book.publish(
        &mut grants,
        share(2),
        request("drafts", DiscoveryMode::Unlisted, "Drafts"),
    )
    .unwrap();
    book.publish(
        &mut grants,
        share(3),
        request("photos", DiscoveryMode::Listed, "Photos"),
    )
    .unwrap();
    (book, grants)
}

fn shares(listing: &[ListingEntry]) -> Vec<ShareId> {
    listing.iter().map(|entry| entry.share.clone()).collect()
}

#[test]
fn the_listing_holds_only_live_listed_shares() {
    let (book, grants) = owner_with_shares();
    let listing = book.listing(&grants, NOW);
    let mut want = vec![share(1), share(3)];
    want.sort();
    assert_eq!(shares(&listing), want);
    for entry in &listing {
        // Each entry resolves to its manifest, and carries only the
        // reference, the title and the description.
        let manifest = book.resolve(&entry.share, &grants, NOW).unwrap();
        assert_eq!(manifest.title, entry.title);
        assert_eq!(manifest.description, entry.description);
        assert_eq!(manifest.discovery, DiscoveryMode::Listed);
    }
}

#[test]
fn the_listing_entry_has_no_counts_or_scope() {
    let (book, grants) = owner_with_shares();
    let entry = book.listing(&grants, NOW).remove(0);
    let json = serde_json::to_value(&entry).unwrap();
    let keys: BTreeSet<&str> = json
        .as_object()
        .unwrap()
        .keys()
        .map(String::as_str)
        .collect();
    assert_eq!(keys, BTreeSet::from(["share", "title", "description"]));
}

#[test]
fn unpublishing_a_listed_share_drops_it_at_the_next_check() {
    let (mut book, mut grants) = owner_with_shares();
    assert!(book.unpublish(&mut grants, &share(1), NOW + 1));
    assert_eq!(shares(&book.listing(&grants, NOW + 1)), vec![share(3)]);
}

#[test]
fn a_revoked_grant_drops_its_share_even_before_unpublish() {
    let (book, mut grants) = owner_with_shares();
    let grant = book
        .audit()
        .find(|record| record.share == share(3))
        .unwrap()
        .grant
        .clone();
    grants.revoke(&grant);
    assert_eq!(shares(&book.listing(&grants, NOW)), vec![share(1)]);
    assert_eq!(book.resolve(&share(3), &grants, NOW), None);
}

#[test]
fn an_expired_listed_share_drops_out() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    let mut req = request("recipes", DiscoveryMode::Listed, "Recipes");
    req.duration = Duration::ExpiresAt(NOW + 10);
    book.publish(&mut grants, share(1), req).unwrap();
    assert_eq!(book.listing(&grants, NOW).len(), 1);
    assert_eq!(
        book.listing(&grants, NOW + 11),
        [] as [id_grants::ListingEntry; 0]
    );
}

#[test]
fn an_owner_with_nothing_listed_has_an_empty_listing() {
    let mut grants = GrantSet::new();
    let mut book = Publications::new("owner");
    book.publish(
        &mut grants,
        share(1),
        request("drafts", DiscoveryMode::Unlisted, "Drafts"),
    )
    .unwrap();
    assert_eq!(
        book.listing(&grants, NOW),
        [] as [id_grants::ListingEntry; 0]
    );
    assert_eq!(
        Publications::new("someone").listing(&grants, NOW),
        [] as [id_grants::ListingEntry; 0]
    );
}
