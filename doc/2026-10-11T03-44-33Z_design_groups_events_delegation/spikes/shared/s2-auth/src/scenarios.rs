//! (a) StrongRemove scenarios, checked over every causal-delivery order.
//! (b) The `after` mapping: naive, auth-frontier, and raw `after`.

use std::collections::BTreeSet;

use p2panda_auth::Access;
use p2panda_auth::test_utils::{TestGroup, TestOperation};

use crate::dag::*;
use crate::panics::catch;

const ORDER_CAP: usize = 5000;

struct Case {
    name: &'static str,
    note: &'static str,
    dag: Dag,
    expected: Vec<(char, Access<()>)>,
}

fn base() -> Dag {
    let mut dag = Dag::default();
    dag.push(auth(
        GROUP,
        0,
        ALICE,
        create(&[(ALICE, manage()), (BOB, manage()), (CLAIRE, manage())]),
        &[],
    ));
    dag
}

fn sorted(mut list: Vec<(char, Access<()>)>) -> Vec<(char, Access<()>)> {
    list.sort_by_key(|(id, _)| *id);
    list
}

fn cases() -> Vec<Case> {
    let mut out = Vec::new();

    // 1. Week-late push: Alice removes Bob (R). Bob's add of Dave was made without seeing R (X).
    let mut d = base();
    d.push(auth(GROUP, 1, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 2, BOB, add(DAVE, manage()), &[0]));
    out.push(Case {
        name: "1 week-late add",
        note: "X by removed Bob, concurrent with R: voided",
        dag: d,
        expected: sorted(vec![(ALICE, manage()), (CLAIRE, manage())]),
    });

    // 2. Causally prior: Bob adds Dave (Y). Alice sees Y, then removes Bob (R after Y).
    let mut d = base();
    d.push(auth(GROUP, 1, BOB, add(DAVE, manage()), &[0]));
    d.push(auth(GROUP, 2, ALICE, remove(BOB), &[1]));
    out.push(Case {
        name: "2 causally prior add",
        note: "Y in R's past: survives",
        dag: d,
        expected: sorted(vec![
            (ALICE, manage()),
            (CLAIRE, manage()),
            (DAVE, manage()),
        ]),
    });

    // 3. Transitive: Bob adds Dave (Z, concurrent with R). Dave, whose authority comes only from Z,
    //    adds Eve after seeing Z (W).
    let mut d = base();
    d.push(auth(GROUP, 1, BOB, add(DAVE, manage()), &[0]));
    d.push(auth(GROUP, 2, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 3, DAVE, add(EVE, read()), &[1]));
    out.push(Case {
        name: "3 transitive void",
        note: "W by Dave, whose authority came from voided Z: voided",
        dag: d,
        expected: sorted(vec![(ALICE, manage()), (CLAIRE, manage())]),
    });

    // 3b. Same shape, but the add after Z is by Claire, who has independent authority.
    let mut d = base();
    d.push(auth(GROUP, 1, BOB, add(DAVE, manage()), &[0]));
    d.push(auth(GROUP, 2, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 3, CLAIRE, add(EVE, read()), &[1]));
    out.push(Case {
        name: "3b dependent add by independent author",
        note: "W by Claire after voided Z: stands under authority rule",
        dag: d,
        expected: sorted(vec![(ALICE, manage()), (CLAIRE, manage()), (EVE, read())]),
    });

    // 4. Mutual removal: Alice removes Bob and Bob removes Alice, concurrently.
    let mut d = base();
    d.push(auth(GROUP, 1, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 2, BOB, remove(ALICE), &[0]));
    out.push(Case {
        name: "4 mutual removal",
        note: "both removed",
        dag: d,
        expected: sorted(vec![(CLAIRE, manage())]),
    });

    // 4b. Three-cycle: Alice removes Bob, Bob removes Claire, Claire removes Alice.
    let mut d = base();
    d.push(auth(GROUP, 1, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 2, BOB, remove(CLAIRE), &[0]));
    d.push(auth(GROUP, 3, CLAIRE, remove(ALICE), &[0]));
    out.push(Case {
        name: "4b three-cycle",
        note: "every member of the cycle removed",
        dag: d,
        expected: vec![],
    });

    // 5. Concurrent re-add: Claire removes and re-adds Bob (1, 2), without seeing Alice's removal
    //    of Bob (3). The re-add is valid where it was made (Bob was absent there) but concurrent
    //    with R, so it is voided and Bob is out.
    let mut d = base();
    d.push(auth(GROUP, 1, CLAIRE, remove(BOB), &[0]));
    d.push(auth(GROUP, 2, CLAIRE, add(BOB, manage()), &[1]));
    d.push(auth(GROUP, 3, ALICE, remove(BOB), &[0]));
    out.push(Case {
        name: "5 concurrent re-add",
        note: "re-add concurrent with R: voided",
        dag: d,
        expected: sorted(vec![(ALICE, manage()), (CLAIRE, manage())]),
    });

    // 5c. p2panda's `concurrent_readds_filtered`: Alice removes and re-adds Bob (1, 2). Claire
    //     removes Bob concurrently (3). The re-add is concurrent with Claire's removal: voided.
    let mut d = base();
    d.push(auth(GROUP, 1, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 2, ALICE, add(BOB, manage()), &[1]));
    d.push(auth(GROUP, 3, CLAIRE, remove(BOB), &[0]));
    out.push(Case {
        name: "5c re-add concurrent with another manager's removal",
        note: "re-add concurrent with Claire's removal: voided",
        dag: d,
        expected: sorted(vec![(ALICE, manage()), (CLAIRE, manage())]),
    });

    // 5b. Control: Alice re-adds Bob after seeing R.
    let mut d = base();
    d.push(auth(GROUP, 1, ALICE, remove(BOB), &[0]));
    d.push(auth(GROUP, 2, ALICE, add(BOB, manage()), &[1]));
    out.push(Case {
        name: "5b causal re-add (control)",
        note: "re-add after R: stands",
        dag: d,
        expected: sorted(vec![(ALICE, manage()), (BOB, manage()), (CLAIRE, manage())]),
    });

    out
}

