//! (e) Delegation cascade prototype, on top of p2panda-auth membership.
//!
//! From p2panda-auth: membership at any causal position (`GroupCrdt::members_at`), the StrongRemove
//! concurrency rules, and access levels. Ours: delegation edges, attenuation, the chain search
//! (UCAN: a link is cut only where no other intact chain carries the authority), the three removal
//! modes, and the veto. A vetoed action is rewritten to a no-op at ingest, so membership never sees it.

use std::collections::{HashMap, HashSet};

use p2panda_auth::Access;
use p2panda_auth::group::GroupAction;
use p2panda_auth::test_utils::{TestGroup, TestGroupState, TestOperation};

use crate::dag::*;

const OWNER: char = ALICE;
const JOE: char = 'J';
const JIM: char = 'M';
const DAN: char = 'N';
const ZED: char = 'Z';
const YAN: char = 'Y';
const QUINN: char = 'Q';
const KIM: char = 'K';
const LOU: char = 'L';
const XAV: char = 'X';

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    /// p2panda-auth alone, with no delegation check (control).
    Membership,
    Cascade,
    KeepPrior,
    Transfer,
}

/// A usable link: a delegation event, or under transfer the removal's re-issue from the successor.
struct Edge {
    id: Id,
    from: char,
    to: char,
    scope: Access<()>,
    redelegate: bool,
    reissued: bool,
}

struct Gov<'a> {
    dag: &'a Dag,
    mode: Mode,
    y: TestGroupState,
}

