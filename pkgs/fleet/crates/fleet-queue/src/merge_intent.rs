//! Merge-intent queue and local PR stack state (#8582, fleet
//! unification slice 4 of #8575).
//!
//! When GitHub writes are unavailable, finished work stacks locally:
//! each new branch is based on the previous branch's tip rather than
//! on main, and the intended merge order is recorded here, in the
//! coordination database — so the stack state (branches, order,
//! owners) is fully visible from the database alone while GitHub is
//! down. A replay pass walks a stack bottom-up: push the branch, open
//! the PR, merge it, rebase the next branch onto the new main.
//!
//! A conflict surfacing at replay belongs to the lane/session that
//! authored the change: [`route_conflict`] names that owner from the
//! recorded intent, and replay routes the conflict back instead of
//! resolving it anonymously.
//!
//! This module is the pure core: the table DDL of record, the row
//! shape, the ordering and state-transition rules, and conflict
//! routing. The broker's storage endpoints (increment 2, in
//! `turso_queue`) build on it; client verbs and the replay executor
//! follow in later increments. Nothing here does I/O.

/// DDL of record for the merge-intent table (fleet-queue database).
/// `position` is the recorded merge order inside one `stack_id`:
/// lower replays first (bottom of the stack). `base_branch` is the
/// branch this intent's branch was stacked on (`main` for the
/// bottom), which is what replay rebases onto the new main tip as it
/// walks up.
pub const MERGE_INTENT_DDL: &str = "\
CREATE TABLE IF NOT EXISTS merge_intents (
  id INTEGER PRIMARY KEY,
  stack_id TEXT NOT NULL,
  position INTEGER NOT NULL,
  repo TEXT NOT NULL,
  branch TEXT NOT NULL,
  base_branch TEXT NOT NULL,
  head_sha TEXT,
  base_head_sha TEXT,
  issue_number INTEGER,
  pr_number INTEGER,
  owner_lane TEXT NOT NULL,
  owner_worker TEXT NOT NULL,
  owner_session TEXT,
  state TEXT NOT NULL DEFAULT 'intended',
  conflict_detail TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  UNIQUE (stack_id, position),
  UNIQUE (repo, branch)
)";

/// Lifecycle of one merge intent.
///
/// - `intended`: recorded during the outage; not yet replayed.
/// - `replaying`: the replay pass is working this intent now.
/// - `landed`: pushed, PR opened and merged in recorded order.
/// - `conflicted`: replay hit a conflict; ownership is routed to the
///   authoring lane (see [`MergeIntent::conflict_route`]) and the
///   stack above it waits — replay never resolves another lane's
///   conflict anonymously.
/// - `abandoned`: the authoring lane withdrew the intent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IntentState {
    Intended,
    Replaying,
    Landed,
    Conflicted,
    Abandoned,
}

impl IntentState {
    pub fn as_str(&self) -> &'static str {
        match self {
            IntentState::Intended => "intended",
            IntentState::Replaying => "replaying",
            IntentState::Landed => "landed",
            IntentState::Conflicted => "conflicted",
            IntentState::Abandoned => "abandoned",
        }
    }

    pub fn parse(s: &str) -> Option<IntentState> {
        match s {
            "intended" => Some(IntentState::Intended),
            "replaying" => Some(IntentState::Replaying),
            "landed" => Some(IntentState::Landed),
            "conflicted" => Some(IntentState::Conflicted),
            "abandoned" => Some(IntentState::Abandoned),
            _ => None,
        }
    }

    /// The only legal moves. A conflicted intent returns to
    /// `replaying` when its owner has resolved the conflict and the
    /// replay pass picks the stack up again; terminal states
    /// (`landed`, `abandoned`) never move.
    pub fn can_transition(self, to: IntentState) -> bool {
        use IntentState::*;
        matches!(
            (self, to),
            (Intended, Replaying)
                | (Intended, Abandoned)
                | (Replaying, Landed)
                | (Replaying, Conflicted)
                | (Replaying, Abandoned)
                | (Conflicted, Replaying)
                | (Conflicted, Abandoned)
        )
    }
}

/// One recorded merge intent: a finished branch, its place in the
/// stack's merge order, and the lane/session that authored it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MergeIntent {
    pub stack_id: String,
    pub position: i64,
    pub repo: String,
    pub branch: String,
    pub base_branch: String,
    pub head_sha: Option<String>,
    /// The recorded head of the base branch's intent at the moment
    /// this branch was stacked (or last restacked) onto it: the
    /// base-tip linkage. None when the base is main or no head was
    /// recorded for the base. If the base intent's head later moves
    /// (a restack rebuilds it), this intent's linkage goes stale
    /// and the intent stays taint-blocked until it is itself
    /// restacked -- refreshing the base alone cannot establish that
    /// an unchanged descendant is clean (Codex P1, PR #8849,
    /// comment 4548246107).
    pub base_head_sha: Option<String>,
    pub issue_number: Option<i64>,
    pub pr_number: Option<i64>,
    pub owner_lane: String,
    pub owner_worker: String,
    pub owner_session: Option<String>,
    pub state: IntentState,
}

/// Where a replay conflict is routed: the authoring lane/session
/// recorded on the intent, with the context that owner needs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConflictRoute {
    pub owner_lane: String,
    pub owner_worker: String,
    pub owner_session: Option<String>,
    pub repo: String,
    pub branch: String,
    pub base_branch: String,
    pub issue_number: Option<i64>,
}

impl MergeIntent {
    /// The conflict routing for this intent: always its own author,
    /// never the replaying lane.
    pub fn conflict_route(&self) -> ConflictRoute {
        ConflictRoute {
            owner_lane: self.owner_lane.clone(),
            owner_worker: self.owner_worker.clone(),
            owner_session: self.owner_session.clone(),
            repo: self.repo.clone(),
            branch: self.branch.clone(),
            base_branch: self.base_branch.clone(),
            issue_number: self.issue_number,
        }
    }
}

