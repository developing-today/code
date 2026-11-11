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
//! routing. Broker endpoints, client verbs, and the replay executor
//! build on it in follow-on increments; nothing here does I/O.

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

/// The replay walk for one stack: the `intended` intents in recorded
/// order, bottom first. Finished entries — `landed` (already merged
/// by an earlier replay pass) and `abandoned` (withdrawn) — are
/// excluded outright: they are done, and a landed bottom entry must
/// not stop a resumed walk (Codex P1, PR #8629). The walk then stops
/// at the first live non-intended entry — a `conflicted` or
/// `replaying` intent blocks everything stacked above it — so only
/// the intended prefix is returned.
pub fn replay_prefix<'a>(intents: &'a [MergeIntent], stack_id: &str) -> Vec<&'a MergeIntent> {
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
        .take_while(|i| i.state == IntentState::Intended)
        .collect()
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

    #[test]
    fn ddl_names_the_table_and_order_key() {
        assert!(MERGE_INTENT_DDL.contains("CREATE TABLE IF NOT EXISTS merge_intents"));
        assert!(MERGE_INTENT_DDL.contains("UNIQUE (stack_id, position)"));
    }
}
