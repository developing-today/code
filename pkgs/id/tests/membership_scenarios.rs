//! The S2 scenarios and failure modes (spike S2, `RESULTS.md`), run through the auth-frontier
//! mapping of `id::membership::frontier` and p2panda-auth's `GroupCrdt`.
//!
//! Each scenario is checked in every causal-delivery order. The final membership must be the same
//! in all of them. The failure modes are regressions: the literal mapping, the raw `after`, the
//! root dependency, and an unknown dependency.

#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::panic::{AssertUnwindSafe, catch_unwind};

use id::membership::frontier::CausalIndex;
use id::membership::{
    AuthId, AuthOperation, CausalEvent, Graph, GroupState, Membership, Outcome, Role,
};
use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupCrdtError, GroupMember};
use p2panda_core::Hash;

/// The group is the create event's ID, which is event 0 in every scenario.
const GROUP: u8 = 0;

fn key(letter: char) -> AuthId {
    AuthId::Key([letter as u8; 32])
}

fn ident(n: u8) -> Hash {
    Hash::digest([n])
}

fn group_id() -> AuthId {
    AuthId::Group(ident(GROUP))
}

/// One event of a scenario. `action` is `None` for an event with no membership effect.
#[derive(Clone)]
struct Ev {
    id: u8,
    author: char,
    after: Vec<u8>,
    action: Option<GroupAction<AuthId, ()>>,
}

fn ev(id: u8, author: char, after: &[u8], action: GroupAction<AuthId, ()>) -> Ev {
    Ev {
        id,
        author,
        after: after.to_vec(),
        action: Some(action),
    }
}

fn other(id: u8, author: char, after: &[u8]) -> Ev {
    Ev {
        id,
        author,
        after: after.to_vec(),
        action: None,
    }
}

fn create() -> GroupAction<AuthId, ()> {
    GroupAction::Create {
        initial_members: [
            ('A', Access::manage()),
            ('B', Access::manage()),
            ('C', Access::manage()),
        ]
        .into_iter()
        .map(|(letter, access)| (GroupMember::Individual(key(letter)), access))
        .collect(),
    }
}

fn add(member: char, access: Access<()>) -> GroupAction<AuthId, ()> {
    GroupAction::Add {
        member: GroupMember::Individual(key(member)),
        access,
    }
}

fn remove(member: char) -> GroupAction<AuthId, ()> {
    GroupAction::Remove {
        member: GroupMember::Individual(key(member)),
    }
}

/// The base of every scenario: group created by Alice with Alice, Bob and Claire as managers.
fn base() -> Vec<Ev> {
    vec![ev(0, 'A', &[], create())]
}

/// Maps the auth events to operations, with the auth frontier as dependencies. Each event is
/// indexed as it is read, the way the layer does, so `after` may name any earlier event.
fn operations(events: &[Ev]) -> Vec<AuthOperation> {
    let mut index = CausalIndex::new();
    let mut out = Vec::new();
    for event in events {
        let after: Vec<Hash> = event.after.iter().map(|n| ident(*n)).collect();
        match &event.action {
            None => index.insert(ident(event.id), after, None).unwrap(),
            Some(action) => {
                let group = if action.is_create() {
                    ident(event.id)
                } else {
                    ident(GROUP)
                };
                let dependencies = index.frontier(&after, group).unwrap();
                index.insert(ident(event.id), after, Some(group)).unwrap();
                out.push(AuthOperation {
                    id: ident(event.id),
                    author: key(event.author),
                    dependencies,
                    group_id: AuthId::Group(group),
                    action: action.clone(),
                });
            }
        }
    }
    out
}

/// Every causal-delivery order of `ops`: each operation after all of its dependencies.
fn causal_orders(ops: &[AuthOperation]) -> Vec<Vec<usize>> {
    fn walk(ops: &[AuthOperation], placed: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if placed.len() == ops.len() {
            out.push(placed.clone());
            return;
        }
        for (i, op) in ops.iter().enumerate() {
            let ready = op
                .dependencies
                .iter()
                .all(|dep| placed.iter().any(|p| ops[*p].id == *dep));
            if ready && !placed.contains(&i) {
                placed.push(i);
                walk(ops, placed, out);
                placed.pop();
            }
        }
    }
    let mut out = Vec::new();
    walk(ops, &mut Vec::new(), &mut out);
    out
}

