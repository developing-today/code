//! Shared fixtures. An event DAG where `after` names every event kind, and the mapping from auth
//! events to p2panda-auth operations. p2panda-auth's `test_utils` types (char members, u32 ids)
//! stand in for VerifyingKey and Hash; the resolver is generic over both.

use std::collections::{HashMap, HashSet};

use p2panda_auth::Access;
use p2panda_auth::group::{GroupAction, GroupMember};
use p2panda_auth::test_utils::{TestGroup, TestGroupState, TestOperation};

pub const ALICE: char = 'A';
pub const BOB: char = 'B';
pub const CLAIRE: char = 'C';
pub const DAVE: char = 'D';
pub const EVE: char = 'E';
pub const GROUP: char = '1';

pub type Id = u32;

#[derive(Clone, Debug)]
pub enum Kind {
    /// A p2panda-auth operation. `successor` is who re-issues the removed member's delegations.
    Auth {
        group: char,
        action: GroupAction<char, ()>,
        successor: Option<char>,
    },
    /// The author grants `scope` to `grantee`, in the spike's single group.
    Delegate {
        grantee: char,
        scope: Access<()>,
        redelegate: bool,
    },
    /// Any other event (a document, a tag, a friend request). Exists only in the causal graph.
    Other,
}

#[derive(Clone, Debug)]
pub struct Event {
    pub id: Id,
    pub author: char,
    pub after: Vec<Id>,
    pub kind: Kind,
}

#[derive(Clone, Default)]
pub struct Dag {
    pub events: Vec<Event>,
    index: HashMap<Id, usize>,
}

impl Dag {
    pub fn push(&mut self, event: Event) {
        for dep in &event.after {
            assert!(
                self.index.contains_key(dep),
                "event {} names unknown {dep}",
                event.id
            );
        }
        self.index.insert(event.id, self.events.len());
        self.events.push(event);
    }

    pub fn get(&self, id: Id) -> &Event {
        &self.events[self.index[&id]]
    }

    /// Every event reachable through `after` from `id`, excluding `id`.
    pub fn past(&self, id: Id) -> HashSet<Id> {
        let mut seen = HashSet::new();
        let mut stack = self.get(id).after.clone();
        while let Some(next) = stack.pop() {
            if seen.insert(next) {
                stack.extend(self.get(next).after.iter().copied());
            }
        }
        seen
    }

    fn is_auth(&self, id: Id, group: char) -> bool {
        matches!(&self.get(id).kind, Kind::Auth { group: g, .. } if *g == group)
    }

    /// Auth frontier of `id` for `group`: the maximal auth operations of that group in the full
    /// causal past of `id`. The walk goes through non-auth events as well.
    pub fn frontier(&self, id: Id, group: char) -> Vec<Id> {
        let auth: Vec<Id> = self
            .past(id)
            .into_iter()
            .filter(|x| self.is_auth(*x, group))
            .collect();
        let pasts: HashMap<Id, HashSet<Id>> = auth.iter().map(|a| (*a, self.past(*a))).collect();
        let mut heads: Vec<Id> = auth
            .iter()
            .copied()
            .filter(|a| !auth.iter().any(|b| b != a && pasts[b].contains(a)))
            .collect();
        heads.sort();
        heads
    }

    /// Incremental auth frontier. An auth `after` event dominates its own past. A non-auth one
    /// contributes its frontier. Dominance is tested through the stored frontiers of auth events,
    /// so no full past is walked. `memo[e]` is the frontier of e's past.
    pub fn frontier_incremental(
        &self,
        id: Id,
        group: char,
        memo: &mut HashMap<Id, Vec<Id>>,
    ) -> Vec<Id> {
        if let Some(h) = memo.get(&id) {
            return h.clone();
        }
        let mut cand = Vec::new();
        for p in self.get(id).after.clone() {
            if self.is_auth(p, group) {
                self.frontier_incremental(p, group, memo);
                cand.push(p);
            } else {
                cand.extend(self.frontier_incremental(p, group, memo));
            }
        }
        cand.sort();
        cand.dedup();
        let mut keep: Vec<Id> = cand
            .iter()
            .copied()
            .filter(|c| !cand.iter().any(|b| b != c && reaches(*c, *b, memo)))
            .collect();
        keep.sort();
        memo.insert(id, keep.clone());
        keep
    }

    /// The naive mapping: auth operations named directly in `after`, everything else dropped.
    pub fn direct_auth_after(&self, id: Id, group: char) -> Vec<Id> {
        self.get(id)
            .after
            .iter()
            .copied()
            .filter(|x| self.is_auth(*x, group))
            .collect()
    }

    /// The p2panda-auth operation for auth event `id` with explicit dependencies.
    pub fn op_with(&self, id: Id, dependencies: Vec<Id>) -> TestOperation {
        let event = self.get(id);
        let Kind::Auth { group, action, .. } = &event.kind else {
            panic!("event {id} is not an auth event");
        };
        TestOperation {
            id,
            author: event.author,
            dependencies,
            group_id: *group,
            action: action.clone(),
        }
    }

    /// The operation for auth event `id`, with its auth-frontier dependencies.
    pub fn op(&self, id: Id) -> TestOperation {
        let Kind::Auth { group, .. } = &self.get(id).kind else {
            panic!("event {id} is not an auth event");
        };
        self.op_with(id, self.frontier(id, *group))
    }