/// The position a new intent takes in `stack_id`: one past the
/// current maximum (0 for an empty stack), so appends preserve the
/// order work was finished in.
pub fn next_position(existing: &[MergeIntent], stack_id: &str) -> i64 {
    existing
        .iter()
        .filter(|i| i.stack_id == stack_id)
        .map(|i| i.position)
        .max()
        .map_or(0, |max| max + 1)
}

/// The branch a new stack entry bases on: the current tip of the
/// stack (the highest-position non-abandoned intent's branch), or
/// `main` when the stack is empty — each new branch is based on the
/// previous branch's tip rather than on main.
pub fn next_base_branch(existing: &[MergeIntent], stack_id: &str) -> String {
    existing
        .iter()
        .filter(|i| i.stack_id == stack_id && i.state != IntentState::Abandoned)
        .max_by_key(|i| i.position)
        .map_or_else(|| "main".to_string(), |i| i.branch.clone())
}

/// A request to append one finished branch to a stack, as recorded
/// by the authoring lane during an outage (#8582 increment 2). The
/// broker's record endpoint builds this from the request body; the
/// append rules (position, base) are decided here, in the pure core,
/// so the storage layer cannot drift from the replay semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendRequest {
    pub stack_id: String,
    pub repo: String,
    pub branch: String,
    pub base_branch: Option<String>,
    pub head_sha: Option<String>,
    pub issue_number: Option<i64>,
    pub pr_number: Option<i64>,
    pub owner_lane: String,
    pub owner_worker: String,
    pub owner_session: Option<String>,
}

/// The fields an append must carry to be recorded: a stack, a repo,
/// a branch, and the authoring owner (lane + worker) a later
/// conflict will be routed to. An append without an owner is
/// rejected — recording it would strand its replay conflict with
/// nobody to route to.
pub fn validate_append(req: &AppendRequest) -> Result<(), String> {
    for (field, value) in [
        ("stack_id", &req.stack_id),
        ("repo", &req.repo),
        ("branch", &req.branch),
        ("owner_lane", &req.owner_lane),
        ("owner_worker", &req.owner_worker),
    ] {
        if value.trim().is_empty() {
            return Err(format!("merge-intent record requires a non-empty {field}"));
        }
    }
    if let Some(base) = &req.base_branch {
        if base.trim().is_empty() {
            return Err(
                "merge-intent record requires a non-empty base_branch when one is given"
                    .to_string(),
            );
        }
    }
    Ok(())
}

/// A stack lives in one repository: an append naming a different
/// repo than the rows already recorded under its `stack_id` is
/// rejected — otherwise [`plan_append`] would base the new branch
/// on a tip from another repository (CodeRabbit finding, PR #8660).
pub fn validate_append_against_stack(
    existing: &[MergeIntent],
    req: &AppendRequest,
) -> Result<(), String> {
    if let Some(other) = existing
        .iter()
        .find(|i| i.stack_id == req.stack_id && i.repo != req.repo)
    {
        return Err(format!(
            "merge-intent stack {} already records repo {}; refusing append for repo {}",
            req.stack_id, other.repo, req.repo
        ));
    }
    Ok(())
}

/// Plan the row an append inserts: position from [`next_position`],
/// base from the request's explicit base or [`next_base_branch`]
/// (the current tip, so each new branch stacks on the previous
/// one), state `intended`. The inferred base is the stack
/// construction contract — a branch recorded without an explicit
/// base asserts it was built on the stack tip; a caller whose
/// branch was built on a different base passes that real base
/// explicitly (see [`AppendRequest::base_branch`]).
pub fn plan_append(existing: &[MergeIntent], req: &AppendRequest) -> MergeIntent {
    let base_branch = req
        .base_branch
        .clone()
        .unwrap_or_else(|| next_base_branch(existing, &req.stack_id));
    // Base-tip linkage: record the base intent's head as of this
    // append, so a later rebuild of the base is detectable from
    // the recorded stack alone.
    let base_head_sha = existing
        .iter()
        .find(|i| i.stack_id == req.stack_id && i.branch == base_branch)
        .and_then(|base| base.head_sha.clone());
    MergeIntent {
        stack_id: req.stack_id.clone(),
        position: next_position(existing, &req.stack_id),
        repo: req.repo.clone(),
        branch: req.branch.clone(),
        base_branch,
        head_sha: req.head_sha.clone(),
        base_head_sha,
        issue_number: req.issue_number,
        pr_number: req.pr_number,
        owner_lane: req.owner_lane.clone(),
        owner_worker: req.owner_worker.clone(),
        owner_session: req.owner_session.clone(),
        state: IntentState::Intended,
    }
}

/// A request to re-record one intent's base after its author has
/// restacked the branch onto a new base (#8582; Codex P2, PR #8849,
/// comment 4547537340). Recording a branch again is impossible — the
/// append path INSERTs and `UNIQUE (repo, branch)` refuses the
/// second row — and the transition verb moves only lifecycle state,
/// so without this refresh a descendant restacked off a withdrawn
/// branch stayed blocked by its stale recorded ancestry forever:
/// [`abandoned_tainted_branches`] derives the taint from the
/// recorded `base_branch` chain, and nothing could update it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RestackRequest {
    pub stack_id: String,
    pub position: i64,
    pub new_base_branch: String,
    /// The restacked branch's new head, when the author recorded it:
    /// a restack rewrites the branch, so the recorded head moves
    /// with the base. `None` keeps the recorded head.
    pub new_head_sha: Option<String>,
}

