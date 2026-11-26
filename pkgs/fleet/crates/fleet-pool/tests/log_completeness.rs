//! Decision-log completeness (#15066, design #13752 §3.2: "a pool
//! decision you cannot read back is a bug").
//!
//! The test runs a scripted outcome sequence, snapshots the pool
//! after every operation, then replays the decision log through a
//! shadow model that sees *only* log entries. After each
//! operation's entries, the shadow's reconstructed state — member
//! states and wall scopes — must equal the snapshot. Any
//! transition or wall event the pool failed to log breaks the
//! replay.

use fleet_pool::{
    AttemptKind, Classification, Decision, DecisionKind, MemberState, NodeId, Ordered, Pool,
    ServedDisposition,
};
use std::collections::{BTreeMap, BTreeSet};

const MEMBERS: [&str; 4] = ["m1", "m2", "m3", "m4"];

fn tree_pool() -> Pool {
    let mut pool = Pool::new();
    pool.add_group("root", None);
    pool.add_group("g1", Some(&NodeId::new("root")));
    pool.add_group("g2", Some(&NodeId::new("root")));
    pool.add_member("m1", Some(&NodeId::new("g1")));
    pool.add_member("m2", Some(&NodeId::new("g1")));
    pool.add_member("m3", Some(&NodeId::new("g2")));
    pool.add_member("m4", Some(&NodeId::new("g2")));
    pool
}

fn ancestors_of(id: &str) -> Vec<&'static str> {
    match id {
        "m1" | "m2" => vec!["g1", "root"],
        "m3" | "m4" => vec!["g2", "root"],
        "g1" | "g2" => vec!["root"],
        _ => vec![],
    }
}

fn serve_failure(scope: &NodeId, reset_hint: Option<u64>) -> Classification {
    Classification {
        kind: AttemptKind::ServeFailure,
        served_disposition: ServedDisposition::Hold,
        advance: true,
        evidence_scope: scope.clone(),
        reset_hint,
        evidence: "completeness serve failure".to_owned(),
    }
}

fn served_wall(scope: &NodeId) -> Classification {
    Classification {
        kind: AttemptKind::ServedFailure,
        served_disposition: ServedDisposition::Wall,
        advance: true,
        evidence_scope: scope.clone(),
        reset_hint: None,
        evidence: "completeness served failure".to_owned(),
    }
}

/// The shadow model: base states and wall scopes, rebuilt from
/// log entries alone.
#[derive(Default)]
struct Shadow {
    base: BTreeMap<String, MemberState>,
    walls: BTreeSet<String>,
}

impl Shadow {
    fn apply(&mut self, entry: &Decision) {
        let subject = entry
            .member
            .as_ref()
            .map(|id| id.as_str().to_owned())
            .unwrap_or_default();
        match entry.kind {
            DecisionKind::Selection => {
                // A selection changes nothing, but what it recorded
                // must match the shadow's state at this point.
                if let Some(member) = &entry.member {
                    assert_eq!(
                        entry.state,
                        Some(self.effective(member.as_str())),
                        "selection entry disagrees with the reconstructed state",
                    );
                }
            }
            DecisionKind::Transition => {
                self.base.insert(
                    subject,
                    entry.state.expect("a transition records the new state"),
                );
            }
            DecisionKind::WallPlaced => {
                self.walls.insert(subject);
            }
            DecisionKind::WallNarrowed | DecisionKind::WallRemoved => {
                self.walls.remove(&subject);
            }
        }
    }

    fn effective(&self, member: &str) -> MemberState {
        if self.walls.contains(member)
            || ancestors_of(member).iter().any(|a| self.walls.contains(*a))
        {
            return MemberState::Walled;
        }
        self.base
            .get(member)
            .copied()
            .unwrap_or(MemberState::Serving)
    }

    fn snapshot(&self) -> (BTreeMap<String, MemberState>, BTreeSet<String>) {
        let states = MEMBERS
            .iter()
            .map(|m| (m.to_string(), self.effective(m)))
            .collect();
        (states, self.walls.clone())
    }
}

fn snapshot(pool: &Pool, now: u64) -> (BTreeMap<String, MemberState>, BTreeSet<String>) {
    let states = MEMBERS
        .iter()
        .map(|m| (m.to_string(), pool.state_of(&NodeId::new(*m), now)))
        .collect();
    let walls = pool.walls().map(|w| w.scope.to_string()).collect();
    (states, walls)
}

#[test]
fn the_full_outcome_sequence_reconstructs_from_the_log_alone() {
    let mut pool = tree_pool();
    let (m1, m2, m3, g1) = (
        NodeId::new("m1"),
        NodeId::new("m2"),
        NodeId::new("m3"),
        NodeId::new("g1"),
    );

    // (log length after the op, snapshot after the op)
    let mut marks: Vec<(usize, (BTreeMap<String, MemberState>, BTreeSet<String>))> = Vec::new();
    let mut mark = |pool: &Pool, now: u64, marks: &mut Vec<_>| {
        marks.push((pool.decisions().len(), snapshot(pool, now)));
    };

    // 1–2: m1 demotes, then walls its group.
    pool.record_outcome(&m1, &serve_failure(&g1, Some(100)), 0);
    mark(&pool, 0, &mut marks);
    pool.record_outcome(&m1, &serve_failure(&g1, Some(100)), 0);
    mark(&pool, 0, &mut marks);

    // 3: a selection while the group is walled (picks m3).
    let mut strategy = Ordered;
    assert_eq!(
        pool.select(&mut strategy, &|_| Some(0.0), 1),
        Some(m3.clone())
    );
    mark(&pool, 1, &mut marks);

    // 4: a sibling success narrows the group wall to m1.
    pool.record_success(&m2, 2);
    mark(&pool, 2, &mut marks);

    // 5: m3 walls at its own scope on a served failure.
    pool.record_outcome(&m3, &served_wall(&m3), 3);
    mark(&pool, 3, &mut marks);

    // 6: m1 succeeds — its base restores, its own wall still gates.
    pool.record_success(&m1, 4);
    mark(&pool, 4, &mut marks);

    // 7: time passes m1's recorded reset; a good probe re-seats it
    // at demoted weight. m3's reset-less wall (inferred bound) is
    // still active.
    pool.tick(101, &mut |_: &NodeId| true);
    mark(&pool, 101, &mut marks);

    // 8: m1's success at demoted weight restores it fully.
    pool.record_success(&m1, 102);
    mark(&pool, 102, &mut marks);

    // The scenario exercised every entry kind.
    let kinds: BTreeSet<&str> = pool
        .decisions()
        .iter()
        .map(|d| match d.kind {
            DecisionKind::Selection => "selection",
            DecisionKind::Transition => "transition",
            DecisionKind::WallPlaced => "wall-placed",
            DecisionKind::WallNarrowed => "wall-narrowed",
            DecisionKind::WallRemoved => "wall-removed",
        })
        .collect();
    assert_eq!(
        kinds.len(),
        5,
        "scenario must produce every kind: {kinds:?}"
    );

    // Replay: the shadow sees only the log.
    let mut shadow = Shadow::default();
    let mut cursor = 0;
    for (log_len, expected) in &marks {
        for entry in &pool.decisions()[cursor..*log_len] {
            shadow.apply(entry);
        }
        cursor = *log_len;
        assert_eq!(
            &shadow.snapshot(),
            expected,
            "reconstruction diverged after {} log entries",
            log_len,
        );
    }
    assert_eq!(cursor, pool.decisions().len());
}
