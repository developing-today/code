//! Alias display over tag claims (Phase 5, P5.7; owner decisions 2026-10-12).
//!
//! Aliases are tag claims, so they are read through [`EventStore::tag_claims`] and shown by
//! `aliases::rows`. The checks:
//!
//! 1. An identity's alias is a claim on its first identity event. Two aliases for one identity are
//!    both shown as contested, and nothing picks one.
//! 2. A server's primary alias is stored and wins for display over the claims. A pet name, which
//!    only this client sees, wins over the primary.
//! 3. A prefix of each subject ID is shown. It is the shortest one that no other row in the view
//!    shares, and the full ID when no prefix is unique.
//! 4. Nothing is unique: two identities may claim the same alias, and both are returned.
//!
//! No identity event kind exists yet (#160), so the identity's first identity event here is the
//! first event of its writer's chain. The subject rule belongs to the caller, so the display does
//! not depend on the event's kind.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id::aliases::{ALIAS_NAME, Named, Object, Shown, aliases_of, rows};
use id::chain::{ChainExt, Event, sign};
use id::event_store::{EventStore, Ingest};
use id::tag_claims::{SET_KIND, TagOp};
use id::waiting::{AuthorKey, WaitingLimits};
use p2panda_core::{Hash, SeqNum, SigningKey};

/// A signer with its own chain. Each event names the one before it as its `after`.
struct Writer {
    key: SigningKey,
    seq: SeqNum,
    last: Option<Hash>,
}

impl Writer {
    fn new(n: u8) -> Self {
        Self {
            key: SigningKey::from_bytes(&[n; 32]),
            seq: 0,
            last: None,
        }
    }

    fn author(&self) -> AuthorKey {
        *self.key.verifying_key().as_bytes()
    }

    /// Signs a `tag.set` of `name`, with `value`, on `subject`.
    fn tag(&mut self, subject: Hash, name: &str, value: Option<&str>) -> Event {
        let ext = ChainExt {
            kind: SET_KIND.to_owned(),
            group: None,
            after: self.last.into_iter().collect(),
            tag: Some(TagOp::Set {
                subject,
                name: name.to_owned(),
                value: value.map(str::to_owned),
            }),
            document: None,
            removes: None,
            chat: None,
        };
        let event = sign(&self.key, self.seq, self.last, None, ext).unwrap();
        self.seq += 1;
        self.last = Some(event.id);
        event
    }
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

/// Delivers each event's bytes, as sync does, and checks that none was refused.
fn deliver(store: &mut EventStore, events: &[&Event]) {
    for event in events {
        let outcome = store.ingest(&event.bytes, 0).unwrap();
        assert!(
            !matches!(outcome, Ingest::Refused(_)),
            "refused: {outcome:?}"
        );
    }
}

fn named(subject: Hash, name: &str) -> Named {
    Named {
        subject,
        name: name.to_owned(),
    }
}

/// A group `team` with primary alias `team`, and its ID. The ID is the subject its alias
/// claims name.
fn team_subject_and_primary() -> (Hash, BTreeMap<Object, String>) {
    let team = id_ending_in(7);
    let primary = BTreeMap::from([(Object::Group(team), "team".to_owned())]);
    (team, primary)
}

/// An ID whose first five bytes are fixed, so two such IDs share their first eight hex digits.
fn id_ending_in(last: u8) -> Hash {
    let mut bytes = [0u8; 32];
    bytes[..4].copy_from_slice(&[0xab, 0xcd, 0xef, 0x01]);
    bytes[4] = last;
    Hash::from_bytes(bytes)
}

#[test]
fn test_an_alias_is_a_tag_claim_on_the_identitys_first_event() {
    let mut ann = Writer::new(1);
    let identity = ann.tag(Hash::digest(b"ann"), "identity", None);
    let alias = ann.tag(identity.id, ALIAS_NAME, Some("Ann"));
    // Other claims on the identity are not aliases, and an alias on another subject is not its.
    let seen = ann.tag(identity.id, "seen", None);
    let elsewhere = ann.tag(Hash::digest(b"elsewhere"), ALIAS_NAME, Some("Ann"));

    let (_dir, mut store) = open();
    deliver(&mut store, &[&identity, &alias, &seen, &elsewhere]);
    let claims = store.tag_claims().unwrap();

    let found = aliases_of(&claims, &identity.id);
    assert_eq!(found.len(), 1, "{found:?}");
    assert_eq!(found[0].text, "Ann");
    assert_eq!(found[0].claim, alias.id);
    assert_eq!(found[0].author, ann.author());
}

#[test]
fn test_two_aliases_for_one_identity_are_both_shown_as_contested() {
    let mut ann = Writer::new(1);
    let mut bo = Writer::new(2);
    let identity = ann.tag(Hash::digest(b"ann"), "identity", None);
    let by_ann = ann.tag(identity.id, ALIAS_NAME, Some("Ann"));
    let by_bo = bo.tag(identity.id, ALIAS_NAME, Some("Annie"));

    let (_dir, mut store) = open();
    deliver(&mut store, &[&identity, &by_ann, &by_bo]);
    let claims = store.tag_claims().unwrap();

    let view = rows(
        &[named(identity.id, "ann")],
        &claims,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].shown, Shown::Contested);
    let mut texts: Vec<&str> = view[0].aliases.iter().map(|a| a.text.as_str()).collect();
    texts.sort_unstable();
    assert_eq!(texts, ["Ann", "Annie"]);
    let authors: Vec<AuthorKey> = view[0].aliases.iter().map(|a| a.author).collect();
    assert!(authors.contains(&ann.author()) && authors.contains(&bo.author()));
}