fn apply(ops: &[AuthOperation], order: &[usize]) -> Result<GroupState, String> {
    order.iter().try_fold(Graph::init(), |state, i| {
        Graph::process(state, &ops[*i]).map_err(|error| format!("{error:?}"))
    })
}

fn members(state: &GroupState) -> Vec<(AuthId, Access<()>)> {
    let mut list = state.members(group_id());
    list.sort_by_key(|(id, _)| *id);
    list
}

/// Every arrival order of `n` events, not only the causal ones.
fn permutations(n: usize) -> Vec<Vec<usize>> {
    fn walk(current: &mut Vec<usize>, rest: &mut Vec<usize>, out: &mut Vec<Vec<usize>>) {
        if rest.is_empty() {
            out.push(current.clone());
            return;
        }
        for i in 0..rest.len() {
            let next = rest.remove(i);
            current.push(next);
            walk(current, rest, out);
            current.pop();
            rest.insert(i, next);
        }
    }
    let mut out = Vec::new();
    walk(&mut Vec::new(), &mut (0..n).collect(), &mut out);
    out
}

/// The membership the layer reaches when `events` arrive in `order`. The layer holds an event
/// until its `after` is decided, so the order need not be causal.
fn layer_members(events: &[Ev], order: &[usize]) -> Vec<(AuthId, Access<()>)> {
    let mut layer = Membership::new();
    for i in order {
        let event = &events[*i];
        let role = match &event.action {
            None => Role::Other,
            Some(action) => Role::Auth {
                author: [event.author as u8; 32],
                group: (!action.is_create()).then(|| ident(GROUP)),
                action: action.clone(),
            },
        };
        let after = event.after.iter().map(|n| ident(*n)).collect();
        let outcomes = layer
            .receive(CausalEvent {
                id: ident(event.id),
                after,
                role,
            })
            .expect("the layer decides every event");
        assert!(
            outcomes
                .iter()
                .all(|(_, outcome)| !matches!(outcome, Outcome::Refused { .. })),
            "a valid S2 event is refused in order {order:?}"
        );
    }
    assert_eq!(
        layer.held_len(),
        0,
        "every event is decided in order {order:?}"
    );
    let mut list = layer.members(ident(GROUP));
    list.sort_by_key(|(id, _)| *id);
    list
}

/// Runs every causal order, and every arrival order through the layer, and checks each gives
/// `expected`.
fn check(name: &str, events: &[Ev], expected: &[(AuthId, Access<()>)]) {
    let ops = operations(events);
    let orders = causal_orders(&ops);
    assert!(!orders.is_empty(), "{name}: no causal order");
    for order in &orders {
        let state = apply(&ops, order).expect(name);
        assert_eq!(members(&state), expected, "{name} in order {order:?}");
    }
    for order in permutations(events.len()) {
        assert_eq!(
            layer_members(events, &order),
            expected,
            "{name} through the layer in arrival order {order:?}"
        );
    }
}

fn expected(list: &[(char, Access<()>)]) -> Vec<(AuthId, Access<()>)> {
    let mut out: Vec<(AuthId, Access<()>)> =
        list.iter().map(|(l, a)| (key(*l), a.clone())).collect();
    out.sort_by_key(|(id, _)| *id);
    out
}

#[test]
fn test_s2_week_late_add_by_a_removed_member_is_voided() {
    // Alice removes Bob (R). Bob's add of Dave (X) was made without seeing R.
    let mut events = base();
    events.push(ev(1, 'A', &[0], remove('B')));
    events.push(ev(2, 'B', &[0], add('D', Access::manage())));
    check(
        "1 week-late add",
        &events,
        &expected(&[('A', Access::manage()), ('C', Access::manage())]),
    );
}

#[test]
fn test_s2_causally_prior_add_survives() {
    // Bob adds Dave (Y). Alice sees Y, then removes Bob (R names Y).
    let mut events = base();
    events.push(ev(1, 'B', &[0], add('D', Access::manage())));
    events.push(ev(2, 'A', &[1], remove('B')));
    check(
        "2 causally prior add",
        &events,
        &expected(&[
            ('A', Access::manage()),
            ('C', Access::manage()),
            ('D', Access::manage()),
        ]),
    );
}

