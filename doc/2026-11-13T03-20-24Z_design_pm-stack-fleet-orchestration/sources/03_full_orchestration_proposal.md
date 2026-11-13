# Fleet Work-Item Orchestration — Agent Queue Workflows, GitHub Integration, Context Hydration, and Lifecycle Management (full supporting draft)

> Source: Google Doc `15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA` — https://docs.google.com/document/d/15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA/edit
> Drive dates: created 2026-11-06T14:51:14Z, modified 2026-11-06T14:51:23Z. Exported (plain text via Google Docs API) 2026-11-13T03:16Z for the PM Stack & Fleet orchestration design-mill intake.
> Role: The full supporting work-item orchestration and priority draft (Drive title: "Untitled document"; internal title as shown).

---

# Fleet Work-Item Orchestration
## Agent Queue Workflows, GitHub Integration, Context Hydration, and Lifecycle Management

**Status:** Initial architecture and implementation proposal  
**Target:** Existing `fleet` binary and its supporting services  
**Purpose:** Establish a unified operational interface for autonomous agents performing queue-driven work.

> This document describes proposed capabilities and command interfaces, not necessarily functionality already implemented. Before making changes, inspect Fleet's existing CLI, queue schema, integrations, libraries, and conventions. Reuse existing functionality wherever practical.

---

# 1. Executive Summary

Fleet should provide a comprehensive, agent-friendly interface for discovering, selecting, claiming, investigating, executing, updating, and completing work items.

Work items are stored in a priority queue and may link to GitHub issues, pull requests, repositories, comments, commits, reviews, and other records.

Currently, an agent may need to independently perform numerous operations:

- Discover available tasks.
- Inspect queue columns and metadata.
- Select a suitable task according to priority and eligibility.
- Claim or reserve the task.
- Fetch linked GitHub issues.
- Discover associated pull requests.
- Retrieve descriptions, discussions, timelines, reviews, commits, and diffs.
- Determine what changed since the previous agent worked on the task.
- Record discoveries, decisions, notes, and progress.
- Update queue metadata and GitHub metadata.
- Manage dependencies and blockers.
- Create, edit, or reply to comments.
- Update PRs, push code, and handle reviews.
- Release, park, resume, or complete work.
- Keep queue records and GitHub state synchronized.

Each agent should not need to implement these workflows independently.

Instead, Fleet should expose consistent low-level commands and convenient higher-level commands that compose them.

The intended standard workflow is:

    Discover
       ↓
    Evaluate / Rank
       ↓
    Claim
       ↓
    Hydrate Context
       ↓
    Investigate
       ↓
    Execute
       ↓
    Record Progress
       ↓
    Reconcile Changes
       ↓
    Complete / Merge / Close

At any point, the agent may also:

    Park
    Block
    Release
    Handoff
    Resume
    Reprioritize
    Request Review

Fleet should coordinate these transitions, preserve state, and manage synchronization with external systems.

**Important:** Fleet must be able to close and complete a queue item directly as part of its own verified completion workflow. It should not depend on a later sweep, webhook, or background polling cycle to recognize a PR merge or issue closure that Fleet itself initiated.

Webhooks and scheduled reconciliation remain necessary for detecting external changes, recovering from interruptions, and repairing inconsistencies.

---

# 2. Goals and Design Principles

## 2.1 Fleet is the agent-facing work control plane

Fleet should be the preferred interface for queue operations, work-item context, GitHub coordination, progress tracking, and work lifecycle changes.

Agents should generally use Fleet rather than independently coordinating GitHub API calls and queue database mutations.

This does not mean reimplementing Git, compilers, test runners, or other mature development tools.

An agent might still use:

    git
    cargo
    go
    pytest
    npm
    nix

Fleet should orchestrate work and maintain its associated state, not replace the development environment.

## 2.2 Both primitives and convenient workflows

Fleet should expose low-level operations such as:

    fleet queue search
    fleet queue claim
    fleet work hydrate
    fleet github issue get
    fleet github pr diff
    fleet work checkpoint
    fleet queue close

It should also provide higher-level commands such as:

    fleet work next
    fleet work open
    fleet work park
    fleet work resume
    fleet work complete

Higher-level commands may call several underlying operations.

Agents should be able to use these commands individually, compose them in shell scripts, or use built-in workflow commands.

Avoid requiring an agent to repeatedly write the same orchestration logic.

## 2.3 Persist comprehensive context, return concise context

A hydration operation should collect all relevant information into a durable context bundle.

The command should return:

- A concise summary.
- Important findings and recent changes.
- Relevant blockers or warnings.
- Paths to generated files.
- A machine-readable result.
- References for retrieving additional details.

Do not flood the agent's immediate context with thousands of lines of commits, comments, and diffs unless specifically requested.

The agent should know where the detailed data is available.

## 2.4 One canonical Fleet work-item identity

Every work item should have a stable Fleet identifier.

A single work item may be associated with multiple issues, PRs, repositories, or external resources.

For example:

    fleet://work/842

    github://acme/project/issues/203
    github://acme/project/pulls/231

Never assume a GitHub issue number is globally unique.

## 2.5 Safe operation across interruptions

Agents may be terminated, reach time limits, lose connections, or be replaced by another session.

Fleet must support:

- Durable progress.
- Recoverable ownership.
- Idempotent mutations.
- Transactional outbox delivery.
- Optimistic concurrency.
- Expiring leases.
- Checkpoints and handoffs.
- Incremental activity retrieval.
- Explicit repair and reconciliation.

An interruption should not leave the system requiring manual reconstruction.

## 2.6 Do not conflate different kinds of state

A work item's execution state is distinct from its GitHub state and agent ownership.

A PR being merged does not, by itself, describe every aspect of the work item's status.

A lease expiring does not mean the work is completed.

An issue closing does not necessarily mean every associated work item is complete.

Maintain these distinctions explicitly.

---

# 3. Proposed Component Architecture

Fleet may implement these capabilities as internal modules rather than separate running services.

The logical responsibilities are:

## 3.1 Queue Broker

Responsible for:

- Searching queue records.
- Filtering candidates.
- Ranking and selecting work.
- Evaluating eligibility.
- Managing assignment and claims.
- Acquiring leases.
- Renewing leases.
- Releasing or recovering abandoned claims.

## 3.2 Work-Item Hydrator

Responsible for:

- Fetching queue metadata.
- Resolving linked issues and PRs.
- Discovering related GitHub resources.
- Fetching timelines and comments.
- Fetching commits and diffs.
- Loading previous notes and checkpoints.
- Producing context bundles.
- Generating concise summaries.

## 3.3 GitHub Adapter

Responsible for standardized access to:

