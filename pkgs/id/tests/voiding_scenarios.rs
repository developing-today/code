//! Authority-based voiding through the event store (P2.3). The S2 scenarios, each delivered in
//! every arrival order. The applied state and the roster must be the same in all of them, and
//! must match the S2 outcomes: an operation is voided when its author lost authority at its
//! position, and a dependent of a voided operation is kept when its author kept authority.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::collections::BTreeMap;

use id::chain::{ChainExt, sign};
use id::event_store::EventStore;
use id::membership::AuthId;
use id::waiting::WaitingLimits;
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_core::{Hash, SeqNum, SigningKey};

/// One event of a scenario: its name in the scenario, its signed bytes, and its action.
#[derive(Clone)]
struct Step {
    name: u8,
    bytes: Vec<u8>,
    action: Option<GroupAction<AuthId, ()>>,
}

/// Signs a scenario. Each author's events form a chain, so the sequence numbers and backlinks
/// are set here. The group is the create event's ID.
#[derive(Default)]
struct Builder {
    ids: BTreeMap<u8, Hash>,
    last: BTreeMap<char, (SeqNum, Hash)>,
    group: Option<Hash>,
    steps: Vec<Step>,
}

fn key(who: char) -> SigningKey {
    SigningKey::from_bytes(&[who as u8; 32])
}

fn auth(who: char) -> AuthId {
    AuthId::Key(*key(who).verifying_key().as_bytes())
}

fn add(who: char, access: Access<()>) -> GroupAction<AuthId, ()> {
    GroupAction::Add {
        member: GroupMember::Individual(auth(who)),
        access,
    }
}

fn remove(who: char) -> GroupAction<AuthId, ()> {
    GroupAction::Remove {
        member: GroupMember::Individual(auth(who)),
    }
}

fn create(managers: &[char]) -> GroupAction<AuthId, ()> {
    GroupAction::Create {
        initial_members: managers
            .iter()
            .map(|who| (GroupMember::Individual(auth(*who)), Access::manage()))
            .collect(),
    }
}

impl Builder {
    /// Signs one event by `who`, after the events named in `after`.
    fn push(
        &mut self,
        name: u8,
        who: char,
        after: &[u8],
        kind: &str,
        action: Option<GroupAction<AuthId, ()>>,
    ) {
        let (seq, backlink) = match self.last.get(&who) {
            Some((seq, hash)) => (seq + 1, Some(*hash)),
            None => (0, None),
        };
        let ext = ChainExt {
            kind: kind.to_owned(),
            group: if matches!(action, Some(GroupAction::Create { .. })) {
                None
            } else {
                self.group
            },
            after: after.iter().map(|n| self.ids[n]).collect(),
        };
        let event = sign(&key(who), seq, backlink, None, ext).unwrap();
        if matches!(action, Some(GroupAction::Create { .. })) {
            self.group = Some(event.id);
        }
        self.ids.insert(name, event.id);
        self.last.insert(who, (seq, event.id));
        self.steps.push(Step {
            name,
            bytes: event.bytes,
            action,
        });
    }

    /// A membership operation by `who`.
    fn op(&mut self, name: u8, who: char, after: &[u8], action: GroupAction<AuthId, ()>) {
        self.push(name, who, after, "group.op", Some(action));
    }

    /// Content in the group: an event with no membership effect.
    fn content(&mut self, name: u8, who: char, after: &[u8]) {
        self.push(name, who, after, "message", None);
    }
}

/// What one delivery order produced: each event's applied flag, and the group's roster.
#[derive(Debug, PartialEq, Eq)]
struct Outcome {
    applied: BTreeMap<u8, bool>,
    roster: Vec<AuthId>,
}

/// Delivers the steps in `order` to a fresh store, and reports what it applied.
fn deliver(steps: &[Step], order: &[usize]) -> Outcome {
    let dir = tempfile::tempdir().unwrap();
    let mut store = EventStore::open(dir.path(), WaitingLimits::default(), 0).unwrap();
    for &index in order {
        let step = &steps[index];
        store
            .ingest_with_action(&step.bytes, step.action.as_ref(), 0)
            .unwrap();
    }
    let group = steps
        .iter()
        .find(|step| matches!(step.action, Some(GroupAction::Create { .. })))
        .map(|step| Hash::digest(&step.bytes))
        .unwrap();
    let applied = steps
        .iter()
        .map(|step| {
            let id = Hash::digest(&step.bytes);
            (step.name, store.event_applied(&id).unwrap())
        })
        .collect();
    let mut roster: Vec<AuthId> = store
        .roster(&group)
        .unwrap()
        .into_iter()
        .map(AuthId::Key)
        .collect();
    roster.sort();
    Outcome { applied, roster }
}

/// Every permutation of `0..n`.
fn permutations(n: usize) -> Vec<Vec<usize>> {
    fn walk(current: &mut Vec<usize>, rest: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if rest.is_empty() {
            out.push(current.clone());
            return;
        }
        for i in 0..rest.len() {
            let item = rest.remove(i);
            current.push(item);
            walk(current, rest, out);
            current.pop();
            rest.insert(i, item);
        }
    }
    let mut out = Vec::new();
    walk(&mut Vec::new(), &mut (0..n).collect(), &mut out);
    out
}

/// Checks a scenario in every arrival order: all orders agree, and they match `expected`.
fn check(name: &str, steps: &[Step], expected: &[(u8, bool)], roster: &[char]) {
    let expected = Outcome {
        applied: expected.iter().copied().collect(),
        roster: {
            let mut keys: Vec<AuthId> = roster.iter().map(|who| auth(*who)).collect();
            keys.sort();
            keys
        },
    };
    for order in permutations(steps.len()) {
        let outcome = deliver(steps, &order);
        assert_eq!(outcome, expected, "{name}: arrival order {order:?}");
    }
}