#[test]
fn test_s2_transitive_void_by_authority() {
    // Dave's authority comes from Bob's add Z, which is concurrent with R. Dave adds Eve after Z.
    let mut events = base();
    events.push(ev(1, 'B', &[0], add('D', Access::manage())));
    events.push(ev(2, 'A', &[0], remove('B')));
    events.push(ev(3, 'D', &[1], add('E', Access::read())));
    check(
        "3 transitive void",
        &events,
        &expected(&[('A', Access::manage()), ('C', Access::manage())]),
    );
}

#[test]
fn test_s2_dependent_add_by_an_independent_author_stands() {
    // The same shape, but the add after Z is by Claire, who has her own authority (rule 3b).
    let mut events = base();
    events.push(ev(1, 'B', &[0], add('D', Access::manage())));
    events.push(ev(2, 'A', &[0], remove('B')));
    events.push(ev(3, 'C', &[1], add('E', Access::read())));
    check(
        "3b dependent add by independent author",
        &events,
        &expected(&[
            ('A', Access::manage()),
            ('C', Access::manage()),
            ('E', Access::read()),
        ]),
    );
}

#[test]
fn test_s2_mutual_removal_removes_both() {
    let mut events = base();
    events.push(ev(1, 'A', &[0], remove('B')));
    events.push(ev(2, 'B', &[0], remove('A')));
    check(
        "4 mutual removal",
        &events,
        &expected(&[('C', Access::manage())]),
    );
}

#[test]
fn test_s2_three_cycle_removes_every_member() {
    let mut events = base();
    events.push(ev(1, 'A', &[0], remove('B')));
    events.push(ev(2, 'B', &[0], remove('C')));
    events.push(ev(3, 'C', &[0], remove('A')));
    check("4b three-cycle", &events, &[]);
}

#[test]
fn test_s2_concurrent_re_add_is_voided() {
    // Claire removes and re-adds Bob, without seeing Alice's removal of Bob.
    let mut events = base();
    events.push(ev(1, 'C', &[0], remove('B')));
    events.push(ev(2, 'C', &[1], add('B', Access::manage())));
    events.push(ev(3, 'A', &[0], remove('B')));
    check(
        "5 concurrent re-add",
        &events,
        &expected(&[('A', Access::manage()), ('C', Access::manage())]),
    );
}

#[test]
fn test_s2_re_add_concurrent_with_another_managers_removal_is_voided() {
    // Alice removes and re-adds Bob. Claire removes Bob concurrently.
    let mut events = base();
    events.push(ev(1, 'A', &[0], remove('B')));
    events.push(ev(2, 'A', &[1], add('B', Access::manage())));
    events.push(ev(3, 'C', &[0], remove('B')));
    check(
        "5c re-add concurrent with a removal",
        &events,
        &expected(&[('A', Access::manage()), ('C', Access::manage())]),
    );
}

#[test]
fn test_s2_causal_re_add_control_stands() {
    let mut events = base();
    events.push(ev(1, 'A', &[0], remove('B')));
    events.push(ev(2, 'A', &[1], add('B', Access::manage())));
    check(
        "5b causal re-add (control)",
        &events,
        &expected(&[
            ('A', Access::manage()),
            ('B', Access::manage()),
            ('C', Access::manage()),
        ]),
    );
}

/// The S2 DAG for the mapping cases: X (Bob adds Dave), D (a document naming X), R (Alice removes
/// Bob, naming D). Bob's add is in Alice's causal past, so it must stand.
fn mapping_dag() -> Vec<Ev> {
    let mut events = base();
    events.push(ev(1, 'B', &[0], add('D', Access::manage())));
    events.push(other(2, 'B', &[1]));
    events.push(ev(3, 'A', &[2], remove('B')));
    events
}

#[test]
fn test_s2_frontier_of_the_remove_names_the_add_not_the_document() {
    let events = mapping_dag();
    let mut index = CausalIndex::new();
    for event in &events[..3] {
        let after: Vec<Hash> = event.after.iter().map(|n| ident(*n)).collect();
        let graph = event.action.as_ref().map(|_| ident(GROUP));
        index.insert(ident(event.id), after, graph).unwrap();
    }
    // The remove names the document (2), which is not a group operation. The frontier is the add.
    assert_eq!(
        index.frontier(&[ident(2)], ident(GROUP)).unwrap(),
        vec![ident(1)]
    );
}

