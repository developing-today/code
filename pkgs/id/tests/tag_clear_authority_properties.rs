//! Generated coverage for the role-aware tag-clear authorization path (#6193).
//!
//! The fixed examples in `tag_clear_authority.rs` cover a grouped subject with a static
//! roster. These property tests generate histories instead: grouped and ungrouped
//! subjects, claims and clears from members and non-members, and membership changes
//! (adds, removals, promotes, demotes, refused operations by non-managers, and the
//! group creator removing itself) interleaved with the tag events.
//!
//! Oracle (named, per the testing doctrine): an independent model in this file. Every
//! event names the whole frontier in `after`, so every claim a clear names is in that
//! clear's causal past and the observed-remove fold is not what varies. A claim is
//! then live in the default view iff no clear naming it was authored by
//!   * the claim's own author, or
//!   * the group's creator, while the creator is still a member at the end of the
//!     history — the settled projection is what `default_view_events` reads, so the
//!     final roster, not the roster at the clear's position, decides.
//! Through the production bridge the creator is the only settled Admin: an access
//! level has no Admin rung (`role_of` tops out at Manage), so a promoted manager is
//! modelled as a member, never as an Admin clearer.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::{BTreeMap, BTreeSet};

use id::chain::{sign, ChainExt, Event};
use id::event_store::{EventStore, Ingest};
use id::membership::body::encode_body;
use id::membership::AuthId;
use id::tag_claims::{TagOp, CLEAR_KIND, SET_KIND};
use id::waiting::WaitingLimits;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::Access;
use p2panda_core::{Hash, SeqNum, SigningKey};
use proptest::prelude::*;

const GROUP_OP: &str = "group.op";
const PEOPLE: [char; 5] = ['A', 'B', 'C', 'D', 'E'];

fn key(who: char) -> SigningKey {
    SigningKey::from_bytes(&[who as u8; 32])
}

fn author(who: char) -> [u8; 32] {
    *key(who).verifying_key().as_bytes()
}

fn member(who: char) -> GroupMember<AuthId> {
    GroupMember::Individual(AuthId::Key(author(who)))
}

/// An access level as a rung: 1 read, 2 write, 3 manage. There is no Admin rung.
fn access(level: u8) -> Access<()> {
    match level {
        1 => Access::read(),
        2 => Access::write(),
        _ => Access::manage(),
    }
}

struct EventWithBody {
    event: Event,
    body: Option<Vec<u8>>,
}

#[derive(Default)]
struct Chains {
    last: BTreeMap<char, (SeqNum, Hash)>,
}

impl Chains {
    fn sign(
        &mut self,
        who: char,
        group: Option<Hash>,
        after: &[Hash],
        kind: &str,
        tag: Option<TagOp>,
        body: Option<Vec<u8>>,
    ) -> EventWithBody {
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, id)) => (seq + 1, Some(*id)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: kind.to_owned(),
            group,
            after: after.to_vec(),
            tag,
            document: None,
            removes: None,
            note: None,
            link: None,
            checkpoint: None,
            rotation: None,
        };
        let event = sign(&key(who), seq, backlink, body.as_deref(), ext).unwrap();
        self.last.insert(who, (seq, event.id));
        EventWithBody { event, body }
    }

    /// The creator `A` (Manage, hence the settled Admin) and `B` at `b_level`.
    fn create_group(&mut self, b_level: u8) -> EventWithBody {
        let action = GroupAction::Create {
            initial_members: [('A', 3), ('B', b_level)]
                .iter()
                .map(|(who, level)| (member(*who), access(*level)))
                .collect(),
        };
        self.sign(
            'A',
            None,
            &[],
            GROUP_OP,
            None,
            Some(encode_body(&action).unwrap()),
        )
    }

    fn group_op(
        &mut self,
        who: char,
        group: Hash,
        action: &GroupAction<AuthId, ()>,
        after: &[Hash],
    ) -> EventWithBody {
        self.sign(
            who,
            Some(group),
            after,
            GROUP_OP,
            None,
            Some(encode_body(action).unwrap()),
        )
    }

    fn subject(&mut self, who: char, group: Option<Hash>, after: &[Hash]) -> EventWithBody {
        self.sign(who, group, after, "test.subject", None, None)
    }

    fn set_tag(&mut self, who: char, subject: Hash, name: &str, after: &[Hash]) -> EventWithBody {
        self.sign(
            who,
            None,
            after,
            SET_KIND,
            Some(TagOp::Set {
                subject,
                name: name.to_owned(),
                value: Some("value".to_owned()),
            }),
            None,
        )
    }

    fn clear(&mut self, who: char, claims: &[Hash], after: &[Hash]) -> EventWithBody {
        self.sign(
            who,
            None,
            after,
            CLEAR_KIND,
            Some(TagOp::Clear {
                claims: claims.to_vec(),
            }),
            None,
        )
    }
}