- Issues.
- Pull requests.
- Comments.
- Reviews.
- Timeline events.
- Labels.
- Milestones.
- GitHub Projects fields, where applicable.
- Commits.
- Diffs.
- Checks.
- Merge operations.
- Issue and PR updates.

Use existing GitHub API libraries or GitHub CLI functionality as appropriate.

Avoid unnecessary duplication.

## 3.4 Activity Reconciler

Responsible for:

- Identifying new activity.
- Comparing source and local state.
- Tracking synchronization cursors.
- Detecting external edits.
- Preventing duplicate processing.
- Resolving or surfacing conflicts.
- Reconstructing missed events.

## 3.5 Dependency Resolver

Maintains typed work-item relationships.

Examples:

    blocks
    blocked_by
    related_to
    duplicates
    parent_of
    child_of
    supersedes

It determines whether hard dependencies have been satisfied.

## 3.6 Work Journal

Maintains durable records of:

- Agent observations.
- Notes and decisions.
- Progress updates.
- Checkpoints.
- Activity observations.
- Lifecycle changes.
- Handoffs.
- Completion reports.

Prefer append-only records.

Corrections should generally produce new events rather than silently modifying history.

## 3.7 Mutation Coordinator

Handles safe mutations involving Fleet and GitHub.

Responsibilities:

- Validate permissions and expected state.
- Validate lease ownership.
- Apply local state changes.
- Create durable outbox entries.
- Deliver remote mutations.
- Retry recoverable failures.
- Detect conflicts.
- Verify effects.
- Finalize lifecycle transitions.

## 3.8 Synchronization Outbox

Maintains durable pending external operations.

Examples:

    Update GitHub label
    Edit managed GitHub comment
    Post progress note
    Update PR description
    Merge PR
    Close GitHub issue
    Verify remote state

The outbox must prevent accidental duplicate operations when an agent retries a command or crashes after a remote request succeeds.

---

# 4. Suggested Fleet Command Structure

The names below are proposals.

Use the existing Fleet command conventions where possible, but maintain a coherent interface.

## 4.1 Agent bootstrap and discovery

    fleet help
    fleet capabilities

    fleet agent bootstrap

    fleet agent bootstrap --scenario queue-worker
    fleet agent bootstrap --scenario assigned-item
    fleet agent bootstrap --scenario resume
    fleet agent bootstrap --scenario triage
    fleet agent bootstrap --scenario review

    fleet agent bootstrap --format compact
    fleet agent bootstrap --json

The bootstrap command should produce scenario-specific instructions, relevant commands, common pitfalls, and tool availability.

The agent should not need to load Fleet's entire command reference to get started.

## 4.2 Queue operations

    fleet queue list
    fleet queue search
    fleet queue get
    fleet queue inspect
    fleet queue rank
    fleet queue claim
    fleet queue release
    fleet queue park
    fleet queue resume
    fleet queue update
    fleet queue close

Example search:

    fleet queue search \
      --status available \
      --ready \
      --limit 20 \
      --order priority

Supported filters should include:

    --status
    --repo
    --queue
    --priority-min
    --priority-max
    --label
    --exclude-label
    --assigned-to
    --unassigned
    --ready
    --blocked
    --created-since
    --updated-since
    --capability
    --estimated-effort
    --limit

## 4.3 High-level work operations

    fleet work next
    fleet work open
    fleet work inspect
    fleet work hydrate
    fleet work refresh
    fleet work changes
    fleet work link
    fleet work checkpoint

    fleet work note add
    fleet work note list
    fleet work note edit

    fleet work block
    fleet work unblock
    fleet work park
    fleet work resume
    fleet work release
    fleet work handoff
    fleet work complete
    fleet work status

Examples:

    fleet work next --claim --hydrate

    fleet work open 842 --claim --hydrate

    fleet work changes 842 --since-start

    fleet work park 842 \
      --reason "Awaiting review"

    fleet work complete 842 \
      --merge-linked-prs \
      --close-linked-issues \
      --close-queue \
      --verify

These are composed workflows, not necessarily separate bespoke implementations.

## 4.4 GitHub operations

    fleet github issue get
    fleet github issue update
    fleet github issue close
    fleet github issue comments
    fleet github issue timeline

    fleet github pr get
    fleet github pr create
    fleet github pr update
    fleet github pr diff
    fleet github pr commits
    fleet github pr reviews
    fleet github pr checks
    fleet github pr merge

    fleet github comment add
    fleet github comment edit
    fleet github comment reply
    fleet github comment list

    fleet github events since

A work-item association should be available through:

    --work 842

For example:

    fleet github pr diff \
      --work 842 \
      --pr acme/project#231

Fleet should infer the repository when unambiguous.

## 4.5 Metadata operations

    fleet metadata get
    fleet metadata patch
    fleet metadata title
    fleet metadata summary
    fleet metadata priority
    fleet metadata labels
    fleet metadata fields

Examples:

    fleet metadata priority 842 1250

    fleet metadata labels 842 \
      --add needs-review

    fleet metadata labels 842 \
      --remove blocked

    fleet metadata summary 842 \
      --file summary.md

Metadata edits should be scoped to their intended records and fields.

## 4.6 Dependency operations

    fleet dependency list
    fleet dependency add
    fleet dependency remove
    fleet dependency graph
    fleet dependency blockers
    fleet dependency ready

Examples:

    fleet dependency add 842 \
      --blocked-by fleet:761 \
      --hard

    fleet dependency add 842 \
      --blocks fleet:900

    fleet dependency graph 842 \
      --depth 3

## 4.7 Activity and reconciliation

    fleet activity list
    fleet activity since
    fleet activity reconcile
    fleet activity watch

    fleet sync status
    fleet sync run
    fleet sync verify

    fleet sync outbox list
    fleet sync outbox inspect
    fleet sync outbox retry

Example:

    fleet activity since 842 \
      --since-checkpoint \
      --external-only

The meaning of `external-only` is activity not attributable to the current Fleet session, rather than simply activity by a different GitHub username.

## 4.8 Common command flags

| Flag | Purpose |
|---|---|
| `--work` | Explicit Fleet work item |
| `--repo` | GitHub repository |
| `--issue` | GitHub issue reference |
| `--pr` | GitHub PR reference |
| `--agent-id` | Stable agent identity |
| `--session-id` | Execution session identity |
| `--lease-token` | Active lease authorization |
| `--since` | Explicit ISO 8601 timestamp |
| `--since-start` | Activity since session start |
| `--since-checkpoint` | Activity since saved checkpoint |
| `--external-only` | Activity not attributed to this session |
| `--output` | Output path |
| `--format` | Output format |
| `--json` | Structured command output |
| `--dry-run` | Preview mutations |
| `--explain` | Explain decisions and effects |
| `--verify` | Verify resulting state |
| `--wait` | Wait for requested operations to settle |
| `--expected-version` | Concurrency protection |
| `--idempotency-key` | Safe operation retry identity |
| `--timeout` | Command-level timeout |

