//! Store-wired writer-set expansion (writer-set spec §9 item 2; issue
//! #4998): `expand` / `expand_retro` over the membership layer, pinned to
//! the set version's frontier. These are the seven spec tests, in order,
//! plus the public-entry refusal that keeps `expand` honest about
//! returning authors only.
//!
//! The oracle for every test is the membership layer itself, fed a known
//! causal history: the expected expansion is read off the history at the
//! pinned frontier by hand, and a later event's effect (or non-effect) is
//! checked against the layer's live roster to prove the pin did the work.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeSet;

use id::groups::Role;
use id::idset::expand::{ExpandError, Expander, SetVersion};
use id::idset::{Entry, Excludes, WriterSet};
use id::membership::{AuthId, CausalEvent, Membership, Outcome, Role as MembershipRole};
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::Hash;

fn author(n: u8) -> [u8; 32] {
    [n; 32]
}

fn hash(n: u8) -> Hash {
    Hash::from([n; 32])
}

const ALICE: u8 = 1;
const BOB: u8 = 2;
const CAROL: u8 = 3;
const DAVE: u8 = 4;
const ERIN: u8 = 5;
const FRANK: u8 = 6;
const MALLORY: u8 = 99;

/// The group: its create event's ID, as in the membership scenarios.
fn group() -> Hash {
    hash(10)
}

fn individual(key: [u8; 32]) -> GroupMember<AuthId> {
    GroupMember::Individual(AuthId::Key(key))
}

fn feed(membership: &mut Membership, id: Hash, after: Vec<Hash>, role: MembershipRole) {
    let outcomes = membership.receive(CausalEvent { id, after, role }).unwrap();
    assert!(
        outcomes
            .iter()
            .any(|(event, outcome)| *event == id && *outcome == Outcome::Applied),
        "event {id} was not applied: {outcomes:?}"
    );
}

fn feed_auth(
    membership: &mut Membership,
    id: Hash,
    after: Vec<Hash>,
    author: [u8; 32],
    group: Option<Hash>,
    action: GroupAction<AuthId, ()>,
) {
    feed(
        membership,
        id,
        after,
        MembershipRole::Auth {
            author,
            group,
            action,
        },
    );
}

/// A group created by Alice: Alice manages, Bob reads, Carol writes. The
/// create event is the frontier a first set version pins.
fn base_layer() -> Membership {
    let mut membership = Membership::new();
    feed_auth(
        &mut membership,
        group(),
        vec![],
        author(ALICE),
        None,
        GroupAction::Create {
            initial_members: vec![
                (individual(author(ALICE)), Access::manage()),
                (individual(author(BOB)), Access::read()),
                (individual(author(CAROL)), Access::write()),
            ],
        },
    );
    membership
}

fn set(entries: Vec<Entry>, excludes: Vec<Entry>) -> WriterSet {
    WriterSet {
        set_id: hash(90),
        version: 3,
        owner: author(ALICE),
        scope: None,
        entries,
        excludes: Excludes(excludes),
    }
}

fn group_entry(role_min: Role, include_history: bool) -> Entry {
    Entry::Group {
        id: group(),
        role_min,
        include_history,
    }
}

fn key_entry(key: u8, include_history: bool) -> Entry {
    Entry::Key {
        author: author(key),
        include_history,
    }
}

fn expand(membership: &mut Membership, set: &WriterSet, frontier: Vec<Hash>) -> BTreeSet<[u8; 32]> {
    Expander::new(membership)
        .expand(&SetVersion {
            set,
            frontier,
            resolution: hash(80),
        })
        .unwrap()
}

/// Spec test 1: expansion is order-independent — the same set with its
/// entries permuted yields the same set of authors.
#[test]
fn test_expand_is_order_independent() {
    let mut membership = base_layer();
    let forward = set(
        vec![
            key_entry(ERIN, false),
            group_entry(Role::Read, false),
            key_entry(FRANK, true),
        ],
        vec![key_entry(BOB, false)],
    );
    let permuted = set(
        vec![
            group_entry(Role::Read, false),
            key_entry(FRANK, true),
            key_entry(ERIN, false),
        ],
        vec![key_entry(BOB, false)],
    );

    let expected: BTreeSet<[u8; 32]> = [ALICE, CAROL, ERIN, FRANK]
        .into_iter()
        .map(author)
        .collect();
    assert_eq!(expand(&mut membership, &forward, vec![group()]), expected);
    assert_eq!(expand(&mut membership, &permuted, vec![group()]), expected);
}

