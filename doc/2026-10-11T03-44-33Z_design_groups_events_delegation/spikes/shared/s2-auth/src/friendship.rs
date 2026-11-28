//! (d) Friendship: a two-member group. p2panda-auth has no member cap and no dissolve. Both are
//! ours: a deterministic ingest rewrite for the cap, and a projection over the membership state.

use std::collections::{BTreeSet, HashSet};

use p2panda_auth::Access;
use p2panda_auth::group::GroupAction;
use p2panda_auth::test_utils::{TestGroup, TestGroupState, TestOperation};

use crate::dag::*;

const FRIENDSHIP: char = '2';
const PAIR: [char; 2] = [ALICE, BOB];
const ORDER_CAP: usize = 5000;

/// Ours. A friendship admits only its pair. An op that names anyone else becomes a valid no-op
/// (see `dag::noop`) before it reaches p2panda-auth. The decision depends only on the op, so
/// every replica makes the same call. The built-in `ignore` set cannot be used: `StrongRemove`
/// clears it on every `process`, and `apply_action` is `pub(crate)`.
fn cap(op: TestOperation) -> TestOperation {
    let target = match &op.action {
        GroupAction::Add { member, .. }
        | GroupAction::Remove { member }
        | GroupAction::Promote { member, .. }
        | GroupAction::Demote { member, .. } => Some(member.id()),
        GroupAction::Create { .. } => None,
    };
    match target {
        Some(t) if !PAIR.contains(&t) => TestOperation {
            action: noop(ALICE),
            ..op
        },
        _ => op,
    }
}

/// Ours. A removal is effective if its target stood at the removal's dependencies and does not
/// stand at the removal itself. Friendship dissolves on the first effective removal of either
/// side, and stays dissolved (a re-friend is a new group with the counter incremented).
fn dissolved(y: &TestGroupState, dag: &Dag) -> bool {
    dag.events.iter().any(|e| {
        let Kind::Auth {
            group,
            action: GroupAction::Remove { member },
            ..
        } = &e.kind
        else {
            return false;
        };
        let target = member.id();
        if *group != FRIENDSHIP || !PAIR.contains(&target) {
            return false;
        }
        let before: HashSet<Id> = dag.frontier(e.id, FRIENDSHIP).into_iter().collect();
        let stood = TestGroup::members_at(y, before, FRIENDSHIP)
            .unwrap()
            .iter()
            .any(|(id, _)| *id == target);
        let stands = TestGroup::members_at(y, HashSet::from([e.id]), FRIENDSHIP)
            .unwrap()
            .iter()
            .any(|(id, _)| *id == target);
        stood && !stands
    })
}

fn sorted(mut list: Vec<(char, Access<()>)>) -> Vec<(char, Access<()>)> {
    list.sort_by_key(|(id, _)| *id);
    list
}

fn base() -> Dag {
    let mut dag = Dag::default();
    // Friend request: Alice creates the group with both sides (Manage over each other).
    dag.push(auth(
        FRIENDSHIP,
        0,
        ALICE,
        create(&[(ALICE, manage()), (BOB, manage())]),
        &[],
    ));
    dag
}

fn run_case(
    name: &str,
    dag: &Dag,
    capped: bool,
    expect: &[(char, Access<()>)],
    expect_dissolved: bool,
) -> usize {
    let ops: Vec<TestOperation> = dag
        .ops()
        .into_iter()
        .map(|op| if capped { cap(op) } else { op })
        .collect();
    let all = orders(&ops, ORDER_CAP);
    let mut outcomes = BTreeSet::new();
    let mut dissolves = BTreeSet::new();
    let mut errors = Vec::new();
    for order in &all {
        match apply(&ops, order) {
            Ok(y) => {
                outcomes.insert(show(&members(&y, FRIENDSHIP)));
                dissolves.insert(dissolved(&y, dag));
            }
            Err(e) => errors.push(e),
        }
    }
    let expected = show(&sorted(expect.to_vec()));
    let ok = errors.is_empty()
        && outcomes.len() == 1
        && outcomes.contains(&expected)
        && dissolves.len() == 1
        && dissolves.contains(&expect_dissolved);
    let detail = format!(
        "{} orders, errors {}; members {}; dissolved {:?}; expected {expected}, dissolved {expect_dissolved}",
        all.len(),
        errors.len(),
        outcomes.iter().cloned().collect::<Vec<_>>().join(" | "),
        dissolves,
    );
    report(name, ok, &detail)
}

pub fn run() -> usize {
    println!("(d) friendship: two-member cap and dissolve-on-removal (cap and dissolve are ours)");
    let mut failures = 0;

    // Concurrent third-party adds: Alice adds Claire, Bob adds Dave, both after the request.
    let mut concurrent_adds = base();
    concurrent_adds.push(auth(FRIENDSHIP, 1, ALICE, add(CLAIRE, manage()), &[0]));
    concurrent_adds.push(auth(FRIENDSHIP, 2, BOB, add(DAVE, manage()), &[0]));
    failures += run_case(
        "F1 concurrent adds beyond cap, p2panda alone (control)",
        &concurrent_adds,
        false,
        &[
            (ALICE, manage()),
            (BOB, manage()),
            (CLAIRE, manage()),
            (DAVE, manage()),
        ],
        false,
    );
    failures += run_case(
        "F2 concurrent adds beyond cap, with cap (every order)",
        &concurrent_adds,
        true,
        &[(ALICE, manage()), (BOB, manage())],
        false,
    );

    // The same, with a concurrent removal of Bob: friendship dissolves.
    let mut with_removal = concurrent_adds.clone();
    with_removal.push(auth(FRIENDSHIP, 3, ALICE, remove(BOB), &[0]));
    failures += run_case(
        "F3 concurrent adds and removal of Bob",
        &with_removal,
        true,
        &[(ALICE, manage())],
        true,
    );

    // Mutual removal: both sides remove each other concurrently.
    let mut mutual = base();
    mutual.push(auth(FRIENDSHIP, 1, ALICE, remove(BOB), &[0]));
    mutual.push(auth(FRIENDSHIP, 2, BOB, remove(ALICE), &[0]));
    failures += run_case("F4 mutual removal dissolves", &mutual, true, &[], true);

    // Dissolve is terminal: Alice re-adds Bob after removing him. Membership returns, dissolution stays.
    let mut readd = base();
    readd.push(auth(FRIENDSHIP, 1, ALICE, remove(BOB), &[0]));
    readd.push(auth(FRIENDSHIP, 2, ALICE, add(BOB, manage()), &[1]));
    failures += run_case(
        "F5 re-add after dissolve: members return, group stays dissolved",
        &readd,
        true,
        &[(ALICE, manage()), (BOB, manage())],
        true,
    );

    failures
}