Agent/session configuration should supply safe defaults to avoid passing repetitive arguments.

---

# 5. High-Level Commands as Composed Workflows

A key design goal is avoiding repeated orchestration boilerplate.

## 5.1 `fleet work next`

Example:

    fleet work next \
      --ready \
      --claim \
      --hydrate full \
      --strategy weighted \
      --output .fleet/work/

Conceptual composition:

    queue.search
    queue.rank
    queue.evaluate_eligibility
    queue.claim_atomically
    work.hydrate
    work.summarize
    journal.record_claim

Selection and claiming must be concurrency-safe.

If a selected item becomes unavailable before the claim, Fleet should retry selection or return an explicit conflict.

## 5.2 `fleet work open`

Example:

    fleet work open 842 \
      --claim \
      --hydrate full

Intended for an agent that is explicitly assigned an item.

Conceptual composition:

    queue.get
    queue.validate
    queue.claim
    work.hydrate
    work.summarize

## 5.3 `fleet work park`

Example:

    fleet work park 842 \
      --reason "Waiting for maintainer response" \
      --checkpoint

Conceptual composition:

    work.checkpoint
    journal.append
    queue.transition_to_parked
    queue.release_lease
    sync.enqueue_configured_updates

The command should return enough information for a later session to resume efficiently.

## 5.4 `fleet work complete`

Example:

    fleet work complete 842 \
      --merge-linked-prs \
      --close-linked-issues \
      --close-queue \
      --verify \
      --wait

Conceptual composition:

    validate_active_claim
    validate_dependencies
    refresh_external_activity
    validate_completion_policy
    verify_required_checks_and_reviews
    transition_to_completing
    enqueue_merge_and_closure_operations
    deliver_and_verify_remote_operations
    close_queue_item_directly
    record_completion
    release_lease
    return_verified_result

The queue completion must not require a subsequent webhook or sweeper to recognize effects that this command has already verified.

---

# 6. Work-Item Data Model

A work item should logically include:

```yaml
id: 842

title: Improve lease recovery
summary: Prevent stale workers from overwriting newer claims

priority:
  value: 1250
  derived_label: priority-high

state:
  status: in_progress
  reason: null

ownership:
  assigned_agent: worker-07
  session_id: session-abc123
  lease_expires_at: null
  fencing_version: 18

links:
  issues:
    - github://acme/project/issues/203
  pull_requests:
    - github://acme/project/pulls/231

dependencies:
  blocked_by: []
  blocks: []
  related_to: []

timestamps:
  created_at: null
  updated_at: null
  claimed_at: null
  completed_at: null

checkpoint:
  id: checkpoint-004

activity:
  cursor_set_id: cursor-004
  last_successful_sync: null

metadata_version: 19
```

The specific schema should reflect existing Fleet storage conventions.

Large comments, diffs, timelines, and content snapshots should not be stored directly in the queue row.

Use references to dedicated structured records or content artifacts.

---

# 7. Priority, Ranking, and Queue Selection

Priority should be a configurable numeric value.

Labels may be derived from numeric ranges.

Example:

    1250 → priority-high

The numeric priority should be the canonical value.

Editing the priority through Fleet should update the relevant derived labels according to policy.

The inverse operation—changing a GitHub label to modify the numeric priority—should occur only if explicitly enabled.

## 7.1 Ranking considerations

Support:

- Numeric priority.
- Age of item.
- Dependency readiness.
- Agent capabilities.
- Estimated complexity.
- Repository affinity.
- Fairness.
- Starvation prevention.
- Weighted randomness.
- Optional preference for lower numerical values.

The selection algorithm should not be determined solely by the density of items in a priority range.

A large clump of similarly ranked items should not unintentionally overwhelm all other ranges.

Allow configurable mixtures of range-level and per-item selection, with a tunable priority preference.

These policies should be encapsulated behind the Queue Broker, allowing the ranking implementation to evolve independently of the agent interface.

Example:

    fleet queue rank \
      --ready \
      --strategy weighted \
      --explain

Fleet should be able to report the policy used, candidate count, eligibility, and selection result.

---

# 8. Context Hydration

Context hydration should be a first-class Fleet operation.

Example:

    fleet work hydrate 842 \
      --full \
      --output .fleet/work/842/context/

## 8.1 Queue metadata

Retrieve:

- Queue ID.
- Title and summary.
- Priority.
- All custom columns.
- Status and status reason.
- Labels.
- Assignment and lease.
- Creation and update times.
- Current and historical blockers.
- Previous notes.
- Previous checkpoints.
- Linked external resources.

## 8.2 Issues

Retrieve:

- Issue title and body.
- State and state reason.
- Assignees.
- Labels.
- Milestones.
- Project fields, where available.
- Comments.
- Relevant edits.
- Timeline events.
- Linked pull requests.
- Cross-references.
- Creation, update, and closure times.

## 8.3 Pull requests

Retrieve:

- PR title and body.
- State.
- Base and head branches.
- Current head SHA.
- Author.
- Reviewers.
- Reviews.
- Review comments.
- Discussion threads.
- Checks and statuses.
- Changed files.
- Commits.
- Diffs.
- Merge status.
- Relevant timestamps.

## 8.4 Commits and diffs

Preserve:

- SHA.
- Author.
- Committer.
- Author timestamp.
- Commit timestamp.
- Message.
- Parent commits.
- Changed paths.
- Renames.
- Additions/deletions.
- Diff hunks.
- Original and new line ranges.

The agent should be able to inspect particular files or commits without retrieving the entire PR again.

## 8.5 Events

Gather both GitHub and Fleet events.

Examples:

    queue_item_created
    queue_item_claimed
    queue_item_parked
    queue_item_blocked
    queue_priority_changed

    issue_opened
    issue_edited
    comment_created
    comment_edited

    pull_request_opened
    review_submitted
    commit_pushed
    check_completed
    pull_request_merged
    issue_closed

Events should retain:

    source
    source_event_id
    actor
    occurred_at
    observed_at
    payload_reference

---

# 9. Hydration Files and Agent Context

A hydration should produce a context directory.

Example:

    .fleet/work/842/context/
      manifest.json
      summary.md
      queue.json
      issues.json
      pull_requests.json
      dependencies.json
      comments.json
      reviews.json
      timeline.json
      commits.json
      checks.json
      activity-cursors.json
      diffs/
        pr-231.patch

The exact files produced may depend on what resources exist.

## 9.1 Summary contents

`summary.md` should contain:

1. Work-item identity and objective.
2. Queue state and priority.
3. Significant linked issues and PRs.
4. Important findings from descriptions.
5. Recent comments and activity.
6. Relevant code changes.
7. Existing blockers.
8. Active reviews or failed checks.
9. Previous progress and checkpoints.
10. Suggested next investigation steps.
11. References to detailed files.