/// Spec test 2: expansion is frontier-pinned — a membership change after
/// the set version does not change `expand(v)`.
#[test]
fn test_expand_is_frontier_pinned() {
    let mut membership = base_layer();
    let set = set(vec![group_entry(Role::Read, false)], vec![]);
    let pinned: BTreeSet<[u8; 32]> = [ALICE, BOB, CAROL].into_iter().map(author).collect();
    assert_eq!(expand(&mut membership, &set, vec![group()]), pinned);

    // After the pin: Dave is added and Bob is removed. Neither may move
    // the pinned expansion.
    let add = hash(11);
    feed_auth(
        &mut membership,
        add,
        vec![group()],
        author(ALICE),
        Some(group()),
        GroupAction::Add {
            member: individual(author(DAVE)),
            access: Access::write(),
        },
    );
    let remove = hash(12);
    feed_auth(
        &mut membership,
        remove,
        vec![add],
        author(ALICE),
        Some(group()),
        GroupAction::Remove {
            member: individual(author(BOB)),
        },
    );

    assert_eq!(expand(&mut membership, &set, vec![group()]), pinned);
    // The pin did the work, not a frozen layer: judged at the later
    // frontier, the same set expands to the new roster.
    let later: BTreeSet<[u8; 32]> = [ALICE, CAROL, DAVE].into_iter().map(author).collect();
    assert_eq!(expand(&mut membership, &set, vec![remove]), later);
}

/// Spec test 3: expansion is unchanged by a later group-membership event —
/// the reproducibility guarantee, with the event written after the pin and
/// the expansion recomputed from scratch (fresh expander, empty cache).
#[test]
fn test_expand_unchanged_by_later_group_membership_event() {
    let mut membership = base_layer();
    let set = set(vec![group_entry(Role::Write, false)], vec![]);
    let before = expand(&mut membership, &set, vec![group()]);

    let add = hash(11);
    feed_auth(
        &mut membership,
        add,
        vec![group()],
        author(ALICE),
        Some(group()),
        GroupAction::Add {
            member: individual(author(DAVE)),
            access: Access::write(),
        },
    );
    // The live roster really did change.
    assert!(
        membership
            .members(group())
            .iter()
            .any(|(id, _)| *id == AuthId::Key(author(DAVE)))
    );

    let after = expand(&mut membership, &set, vec![group()]);
    assert_eq!(before, after);
    assert!(!after.contains(&author(DAVE)));
}

/// Spec test 4: `expand_retro(v) ⊆ expand(v)`, per entry — one entry's
/// `include_history` never imports another entry's backlog — and a set
/// with no `include_history` entries has an empty retro expansion while
/// its plain expansion is not empty.
#[test]
fn test_expand_retro_subset_and_empty_without_history_entries() {
    let mut membership = base_layer();
    let flagged = set(
        vec![
            key_entry(ERIN, true),
            key_entry(FRANK, false),
            group_entry(Role::Write, true),
        ],
        vec![],
    );
    let version = SetVersion {
        set: &flagged,
        frontier: vec![group()],
        resolution: hash(80),
    };
    let mut expander = Expander::new(&mut membership);
    let full = expander.expand(&version).unwrap();
    let retro = expander.expand_retro(&version).unwrap();

    let expected_full: BTreeSet<[u8; 32]> = [ALICE, CAROL, ERIN, FRANK]
        .into_iter()
        .map(author)
        .collect();
    let expected_retro: BTreeSet<[u8; 32]> = [ALICE, CAROL, ERIN].into_iter().map(author).collect();
    assert_eq!(full, expected_full);
    assert_eq!(retro, expected_retro);
    assert!(retro.is_subset(&full));
    // Frank's unflagged key entry and Bob's unflagged-at-Write standing
    // contribute nothing retroactively.
    assert!(!retro.contains(&author(FRANK)));
    assert!(!retro.contains(&author(BOB)));

    let no_history = set(
        vec![key_entry(ERIN, false), group_entry(Role::Read, false)],
        vec![],
    );
    let version = SetVersion {
        set: &no_history,
        frontier: vec![group()],
        resolution: hash(80),
    };
    assert!(!expander.expand(&version).unwrap().is_empty());
    assert!(expander.expand_retro(&version).unwrap().is_empty());
}

