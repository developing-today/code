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

/// An admin source with no admin data at any frontier: Admin thresholds
/// fail closed (admit nobody) for expanders built on it. Tests that model
/// admins supply their own source keyed on the frontier, the way the
/// groups fold's replay would answer for a real causal position.
fn no_admins(_: &[Hash], _: Hash) -> BTreeSet<[u8; 32]> {
    BTreeSet::new()
}

fn expand(membership: &mut Membership, set: &WriterSet, frontier: Vec<Hash>) -> BTreeSet<[u8; 32]> {
    Expander::new(membership, &no_admins)
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
    let mut expander = Expander::new(&mut membership, &no_admins);
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
    let mut expander = Expander::new(&mut membership, &no_admins);
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
    let under_r2 = Expander::new(&mut forked, &no_admins)
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
    let mut expander = Expander::new(&mut membership, &no_admins);
    assert_eq!(expander.expand(&version), Err(ExpandError::PublicEntry));
    assert_eq!(expander.expand_retro(&version).unwrap(), BTreeSet::new());
}

/// Codex P2 on PR #6787: a public exclusion is the §D6 emergency stop —
/// the expansion is the finite empty set, so `expand` must return it
/// instead of refusing the public admission it nullifies.
#[test]
fn test_expand_public_exclusion_admits_nobody_not_refused() {
    let mut membership = base_layer();
    let stopped = set(
        vec![Entry::Public {
            include_history: true,
        }],
        vec![Entry::Public {
            include_history: false,
        }],
    );
    let version = SetVersion {
        set: &stopped,
        frontier: vec![group()],
        resolution: hash(80),
    };
    let mut expander = Expander::new(&mut membership, &no_admins);
    assert_eq!(expander.expand(&version).unwrap(), BTreeSet::new());
    assert_eq!(expander.expand_retro(&version).unwrap(), BTreeSet::new());

    // The stop nullifies named admissions too.
    let named = set(
        vec![key_entry(ERIN, false)],
        vec![Entry::Public {
            include_history: false,
        }],
    );
    assert_eq!(
        expand(&mut membership, &named, vec![group()]),
        BTreeSet::new()
    );
}

/// #6798, fail-closed baseline: the membership layer records Admin as
/// Manage and the default test source carries no admin data, so an Admin
/// `role_min` admits nobody — it no longer widens to Manage (the pre-#6798
/// clamp, pinned by the test this one replaces). Manage itself is still
/// answered exactly by the layer: Alice (Manage) clears a Manage entry,
/// Bob (Read) and Carol (Write) stay below it.
#[test]
fn test_expand_role_min_admin_without_admin_data_admits_nobody() {
    let mut membership = base_layer();
    let admin_set = set(vec![group_entry(Role::Admin, false)], vec![]);
    assert!(expand(&mut membership, &admin_set, vec![group()]).is_empty());
    let manage_set = set(vec![group_entry(Role::Manage, false)], vec![]);
    let expanded = expand(&mut membership, &manage_set, vec![group()]);
    assert_eq!(expanded, [author(ALICE)].into_iter().collect());
}

/// #6798, exactness: with an admin source naming Alice at the frontier,
/// an Admin `role_min` admits Alice and NOT Erin, a second manager who
/// holds the layer's top access (Manage) but not the Admin policy role.
/// (Before the plumbing, the clamp admitted every manager.)
#[test]
fn test_expand_role_min_admin_admits_admin_not_non_admin_manager() {
    let mut membership = base_layer();
    let add_erin = hash(11);
    feed_auth(
        &mut membership,
        add_erin,
        vec![group()],
        author(ALICE),
        Some(group()),
        GroupAction::Add {
            member: individual(author(ERIN)),
            access: Access::manage(),
        },
    );
    // The fold-side answer at this frontier: Alice is the group's admin;
    // Erin is a manager, never an admin.
    let admins = |frontier: &[Hash], _group: Hash| -> BTreeSet<[u8; 32]> {
        if frontier == [add_erin] {
            [author(ALICE)].into_iter().collect()
        } else {
            BTreeSet::new()
        }
    };
    let admin_set = set(vec![group_entry(Role::Admin, false)], vec![]);
    let expanded = Expander::new(&mut membership, &admins)
        .expand(&SetVersion {
            set: &admin_set,
            frontier: vec![add_erin],
            resolution: hash(80),
        })
        .unwrap();
    assert_eq!(expanded, [author(ALICE)].into_iter().collect());
    // Sanity: a Manage threshold at the same frontier admits both
    // managers, so Erin's exclusion above is the Admin distinction, not
    // a membership miss.
    let manage_set = set(vec![group_entry(Role::Manage, false)], vec![]);
    let managers = expand(&mut membership, &manage_set, vec![add_erin]);
    assert_eq!(
        managers,
        [author(ALICE), author(ERIN)].into_iter().collect()
    );
}