    /// Every auth event as an operation, with the auth-frontier dependencies, in DAG order.
    pub fn ops(&self) -> Vec<TestOperation> {
        self.events
            .iter()
            .filter(|e| matches!(e.kind, Kind::Auth { .. }))
            .map(|e| self.op(e.id))
            .collect()
    }
}

/// Every causal-delivery order of `ops` (each op after its dependencies), up to `cap`.
pub fn orders(ops: &[TestOperation], cap: usize) -> Vec<Vec<usize>> {
    fn walk(ops: &[TestOperation], placed: &mut Vec<usize>, out: &mut Vec<Vec<usize>>, cap: usize) {
        if out.len() >= cap {
            return;
        }
        if placed.len() == ops.len() {
            out.push(placed.clone());
            return;
        }
        for (i, op) in ops.iter().enumerate() {
            let ready = op
                .dependencies
                .iter()
                .all(|d| placed.iter().any(|p| ops[*p].id == *d));
            if ready && !placed.contains(&i) {
                placed.push(i);
                walk(ops, placed, out, cap);
                placed.pop();
            }
        }
    }
    let mut out = Vec::new();
    walk(ops, &mut Vec::new(), &mut out, cap);
    out
}

/// Whether auth op `a` lies in the causal past of auth op `b`, read from stored frontiers.
fn reaches(a: Id, b: Id, memo: &HashMap<Id, Vec<Id>>) -> bool {
    let past_b = &memo[&b];
    past_b.contains(&a) || past_b.iter().any(|c| reaches(a, *c, memo))
}

/// Events whose incremental frontier differs from the full-walk frontier. Zero means they agree.
pub fn incremental_mismatches(dag: &Dag, group: char) -> usize {
    let mut memo = HashMap::new();
    dag.events
        .iter()
        .filter(|e| {
            let mut fast = dag.frontier_incremental(e.id, group, &mut memo);
            fast.sort();
            fast != dag.frontier(e.id, group)
        })
        .count()
}

/// Applies `ops` in `order` through `GroupCrdt::process`.
pub fn apply(ops: &[TestOperation], order: &[usize]) -> Result<TestGroupState, String> {
    let mut y = TestGroup::init();
    for i in order {
        y = TestGroup::process(y, &ops[*i]).map_err(|e| format!("{e:?}"))?;
    }
    Ok(y)
}

/// Active members of `group` with their access levels, sorted by id.
pub fn members(y: &TestGroupState, group: char) -> Vec<(char, Access<()>)> {
    let mut out = y.members(group);
    out.sort_by_key(|(id, _)| *id);
    out
}

pub fn show(list: &[(char, Access<()>)]) -> String {
    if list.is_empty() {
        return "(none)".to_string();
    }
    list.iter()
        .map(|(id, a)| format!("{id}:{a}"))
        .collect::<Vec<_>>()
        .join(" ")
}

pub fn manage() -> Access<()> {
    Access::manage()
}

pub fn write() -> Access<()> {
    Access::write()
}

pub fn read() -> Access<()> {
    Access::read()
}

pub fn create(initial: &[(char, Access<()>)]) -> GroupAction<char, ()> {
    GroupAction::Create {
        initial_members: initial
            .iter()
            .map(|(id, a)| (GroupMember::Individual(*id), a.clone()))
            .collect(),
    }
}

pub fn add(member: char, access: Access<()>) -> GroupAction<char, ()> {
    GroupAction::Add {
        member: GroupMember::Individual(member),
        access,
    }
}

pub fn remove(member: char) -> GroupAction<char, ()> {
    GroupAction::Remove {
        member: GroupMember::Individual(member),
    }
}

/// A valid no-op: promoting a member who already has Manage changes nothing (p2panda-auth
/// `state::promote` returns the state unchanged), and Read adds no delegation edge to the
/// authority graph. Used where our layer voids an op at ingest (friendship cap, delegation veto).
pub fn noop(creator: char) -> GroupAction<char, ()> {
    GroupAction::Promote {
        member: GroupMember::Individual(creator),
        access: read(),
    }
}

pub fn auth(
    group: char,
    id: Id,
    author: char,
    action: GroupAction<char, ()>,
    after: &[Id],
) -> Event {
    Event {
        id,
        author,
        after: after.to_vec(),
        kind: Kind::Auth {
            group,
            action,
            successor: None,
        },
    }
}

pub fn delegate(
    id: Id,
    grantor: char,
    grantee: char,
    scope: Access<()>,
    redelegate: bool,
    after: &[Id],
) -> Event {
    Event {
        id,
        author: grantor,
        after: after.to_vec(),
        kind: Kind::Delegate {
            grantee,
            scope,
            redelegate,
        },
    }
}

pub fn other(id: Id, author: char, after: &[Id]) -> Event {
    Event {
        id,
        author,
        after: after.to_vec(),
        kind: Kind::Other,
    }
}

/// One line per check. Returns 1 when the check fails, so callers can sum failures.
pub fn report(name: &str, ok: bool, detail: &str) -> usize {
    println!("  [{}] {name}: {detail}", if ok { "PASS" } else { "FAIL" });
    usize::from(!ok)
}