#[test]
fn test_a_server_primary_alias_wins_over_the_claims_for_display() {
    let mut ann = Writer::new(1);
    let mut bo = Writer::new(2);
    let (team, primary) = team_subject_and_primary();
    let by_bo = bo.tag(team, ALIAS_NAME, Some("Team Alpha"));
    let by_ann = ann.tag(team, ALIAS_NAME, Some("alpha"));

    let (_dir, mut store) = open();
    deliver(&mut store, &[&by_bo, &by_ann]);
    let claims = store.tag_claims().unwrap();

    let view = rows(&[named(team, "Team")], &claims, &primary, &BTreeMap::new());
    assert_eq!(view.len(), 1);
    assert_eq!(view[0].shown, Shown::Primary("team".to_owned()));
    // The claims are still there for anyone who looks at the full set.
    assert_eq!(view[0].aliases.len(), 2);
}

#[test]
fn test_a_pet_name_wins_over_the_primary_alias_and_adds_no_claim() {
    let mut ann = Writer::new(1);
    let (team, primary) = team_subject_and_primary();
    let by_ann = ann.tag(team, ALIAS_NAME, Some("alpha"));

    let (_dir, mut store) = open();
    deliver(&mut store, &[&by_ann]);
    let claims = store.tag_claims().unwrap();
    let before = claims.len();

    let pets = BTreeMap::from([(team, "Our crew".to_owned())]);
    let view = rows(&[named(team, "Team")], &claims, &primary, &pets);
    assert_eq!(view[0].shown, Shown::PetName("Our crew".to_owned()));
    // A pet name is local. It is not an event, so the store holds the same claims.
    assert_eq!(store.tag_claims().unwrap().len(), before);
}

#[test]
fn test_two_identities_may_claim_the_same_alias_and_both_are_returned() {
    let mut ann = Writer::new(1);
    let mut bo = Writer::new(2);
    let ann_identity = ann.tag(Hash::digest(b"ann"), "identity", None);
    let bo_identity = bo.tag(Hash::digest(b"bo"), "identity", None);
    let ann_alias = ann.tag(ann_identity.id, ALIAS_NAME, Some("Sam"));
    let bo_alias = bo.tag(bo_identity.id, ALIAS_NAME, Some("Sam"));

    let (_dir, mut store) = open();
    deliver(
        &mut store,
        &[&ann_identity, &bo_identity, &ann_alias, &bo_alias],
    );
    let claims = store.tag_claims().unwrap();

    let ann_found = aliases_of(&claims, &ann_identity.id);
    let bo_found = aliases_of(&claims, &bo_identity.id);
    assert_eq!(ann_found.len(), 1, "{ann_found:?}");
    assert_eq!(bo_found.len(), 1, "{bo_found:?}");
    assert_eq!(ann_found[0].claim, ann_alias.id);
    assert_eq!(bo_found[0].claim, bo_alias.id);

    let view = rows(
        &[named(ann_identity.id, "ann"), named(bo_identity.id, "bo")],
        &claims,
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    assert_eq!(view.len(), 2);
    for row in &view {
        assert!(
            matches!(&row.shown, Shown::Only(alias) if alias.text == "Sam"),
            "{:?}",
            row.shown
        );
    }
}

#[test]
fn test_each_row_shows_a_prefix_of_its_subject_unique_among_the_rows() {
    // Two identities named `ann` whose IDs share their first eight hex digits, and one that does
    // not share them.
    let (x, y, z) = (id_ending_in(0x10), id_ending_in(0x20), Hash::digest(b"zed"));
    let view = rows(
        &[named(x, "ann"), named(y, "ann"), named(z, "zed")],
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    assert_eq!(view.len(), 3);
    // They differ at the ninth hex digit, so each prefix is nine digits.
    assert_eq!(view[0].prefix, "abcdef011");
    assert_eq!(view[1].prefix, "abcdef012");
    for (i, row) in view.iter().enumerate() {
        assert!(
            row.subject.to_hex().starts_with(&row.prefix),
            "{} is not a prefix of {}",
            row.prefix,
            row.subject.to_hex()
        );
        for (j, other) in view.iter().enumerate() {
            if i != j {
                assert!(
                    !other.subject.to_hex().starts_with(&row.prefix),
                    "{} is shared with another row",
                    row.prefix
                );
            }
        }
    }
}

#[test]
fn test_the_full_id_is_shown_when_no_prefix_is_unique() {
    // Two rows with the same subject share every prefix, so only the full ID is unique to neither.
    let subject = Hash::digest(b"ann");
    let view = rows(
        &[named(subject, "ann"), named(subject, "ann")],
        &[],
        &BTreeMap::new(),
        &BTreeMap::new(),
    );
    assert_eq!(view.len(), 2);
    for row in &view {
        assert_eq!(row.prefix, subject.to_hex());
    }
}