The summary is a navigation aid, not a replacement for source evidence.

## 9.2 Example command output

    Work item: 842
    Status: claimed
    Priority: 1250
    Lease: active

    Summary:
      .fleet/work/842/context/summary.md

    Manifest:
      .fleet/work/842/context/manifest.json

    Linked issue:
      acme/project#203

    Linked PR:
      acme/project#231

    Important findings:
      - One unresolved review thread
      - One failing integration check
      - Two previous agent checkpoints

    Suggested reading:
      summary.md
      reviews.json
      diffs/pr-231.patch

    Hydration status: complete

Every artifact should carry enough provenance to establish its source and freshness.

If retrieval is incomplete, Fleet should explicitly report missing or stale evidence.

Do not silently label partial hydration as complete.

---

# 10. Titles, Descriptions, Comments, and Synchronization Authority

Fleet should distinguish queue content from GitHub content.

## 10.1 Recommended field ownership

| Field | Authority |
|---|---|
| Queue title | Fleet |
| Queue summary | Fleet |
| Numeric priority | Fleet |
| Derived priority label | Fleet |
| Work status | Fleet |
| Lease and assignment | Fleet |
| Dependencies | Fleet |
| Issue title/body | GitHub |
| PR title/body | GitHub |
| GitHub discussion | GitHub |
| Agent notes | Fleet journal |

When a queue item is initially created from an issue or PR, Fleet may initialize its title and summary from that resource.

After creation, these should normally be independently editable.

Queue titles describe the work to perform.

PR titles describe the proposed code changes.

They may reasonably diverge.

## 10.2 Selective synchronization

Support synchronization rules rather than universal bidirectional copying.

Examples:

- Fleet priority changes update GitHub priority labels.
- Fleet progress notes may update a managed GitHub comment.
- GitHub comments appear in Fleet's activity view.
- GitHub PR edits refresh Fleet's linked context.
- GitHub closures may trigger configurable lifecycle reconciliation.
- Queue title changes do not automatically rewrite PR titles.

Synchronization should be configurable per field and event type.

## 10.3 Managed description sections

Fleet should be able to edit only a designated section of a GitHub issue or PR body.

Example:

    <!-- fleet:progress:start -->

    Work is in progress.
    Current blocker: #761

    <!-- fleet:progress:end -->

Example command:

    fleet github issue update \
      --work 842 \
      --section progress \
      --file progress.md

Fleet should:

1. Fetch the current body and version.
2. Locate the managed section.
3. Check for conflicting edits.
4. Replace only the intended content.
5. Submit the mutation.
6. Record and verify the result.

Do not blindly overwrite an entire description when only one section needs updating.

## 10.4 Managed comments

Fleet may maintain a single progress comment per work item and linked issue.

Example:

    fleet work note add 842 \
      --text "Implementation passes unit tests" \
      --mirror github

Or:

    fleet github comment edit \
      --work 842 \
      --managed progress \
      --file progress.md

Store the remote comment ID.

Do not create duplicate comments on every synchronization attempt.

Regular agent notes should remain in the journal even when they are not mirrored publicly.

---

# 11. Activity Tracking and Incremental Updates

Agents must be able to discover changes since a specific point.

Examples:

    fleet work changes 842 --since-start

    fleet work changes 842 --since-checkpoint

    fleet work changes 842 \
      --since 2026-11-06T12:00:00Z

    fleet work changes 842 \
      --since-start \
      --external-only

## 11.1 Session attribution

Fleet should record the origin of its own mutations:

    agent_id
    session_id
    work_item_id
    operation_id
    outbox_id

This allows Fleet to distinguish operations initiated by the current session from external changes.

Do not assume that every GitHub event authored by the same bot account belongs to the current session.

Other sessions may share credentials.

Events without reliable attribution should be treated as uncertain, not automatically excluded.

## 11.2 Source-specific cursors

Do not rely on one global `last_checked_at` timestamp.

Track retrieval state independently for:

- Issue comments.
- Issue timeline events.
- PR discussion.
- PR reviews.
- Review threads.
- PR commits.
- PR checks.
- Queue mutations.
- Other linked sources.

Different endpoints may use different event models or pagination.

Use overlapping retrieval windows and stable event IDs for deduplication.

## 11.3 Important timestamps

Distinguish:

    occurred_at
    observed_at
    source_updated_at
    last_successful_sync_at
    session_started_at

Advancing a cursor means the relevant events have been durably processed, not merely requested.

## 11.4 Completion-time reconciliation

Before completing work, an agent should check for activity that occurred during execution.

Example:

    fleet work changes 842 \
      --since-start \
      --external-only

This might discover:

    New issue comment
    New PR review
    Changed requirements
    New commit
    Updated labels
    Closed dependency

A completion workflow should perform its own final refresh and concurrency check as well.

The agent's earlier activity check alone does not eliminate the possibility of a last-second external change.

---

# 12. Dependencies and Blockers

Fleet should support typed directed relationships.

Examples:

    842 blocked_by 761
    842 blocks 900
    842 related_to 903

Distinguish:

- Hard blockers.
- Soft or advisory dependencies.
- Related work.
- Parent/child relationships.

A hard blocker can make a work item ineligible for normal execution.

Example:

    fleet dependency add 842 \
      --blocked-by fleet:761 \
      --hard

Then:

    fleet work block 842 \
      --reason "Requires dependency 761"

The dependency resolver should:

- Identify unresolved blockers.
- Detect cycles.
- Check readiness.
- Prevent unsafe completion.
- Recognize satisfied dependencies.
- Support external references.
- Provide dependency graphs.

A resolved dependency should normally remain in history.

Unblocking does not require deleting its relationship.

For example:

    fleet work unblock 842 \
      --reason "Dependency 761 completed"

This may transition the item back to available or in progress, depending on ownership and lifecycle policy.

Unblocking must not accidentally override another unresolved hard blocker.

---

# 13. Work Lifecycle and Ownership

The operational states should include:

    available
    claimed
    in_progress
    parked
    blocked
    completing
    completed
    cancelled
    failed

Potential flow:

    available
       |
       v
    claimed
       |
       v
    in_progress
       | \
       |  +------> parked
       |             |
       |             v
       |          available
       |
       +---------> blocked
       |             |
       |             v
       |          available / in_progress
       |
       v
    completing
       |
       v
    completed

Some transitions may require validation.

## 13.1 Leases

A claim should issue:

    lease_token
    lease_expiration
    fencing_version

Example:

    fleet queue claim 842 \
      --agent-id worker-07 \
      --lease 30m

The claim must be atomic.

An expired lease may be reclaimed, but stale workers must not be permitted to mutate newer state using an old token.