/// Plan the metadata refresh a restack acknowledgement applies: the
/// intent at `req.position` in `req.stack_id`, with its recorded
/// base (and head, when given) replaced. The rules keep the refresh
/// from becoming a laundering path for withdrawn changes:
///
/// - terminal intents (`landed`, `abandoned`) are sealed — their
///   recorded base is history, never refreshed;
/// - the new base is `main`, or the branch of another intent in the
///   same stack that is neither abandoned nor itself tainted by an
///   abandoned ancestor (restacking onto a still-tainted branch
///   would keep the withdrawn commits in the ancestry);
/// - an intent cannot be based on its own branch.
///
/// Taint is derived from the recorded chain on every walk. This
/// refresh unblocks the restacked intent only: its descendants
/// keep the base-tip linkage they recorded when they were stacked
/// on this branch's OLD head, so they stay blocked until each is
/// itself restacked onto the new tip (Codex P1, PR #8849, comment
/// 4548246107). The refresh also re-records this intent's own
/// linkage to its new base's current head.
pub fn plan_restack(
    intents: &[MergeIntent],
    req: &RestackRequest,
) -> Result<MergeIntent, String> {
    if req.new_base_branch.trim().is_empty() {
        return Err("merge-intent restack requires a non-empty new base_branch".to_string());
    }
    if let Some(sha) = &req.new_head_sha {
        if sha.trim().is_empty() {
            return Err(
                "merge-intent restack requires a non-empty head_sha when one is given"
                    .to_string(),
            );
        }
    }
    let Some(intent) = intents
        .iter()
        .find(|i| i.stack_id == req.stack_id && i.position == req.position)
    else {
        return Err(format!(
            "merge-intent restack: no intent at stack {} position {}",
            req.stack_id, req.position
        ));
    };
    match intent.state {
        IntentState::Landed | IntentState::Abandoned => {
            return Err(format!(
                "merge-intent restack: intent at stack {} position {} is {} (terminal); \
                 its recorded base is sealed",
                req.stack_id,
                req.position,
                intent.state.as_str()
            ));
        }
        _ => {}
    }
    if req.new_base_branch == intent.branch {
        return Err(
            "merge-intent restack: an intent cannot be based on its own branch".to_string(),
        );
    }
    if req.new_base_branch != "main" {
        let Some(base) = intents
            .iter()
            .find(|i| i.stack_id == req.stack_id && i.branch == req.new_base_branch)
        else {
            return Err(format!(
                "merge-intent restack: new base {:?} is not \"main\" and no intent in \
                 stack {} records it",
                req.new_base_branch, req.stack_id
            ));
        };
        if base.position > intent.position {
            return Err(format!(
                "merge-intent restack: new base {:?} is recorded ABOVE the intent at                  stack {} position {} (base at position {}); restacking onto a                  descendant would cycle the stack -- restack onto main or a                  branch recorded below it",
                req.new_base_branch, req.stack_id, req.position, base.position
            ));
        }
        let tainted = abandoned_tainted_branches(intents, &req.stack_id);
        if base.state == IntentState::Abandoned || tainted.contains(base.branch.as_str()) {
            return Err(format!(
                "merge-intent restack: new base {:?} still carries an abandoned \
                 intent's changes; restack onto a live base or main",
                req.new_base_branch
            ));
        }
    }
    let mut updated = intent.clone();
    updated.base_branch = req.new_base_branch.clone();
    if let Some(sha) = &req.new_head_sha {
        updated.head_sha = Some(sha.clone());
    }
    updated.base_head_sha = if req.new_base_branch == "main" {
        None
    } else {
        intents
            .iter()
            .find(|i| i.stack_id == req.stack_id && i.branch == req.new_base_branch)
            .and_then(|base| base.head_sha.clone())
    };
    Ok(updated)
}

/// The replay walk for one stack: the `intended` intents in recorded
/// order, bottom first. Finished entries — `landed` (already merged
/// by an earlier replay pass) and `abandoned` (withdrawn) — are
/// excluded outright: they are done, and a landed bottom entry must
/// not stop a resumed walk (Codex P1, PR #8629). The walk then stops
/// at the first live non-intended entry — a `conflicted` or
/// `replaying` intent blocks everything stacked above it — so only
/// the intended prefix is returned.
/// Branches in `stack_id` that still carry an abandoned intent's
/// changes: the abandoned intents' own branches, plus -- transitively --
/// every branch stacked on one of them (its base_branch names a
/// tainted branch). An abandoned intent is its author withdrawing
/// the change; a descendant replayed without restacking would merge
/// the withdrawn commits along with its own (Codex P1, PR #8849), so
/// both walks stop before the first tainted intent: replay refuses
/// descendants that still contain abandoned changes until their
/// author restacks them off the withdrawn branch.
///
/// A restack acknowledgement for one intent does NOT clear its
/// descendants (Codex P1, PR #8849, comment 4548246107): rebuilding
/// branch B on main says nothing about branch C stacked on B's old
/// tip -- C's history still contains the original commits. So when
/// the stack holds an abandoned intent, an intent whose recorded
/// base-tip linkage (base_head_sha) no longer matches its base
/// intent's current head is tainted as well, until that intent is
/// itself restacked (which re-records its linkage). Stacks with
/// no abandoned intent are unaffected: a stale linkage there has
/// no withdrawn changes to launder.
fn abandoned_tainted_branches<'a>(
    intents: &'a [MergeIntent],
    stack_id: &str,
) -> std::collections::HashSet<&'a str> {
    let mut tainted: std::collections::HashSet<&'a str> = intents
        .iter()
        .filter(|i| i.stack_id == stack_id && i.state == IntentState::Abandoned)
        .map(|i| i.branch.as_str())
        .collect();
    let has_abandoned = !tainted.is_empty();
    loop {
        let mut grew = false;
        for i in intents.iter().filter(|i| i.stack_id == stack_id) {
            if tainted.contains(i.branch.as_str()) {
                continue;
            }
            if tainted.contains(i.base_branch.as_str()) {
                tainted.insert(i.branch.as_str());
                grew = true;
                continue;
            }
            // Stale base-tip linkage: this intent was stacked on a
            // tip of its base that the base no longer records --
            // the base was rebuilt (restacked) and this intent was
            // not. Its ancestry is unverified against the new base.
            if has_abandoned {
                if let Some(base) = intents
                    .iter()
                    .find(|b| b.stack_id == stack_id && b.branch == i.base_branch)
                {
                    if i.base_head_sha != base.head_sha {
                        tainted.insert(i.branch.as_str());
                        grew = true;
                    }
                }
            }
        }
        if !grew {
            break;
        }
    }
    tainted
}