impl Gov<'_> {
    /// `who` holds at least `scope` at the causal position of event `pos` (p2panda-auth membership).
    fn holds(&self, who: char, scope: &Access<()>, pos: Id) -> bool {
        let heads: HashSet<Id> = self.dag.frontier(pos, GROUP).into_iter().collect();
        TestGroup::members_at(&self.y, heads, GROUP)
            .expect("members at position")
            .iter()
            .any(|(id, access)| *id == who && access >= scope)
    }

    /// Ours. A delegation is void in every mode when its grantor is removed by a concurrent removal.
    fn void_by_concurrent_removal(&self, d: Id) -> bool {
        let grantor = self.dag.get(d).author;
        let past_d = self.dag.past(d);
        self.dag.events.iter().any(|r| {
            matches!(&r.kind, Kind::Auth { action: GroupAction::Remove { member }, .. } if member.id() == grantor)
                && !past_d.contains(&r.id)
                && !self.dag.past(r.id).contains(&d)
        })
    }

    /// Transfer: the grantor's own links are superseded once it has been removed in the past of `pos`.
    fn removed_before(&self, who: char, pos: Id) -> bool {
        self.dag.past(pos).iter().any(|r| {
            matches!(&self.dag.get(*r).kind, Kind::Auth { action: GroupAction::Remove { member }, .. } if member.id() == who)
        })
    }

    /// Links usable in the causal past of `pos`: delegations, and under transfer the re-issues.
    fn edges(&self, pos: Id) -> Vec<Edge> {
        let mut out = Vec::new();
        for id in self.dag.past(pos) {
            let ev = self.dag.get(id);
            match &ev.kind {
                Kind::Delegate {
                    grantee,
                    scope,
                    redelegate,
                } => out.push(Edge {
                    id,
                    from: ev.author,
                    to: *grantee,
                    scope: scope.clone(),
                    redelegate: *redelegate,
                    reissued: false,
                }),
                Kind::Auth {
                    action: GroupAction::Remove { member },
                    successor: Some(s),
                    ..
                } if self.mode == Mode::Transfer => {
                    for d in self.dag.past(id) {
                        let dv = self.dag.get(d);
                        if let Kind::Delegate {
                            grantee,
                            scope,
                            redelegate,
                            ..
                        } = &dv.kind
                            && dv.author == member.id()
                            && self.issued(d)
                        {
                            out.push(Edge {
                                id,
                                from: *s,
                                to: *grantee,
                                scope: scope.clone(),
                                redelegate: *redelegate,
                                reissued: true,
                            });
                        }
                    }
                }
                _ => {}
            }
        }
        out
    }

    /// A delegation was valid where it was made: its grantor held the scope at its own position.
    fn issued(&self, d: Id) -> bool {
        let ev = self.dag.get(d);
        match &ev.kind {
            Kind::Delegate { scope, .. } => self.authorized(ev.author, scope, d),
            _ => false,
        }
    }

    /// Whether a link can carry authority at `pos`.
    fn link_ok(&self, e: &Edge, pos: Id) -> bool {
        if !e.reissued && self.void_by_concurrent_removal(e.id) {
            return false;
        }
        if !e.reissued && self.mode == Mode::Transfer && self.removed_before(e.from, pos) {
            return false;
        }
        let made_ok = if e.reissued {
            self.authorized(e.from, &e.scope, e.id)
        } else {
            self.issued(e.id)
        };
        made_ok
            && match self.mode {
                // Cascade: the grantor still holds what it passes on, at the action's position.
                Mode::Cascade => self.holds(e.from, &e.scope, pos),
                // A re-issue is a new grant from the successor: the successor must still hold.
                Mode::Transfer if e.reissued => self.holds(e.from, &e.scope, pos),
                _ => true,
            }
    }

    /// May `actor` exercise `scope` at the causal position of `pos`? Any intact chain from the owner
    /// suffices. Authority is cut only where no other chain carries it (UCAN).
    fn authorized(&self, actor: char, scope: &Access<()>, pos: Id) -> bool {
        if self.mode == Mode::Membership || actor == OWNER {
            return true;
        }
        if !self.holds(actor, scope, pos) {
            return false;
        }
        let edges = self.edges(pos);
        let mut reached = HashSet::from([OWNER]);
        let mut frontier = vec![OWNER];
        while let Some(u) = frontier.pop() {
            for e in edges.iter().filter(|e| e.from == u && e.scope >= *scope) {
                if reached.contains(&e.to)
                    || (e.to != actor && !e.redelegate)
                    || !self.link_ok(e, pos)
                {
                    continue;
                }
                if e.to == actor {
                    return true;
                }
                reached.insert(e.to);
                frontier.push(e.to);
            }
        }
        false
    }

    /// Replays the DAG through p2panda-auth. A manager action with no valid chain is rewritten to a
    /// no-op, and a delegation that was not valid where made is refused. Returns each event's verdict.
    fn ingest(dag: &Dag, mode: Mode) -> (TestGroupState, HashMap<Id, &'static str>) {
        let mut gov = Gov {
            dag,
            mode,
            y: TestGroup::init(),
        };
        let mut verdicts = HashMap::new();
        for ev in &dag.events {
            match &ev.kind {
                Kind::Auth { group, action, .. } => {
                    let mut action = action.clone();
                    let mut verdict = "applied";
                    let checked = !matches!(action, GroupAction::Create { .. });
                    if checked && !gov.authorized(ev.author, &manage(), ev.id) {
                        action = noop(OWNER);
                        verdict = "vetoed";
                    }
                    let op = TestOperation {
                        id: ev.id,
                        author: ev.author,
                        dependencies: dag.frontier(ev.id, *group),
                        group_id: *group,
                        action,
                    };
                    let y = std::mem::take(&mut gov.y);
                    gov.y = TestGroup::process(y, &op).expect("process");
                    verdicts.insert(ev.id, verdict);
                }
                Kind::Delegate { .. } => {
                    let verdict = if gov.issued(ev.id) {
                        "issued"
                    } else {
                        "refused"
                    };
                    verdicts.insert(ev.id, verdict);
                }
                Kind::Other => {}
            }
        }
        (gov.y, verdicts)
    }
}