fn deliver(store: &mut EventStore, sent: &EventWithBody) {
    let result = store
        .ingest_with_body(&sent.event.bytes, sent.body.as_deref(), 0)
        .unwrap();
    // A refused group operation (a non-member's or non-manager's membership
    // change, which the histories generate on purpose) is stored as Voided
    // rather than Accepted; every other event must be Accepted.
    let ok = if sent.event.header.extensions.kind == GROUP_OP {
        matches!(result, Ingest::Accepted { .. } | Ingest::Voided { .. })
    } else {
        matches!(result, Ingest::Accepted { .. })
    };
    assert!(ok, "event was not accepted: {result:?}");
}

fn open() -> (tempfile::TempDir, EventStore) {
    let dir = tempfile::tempdir().unwrap();
    let store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    (dir, store)
}

fn live_ids(store: &EventStore) -> BTreeSet<[u8; 32]> {
    store
        .tag_claims()
        .unwrap()
        .iter()
        .map(|claim| *claim.id.as_bytes())
        .collect()
}

/// SplitMix64, as in `tag_claims.rs`. A history is a pure function of its seed.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        usize::try_from(self.next_u64() % u64::try_from(n).unwrap()).unwrap()
    }

    fn pick(&mut self, people: &[char]) -> char {
        people[self.below(people.len())]
    }
}

/// A generated history plus the model's record of it.
struct Gen {
    rng: Rng,
    chains: Chains,
    events: Vec<EventWithBody>,
    frontier: Vec<Hash>,
    group: Hash,
    last_group: Hash,
    grouped_subject: Hash,
    plain_subject: Hash,
    /// Settled-roster model: each member's access rung. The creator is the Admin
    /// exactly while it is present here.
    members: BTreeMap<char, u8>,
    admin_active: bool,
    claims: Vec<(Hash, char, bool)>,
    clears: Vec<(char, Vec<Hash>)>,
    claim_count: usize,
}

impl Gen {
    fn new(seed: u64) -> Self {
        let mut chains = Chains::default();
        let create = chains.create_group(1);
        let group = create.event.id;
        let grouped = chains.subject('B', Some(group), &[group]);
        let grouped_subject = grouped.event.id;
        let plain = chains.subject('B', None, &[grouped_subject]);
        let plain_subject = plain.event.id;
        let frontier = vec![group, grouped_subject, plain_subject];
        Self {
            rng: Rng(seed),
            chains,
            events: vec![create, grouped, plain],
            frontier,
            group,
            last_group: group,
            grouped_subject,
            plain_subject,
            members: [('A', 3), ('B', 1)].into_iter().collect(),
            admin_active: true,
            claims: Vec::new(),
            clears: Vec::new(),
            claim_count: 0,
        }
    }

    fn push(&mut self, event: EventWithBody) {
        self.frontier.push(event.event.id);
        self.events.push(event);
    }

    fn claim(&mut self) {
        let who = self.rng.pick(&PEOPLE);
        let grouped = self.rng.below(4) != 0;
        let subject = if grouped {
            self.grouped_subject
        } else {
            self.plain_subject
        };
        let name = format!("label{}", self.claim_count);
        self.claim_count += 1;
        let after = self.frontier.clone();
        let event = self.chains.set_tag(who, subject, &name, &after);
        self.claims.push((event.event.id, who, grouped));
        self.push(event);
    }

    fn clear(&mut self) {
        if self.claims.is_empty() {
            self.claim();
            return;
        }
        let who = self.rng.pick(&PEOPLE);
        let want = 1 + self.rng.below(3.min(self.claims.len()));
        let mut named: Vec<Hash> = Vec::new();
        while named.len() < want {
            // A clear naming a claim twice is invalid input since #4765.
            let id = self.claims[self.rng.below(self.claims.len())].0;
            if !named.contains(&id) {
                named.push(id);
            }
        }
        let after = self.frontier.clone();
        let event = self.chains.clear(who, &named, &after);
        self.clears.push((who, named));
        self.push(event);
    }