pub fn replay_prefix<'a>(intents: &'a [MergeIntent], stack_id: &str) -> Vec<&'a MergeIntent> {
    let tainted = abandoned_tainted_branches(intents, stack_id);
    let mut ordered: Vec<&MergeIntent> = intents
        .iter()
        .filter(|i| {
            i.stack_id == stack_id
                && i.state != IntentState::Abandoned
                && i.state != IntentState::Landed
        })
        .collect();
    ordered.sort_by_key(|i| i.position);
    ordered
        .into_iter()
        .take_while(|i| i.state == IntentState::Intended && !tainted.contains(i.branch.as_str()))
        .collect()
}


/// One step of a replay pass, planned by [`replay_plan`] from the
/// recorded stack state alone (#8582 increment 4). The executor
/// (client `stack replay`) performs the steps in plan order; the
/// plan itself is pure, so a `--dry-run` prints exactly what a live
/// pass would do, and the drill (increment 5) can assert on it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReplayAction {
    /// Mark the intent `replaying` (intended -> replaying). An
    /// intent a previous interrupted pass already left `replaying`
    /// is resumed: the executor skips the transition for it.
    Begin,
    /// Rebase the intent's branch onto the current main tip. Every
    /// intent above the walk's bottom was stacked on the previous
    /// branch; once that branch lands, this one is rebased onto the
    /// new main before it is pushed. The step is planned under the
    /// REBASED intent's own position: a conflict here belongs to
    /// that intent's author, never to the landed intent below it.
    RebaseOntoMain,
    /// Push the branch (its first push, after the outage).
    Push,
    /// Open the PR against `main`, or reuse the recorded PR number.
    OpenPr,
    /// Merge the PR, in recorded order.
    Merge,
    /// Mark the intent `landed` (recording the PR number) and post
    /// its outcome note in the same pass.
    Land,
}

impl ReplayAction {
    pub fn as_str(&self) -> &'static str {
        match self {
            ReplayAction::Begin => "begin",
            ReplayAction::RebaseOntoMain => "rebase_onto_main",
            ReplayAction::Push => "push",
            ReplayAction::OpenPr => "open_pr",
            ReplayAction::Merge => "merge",
            ReplayAction::Land => "land",
        }
    }
}

/// One planned replay step: the intent it serves (by stack
/// position), that intent's branch, and the action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlannedStep {
    pub position: i64,
    pub branch: String,
    pub action: ReplayAction,
}

/// The replay plan for a walk (see [`replay_prefix`]): each intent,
/// bottom first, is begun, rebased onto main when it is not the
/// walk's bottom, pushed, PR-opened, merged, and landed — then the
/// next intent up the stack. Order is the contract: a step for a
/// higher position never precedes the `Land` step below it.
pub fn plan_for_walk(walk: &[&MergeIntent]) -> Vec<PlannedStep> {
    let mut steps = Vec::new();
    for (idx, intent) in walk.iter().enumerate() {
        let mut step = |action| {
            steps.push(PlannedStep {
                position: intent.position,
                branch: intent.branch.clone(),
                action,
            })
        };
        step(ReplayAction::Begin);
        if idx > 0 {
            step(ReplayAction::RebaseOntoMain);
        }
        step(ReplayAction::Push);
        step(ReplayAction::OpenPr);
        step(ReplayAction::Merge);
        step(ReplayAction::Land);
    }
    steps
}

/// The replay plan for a stack's recorded state: the plan for its
/// [`replay_prefix`] walk.
pub fn replay_plan(intents: &[MergeIntent], stack_id: &str) -> Vec<PlannedStep> {
    plan_for_walk(&replay_prefix(intents, stack_id))
}

/// The walk a resumed replay pass serves: like [`replay_prefix`],
/// except a head intent left `replaying` by an interrupted pass is
/// resumed (included, with the intended intents above it) instead
/// of blocking its own resume. A `conflicted` head still blocks:
/// only its authoring lane clears it, by resolving the conflict
/// and moving it back to `replaying`.
pub fn resumable_walk<'a>(intents: &'a [MergeIntent], stack_id: &str) -> Vec<&'a MergeIntent> {
    let mut ordered: Vec<&MergeIntent> = intents
        .iter()
        .filter(|i| {
            i.stack_id == stack_id
                && i.state != IntentState::Abandoned
                && i.state != IntentState::Landed
        })
        .collect();
    ordered.sort_by_key(|i| i.position);
    let tainted = abandoned_tainted_branches(intents, stack_id);
    let mut out = Vec::new();
    for (idx, i) in ordered.iter().enumerate() {
        if tainted.contains(i.branch.as_str()) {
            break;
        }
        match i.state {
            IntentState::Intended => out.push(*i),
            IntentState::Replaying if idx == 0 => out.push(*i),
            _ => break,
        }
    }
    out
}