/// Alice owns the group. Joe and Dan are managers she delegates to. Joe delegates to Jim. Joe is
/// removed (R), with `successor` as the transfer target. Jim acts after R (event 10) and concurrently
/// with R (event 11). `alt` adds the Dan-to-Jim chain (event 16).
fn build(alt: bool, successor: Option<char>) -> Dag {
    let mut d = Dag::default();
    d.push(auth(GROUP, 0, OWNER, create(&[(OWNER, manage())]), &[]));
    d.push(auth(GROUP, 1, OWNER, add(JOE, manage()), &[0]));
    d.push(auth(GROUP, 2, OWNER, add(DAN, manage()), &[1]));
    d.push(delegate(3, OWNER, JOE, manage(), true, &[1]));
    d.push(delegate(4, OWNER, DAN, manage(), true, &[2]));
    d.push(auth(GROUP, 5, JOE, add(JIM, manage()), &[3]));
    d.push(delegate(6, JOE, JIM, manage(), true, &[5]));
    d.push(auth(GROUP, 7, OWNER, add(QUINN, manage()), &[2]));
    d.push(delegate(8, JOE, QUINN, manage(), true, &[5]));
    let mut removal_after = vec![6, 4];
    if alt {
        d.push(delegate(16, DAN, JIM, manage(), true, &[4]));
        removal_after.push(16);
    }
    d.push(Event {
        id: 9,
        author: OWNER,
        after: removal_after,
        kind: Kind::Auth {
            group: GROUP,
            action: remove(JOE),
            successor,
        },
    });
    d.push(auth(GROUP, 10, JIM, add(ZED, manage()), &[9]));
    d.push(auth(GROUP, 11, JIM, add(YAN, manage()), &[6]));
    d.push(auth(GROUP, 12, QUINN, add(EVE, read()), &[7, 8]));
    d.push(auth(GROUP, 13, OWNER, add(KIM, write()), &[2]));
    d.push(delegate(14, KIM, LOU, manage(), true, &[13]));
    d.push(delegate(15, JOE, XAV, manage(), true, &[9]));
    d
}

pub fn run() -> usize {
    println!(
        "(e) delegation cascade: Alice -> Joe -> Jim; Joe removed; alt chain Alice -> Dan -> Jim"
    );
    let mut failures = 0;
    // (name, alt chain, successor, mode, Jim's later action 10 applies, Quinn's action 12 applies)
    let runs = [
        (
            "control: p2panda alone",
            false,
            None,
            Mode::Membership,
            true,
            true,
        ),
        (
            "cascade, only Joe's chain",
            false,
            None,
            Mode::Cascade,
            false,
            false,
        ),
        (
            "cascade, alt chain via Dan",
            true,
            None,
            Mode::Cascade,
            true,
            false,
        ),
        (
            "keep_prior, only Joe's chain",
            false,
            None,
            Mode::KeepPrior,
            true,
            false,
        ),
        (
            "transfer to Dan at removal",
            false,
            Some(DAN),
            Mode::Transfer,
            true,
            false,
        ),
    ];
    for (name, alt, successor, mode, jim_later_ok, quinn_ok) in runs {
        let dag = build(alt, successor);
        let (y, v) = Gov::ingest(&dag, mode);
        let got = members(&y, GROUP);
        let has = |id: char| got.iter().any(|(m, _)| *m == id);
        let verdict = |id: Id| *v.get(&id).expect("every event has a verdict");
        // The control runs no delegation check, so its delegations are only recorded as issued.
        let refusals =
            mode == Mode::Membership || (verdict(14) == "refused" && verdict(15) == "refused");
        let ok = verdict(10) == if jim_later_ok { "applied" } else { "vetoed" }
            && has(ZED) == jim_later_ok
            && verdict(11) == "applied"
            && has(YAN)
            && verdict(12) == if quinn_ok { "applied" } else { "vetoed" }
            && has(EVE) == quinn_ok
            && !has(JOE)
            && has(JIM)
            && refusals;
        let detail = format!(
            "10 {}, 11 {}, 12 {}; members {}",
            verdict(10),
            verdict(11),
            verdict(12),
            show(&got)
        );
        failures += report(name, ok, &detail);
    }

    // Structural checks on the fully ingested cascade run. Event 6 is a positive control.
    let dag = build(false, None);
    let (y, _) = Gov::ingest(&dag, Mode::Cascade);
    let gov = Gov {
        dag: &dag,
        mode: Mode::Cascade,
        y,
    };
    failures += report(
        "control: delegation 6 valid where made",
        gov.issued(6),
        "issued(6)",
    );
    let mismatches = incremental_mismatches(&dag, GROUP);
    failures += report(
        "incremental frontier equals the full walk, every event",
        mismatches == 0,
        &format!("{mismatches} mismatches"),
    );
    failures += report(
        "delegation 8 void: concurrent with R",
        gov.void_by_concurrent_removal(8),
        "8 ∥ 9",
    );
    failures += report(
        "delegation 6 not void: in R's past",
        !gov.void_by_concurrent_removal(6),
        "6 < 9",
    );
    failures += report(
        "attenuation: write cannot grant manage",
        !gov.issued(14),
        "14 refused",
    );
    failures += report("no grant after own removal", !gov.issued(15), "15 refused");
    failures
}