    fn group_event(&mut self, who: char, action: &GroupAction<AuthId, ()>) {
        let after = vec![self.last_group];
        let event = self.chains.group_op(who, self.group, action, &after);
        self.last_group = event.event.id;
        self.push(event);
    }

    /// One membership change. Operations by the Admin apply; operations by a
    /// non-member or a member below Manage are refused by the layers and are
    /// modelled as no-ops, so both kinds are generated.
    fn membership(&mut self) {
        if !self.admin_active {
            self.claim();
            return;
        }
        match self.rng.below(5) {
            0 => {
                let absent: Vec<char> = PEOPLE
                    .iter()
                    .copied()
                    .filter(|who| !self.members.contains_key(who))
                    .collect();
                if absent.is_empty() {
                    self.claim();
                    return;
                }
                let target = self.rng.pick(&absent);
                let level = 1 + u8::try_from(self.rng.below(3)).unwrap();
                self.group_event(
                    'A',
                    &GroupAction::Add {
                        member: member(target),
                        access: access(level),
                    },
                );
                self.members.insert(target, level);
            }
            1 => {
                let removable: Vec<char> = self
                    .members
                    .keys()
                    .copied()
                    .filter(|who| *who != 'A')
                    .collect();
                if removable.is_empty() {
                    self.claim();
                    return;
                }
                let target = self.rng.pick(&removable);
                self.group_event(
                    'A',
                    &GroupAction::Remove {
                        member: member(target),
                    },
                );
                self.members.remove(&target);
            }
            2 => {
                let present: Vec<char> = self
                    .members
                    .keys()
                    .copied()
                    .filter(|who| *who != 'A')
                    .collect();
                if present.is_empty() {
                    self.claim();
                    return;
                }
                let target = self.rng.pick(&present);
                self.group_event(
                    'A',
                    &GroupAction::Promote {
                        member: member(target),
                        access: Access::manage(),
                    },
                );
                self.members.insert(target, 3);
            }
            3 => {
                let present: Vec<char> = self
                    .members
                    .keys()
                    .copied()
                    .filter(|who| *who != 'A')
                    .collect();
                if present.is_empty() {
                    self.claim();
                    return;
                }
                let target = self.rng.pick(&present);
                self.group_event(
                    'A',
                    &GroupAction::Demote {
                        member: member(target),
                        access: Access::read(),
                    },
                );
                self.members.insert(target, 1);
            }
            _ => {
                // A refused operation: its author is a non-member or below Manage.
                let authors: Vec<char> = PEOPLE
                    .iter()
                    .copied()
                    .filter(|who| {
                        *who != 'A' && self.members.get(who).map_or(true, |level| *level < 3)
                    })
                    .collect();
                if authors.is_empty() {
                    self.claim();
                    return;
                }
                let who = self.rng.pick(&authors);
                if self.rng.below(2) == 0 {
                    let absent: Vec<char> = PEOPLE
                        .iter()
                        .copied()
                        .filter(|t| !self.members.contains_key(t))
                        .collect();
                    if absent.is_empty() {
                        self.claim();
                        return;
                    }
                    let target = self.rng.pick(&absent);
                    self.group_event(
                        who,
                        &GroupAction::Add {
                            member: member(target),
                            access: Access::manage(),
                        },
                    );
                } else {
                    let present: Vec<char> = self.members.keys().copied().collect();
                    let target = self.rng.pick(&present);
                    self.group_event(
                        who,
                        &GroupAction::Remove {
                            member: member(target),
                        },
                    );
                }
                // Model unchanged: the operation is refused.
            }
        }
    }

    /// The creator removes itself: the settled Admin is gone for the rest of the
    /// history, and no further membership change is generated.
    fn creator_self_remove(&mut self) {
        if !self.admin_active {
            self.claim();
            return;
        }
        self.group_event(
            'A',
            &GroupAction::Remove {
                member: member('A'),
            },
        );
        self.members.remove(&'A');
        self.admin_active = false;
    }