fn key(list: &[(char, Access<()>)]) -> String {
    show(list)
}

pub fn strong_remove() -> usize {
    let mut failures = 0;
    println!("(a) StrongRemove scenarios, every causal-delivery order (cap {ORDER_CAP})");
    for case in cases() {
        let ops: Vec<TestOperation> = case.dag.ops();
        let all = orders(&ops, ORDER_CAP);
        let mut outcomes = BTreeSet::new();
        let mut errors = Vec::new();
        for order in &all {
            match apply(&ops, order) {
                Ok(y) => {
                    outcomes.insert(key(&members(&y, GROUP)));
                }
                Err(e) => errors.push(e),
            }
        }
        let expected = key(&case.expected);
        let ok = errors.is_empty() && outcomes.len() == 1 && outcomes.contains(&expected);
        let detail = format!(
            "{} orders, {} distinct outcome(s), errors {}; members {}; expected {}. {}",
            all.len(),
            outcomes.len(),
            errors.len(),
            outcomes.iter().cloned().collect::<Vec<_>>().join(" | "),
            expected,
            case.note,
        );
        failures += report(case.name, ok, &detail);
    }
    failures
}

/// (b) Case: Alice removes Bob, after a document that names Bob's add of Dave. Bob's add is in
/// Alice's causal past, so it must stand.
pub fn mapping() -> usize {
    let mut failures = 0;
    println!(
        "(b) the after mapping: X (Bob adds Dave) -> D (doc naming X) -> R (Alice removes Bob, naming D)"
    );

    let mut d = base();
    d.push(auth(GROUP, 1, BOB, add(DAVE, manage()), &[0]));
    d.push(other(2, BOB, &[1]));
    d.push(auth(GROUP, 3, ALICE, remove(BOB), &[2]));
    let expected = sorted(vec![
        (ALICE, manage()),
        (CLAIRE, manage()),
        (DAVE, manage()),
    ]);
    let order = [0usize, 1, 2];

    // Auth frontier: R names X, the maximal auth op in its full causal past.
    let frontier_r = d.frontier(3, GROUP);
    let ops_frontier = vec![d.op(0), d.op(1), d.op(3)];
    let y = apply(&ops_frontier, &[0, 1, 2]).expect("auth frontier mapping applies");
    let got = members(&y, GROUP);
    failures += report(
        "frontier mapping: R names X",
        frontier_r == vec![1],
        &format!("frontier(R) = {frontier_r:?}"),
    );
    // p2panda's own dependency choice at the remover (GroupCrdt::heads on the local state after
    // X and D) should agree with the frontier, because the remover has applied its whole past.
    let before_r = apply(&ops_frontier, &[0, 1]).expect("state before R");
    let mut local_heads = TestGroup::heads(&before_r, GROUP, &remove(BOB));
    local_heads.sort();
    failures += report(
        "GroupCrdt::heads at the remover equals the auth frontier",
        local_heads == frontier_r,
        &format!("heads {local_heads:?} vs frontier {frontier_r:?}"),
    );
    failures += report(
        "frontier mapping: X stands",
        got == expected,
        &format!("members {}", show(&got)),
    );

    // Naive literal: R's dependencies are the auth ops directly in `after`. D is dropped, so [].
    let naive_deps = d.direct_auth_after(3, GROUP);
    let ops_naive = vec![d.op(0), d.op(1), d.op_with(3, naive_deps.clone())];
    let naive = catch(|| apply(&ops_naive, &order));
    println!("  naive literal (R deps {naive_deps:?}): {naive:?}");
    let naive_panicked = matches!(&naive, Err(m) if m.contains("group already present"));
    failures += report(
        "naive literal: panics in validate (no auth dependency)",
        naive_panicked,
        &format!("{naive:?}"),
    );

    // Naive with a root dependency (what a remover's view gives when it skips D): X is concurrent
    // with R in the auth graph, so it is voided even though Alice saw it.
    let ops_root = vec![d.op(0), d.op(1), d.op_with(3, vec![0])];
    let root = apply(&ops_root, &order).expect("naive root applies");
    let got_root = members(&root, GROUP);
    println!("  naive with root dependency: members {}", show(&got_root));
    failures += report(
        "naive with root dependency voids X (wrong)",
        got_root == sorted(vec![(ALICE, manage()), (CLAIRE, manage())]),
        &format!("members {}", show(&got_root)),
    );

    // Raw `after` passed as dependencies: names a non-auth event, so process refuses it.
    let raw = d.op_with(3, d.get(3).after.clone());
    let raw_result = apply(&[d.op(0), d.op(1), raw], &order);
    println!("  raw after as deps: {raw_result:?}");
    failures += report(
        "raw after as deps is refused (MissingDependencies)",
        matches!(&raw_result, Err(e) if e.contains("MissingDependencies")),
        &format!("{raw_result:?}"),
    );

    // Minimal frontier: X1 -> D1 -> X2 -> D2 -> R. X2's frontier is {X1} (not {0, X1}); R's is {X2}.
    let mut m = base();
    m.push(auth(GROUP, 1, BOB, add(DAVE, manage()), &[0]));
    m.push(other(2, BOB, &[1]));
    m.push(auth(GROUP, 3, CLAIRE, add(EVE, read()), &[2]));
    m.push(other(4, CLAIRE, &[3]));
    m.push(auth(GROUP, 5, ALICE, remove(BOB), &[4]));
    let f3 = m.frontier(3, GROUP);
    let f5 = m.frontier(5, GROUP);
    failures += report(
        "frontier is minimal across non-auth hops",
        f3 == vec![1] && f5 == vec![3],
        &format!("frontier(X2) = {f3:?}, frontier(R) = {f5:?}"),
    );
    let mismatches = incremental_mismatches(&d, GROUP) + incremental_mismatches(&m, GROUP);
    failures += report(
        "incremental frontier (from after-heads) equals the full walk, every event",
        mismatches == 0,
        &format!("{mismatches} mismatches"),
    );

    // Order check on the correct mapping: every causal order gives the same membership.
    let ops_ok = d.ops();
    let all = orders(&ops_ok, ORDER_CAP);
    let same = all.iter().all(|o| {
        apply(&ops_ok, o)
            .map(|y| members(&y, GROUP) == expected)
            .unwrap_or(false)
    });
    failures += report(
        "frontier mapping is order-independent",
        same,
        &format!("{} orders", all.len()),
    );
    failures
}

pub fn run() -> usize {
    strong_remove() + mapping()
}