/// #6798, frontier pinning for the policy role (the Admin analogue of
/// spec test 6): Erin is promoted to Admin only at the later position
/// (modelled by the source, whose answer changes at the hash(12)
/// frontier — the fold replays exactly this way over settled events).
/// A set version pinned at the earlier frontier must not admit her; the
/// same set judged at the later frontier does.
#[test]
fn test_expand_role_min_admin_promoted_after_frontier_is_not_admitted() {
    let mut membership = base_layer();
    let add_erin = hash(11);
    feed_auth(
        &mut membership,
        add_erin,
        vec![group()],
        author(ALICE),
        Some(group()),
        GroupAction::Add {
            member: individual(author(ERIN)),
            access: Access::manage(),
        },
    );
    // A later layer event marks the position at which the fold-side
    // promotion to Admin has happened.
    let later = hash(12);
    feed_auth(
        &mut membership,
        later,
        vec![add_erin],
        author(ALICE),
        Some(group()),
        GroupAction::Add {
            member: individual(author(FRANK)),
            access: Access::read(),
        },
    );
    let admins = |frontier: &[Hash], _group: Hash| -> BTreeSet<[u8; 32]> {
        if frontier == [later] {
            [author(ERIN)].into_iter().collect()
        } else {
            BTreeSet::new()
        }
    };
    let set = set(vec![group_entry(Role::Admin, false)], vec![]);
    let pinned = Expander::new(&mut membership, &admins)
        .expand(&SetVersion {
            set: &set,
            frontier: vec![add_erin],
            resolution: hash(80),
        })
        .unwrap();
    assert!(
        pinned.is_empty(),
        "promotion after the frontier leaked into the pinned expansion: {pinned:?}"
    );
    let at_promotion = Expander::new(&mut membership, &admins)
        .expand(&SetVersion {
            set: &set,
            frontier: vec![later],
            resolution: hash(80),
        })
        .unwrap();
    assert_eq!(at_promotion, [author(ERIN)].into_iter().collect());
}

// ---------------------------------------------------------------------------
// Property tests (Codex P1 on PR #6787; testing doctrine: invariants with
// a generator, not examples only). The oracle for each property is stated
// in its comment: permutation/subset properties are self-oracles over the
// generated input, and the pinning property's oracle is the expansion
// evaluated before the generated history is appended — any later event
// that moved the pinned answer is a counterexample.
// ---------------------------------------------------------------------------

use proptest::prelude::*;

fn role_strategy() -> impl Strategy<Value = Role> {
    prop_oneof![
        Just(Role::Pull),
        Just(Role::Read),
        Just(Role::Write),
        Just(Role::Manage),
        Just(Role::Admin),
    ]
}

fn entry_strategy() -> impl Strategy<Value = Entry> {
    prop_oneof![
        (2..8u8, any::<bool>()).prop_map(|(n, include_history)| Entry::Key {
            author: author(n),
            include_history,
        }),
        (role_strategy(), any::<bool>()).prop_map(|(role_min, include_history)| Entry::Group {
            id: group(),
            role_min,
            include_history,
        }),
        any::<bool>().prop_map(|include_history| Entry::Public { include_history }),
    ]
}

