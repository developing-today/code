//! (c) Unknown and missing dependencies: what p2panda-auth does, and the admission guard we need.

use std::collections::HashSet;
use std::panic::{self, AssertUnwindSafe};
use std::sync::Mutex;

use p2panda_auth::group::GroupAction;
use p2panda_auth::test_utils::{TestGroup, TestGroupState, TestOperation};

use crate::dag::*;

static LAST_PANIC: Mutex<String> = Mutex::new(String::new());

/// Runs `f`. On panic, returns "file:line: message" taken from the panic hook.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, String> {
    let previous = panic::take_hook();
    panic::set_hook(Box::new(|info| {
        let payload = info.payload();
        let message = payload
            .downcast_ref::<&str>()
            .map(|s| s.to_string())
            .or_else(|| payload.downcast_ref::<String>().cloned())
            .unwrap_or_default();
        let location = info
            .location()
            .map(|l| {
                let file = l.file().rsplit("p2panda-auth/").next().unwrap_or(l.file());
                format!("{file}:{}", l.line())
            })
            .unwrap_or_default();
        *LAST_PANIC.lock().unwrap() = format!("{location}: {message}");
    }));
    let outcome = panic::catch_unwind(AssertUnwindSafe(f));
    panic::set_hook(previous);
    outcome.map_err(|_| LAST_PANIC.lock().unwrap().clone())
}