/// Spec test 5: a fork proof that invalidates a group grant changes the
/// expansion, and the cache does not serve the stale one — the cache key
/// is `(group, frontier, R)`, so the same frontier under a new resolution
/// state recomputes instead of reusing the pre-proof entry.
#[test]
fn test_expand_fork_invalidated_grant_changes_expansion_and_cache_keys_on_resolution() {
    let grant = hash(11);
    let add_dave = |membership: &mut Membership, as_action: bool| {
        let role = if as_action {
            MembershipRole::Auth {
                author: author(ALICE),
                group: Some(group()),
                action: GroupAction::Add {
                    member: individual(author(DAVE)),
                    access: Access::write(),
                },
            }
        } else {
            // Exactly what the resolver feeds the layer for a fork-excluded
            // action (event_store/resolve.rs): the event stays in the
            // index, its membership effect is dropped.
            MembershipRole::Other
        };
        feed(membership, grant, vec![group()], role);
    };

    let set = set(vec![group_entry(Role::Write, false)], vec![]);
    let frontier = vec![grant];

    // The grant stands: Dave is in the expansion under R1.
    let mut membership = base_layer();
    add_dave(&mut membership, true);
    let mut expander = Expander::new(&mut membership);
    let under_r1 = expander
        .expand(&SetVersion {
            set: &set,
            frontier: frontier.clone(),
            resolution: hash(80),
        })
        .unwrap();
    assert!(under_r1.contains(&author(DAVE)));
    assert_eq!(expander.cache_entries(), 1);
    // A repeat under the same (group, frontier, R) key is served by the
    // cache: no new entry.
    expander
        .expand(&SetVersion {
            set: &set,
            frontier: frontier.clone(),
            resolution: hash(80),
        })
        .unwrap();
    assert_eq!(expander.cache_entries(), 1);
    // The same (group, frontier) under a new resolution state is a new
    // cache entry, not the stale one.
    expander
        .expand(&SetVersion {
            set: &set,
            frontier: frontier.clone(),
            resolution: hash(81),
        })
        .unwrap();
    assert_eq!(expander.cache_entries(), 2);

    // The fork proof lands: under the new resolution state the grant is
    // excluded before the layer sees it, and the same set version at the
    // same frontier no longer expands to Dave.
    let mut forked = base_layer();
    add_dave(&mut forked, false);
    let under_r2 = Expander::new(&mut forked)
        .expand(&SetVersion {
            set: &set,
            frontier,
            resolution: hash(81),
        })
        .unwrap();
    assert!(!under_r2.contains(&author(DAVE)));
    assert!(under_r2.contains(&author(CAROL)));
}

/// Spec test 6: `role_min` filters at the frontier — a member who holds
/// the role only later is not admitted by an earlier version.
#[test]
fn test_expand_role_min_filters_at_the_frontier() {
    let mut membership = base_layer();
    let set = set(vec![group_entry(Role::Write, false)], vec![]);

    // At the create frontier Bob only reads: the Write threshold omits him.
    let at_create = expand(&mut membership, &set, vec![group()]);
    assert!(!at_create.contains(&author(BOB)));
    assert!(at_create.contains(&author(CAROL)));

    // Alice promotes Bob to Write after the pin.
    let promote = hash(11);
    feed_auth(
        &mut membership,
        promote,
        vec![group()],
        author(ALICE),
        Some(group()),
        GroupAction::Promote {
            member: individual(author(BOB)),
            access: Access::write(),
        },
    );

    // The earlier version still does not admit him; the version pinned at
    // the promotion does.
    assert!(!expand(&mut membership, &set, vec![group()]).contains(&author(BOB)));
    assert!(expand(&mut membership, &set, vec![promote]).contains(&author(BOB)));
}

/// Spec test 7: an exclusion removes an admitted writer, and an exclusion
/// of someone never admitted is a no-op rather than an error.
#[test]
fn test_expand_exclusion_removes_admitted_and_ignores_never_admitted() {
    let mut membership = base_layer();
    let admitted = set(vec![group_entry(Role::Read, false)], vec![]);
    let baseline = expand(&mut membership, &admitted, vec![group()]);
    assert!(baseline.contains(&author(BOB)));

    let excluding = set(
        vec![group_entry(Role::Read, false)],
        vec![key_entry(BOB, false), key_entry(MALLORY, false)],
    );
    let expanded = expand(&mut membership, &excluding, vec![group()]);
    assert!(!expanded.contains(&author(BOB)));
    assert!(expanded.contains(&author(CAROL)));
    assert_eq!(expanded.len(), baseline.len() - 1);
}

/// A public entry admits every author, so `expand` cannot materialize it
/// as a set of authors: it refuses instead of silently returning only the
/// named authors. A public entry without `include_history` contributes
/// nothing to the retro expansion and does not refuse it.
#[test]
fn test_expand_public_entry_refused_not_materialized() {
    let mut membership = base_layer();
    let public = set(
        vec![
            Entry::Public {
                include_history: false,
            },
            key_entry(ERIN, false),
        ],
        vec![],
    );
    let version = SetVersion {
        set: &public,
        frontier: vec![group()],
        resolution: hash(80),
    };
    let mut expander = Expander::new(&mut membership);
    assert_eq!(expander.expand(&version), Err(ExpandError::PublicEntry));
    assert_eq!(expander.expand_retro(&version).unwrap(), BTreeSet::new());
}