## 13.2 Checkpointing

Agents should checkpoint meaningful progress.

Example:

    fleet work checkpoint 842 \
      --note "Root cause identified; tests pending"

A checkpoint may record:

- Current objective.
- Completed steps.
- Outstanding work.
- Important files.
- Branch or commit.
- Known blockers.
- Session activity cursors.
- Suggested next action.

The next agent should not need to reconstruct previous reasoning from a large unstructured transcript.

## 13.3 Time budgets

Agents may continue taking work items until they approach their execution deadline.

Fleet should support a configurable policy such as:

    Do not claim new work when insufficient
    execution time remains.

It should also support:

    fleet work checkpoint
    fleet work park
    fleet work release
    fleet work handoff

The examples below assume one work item per agent execution.

Eventually, a worker loop might repeatedly call:

    fleet work next --claim --hydrate

until its remaining time budget is insufficient for another task.

That scheduling policy is separate from work-item lifecycle semantics.

---

# 14. Transactional Outbox and Reliable Synchronization

Fleet should not require every agent to coordinate queue writes and GitHub updates manually.

A single Fleet command should be able to express the intended operation.

Example:

    fleet metadata labels 842 \
      --add needs-review \
      --mirror github

Conceptually:

1. Validate the mutation.
2. Update Fleet's authoritative state.
3. Create the corresponding outbox record in the same local transaction.
4. Commit the transaction.
5. Deliver the GitHub mutation.
6. Record the remote response.
7. Verify the resulting state.
8. Mark the outbox operation confirmed.

If GitHub is unavailable, the local operation and outbox intent should remain durable.

## 14.1 Outbox states

Suggested states:

    pending
    delivering
    confirmed
    retryable_failure
    conflict
    terminal_failure

## 14.2 Idempotency

Each logical mutation should have a stable operation ID.

Retries must not create duplicate comments, duplicate journal entries, or duplicate lifecycle operations.

GitHub mutations do not universally support server-side idempotency keys.

Therefore Fleet should use a combination of:

- Stable operation IDs.
- Local idempotency records.
- Remote state checks.
- Known remote resource IDs.
- Managed content markers.
- Verification before retry.

The goal is effectively-once application where possible, not an unsupported claim of universal exactly-once delivery.

## 14.3 Conflict handling

If an external user edits a description or comment while Fleet is updating it:

- Detect the conflict.
- Avoid overwriting unrelated edits.
- Retry safely where possible.
- Report unresolved conflicts.
- Preserve the original intended mutation.

Blind last-writer-wins behavior should not be the default for collaborative content.

## 14.4 Reconciliation

Webhooks and periodic sweeps remain valuable.

They should:

- Detect modifications made outside Fleet.
- Repair missed notifications.
- Reconcile interrupted operations.
- Recognize external closures.
- Resolve incomplete outbox deliveries.

They are recovery and observation mechanisms, not mandatory dependencies for immediate completion of operations initiated by Fleet.

---

# 15. Agent Bootstrap Scenarios

Fleet should provide scenario-specific startup instructions.

These may be injected into an agent prompt by its scheduler or supplied through the CLI.

## 15.1 Scenario A: General queue worker

Example starter instruction:

> You are a queue-driven Fleet worker. Discover Fleet's available capabilities, select an eligible work item according to queue policy, claim it, and hydrate its context. Use Fleet for queue, GitHub coordination, progress reporting, and lifecycle changes. Perform implementation work with the appropriate development tools. Check for external activity before completion. Checkpoint or park unfinished work before your execution deadline.

Suggested startup:

    fleet agent bootstrap \
      --scenario queue-worker \
      --format compact

    fleet work next \
      --ready \
      --claim \
      --hydrate full

## 15.2 Scenario B: Explicitly assigned issue

Example starter instruction:

> Work on Fleet item 842. Retrieve the current state, acquire or validate ownership, load the associated context, and continue from the latest checkpoint. Use Fleet to update linked resources and record the final outcome.

Suggested startup:

    fleet agent bootstrap \
      --scenario assigned-item

    fleet work open 842 \
      --claim \
      --hydrate full

## 15.3 Scenario C: Resume previously parked work

Example starter instruction:

> Resume Fleet item 842 if its dependencies are satisfied. Inspect the latest checkpoint, refresh external context, identify changes since the previous session, and continue the remaining work.

Suggested startup:

    fleet agent bootstrap \
      --scenario resume

    fleet work inspect 842

    fleet work changes 842 \
      --since-checkpoint

    fleet work resume 842 \
      --claim \
      --hydrate incremental

If hard blockers remain unresolved, Fleet should report that fact rather than silently forcing an ineligible item into progress.

## 15.4 Scenario D: Triage worker

Example starter instruction:

> Review untriaged queue items, inspect linked issues and PRs, correct metadata, establish dependencies, adjust priority where justified, and park or release the item for an implementation worker. Do not implement unrelated code changes.

Suggested startup:

    fleet agent bootstrap \
      --scenario triage

    fleet queue search \
      --label needs-triage

    fleet work open 842 \
      --claim \
      --hydrate full

---

# 16. Walkthrough One: Full Agent Execution

This walkthrough describes the desired end-to-end behavior.

The numerical IDs and command outputs are illustrative.

## Step 1: Agent starts

An autonomous agent starts with the general queue-worker instruction.

It knows that `fleet` is available.

It runs:

    fleet agent bootstrap \
      --scenario queue-worker \
      --format compact

Fleet returns a concise command guide.

The agent learns:

- How to find work.
- How to claim it.
- How to load context.
- How to record progress.
- How to manage blockers.
- How to complete work safely.

## Step 2: Agent chooses work

The agent runs:

    fleet work next \
      --ready \
      --claim \
      --hydrate full \
      --explain

Internally Fleet:

1. Searches the queue.
2. Filters ineligible work.
3. Applies priority policy.
4. Selects an item.
5. Atomically claims the item.
6. Starts an execution session.
7. Creates the lease.
8. Hydrates linked context.
9. Creates context files.
10. Returns a summary.

Illustrative result:

    Work item: 842

    Title:
      Improve queue lease recovery

    Priority:
      1250

    Linked issue:
      acme/project#203

    Status:
      claimed

    Context summary:
      .fleet/work/842/context/summary.md

    Full context:
      .fleet/work/842/context/manifest.json

## Step 3: Agent examines context

The agent reads:

    .fleet/work/842/context/summary.md

It discovers:

- The issue concerns a stale lease race.
- A previous attempt did not fully resolve the problem.
- Two PRs are associated with the issue.
- One PR was closed.
- Another contains an incomplete implementation.
- The latest review identified an edge case.

The agent then selectively reads:

    pull_requests.json
    reviews.json
    commits.json
    diffs/pr-231.patch