/// Route a replay conflict at `position` in `stack_id` to the
/// authoring lane recorded on that intent — never to the lane
/// running the replay (#8582). `None` when no such intent exists.
pub fn route_replay_conflict(
    intents: &[MergeIntent],
    stack_id: &str,
    position: i64,
) -> Option<ConflictRoute> {
    intents
        .iter()
        .find(|i| i.stack_id == stack_id && i.position == position)
        .map(|i| i.conflict_route())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn intent(stack: &str, position: i64, branch: &str, state: IntentState) -> MergeIntent {
        MergeIntent {
            stack_id: stack.to_string(),
            position,
            repo: "developing-today/code".to_string(),
            branch: branch.to_string(),
            base_branch: "main".to_string(),
            head_sha: None,
            base_head_sha: None,
            issue_number: Some(8582),
            pr_number: None,
            owner_lane: "any-b".to_string(),
            owner_worker: "repo-worker-code".to_string(),
            owner_session: Some("ses_test".to_string()),
            state,
        }
    }

    #[test]
    fn positions_append_in_finish_order() {
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Intended),
            intent("s1", 1, "queue/b", IntentState::Intended),
            intent("s2", 0, "queue/c", IntentState::Intended),
        ];
        assert_eq!(next_position(&stack, "s1"), 2);
        assert_eq!(next_position(&stack, "s2"), 1);
        assert_eq!(next_position(&stack, "empty"), 0);
    }

    #[test]
    fn base_branch_is_previous_tip_or_main() {
        let mut stack = vec![intent("s1", 0, "queue/a", IntentState::Intended)];
        assert_eq!(next_base_branch(&stack, "s1"), "queue/a");
        assert_eq!(next_base_branch(&stack, "empty"), "main");
        stack.push(intent("s1", 1, "queue/b", IntentState::Abandoned));
        // An abandoned tip does not become the next base.
        assert_eq!(next_base_branch(&stack, "s1"), "queue/a");
    }

    #[test]
    fn replay_walks_the_intended_prefix_bottom_up() {
        let stack = vec![
            intent("s1", 1, "queue/b", IntentState::Intended),
            intent("s1", 0, "queue/a", IntentState::Intended),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/a", "queue/b", "queue/c"]);
    }

    #[test]
    fn a_conflicted_entry_blocks_the_stack_above_it() {
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Intended),
            intent("s1", 1, "queue/b", IntentState::Conflicted),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/a"]);
    }

    #[test]
    fn replay_resumes_past_landed_entries() {
        // The PR #8629 Codex P1 scenario: a first replay pass landed
        // the bottom entry; the resumed walk must serve what is
        // still intended above it, not stop at the landed row.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Landed),
            intent("s1", 1, "queue/b", IntentState::Intended),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/b", "queue/c"]);
        // Several landed, one intended left.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Landed),
            intent("s1", 1, "queue/b", IntentState::Landed),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/c"]);
    }

    #[test]
    fn a_conflicted_entry_above_landed_still_blocks() {
        // Landed entries are skipped, but a live conflicted entry
        // above them still blocks everything stacked over it.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Landed),
            intent("s1", 1, "queue/b", IntentState::Conflicted),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        assert!(replay_prefix(&stack, "s1").is_empty());
    }

    #[test]
    fn partial_replay_lifecycle_walks_forward() {
        // A full partial-replay lifecycle, driven through the legal
        // transition table: land the bottom, conflict on the next,
        // resolve it, land the rest. After every step the walk must
        // match the #8582 replay semantics (the test's oracle):
        // finished entries never block; the first live non-intended
        // entry blocks everything above it.
        use IntentState::*;
        let mut stack = vec![
            intent("s1", 0, "queue/a", Intended),
            intent("s1", 1, "queue/b", Intended),
            intent("s1", 2, "queue/c", Intended),
        ];
        fn walk(s: &[MergeIntent]) -> Vec<&str> {
            replay_prefix(s, "s1")
                .iter()
                .map(|i| i.branch.as_str())
                .collect()
        }
        let mut advance = |s: &mut [MergeIntent], pos: usize, to: IntentState| {
            assert!(
                s[pos].state.can_transition(to),
                "{:?} -> {to:?}",
                s[pos].state
            );
            s[pos].state = to;
        };
        assert_eq!(walk(&stack), vec!["queue/a", "queue/b", "queue/c"]);
        advance(&mut stack, 0, Replaying);
        assert_eq!(walk(&stack), Vec::<&str>::new());
        advance(&mut stack, 0, Landed);
        assert_eq!(walk(&stack), vec!["queue/b", "queue/c"]);
        advance(&mut stack, 1, Replaying);
        advance(&mut stack, 1, Conflicted);
        assert_eq!(walk(&stack), Vec::<&str>::new());
        // The owner resolves; replay picks the entry up again.
        advance(&mut stack, 1, Replaying);
        advance(&mut stack, 1, Landed);
        assert_eq!(walk(&stack), vec!["queue/c"]);
        advance(&mut stack, 2, Replaying);
        advance(&mut stack, 2, Landed);
        assert_eq!(walk(&stack), Vec::<&str>::new());
    }

    #[test]
    fn conflict_routes_to_the_authoring_lane() {
        let i = intent("s1", 0, "queue/a", IntentState::Conflicted);
        let route = i.conflict_route();
        assert_eq!(route.owner_lane, "any-b");
        assert_eq!(route.owner_worker, "repo-worker-code");
        assert_eq!(route.owner_session.as_deref(), Some("ses_test"));
        assert_eq!(route.branch, "queue/a");
        assert_eq!(route.base_branch, "main");
    }

    #[test]
    fn state_machine_rejects_illegal_moves() {
        use IntentState::*;
        assert!(Intended.can_transition(Replaying));
        assert!(Replaying.can_transition(Landed));
        assert!(Replaying.can_transition(Conflicted));
        assert!(Conflicted.can_transition(Replaying));
        assert!(!Intended.can_transition(Landed));
        assert!(!Landed.can_transition(Replaying));
        assert!(!Abandoned.can_transition(Replaying));
        assert_eq!(IntentState::parse("conflicted"), Some(Conflicted));
        assert_eq!(IntentState::parse("bogus"), None);
    }

    use proptest::prelude::*;

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Model-based property (oracle: the #8582 replay semantics,
        /// stated independently of the implementation): for ANY
        /// combination of lifecycle states in a stack, the replay
        /// walk is exactly the branches obtained by scanning in
        /// recorded order, skipping finished (landed/abandoned)
        /// entries, collecting intended entries, and stopping at
        /// the first live non-intended entry.
        #[test]
        fn replay_prefix_matches_the_model_for_any_state_mix(
            states in prop::collection::vec(0u8..5, 0..8),
        ) {
            let to_state = |c: u8| match c {
                0 => IntentState::Intended,
                1 => IntentState::Replaying,
                2 => IntentState::Landed,
                3 => IntentState::Conflicted,
                _ => IntentState::Abandoned,
            };
            let stack: Vec<MergeIntent> = states
                .iter()
                .enumerate()
                .map(|(idx, c)| {
                    intent("s1", idx as i64, &format!("queue/{idx}"), to_state(*c))
                })
                .collect();
            // The model, written from the semantics, not the code.
            let mut expected: Vec<String> = Vec::new();
            for i in &stack {
                match i.state {
                    IntentState::Landed | IntentState::Abandoned => continue,
                    IntentState::Intended => expected.push(i.branch.clone()),
                    IntentState::Replaying | IntentState::Conflicted => break,
                }
            }
            let walk: Vec<String> = replay_prefix(&stack, "s1")
                .iter()
                .map(|i| i.branch.clone())
                .collect();
            prop_assert_eq!(walk, expected);
        }
    }

    fn append_req(stack: &str, branch: &str) -> AppendRequest {
        AppendRequest {
            stack_id: stack.to_string(),
            repo: "developing-today/code".to_string(),
            branch: branch.to_string(),
            base_branch: None,
            head_sha: None,
            issue_number: Some(8582),
            pr_number: None,
            owner_lane: "code-c".to_string(),
            owner_worker: "repo-worker-code".to_string(),
            owner_session: None,
        }
    }

    #[test]
    fn plan_append_stacks_on_the_tip_in_finish_order() {
        // Oracle: the #8582 stacking rule stated in the issue — each
        // new branch is based on the previous branch's tip, and the
        // recorded position is the order work finished in.
        let first = plan_append(&[], &append_req("s1", "queue/a"));
        assert_eq!(first.position, 0);
        assert_eq!(first.base_branch, "main");
        assert_eq!(first.state, IntentState::Intended);
        let second = plan_append(&[first], &append_req("s1", "queue/b"));
        assert_eq!(second.position, 1);
        assert_eq!(second.base_branch, "queue/a");
        // A different stack is independent: it starts at main again.
        let other = plan_append(&[second.clone()], &append_req("s2", "queue/c"));
        assert_eq!(other.position, 0);
        assert_eq!(other.base_branch, "main");
        // An abandoned tip never becomes the next base.
        let mut dead = second;
        dead.state = IntentState::Abandoned;
        let third = plan_append(&[dead], &append_req("s1", "queue/d"));
        assert_eq!(third.base_branch, "main");
    }

    #[test]
    fn plan_append_honors_an_explicit_base() {
        let mut req = append_req("s1", "queue/b");
        req.base_branch = Some("release/x".to_string());
        let planned = plan_append(&[intent("s1", 0, "queue/a", IntentState::Intended)], &req);
        assert_eq!(planned.base_branch, "release/x");
        assert_eq!(planned.position, 1);
    }

    #[test]
    fn validate_append_requires_stack_repo_branch_and_owner() {
        assert!(validate_append(&append_req("s1", "queue/a")).is_ok());
        for blank in ["stack_id", "repo", "branch", "owner_lane", "owner_worker"] {
            let mut req = append_req("s1", "queue/a");
            match blank {
                "stack_id" => req.stack_id = " ".to_string(),
                "repo" => req.repo = String::new(),
                "branch" => req.branch = String::new(),
                "owner_lane" => req.owner_lane = String::new(),
                _ => req.owner_worker = String::new(),
            }
            let err = validate_append(&req).expect_err("blank field must be rejected");
            assert!(err.contains(blank), "{err}");
        }
    }

    #[test]
    fn validate_append_rejects_a_blank_explicit_base() {
        // Oracle: a recorded base must be a branch replay can rebase
        // onto; an explicit blank base is not one (CodeRabbit, PR #8660).
        let mut req = append_req("s1", "queue/a");
        req.base_branch = Some("  ".to_string());
        let err = validate_append(&req).expect_err("blank explicit base must be rejected");
        assert!(err.contains("base_branch"), "{err}");
    }

    #[test]
    fn validate_append_against_stack_rejects_a_repo_mismatch() {
        // Oracle: one stack, one repository — a tip from another
        // repo can never be this stack's base (CodeRabbit, PR #8660).
        let existing = vec![intent("s1", 0, "queue/a", IntentState::Intended)];
        assert!(validate_append_against_stack(&existing, &append_req("s1", "queue/b")).is_ok());
        assert!(validate_append_against_stack(&existing, &append_req("s2", "queue/b")).is_ok());
        let mut foreign = append_req("s1", "queue/b");
        foreign.repo = "developing-today-forks/opencode".to_string();
        let err = validate_append_against_stack(&existing, &foreign)
            .expect_err("cross-repo append must be rejected");
        assert!(err.contains("already records repo"), "{err}");
    }


    #[test]
    fn replay_plan_walks_bottom_up_with_rebase_only_above_the_bottom() {
        // Oracle: the module doc's replay walk — push, PR, merge,
        // rebase the next branch onto the new main — stated as a
        // per-intent step order (begin/land are the state marks at
        // the ends of each intent's turn).
        let stack = vec![
            intent("s1", 1, "queue/b", IntentState::Intended),
            intent("s1", 0, "queue/a", IntentState::Intended),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let plan: Vec<(i64, ReplayAction)> = replay_plan(&stack, "s1")
            .iter()
            .map(|s| (s.position, s.action))
            .collect();
        use ReplayAction::*;
        assert_eq!(
            plan,
            vec![
                (0, Begin),
                (0, Push),
                (0, OpenPr),
                (0, Merge),
                (0, Land),
                (1, Begin),
                (1, RebaseOntoMain),
                (1, Push),
                (1, OpenPr),
                (1, Merge),
                (1, Land),
                (2, Begin),
                (2, RebaseOntoMain),
                (2, Push),
                (2, OpenPr),
                (2, Merge),
                (2, Land),
            ]
        );
    }

    #[test]
    fn replay_plan_stops_at_the_conflicted_prefix_boundary() {
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Intended),
            intent("s1", 1, "queue/b", IntentState::Conflicted),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let plan = replay_plan(&stack, "s1");
        assert!(plan.iter().all(|s| s.position == 0), "{plan:?}");
        assert_eq!(plan.len(), 5);
        // A conflicted head means no plan at all.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Conflicted),
            intent("s1", 1, "queue/b", IntentState::Intended),
        ];
        assert!(replay_plan(&stack, "s1").is_empty());
        assert!(replay_plan(&stack, "unknown-stack").is_empty());
    }

    #[test]
    fn resumable_walk_resumes_a_replaying_head_only() {
        // An interrupted pass left the head replaying: resume serves
        // it and the intended intents above it.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Replaying),
            intent("s1", 1, "queue/b", IntentState::Intended),
        ];
        let walk: Vec<&str> = resumable_walk(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/a", "queue/b"]);
        // A replaying intent that is NOT the head still blocks the
        // intended intent below it is served, nothing above moves.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Intended),
            intent("s1", 1, "queue/b", IntentState::Replaying),
            intent("s1", 2, "queue/c", IntentState::Intended),
        ];
        let walk: Vec<&str> = resumable_walk(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/a"]);
        // A conflicted head blocks its own resume too.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Conflicted),
            intent("s1", 1, "queue/b", IntentState::Intended),
        ];
        assert!(resumable_walk(&stack, "s1").is_empty());
    }

    fn stacked(
        stack: &str,
        position: i64,
        branch: &str,
        base: &str,
        state: IntentState,
    ) -> MergeIntent {
        let mut i = intent(stack, position, branch, state);
        i.base_branch = base.to_string();
        i
    }

    #[test]
    fn replay_refuses_descendants_of_an_abandoned_intent() {
        // Oracle: `abandoned` is the author withdrawing the change
        // (#8582 lifecycle). A branch stacked on the withdrawn branch
        // still contains its commits; replaying it would merge the
        // withdrawn changes, so the walk stops before it — and before
        // everything stacked on it, transitively (Codex P1, PR #8849).
        let stack = vec![
            stacked("s1", 0, "queue/a", "main", IntentState::Abandoned),
            stacked("s1", 1, "queue/b", "queue/a", IntentState::Intended),
            stacked("s1", 2, "queue/c", "queue/b", IntentState::Intended),
        ];
        assert!(replay_prefix(&stack, "s1").is_empty());
        assert!(resumable_walk(&stack, "s1").is_empty());
        assert!(replay_plan(&stack, "s1").is_empty());
    }

    #[test]
    fn replay_still_walks_the_clean_prefix_below_an_abandoned_tip() {
        // An abandoned entry higher in the stack does not taint the
        // intents below it: their bases never contained its changes.
        let stack = vec![
            stacked("s1", 0, "queue/a", "main", IntentState::Intended),
            stacked("s1", 1, "queue/b", "queue/a", IntentState::Abandoned),
        ];
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/a"]);
    }

    #[test]
    fn replay_conflict_routes_to_the_recording_author() {
        // The intent() fixture records owner any-b / repo-worker-code
        // / ses_test: the route names that author, at the position
        // the conflict surfaced, with the branch context to resolve.
        let stack = vec![
            intent("s1", 0, "queue/a", IntentState::Landed),
            intent("s1", 1, "queue/b", IntentState::Conflicted),
        ];
        let route = route_replay_conflict(&stack, "s1", 1).expect("intent exists");
        assert_eq!(route.owner_lane, "any-b");
        assert_eq!(route.owner_worker, "repo-worker-code");
        assert_eq!(route.branch, "queue/b");
        assert_eq!(route.issue_number, Some(8582));
        assert!(route_replay_conflict(&stack, "s1", 9).is_none());
        assert!(route_replay_conflict(&stack, "other", 1).is_none());
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(256))]

        /// Model-based property (oracle: the #8582 replay semantics,
        /// stated independently of the planner): for ANY mix of
        /// lifecycle states, the plan serves exactly the walk the
        /// model derives (skip finished, collect intended, stop at
        /// the first live non-intended), and each served intent's
        /// steps are begin, [rebase if not the walk's bottom], push,
        /// open-pr, merge, land, in that order, grouped by ascending
        /// position with no interleaving.
        #[test]
        fn replay_plan_matches_the_model_for_any_state_mix(
            states in prop::collection::vec(0u8..5, 0..6),
        ) {
            let to_state = |c: u8| match c {
                0 => IntentState::Intended,
                1 => IntentState::Replaying,
                2 => IntentState::Landed,
                3 => IntentState::Conflicted,
                _ => IntentState::Abandoned,
            };
            let stack: Vec<MergeIntent> = states
                .iter()
                .enumerate()
                .map(|(idx, c)| {
                    intent("s1", idx as i64, &format!("queue/{idx}"), to_state(*c))
                })
                .collect();
            // The model walk, from the semantics.
            let mut walk_positions: Vec<i64> = Vec::new();
            for i in &stack {
                match i.state {
                    IntentState::Landed | IntentState::Abandoned => continue,
                    IntentState::Intended => walk_positions.push(i.position),
                    IntentState::Replaying | IntentState::Conflicted => break,
                }
            }
            let mut expected: Vec<(i64, ReplayAction)> = Vec::new();
            for (idx, pos) in walk_positions.iter().enumerate() {
                expected.push((*pos, ReplayAction::Begin));
                if idx > 0 {
                    expected.push((*pos, ReplayAction::RebaseOntoMain));
                }
                expected.push((*pos, ReplayAction::Push));
                expected.push((*pos, ReplayAction::OpenPr));
                expected.push((*pos, ReplayAction::Merge));
                expected.push((*pos, ReplayAction::Land));
            }
            let plan: Vec<(i64, ReplayAction)> = replay_plan(&stack, "s1")
                .iter()
                .map(|s| (s.position, s.action))
                .collect();
            prop_assert_eq!(plan, expected);
        }
    }

    #[test]
    fn restack_onto_a_branch_recorded_above_is_refused() {
        // Oracle: a restack base recorded ABOVE the intent is its
        // descendant; basing an intent on its own descendant cycles
        // the recorded stack (CodeRabbit, PR #8849, comment
        // 4548301698). plan_restack refuses it; main and bases
        // recorded below remain legal.
        let stack = vec![
            stacked("s1", 0, "queue/a", "main", IntentState::Intended),
            stacked("s1", 1, "queue/b", "queue/a", IntentState::Intended),
            stacked("s1", 2, "queue/c", "queue/b", IntentState::Intended),
        ];
        let onto_c = RestackRequest {
            stack_id: "s1".to_string(),
            position: 1,
            new_base_branch: "queue/c".to_string(),
            new_head_sha: None,
        };
        assert!(plan_restack(&stack, &onto_c).is_err());
        let onto_a = RestackRequest {
            new_base_branch: "queue/a".to_string(),
            ..onto_c.clone()
        };
        assert!(plan_restack(&stack, &onto_a).is_ok());
    }

    #[test]
    fn restack_acknowledgement_unblocks_a_restacked_descendant() {
        // Oracle: the documented recovery for a descendant of an
        // abandoned intent is its author restacking it off the
        // withdrawn branch (#8582). Recording that restack
        // (plan_restack) must unblock the descendant and everything
        // stacked on it, because the taint is derived from the
        // recorded base chain (Codex P2, PR #8849).
        let stack = vec![
            stacked("s1", 0, "queue/a", "main", IntentState::Abandoned),
            stacked("s1", 1, "queue/b", "queue/a", IntentState::Intended),
            stacked("s1", 2, "queue/c", "queue/b", IntentState::Intended),
        ];
        assert!(replay_prefix(&stack, "s1").is_empty(), "tainted stack is blocked");
        // Restacking onto a still-tainted (or the abandoned) base
        // is refused: the withdrawn commits would remain.
        let onto_a = RestackRequest {
            stack_id: "s1".to_string(),
            position: 1,
            new_base_branch: "queue/a".to_string(),
            new_head_sha: None,
        };
        assert!(plan_restack(&stack, &onto_a).is_err());
        // Restacking onto an unrecorded base is refused.
        let onto_nowhere = RestackRequest {
            new_base_branch: "release/x".to_string(),
            ..onto_a.clone()
        };
        assert!(plan_restack(&stack, &onto_nowhere).is_err());
        // The real restack: queue/b rebuilt on main, new head noted.
        let req = RestackRequest {
            stack_id: "s1".to_string(),
            position: 1,
            new_base_branch: "main".to_string(),
            new_head_sha: Some("deadbeef".to_string()),
        };
        let updated = plan_restack(&stack, &req).expect("restack onto main is legal");
        assert_eq!(updated.base_branch, "main");
        assert_eq!(updated.head_sha.as_deref(), Some("deadbeef"));
        assert_eq!(updated.state, IntentState::Intended);
        let stack: Vec<MergeIntent> = stack
            .into_iter()
            .map(|i| if i.position == 1 { updated.clone() } else { i })
            .collect();
        // Only the restacked intent unblocks (Codex P1, PR #8849,
        // comment 4548246107): queue/c was stacked on queue/b's old
        // tip and was NOT rebuilt, so its recorded base-tip linkage
        // (None) no longer matches queue/b's new head (deadbeef) --
        // its history still contains the original queue/a commits
        // and it stays blocked until its own author restacks it.
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(walk, vec!["queue/b"], "unchanged descendants stay blocked");
        // Restacking queue/c onto the rebuilt queue/b re-records
        // its linkage to the new tip and unblocks it.
        let c_req = RestackRequest {
            stack_id: "s1".to_string(),
            position: 2,
            new_base_branch: "queue/b".to_string(),
            new_head_sha: Some("cafef00d".to_string()),
        };
        let c_updated = plan_restack(&stack, &c_req).expect("restack onto rebuilt base is legal");
        assert_eq!(c_updated.base_head_sha.as_deref(), Some("deadbeef"));
        let stack: Vec<MergeIntent> = stack
            .into_iter()
            .map(|i| {
                if i.position == 2 {
                    c_updated.clone()
                } else {
                    i
                }
            })
            .collect();
        let walk: Vec<&str> = replay_prefix(&stack, "s1")
            .iter()
            .map(|i| i.branch.as_str())
            .collect();
        assert_eq!(
            walk,
            vec!["queue/b", "queue/c"],
            "restacked descendant unblocks"
        );
        // Terminal intents are sealed against a refresh.
        let sealed = RestackRequest {
            stack_id: "s1".to_string(),
            position: 0,
            new_base_branch: "main".to_string(),
            new_head_sha: None,
        };
        assert!(plan_restack(&stack, &sealed).is_err(), "abandoned is terminal");
        // An intent cannot be based on itself.
        let self_based = RestackRequest {
            position: 1,
            new_base_branch: "queue/b".to_string(),
            ..sealed
        };
        assert!(plan_restack(&stack, &self_based).is_err());
    }

    #[test]
    fn ddl_names_the_table_and_order_key() {
        assert!(MERGE_INTENT_DDL.contains("CREATE TABLE IF NOT EXISTS merge_intents"));
        assert!(MERGE_INTENT_DDL.contains("UNIQUE (stack_id, position)"));
    }
}