#[test]
fn test_s2_week_late_add_by_a_removed_member_is_voided() {
    // Alice removes Bob (R). Bob's add of Dave (X) was made without seeing R.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], remove('B'));
    b.op(2, 'B', &[0], add('D', Access::manage()));
    check(
        "1 week-late add",
        &b.steps,
        &[(0, true), (1, true), (2, false)],
        &['A', 'C'],
    );
}

#[test]
fn test_s2_causally_prior_add_survives() {
    // Bob adds Dave (Y). Alice sees Y, then removes Bob (R names Y).
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'B', &[0], add('D', Access::manage()));
    b.op(2, 'A', &[1], remove('B'));
    check(
        "2 causally prior add",
        &b.steps,
        &[(0, true), (1, true), (2, true)],
        &['A', 'C', 'D'],
    );
}

#[test]
fn test_s2_transitive_dependent_of_a_voided_add_is_voided_for_a_removed_author() {
    // Dave's authority comes from Bob's add Z, which is concurrent with R. Dave adds Eve after Z.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'B', &[0], add('D', Access::manage()));
    b.op(2, 'A', &[0], remove('B'));
    b.op(3, 'D', &[1], add('E', Access::read()));
    check(
        "3 transitive void by authority",
        &b.steps,
        &[(0, true), (1, false), (2, true), (3, false)],
        &['A', 'C'],
    );
}

#[test]
fn test_s2_transitive_dependent_of_a_voided_add_is_kept_for_an_independent_author() {
    // The same shape, but the add after Z is by Claire, who has her own authority (rule 3b).
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'B', &[0], add('D', Access::manage()));
    b.op(2, 'A', &[0], remove('B'));
    b.op(3, 'C', &[1], add('E', Access::read()));
    check(
        "3b dependent add by an independent author",
        &b.steps,
        &[(0, true), (1, false), (2, true), (3, true)],
        &['A', 'C', 'E'],
    );
}

#[test]
fn test_s2_content_by_a_removed_author_after_a_voided_add_is_voided() {
    // Content is kept on removal, except where its author lost authority at the voided add.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'B', &[0], add('D', Access::manage()));
    b.op(2, 'A', &[0], remove('B'));
    b.content(3, 'D', &[1]);
    b.content(4, 'C', &[1]);
    check(
        "3c content after a voided add",
        &b.steps,
        &[(0, true), (1, false), (2, true), (3, false), (4, true)],
        &['A', 'C'],
    );
}

#[test]
fn test_s2_mutual_removal_removes_both() {
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], remove('B'));
    b.op(2, 'B', &[0], remove('A'));
    check(
        "4 mutual removal",
        &b.steps,
        &[(0, true), (1, true), (2, true)],
        &['C'],
    );
}

#[test]
fn test_s2_three_cycle_removes_every_member() {
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], remove('B'));
    b.op(2, 'B', &[0], remove('C'));
    b.op(3, 'C', &[0], remove('A'));
    check(
        "4b three-cycle",
        &b.steps,
        &[(0, true), (1, true), (2, true), (3, true)],
        &[],
    );
}

#[test]
fn test_s2_concurrent_re_add_is_voided() {
    // Claire removes and re-adds Bob, without seeing Alice's removal of Bob.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'C', &[0], remove('B'));
    b.op(2, 'C', &[1], add('B', Access::manage()));
    b.op(3, 'A', &[0], remove('B'));
    check(
        "5 concurrent re-add",
        &b.steps,
        &[(0, true), (1, true), (2, false), (3, true)],
        &['A', 'C'],
    );
}

#[test]
fn test_s2_re_add_concurrent_with_another_managers_removal_is_voided() {
    // Alice removes and re-adds Bob. Claire removes Bob concurrently.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], remove('B'));
    b.op(2, 'A', &[1], add('B', Access::manage()));
    b.op(3, 'C', &[0], remove('B'));
    check(
        "5c re-add beside another manager's removal",
        &b.steps,
        &[(0, true), (1, true), (2, false), (3, true)],
        &['A', 'C'],
    );
}

#[test]
fn test_s2_causal_re_add_control_stands() {
    // Claire re-adds Bob after she has seen Alice's removal.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], remove('B'));
    b.op(2, 'C', &[1], add('B', Access::manage()));
    check(
        "5b causal re-add",
        &b.steps,
        &[(0, true), (1, true), (2, true)],
        &['A', 'B', 'C'],
    );
}

#[test]
fn test_s2_a_fork_free_chain_is_not_voided_by_a_removal_of_another_author() {
    // Regression for the Phase 1 cascade: a removal voids only the removed member's concurrent
    // operations. Claire's add after Alice's add is honest and stands.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], add('D', Access::read()));
    b.op(2, 'B', &[0], remove('C'));
    b.op(3, 'A', &[1], add('E', Access::read()));
    check(
        "honest dependents apply",
        &b.steps,
        &[(0, true), (1, true), (2, true), (3, true)],
        &['A', 'B', 'D', 'E'],
    );
}

#[test]
fn test_a_voided_event_does_not_stall_its_author() {
    // Regression: a voided event stays stored, so its author's next event links to it. Bob's add is
    // concurrent with Alice's removal of him, so it is voided. Bob's content after it is kept,
    // because it names only the create, which applies.
    let mut b = Builder::default();
    b.op(0, 'A', &[], create(&['A', 'B', 'C']));
    b.op(1, 'A', &[0], remove('B'));
    b.op(2, 'B', &[0], add('D', Access::manage()));
    b.content(3, 'B', &[0]);
    check(
        "a voided predecessor does not hold the author's next event",
        &b.steps,
        &[(0, true), (1, true), (2, false), (3, true)],
        &['A', 'C'],
    );
}