It does not need to manually discover and fetch each resource from GitHub.

## Step 4: Agent begins work

The agent records:

    fleet work note add 842 \
      --text "Investigating stale lease race and prior PR"

It may update status:

    fleet queue update 842 \
      --status in_progress

It uses normal development tools to inspect the repository.

For example:

    git status
    git log --oneline -10
    cargo test

## Step 5: Agent encounters a blocker

The agent discovers that another Fleet item must be completed first.

Dependency:

    fleet://work/761

It records the relationship:

    fleet dependency add 842 \
      --blocked-by fleet:761 \
      --hard

It updates the label:

    fleet metadata labels 842 \
      --add blocked

It records a note:

    fleet work note add 842 \
      --text "Requires revised fencing-token implementation from item 761" \
      --mirror github

Fleet:

- Records the journal event.
- Updates the queue record.
- Creates relevant outbox operations.
- Updates the configured GitHub label or managed comment.
- Records the relationship.
- Verifies remote changes when available.

## Step 6: Agent parks the item

The agent determines that useful progress is temporarily exhausted.

It runs:

    fleet work park 842 \
      --reason "Blocked by item 761" \
      --checkpoint \
      --release-lease

Fleet:

1. Saves progress.
2. Records the blocker.
3. Preserves the dependency.
4. Moves the item to the appropriate blocked/parked state.
5. Releases its execution lease.
6. Synchronizes configured external metadata.
7. Returns a resumable checkpoint reference.

The agent's execution ends.

No queue item is falsely marked completed.

---

# 17. Walkthrough Two: Resume, Review, Merge, and Complete

A later agent session is assigned item 842.

Assume item 761 has now been completed.

## Step 1: Load previous state

The agent runs:

    fleet agent bootstrap \
      --scenario resume \
      --format compact

    fleet work inspect 842

It learns:

- The item is blocked/parked.
- Item 761 was the hard dependency.
- The previous checkpoint describes the remaining work.
- The linked PR remains open.

## Step 2: Check what changed

The agent runs:

    fleet work changes 842 \
      --since-checkpoint

Fleet finds:

- Dependency 761 completed.
- A maintainer added a comment.
- A PR check was rerun.
- No new conflicting commits appeared.

Fleet returns the event summary and detailed references.

## Step 3: Unblock and resume

Fleet validates the dependency state.

The agent runs:

    fleet work unblock 842 \
      --reason "Dependency 761 completed"

    fleet metadata labels 842 \
      --remove blocked

    fleet work resume 842 \
      --claim \
      --hydrate incremental

Fleet preserves the historical dependency relationship.

It does not delete the record merely because the dependency is satisfied.

The resumed agent receives a new lease and updated context.

## Step 4: Continue implementation

The agent checks out the relevant branch or creates a worktree.

It implements the remaining change.

It runs the appropriate tests.

It records progress:

    fleet work note add 842 \
      --text "Implemented fencing-token validation and added regression tests"

It commits and pushes its changes using Git:

    git add .
    git commit -m "Fix stale lease recovery"
    git push

Fleet's GitHub integration can then refresh the linked PR:

    fleet work refresh 842 \
      --include commits,diffs,checks

The refreshed context includes the new commit and diff.

## Step 5: Update review state

The agent adds a review label:

    fleet metadata labels 842 \
      --add needs-review \
      --mirror github

It updates the managed progress comment:

    fleet github comment edit \
      --work 842 \
      --managed progress \
      --file progress.md

The operation is recorded locally and delivered through Fleet's outbox.

## Step 6: Check for new external activity

Before completing, the agent runs:

    fleet work changes 842 \
      --since-start \
      --external-only

Fleet discovers:

    New comment on acme/project#203

    Comment ID:
      44881

    Author:
      maintainer

    Message summary:
      Requests handling lease renewal during reconnect

This comment occurred after the agent began work.

It must not be ignored.

## Step 7: Respond to the new comment

The agent investigates the edge case.

It updates the implementation, adds a test, and pushes another commit.

It then responds through Fleet:

    fleet github comment reply \
      --work 842 \
      --comment-id 44881 \
      --body-file response.md

Fleet records:

- The intended reply.
- The queue activity event.
- The GitHub mutation.
- The remote comment ID.
- Delivery status.
- Session attribution.

The agent does not need to separately post the GitHub comment and manually copy its details into the queue.

## Step 8: Refresh and verify

The agent runs:

    fleet work refresh 842 \
      --include reviews,commits,checks,comments

    fleet github pr checks \
      --work 842 \
      --pr acme/project#231

It confirms:

- Required tests pass.
- Required reviews are satisfied.
- No unresolved blocking review conditions remain.
- The branch is mergeable.
- No new external changes require attention.

It records:

    fleet work note add 842 \
      --text "All required checks pass; latest maintainer feedback addressed"

## Step 9: Complete the work

The agent runs:

    fleet work complete 842 \
      --merge-pr acme/project#231 \
      --close-issues acme/project#203 \
      --close-queue \
      --require-clean-reconciliation \
      --verify \
      --wait

Fleet performs the completion workflow.

### Internal completion sequence

1. Verify the active lease and permissions.
2. Refresh important external activity.
3. Check for unreviewed external changes.
4. Validate dependencies.
5. Validate PR head SHA, required checks, and merge policy.
6. Transition the work item to `completing`.
7. Record durable completion operations.
8. Request the authorized PR merge.
9. Verify the PR is merged.
10. Close the designated linked issue.
11. Verify the issue is closed.
12. Directly transition the queue item to `completed`.
13. Record completion metadata and final notes.
14. Release the execution lease.
15. Return the verified final result.

The local queue item is closed directly by Fleet after the required external operations are verified.

**No later webhook or sweep is required to finish the normal successful path.**

Illustrative result:

    Work item: 842

    PR:
      acme/project#231
      merged

    Issue:
      acme/project#203
      closed

    Queue:
      completed

    Completion verified:
      true

    Outbox:
      all required operations confirmed

    Final checkpoint:
      .fleet/work/842/completion.json

If a remote operation fails after the PR merges, Fleet must not falsely report full completion.

Instead, it should retain a recoverable `completing` state, preserve the pending outbox operation, and report exactly which effects succeeded.

---

# 18. Walkthrough Three: Assigned Existing PR and Near-Timeout Handoff

An agent starts with an explicit instruction:

> Continue Fleet work item 905. Review the existing PR, address relevant feedback, and finish if possible. If execution time becomes insufficient, checkpoint and release the work safely.

## Step 1: Open the item

    fleet agent bootstrap \
      --scenario assigned-item \
      --format compact

    fleet work open 905 \
      --claim \
      --hydrate full

Fleet retrieves the queue record, linked issue, existing PR, reviews, diffs, comments, and prior work.