fn permute(entries: &[Entry], seed: usize) -> Vec<Entry> {
    let mut out = entries.to_vec();
    out.reverse();
    let len = out.len();
    if len > 0 {
        out.rotate_left(seed % len);
    }
    out
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// Invariant: expansion is a function of the entry *set*, not its
    /// spelling — any permutation of entries and excludes expands the
    /// same (including refusing the same).
    #[test]
    fn prop_expand_is_order_independent(
        entries in proptest::collection::vec(entry_strategy(), 0..6),
        excludes in proptest::collection::vec(entry_strategy(), 0..3),
        seed in 0..720usize,
    ) {
        let mut membership = base_layer();
        let plain = set(entries.clone(), excludes.clone());
        let shuffled = set(permute(&entries, seed), permute(&excludes, seed + 1));
        let a = Expander::new(&mut membership, &no_admins).expand(&SetVersion {
            set: &plain,
            frontier: vec![group()],
            resolution: hash(80),
        });
        let b = Expander::new(&mut membership, &no_admins).expand(&SetVersion {
            set: &shuffled,
            frontier: vec![group()],
            resolution: hash(80),
        });
        prop_assert_eq!(a, b);
    }

    /// Invariant: `expand_retro(v) ⊆ expand(v)` for every generated set.
    /// When `expand` succeeds the retro expansion must also succeed (it
    /// narrows the same admissions) and be a subset.
    #[test]
    fn prop_expand_retro_is_subset_of_expand(
        entries in proptest::collection::vec(entry_strategy(), 0..6),
        excludes in proptest::collection::vec(entry_strategy(), 0..3),
    ) {
        let mut membership = base_layer();
        let set = set(entries, excludes);
        let version = SetVersion {
            set: &set,
            frontier: vec![group()],
            resolution: hash(80),
        };
        let mut expander = Expander::new(&mut membership, &no_admins);
        if let Ok(full) = expander.expand(&version) {
            let retro = expander.expand_retro(&version).unwrap();
            prop_assert!(retro.is_subset(&full), "retro {retro:?} not a subset of {full:?}");
        }
    }

    /// Invariant: a set version's expansion is pinned to its frontier —
    /// appending a generated membership history (adds, removes, promotes
    /// by the group's manager, chained after the pin) never changes it.
    /// Oracle: the expansion evaluated before the history is appended.
    #[test]
    fn prop_expand_is_frontier_pinned_under_generated_history(
        entries in proptest::collection::vec(entry_strategy(), 0..5),
        ops in proptest::collection::vec((2..8u8, 0..3u8, 0..4u8), 0..8),
    ) {
        let mut membership = base_layer();
        let set = set(entries, vec![]);
        let version = SetVersion {
            set: &set,
            frontier: vec![group()],
            resolution: hash(80),
        };
        let before = Expander::new(&mut membership, &no_admins).expand(&version);

        let mut after = vec![group()];
        for (i, (who, kind, level)) in ops.iter().enumerate() {
            let access = match level {
                0 => Access::pull(),
                1 => Access::read(),
                2 => Access::write(),
                _ => Access::manage(),
            };
            let action = match kind {
                0 => GroupAction::Add {
                    member: individual(author(*who)),
                    access,
                },
                1 => GroupAction::Remove {
                    member: individual(author(*who)),
                },
                _ => GroupAction::Promote {
                    member: individual(author(*who)),
                    access,
                },
            };
            let id = hash(20 + i as u8);
            // Generated ops may be refused by the gate (removing a
            // non-member, re-adding a member); a refusal is a no-op in
            // the layer, which is fine — the pin must hold either way.
            let _ = membership.receive(CausalEvent {
                id,
                after: after.clone(),
                role: MembershipRole::Auth {
                    author: author(ALICE),
                    group: Some(group()),
                    action,
                },
            });
            after = vec![id];
        }

        let after_expand = Expander::new(&mut membership, &no_admins).expand(&version);
        prop_assert_eq!(before, after_expand);
    }
}