#[test]
fn test_s2_frontier_mapping_keeps_the_add_in_every_order() {
    let ops = operations(&mapping_dag());
    assert_eq!(ops[2].dependencies, vec![ident(1)]);
    let expected = expected(&[
        ('A', Access::manage()),
        ('C', Access::manage()),
        ('D', Access::manage()),
    ]);
    for order in causal_orders(&ops) {
        let state = apply(&ops, &order).unwrap();
        assert_eq!(members(&state), expected, "order {order:?}");
    }
}

/// The state after the create and the add, which is every op before the remove.
fn state_before_remove(ops: &[AuthOperation]) -> GroupState {
    apply(&ops[..2], &[0, 1]).unwrap()
}

#[test]
fn test_s2_regression_raw_after_as_dependencies_stalls_with_missing_dependencies() {
    // Naming the document (a non-auth event) as the dependency: p2panda-auth holds the remove on
    // an operation that does not exist, forever.
    let ops = operations(&mapping_dag());
    let state = state_before_remove(&ops);
    let raw = AuthOperation {
        dependencies: vec![ident(2)],
        ..ops[2].clone()
    };
    let result = Graph::process(state, &raw);
    assert!(
        matches!(result, Err(GroupCrdtError::MissingDependencies(dep, _)) if dep == ident(2)),
        "raw after must stall on the document"
    );
}

#[test]
fn test_s2_regression_literal_mapping_with_no_auth_dependency_does_not_apply() {
    // The literal mapping gives the remove no auth dependency, because the auth ops named in `after`
    // are none. p2panda-auth panics on a non-create with no dependencies. The mapping must never
    // produce that case for a group that exists.
    let ops = operations(&mapping_dag());
    let state = state_before_remove(&ops);
    let literal = AuthOperation {
        dependencies: Vec::new(),
        ..ops[2].clone()
    };
    let outcome = catch_unwind(AssertUnwindSafe(|| Graph::process(state, &literal)));
    assert!(
        outcome.is_err(),
        "the crate panics on a non-create with no dependencies"
    );
}

#[test]
fn test_s2_regression_root_dependency_voids_a_valid_add() {
    // Naming only the create (a root) drops Bob's add from the remove's past. Dave is then
    // removed although Alice had seen his add. The frontier keeps him (checked above).
    let ops = operations(&mapping_dag());
    let state = state_before_remove(&ops);
    let root = AuthOperation {
        dependencies: vec![ident(0)],
        ..ops[2].clone()
    };
    let after_root = Graph::process(state, &root).unwrap();
    let got = members(&after_root);
    assert!(
        !got.iter().any(|(id, _)| *id == key('D')),
        "the root dependency voids Dave's add"
    );
}

#[test]
fn test_s2_regression_unknown_dependency_is_an_error_not_a_panic() {
    let ops = operations(&mapping_dag());
    let state = state_before_remove(&ops);
    let unknown = AuthOperation {
        dependencies: vec![ident(99)],
        ..ops[2].clone()
    };
    let result = catch_unwind(AssertUnwindSafe(|| Graph::process(state, &unknown)));
    assert!(
        matches!(result, Ok(Err(GroupCrdtError::MissingDependencies(dep, _))) if dep == ident(99)),
        "an unknown dependency is an error"
    );
}

#[test]
fn test_s2_frontier_is_order_independent_across_every_causal_order_of_the_scenarios() {
    // Every scenario above gives one outcome in every order. This asserts it once more on the
    // longest of them, the three-cycle, so a change in the mapping shows up here too.
    let mut events = base();
    events.push(ev(1, 'A', &[0], remove('B')));
    events.push(ev(2, 'B', &[0], remove('C')));
    events.push(ev(3, 'C', &[0], remove('A')));
    let ops = operations(&events);
    let outcomes: std::collections::BTreeSet<String> = causal_orders(&ops)
        .iter()
        .map(|order| format!("{:?}", members(&apply(&ops, order).unwrap())))
        .collect();
    assert_eq!(outcomes.len(), 1);
}