## Step 2: Inspect changes

    fleet work changes 905 \
      --since-checkpoint \
      --external-only

Fleet reports new review requests.

The agent works on the requested corrections.

## Step 3: Record progress

    fleet work note add 905 \
      --text "Resolved first review thread; second requires additional integration testing"

    fleet work checkpoint 905 \
      --note "Changes committed locally; integration tests still pending"

The checkpoint should also record the branch, current commit, changed files, and other relevant continuation data.

## Step 4: Execution time becomes insufficient

The agent has insufficient remaining time for a reliable completion.

It should not begin an unsafe merge merely to finish before its deadline.

It runs:

    fleet work park 905 \
      --reason "Execution window nearly exhausted" \
      --checkpoint \
      --release-lease

Fleet preserves:

- Current progress.
- Relevant commit.
- Remaining review issue.
- Test status.
- Latest activity cursor.
- Suggested next action.
- Resumption instructions.

The next session can continue using:

    fleet work resume 905 \
      --claim \
      --hydrate incremental

The current session ends without leaving an indefinite lease or ambiguous queue state.

---

# 19. Direct Completion Versus Webhooks and Sweeps

This is a central behavioral requirement.

Consider the sequence:

    Agent merges PR
       ↓
    GitHub emits webhook
       ↓
    Webhook worker processes event
       ↓
    Queue sweeper eventually updates work item

This sequence may remain useful for observing changes initiated outside Fleet.

It should not be the required path when Fleet itself performs the merge.

Preferred sequence:

    Agent requests Fleet completion
       ↓
    Fleet validates work
       ↓
    Fleet merges PR
       ↓
    Fleet verifies merge
       ↓
    Fleet closes linked issue
       ↓
    Fleet verifies closure
       ↓
    Fleet closes queue item directly
       ↓
    Fleet records completion
       ↓
    Agent receives confirmed result

Webhooks may arrive afterward.

They should be deduplicated and treated as confirming observations rather than reopening or duplicating completed operations.

If Fleet crashes after an external merge but before recording success, recovery must inspect GitHub's actual state before retrying.

The completion coordinator must not attempt to reverse an already successful merge simply because a later closure step failed.

---

# 20. Error Handling and Recovery

Fleet should handle at least these cases.

## 20.1 Claim race

Two agents select the same work item.

Only one obtains the claim.

The other receives a conflict or automatically selects another eligible item.

## 20.2 Agent termination

The agent disappears.

Its lease expires.

Another agent can recover the item using its checkpoint.

Stale tokens cannot overwrite newer state.

## 20.3 GitHub unavailable

A mutation is persisted to the outbox.

Fleet reports that remote synchronization is pending.

It does not claim that the external mutation succeeded.

## 20.4 Comment posted, acknowledgment lost

Before retrying, Fleet checks for the operation's previously created comment.

It records the remote ID and avoids duplicate posting.

## 20.5 PR merges but issue closure fails

The item remains in `completing`.

The outbox retains the issue-closure operation.

Fleet reports partial completion.

The next reconciliation can finish the workflow.

## 20.6 External edit during managed update

Fleet detects the version conflict.

It preserves the external edit and either safely retries the managed section or reports a conflict.

## 20.7 New external comment during completion

Fleet checks activity again before irreversible operations.

If material new feedback appears, completion should be paused according to policy.

A final check cannot make all distributed actions perfectly atomic, but it should prevent known stale-state completion.

## 20.8 Dependency cycle

A requested blocker relationship would create a cycle.

Fleet rejects it or explicitly flags the cycle for authorized resolution.

---

# 21. Structured Output Contracts

All agent-oriented commands should support stable machine-readable output.

For example, `fleet work next --json` might return:

```json
{
  "ok": true,
  "work_item_id": 842,
  "status": "claimed",
  "session_id": "session-abc123",
  "lease": {
    "active": true,
    "token_ref": "lease-local-reference"
  },
  "context": {
    "summary_path": ".fleet/work/842/context/summary.md",
    "manifest_path": ".fleet/work/842/context/manifest.json",
    "complete": true
  },
  "linked_resources": {
    "issues": 1,
    "pull_requests": 1
  },
  "warnings": [],
  "next_actions": [
    "Read context summary",
    "Inspect relevant review comments"
  ]
}
```

Avoid exposing raw credentials or unnecessary secrets in command output.

Use stable error identifiers, such as:

    CLAIM_CONFLICT
    LEASE_EXPIRED
    DEPENDENCY_BLOCKED
    CONTEXT_INCOMPLETE
    EXTERNAL_STATE_CHANGED
    OUTBOX_PENDING
    REMOTE_MUTATION_FAILED
    COMPLETION_INCOMPLETE

Commands should distinguish:

- Local mutation committed.
- External synchronization pending.
- External synchronization verified.
- Operation rejected.
- Operation partially completed.

Process exit codes should reflect meaningful success or failure.

---

# 22. Implementation Requirements

Before building new functionality, inspect the existing Fleet repository.

Identify:

1. Existing CLI command framework.
2. Queue storage schema and source of truth.
3. Queue priority implementation.
4. GitHub integration libraries and credentials.
5. Existing issue/PR synchronization logic.
6. Current webhook and sweep behavior.
7. Existing transactional outbox functionality.
8. Agent identity and session metadata.
9. Existing lease or claim implementation.
10. Existing journals, notes, and checkpoint mechanisms.

Reuse mature existing components rather than introduce parallel implementations.

## 22.1 Storage requirements

The system will likely need durable representations of:

    work_items
    work_item_links
    work_item_dependencies
    work_item_leases
    agent_sessions
    work_checkpoints
    work_journal_events
    external_resource_snapshots
    external_activity_cursors
    external_events
    outbox_operations
    synchronization_results

These are logical entities, not necessarily a prescription for separate database tables.

## 22.2 Authentication and authorization

Fleet must establish whether the agent is authorized to:

- Read queue items.
- Claim work.
- Modify priority.
- Change labels.
- Post comments.
- Modify issue/PR descriptions.
- Push or coordinate code changes.
- Merge PRs.
- Close issues.
- Complete queue items.

Read-only hydration should not require unnecessary write permissions.

Merge and closure operations should enforce repository branch protection, required checks, required reviews, and other configured policies.

No implicit privilege escalation.

## 22.3 Observability

Track:

- Queue selection outcomes.
- Claim conflicts.
- Lease expirations.
- Hydration duration and completeness.
- GitHub API failures.
- Outbox backlog and age.
- Synchronization lag.
- Reconciliation conflicts.
- Work time by lifecycle state.
- Completion success and partial failures.
- Number of resumed or abandoned items.

Logs should include operation and correlation IDs.

Sensitive content and credentials should be protected.

---

# 23. Suggested Implementation Phases