pub enum Admission {
    Admit,
    /// Hold the op until these dependencies are present.
    Wait(Vec<Id>),
    Reject(&'static str),
}

/// The admission guard we propose: decide before calling `GroupCrdt::process`.
pub fn admit(y: &TestGroupState, op: &TestOperation) -> Admission {
    let missing: Vec<Id> = op
        .dependencies
        .iter()
        .copied()
        .filter(|d| !y.inner.operations.contains_key(d))
        .collect();
    if !missing.is_empty() {
        return Admission::Wait(missing);
    }
    if op.action.is_create() {
        return Admission::Admit;
    }
    if op.dependencies.is_empty() {
        return Admission::Reject("non-create op with no dependencies");
    }
    let deps: HashSet<Id> = op.dependencies.iter().copied().collect();
    match y.inner.state_at(&deps) {
        Ok(states) if states.contains_key(&op.group_id) => Admission::Admit,
        _ => Admission::Reject("group does not exist at the operation's dependencies"),
    }
}

fn op(
    id: Id,
    author: char,
    group: char,
    action: GroupAction<char, ()>,
    deps: &[Id],
) -> TestOperation {
    TestOperation {
        id,
        author,
        dependencies: deps.to_vec(),
        group_id: group,
        action,
    }
}

pub fn run() -> usize {
    let mut failures = 0;
    println!("(c) unknown and missing dependencies");

    // Base state: group 1 created by Alice with Alice, Bob, Claire (Manage).
    let create_op = op(
        0,
        ALICE,
        GROUP,
        create(&[(ALICE, manage()), (BOB, manage()), (CLAIRE, manage())]),
        &[],
    );
    let y0: TestGroupState = apply(std::slice::from_ref(&create_op), &[0]).expect("base state");

    // P1: a dependency hash that is not in the graph, through the public `process`.
    let unknown = op(1, BOB, GROUP, add(DAVE, manage()), &[999]);
    let p1 = catch(|| {
        TestGroup::process(y0.clone(), &unknown)
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    });
    println!("  P1 process(dep 999 unknown): {p1:?}");
    let p1_ok = matches!(&p1, Ok(Err(e)) if e.contains("MissingDependencies"));
    failures += report(
        "P1 unknown dependency is an error, not a panic",
        p1_ok,
        &format!("{p1:?}"),
    );

    // P2: an unknown head given to `members_at` (a public API that takes raw heads).
    let p2 = catch(|| {
        TestGroup::members_at(&y0, HashSet::from([999]), GROUP)
            .map(|m| m.len())
            .map_err(|e| format!("{e:?}"))
    });
    println!("  P2 members_at(heads {{999}}): {p2:?}");

    // P3: a non-create op with no dependencies, for a group that exists (the naive literal mapping).
    let empty_deps = op(2, BOB, GROUP, remove(ALICE), &[]);
    let p3 = catch(|| {
        TestGroup::process(y0.clone(), &empty_deps)
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    });
    println!("  P3 process(non-create, deps []): {p3:?}");
    failures += report(
        "P3 empty dependencies panic in validate",
        p3.is_err(),
        &format!("{p3:?}"),
    );

    // P4: deps that exist but do not contain the group (group 2 was never created on this path).
    let other_group = op(3, BOB, '2', add(DAVE, manage()), &[0]);
    let p4 = catch(|| {
        TestGroup::process(y0.clone(), &other_group)
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    });
    println!("  P4 process(group 2 unknown at deps [0]): {p4:?}");
    failures += report(
        "P4 group absent at dependencies panics",
        p4.is_err(),
        &format!("{p4:?}"),
    );

    // P5: a non-create op on an empty state.
    let empty_state = TestGroup::init();
    let p5 = catch(|| {
        TestGroup::process(empty_state, &op(4, BOB, GROUP, add(DAVE, manage()), &[]))
            .map(|_| ())
            .map_err(|e| format!("{e:?}"))
    });
    println!("  P5 process(non-create on empty state): {p5:?}");
    failures += report(
        "P5 unknown group on empty state panics",
        p5.is_err(),
        &format!("{p5:?}"),
    );

    // P6: the guard holds an op with an unknown dependency, rejects P3 and P4, and admits once
    // the missing op arrives.
    let mut y = y0.clone();
    let held = op(1, BOB, GROUP, add(DAVE, manage()), &[5]);
    let decision = |y: &TestGroupState, o: &TestOperation| match admit(y, o) {
        Admission::Admit => "admit".to_string(),
        Admission::Wait(m) => format!("wait for {m:?}"),
        Admission::Reject(r) => format!("reject ({r})"),
    };
    let d_wait = decision(&y, &held);
    let d_empty = decision(&y, &empty_deps);
    let d_group = decision(&y, &other_group);
    println!("  P6 guard: held={d_wait}; empty-deps={d_empty}; group-absent={d_group}");
    let guard_ok = d_wait.starts_with("wait")
        && d_empty.starts_with("reject")
        && d_group.starts_with("reject");
    failures += report(
        "P6 guard waits, rejects, then admits",
        guard_ok,
        &format!("{d_wait}; {d_empty}; {d_group}"),
    );

    // Dependency 5 arrives (Alice adds Eve, after the create). The held op is then admitted.
    let late = op(5, ALICE, GROUP, add(EVE, manage()), &[0]);
    y = TestGroup::process(y, &late).expect("late dependency");
    let admitted_after = decision(&y, &held) == "admit";
    if admitted_after {
        y = TestGroup::process(y, &held).expect("held op");
    }
    failures += report(
        "P6 held op admitted and applied once its dependency arrives",
        admitted_after && members(&y, GROUP).iter().any(|(id, _)| *id == DAVE),
        &format!("members {}", show(&members(&y, GROUP))),
    );

    // P7: an op that is invalid at its own position (Eve has Read and tries to add) is refused by
    // process and never enters the graph. Its descendant then stalls on a missing dependency.
    let g3 = op(
        10,
        ALICE,
        '3',
        create(&[(ALICE, manage()), (EVE, read())]),
        &[],
    );
    let y3 = TestGroup::process(TestGroup::init(), &g3).expect("group 3");
    let invalid = op(11, EVE, '3', add(DAVE, manage()), &[10]);
    let refused = TestGroup::process(y3.clone(), &invalid)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"));
    let child = op(12, ALICE, '3', add(CLAIRE, read()), &[11]);
    let stalled = TestGroup::process(y3.clone(), &child)
        .map(|_| ())
        .map_err(|e| format!("{e:?}"));
    println!("  P7 invalid op refused: {refused:?}; its child: {stalled:?}");
    failures += report(
        "P7 a refused op stalls its descendants (layer must admit a placeholder)",
        matches!(&refused, Err(e) if e.contains("StateChangeError"))
            && matches!(&stalled, Err(e) if e.contains("MissingDependencies")),
        &format!("refused {refused:?}; child {stalled:?}"),
    );
    failures
}