    fn expected_live(&self) -> BTreeSet<[u8; 32]> {
        self.claims
            .iter()
            .filter(|(id, claim_author, grouped)| {
                !self.clears.iter().any(|(clear_author, named)| {
                    named.contains(id)
                        && (*clear_author == *claim_author
                            || (*grouped && self.admin_active && *clear_author == 'A'))
                })
            })
            .map(|(id, _, _)| *id.as_bytes())
            .collect()
    }
}

fn scenario(seed: u64) -> (Vec<EventWithBody>, BTreeSet<[u8; 32]>) {
    let mut hist = Gen::new(seed);
    let steps = 6 + hist.rng.below(7);
    for _ in 0..steps {
        match hist.rng.below(10) {
            0..=3 => hist.claim(),
            4..=6 => hist.clear(),
            7..=8 => hist.membership(),
            _ => hist.creator_self_remove(),
        }
    }
    if hist.claims.is_empty() {
        hist.claim();
    }
    if hist.clears.is_empty() {
        hist.clear();
    }
    let expected = hist.expected_live();
    (hist.events, expected)
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 24, ..ProptestConfig::default() })]

    /// Generated grouped/ungrouped histories with membership changes reach the
    /// settled-Admin oracle through the production store path.
    #[test]
    fn test_generated_histories_match_the_settled_admin_oracle(seed in any::<u64>()) {
        let (events, expected) = scenario(seed);
        let (_dir, mut store) = open();
        for event in &events {
            deliver(&mut store, event);
        }
        prop_assert_eq!(live_ids(&store), expected);
    }
}

#[test]
fn test_a_manager_member_is_not_a_settled_admin_for_tag_clear() {
    // B is a Manage member of the group, not its creator: Manage is not Admin, so
    // B's clear of C's claim on the grouped subject leaves the claim live.
    let (_dir, mut store) = open();
    let mut chains = Chains::default();
    let create = chains.create_group(3);
    let group = create.event.id;
    let subject = chains.subject('B', Some(group), &[group]);
    let claim = chains.set_tag('C', subject.event.id, "label", &[subject.event.id]);
    let clear = chains.clear('B', &[claim.event.id], &[claim.event.id]);
    for event in [&create, &subject, &claim, &clear] {
        deliver(&mut store, event);
    }
    assert_eq!(
        live_ids(&store),
        BTreeSet::from([*claim.event.id.as_bytes()])
    );
}

#[test]
fn test_a_promoted_member_is_not_a_settled_admin_for_tag_clear() {
    // A promotes B from Read to Manage mid-history; the settled role is still
    // Manage, so B's clear of C's grouped claim leaves it live.
    let (_dir, mut store) = open();
    let mut chains = Chains::default();
    let create = chains.create_group(1);
    let group = create.event.id;
    let promote = chains.group_op(
        'A',
        group,
        &GroupAction::Promote {
            member: member('B'),
            access: Access::manage(),
        },
        &[group],
    );
    let subject = chains.subject('B', Some(group), &[promote.event.id]);
    let claim = chains.set_tag('C', subject.event.id, "label", &[subject.event.id]);
    let clear = chains.clear('B', &[claim.event.id], &[claim.event.id]);
    for event in [&create, &promote, &subject, &claim, &clear] {
        deliver(&mut store, event);
    }
    assert_eq!(
        live_ids(&store),
        BTreeSet::from([*claim.event.id.as_bytes()])
    );
}

#[test]
fn test_a_removed_creator_is_no_longer_a_settled_admin_for_tag_clear() {
    // A creates the group, then removes itself. Authorization reads the settled
    // projection, so A's later clear of B's grouped claim no longer hides it,
    // even though A was Admin when the group was created.
    let (_dir, mut store) = open();
    let mut chains = Chains::default();
    let create = chains.create_group(1);
    let group = create.event.id;
    let subject = chains.subject('B', Some(group), &[group]);
    let claim = chains.set_tag('B', subject.event.id, "label", &[subject.event.id]);
    let remove = chains.group_op(
        'A',
        group,
        &GroupAction::Remove {
            member: member('A'),
        },
        &[group],
    );
    let clear = chains.clear('A', &[claim.event.id], &[remove.event.id]);
    for event in [&create, &subject, &claim, &remove, &clear] {
        deliver(&mut store, event);
    }
    assert_eq!(
        live_ids(&store),
        BTreeSet::from([*claim.event.id.as_bytes()])
    );
}