## Phase 1: Inspect and consolidate

Review existing Fleet capabilities.

Document overlapping tools, missing abstractions, current source-of-truth rules, and available storage.

Establish stable work-item references and shared CLI conventions.

## Phase 2: Discovery, claiming, and hydration

Implement or consolidate:

    fleet agent bootstrap
    fleet queue search
    fleet queue claim
    fleet work next
    fleet work open
    fleet work hydrate

Produce reliable context bundles and concise summaries.

## Phase 3: Notes, metadata, and dependencies

Implement:

    fleet work note
    fleet work checkpoint
    fleet metadata
    fleet dependency
    fleet work block
    fleet work park
    fleet work resume

Ensure state changes are durable and recoverable.

## Phase 4: GitHub operations and outbox

Consolidate GitHub issue, PR, comment, label, review, and timeline operations.

Implement the mutation coordinator and transactional outbox.

Support managed content sections and remote mutation verification.

## Phase 5: Incremental reconciliation

Implement:

    fleet work changes
    fleet work refresh
    fleet activity since
    fleet sync verify

Track source-specific cursors and session-originated mutations.

## Phase 6: Direct completion orchestration

Implement:

    fleet work complete

It should coordinate:

- Pre-completion validation.
- External activity checks.
- PR merges.
- Issue closure.
- Direct queue closure.
- Final journaling.
- Outbox settlement.
- Recovery from partial completion.

## Phase 7: Advanced workflow composition

Support reusable workflow recipes, shell-friendly scripts, and scenario-specific agent startup guidance.

Add more sophisticated work selection, scheduling, fairness, capability matching, and automated handoff.

---

# 24. Verification and Acceptance Criteria

The implementation should be tested with realistic workflows.

## Core selection

- [ ] Agents can discover eligible queue items.
- [ ] All required metadata can be inspected.
- [ ] Priority ranking is configurable.
- [ ] A claim is atomic.
- [ ] Two agents cannot own the same exclusive lease.
- [ ] Stale leases cannot overwrite newer ownership.

## Hydration

- [ ] Linked issues are retrieved.
- [ ] Linked PRs are discovered.
- [ ] Relevant comments and reviews are retrieved.
- [ ] Commits and diffs include accurate metadata.
- [ ] Context files are persisted.
- [ ] A concise summary is returned.
- [ ] Missing or partial data is reported.
- [ ] The context is traceable to its sources.

## Activity

- [ ] Changes can be queried since a timestamp.
- [ ] Changes can be queried since a checkpoint.
- [ ] Session-originated mutations can be identified.
- [ ] External events are not incorrectly suppressed.
- [ ] Repeated retrieval does not duplicate events.
- [ ] Cursor advancement is safe across interruptions.

## Metadata and dependencies

- [ ] Numeric priority is authoritative.
- [ ] Derived labels update correctly.
- [ ] Queue and PR titles can diverge.
- [ ] Notes can be recorded independently.
- [ ] Managed comments can be updated without duplication.
- [ ] Managed body sections do not overwrite unrelated content.
- [ ] Hard blockers affect work eligibility.
- [ ] Dependency cycles are detected.
- [ ] Resolved dependencies remain auditable.

## Outbox

- [ ] A local mutation and outbox intent are committed atomically.
- [ ] Remote failures leave durable retryable records.
- [ ] Retries do not unnecessarily duplicate comments or mutations.
- [ ] Outbox statuses are inspectable.
- [ ] External results are verified.
- [ ] Conflicts are surfaced rather than silently overwritten.

## Lifecycle

- [ ] Items can be claimed, parked, blocked, and resumed.
- [ ] Checkpoints preserve useful continuation context.
- [ ] Lease expiration is recoverable.
- [ ] Completion validates required conditions.
- [ ] Fleet can merge an authorized PR.
- [ ] Fleet can close explicitly designated linked issues.
- [ ] Fleet closes the queue item directly after verifying the required external operations.
- [ ] No webhook or sweep is required for successful direct completion.
- [ ] An interrupted completion can resume without repeating successful effects.
- [ ] Completion events and final notes are auditable.

## End-to-end agent scenarios

- [ ] New agent claims and hydrates a task.
- [ ] Agent investigates linked issues, PRs, reviews, and diffs.
- [ ] Agent records work and adds a blocker.
- [ ] Agent parks the task and releases its lease.
- [ ] Another session resumes after dependency resolution.
- [ ] Agent pushes a fix and updates progress using Fleet.
- [ ] Agent detects and responds to a new external comment.
- [ ] Agent verifies checks and reviews.
- [ ] Agent merges and closes through Fleet.
- [ ] Fleet immediately records verified completion.
- [ ] A separate near-timeout session can checkpoint safely without completing unfinished work.

---

# 25. Preferred Agent Experience

The ideal normal workflow should require only a few Fleet commands:

    fleet agent bootstrap --scenario queue-worker

    fleet work next --claim --hydrate full

    # Read returned context summary.
    # Investigate and implement the work.

    fleet work note add 842 \
      --text "Implementation complete"

    fleet work changes 842 \
      --since-start \
      --external-only

    # Resolve any newly discovered activity.

    fleet work complete 842 \
      --merge-linked-prs \
      --close-linked-issues \
      --close-queue \
      --verify \
      --wait

For interrupted work:

    fleet work checkpoint 842 \
      --note "Remaining work documented"

    fleet work park 842 \
      --checkpoint \
      --release-lease

For a later session:

    fleet work resume 842 \
      --claim \
      --hydrate incremental

Agents should still have access to every lower-level operation when the standard workflow is insufficient.

The common workflow should be straightforward, recoverable, and auditable.

---

# 26. Final Design Direction

Fleet should evolve into the primary coordination layer for queue-driven autonomous work.

Its principal responsibilities are:

**Queue Broker:** Select and claim appropriate work.

**Work-Item Hydrator:** Gather comprehensive context and present the important information efficiently.

**GitHub Adapter:** Provide consistent access to issues, PRs, commits, diffs, comments, reviews, and mutations.

**Activity Reconciler:** Detect changes and maintain accurate synchronization.

**Dependency Resolver:** Manage blockers, readiness, and work relationships.

**Work Journal:** Persist notes, progress, checkpoints, and decisions.

**Mutation Coordinator and Outbox:** Apply changes reliably across Fleet and GitHub.

**Completion Coordinator:** Verify and finalize work directly without depending on eventual webhook or sweep processing.

The implementation should favor a small set of powerful, composable primitives with convenient agent-facing workflow commands.

An agent should be able to receive a short instruction, discover Fleet's capabilities, select or open a work item, obtain complete context, perform the work, synchronize its changes, respond to new activity, and finish with verified queue and GitHub state.

Fleet should handle the operational complexity so that agents can concentrate on the work itself.

