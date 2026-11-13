# Fleet & PM Stack — Priority, Work-Item Orchestration, and Direct Dispatch | Execution Charter v1.1

> Source: Google Doc `1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw` — https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw/edit
> Drive dates: created 2026-11-06T15:13:54Z, modified 2026-11-06T15:15:32Z. Exported (plain text via Google Docs API) 2026-11-13T03:16Z for the PM Stack & Fleet orchestration design-mill intake.
> Role: The v1.1 integrated execution charter (6 November 2026) — the governing requirements text for both design tracks.

---

Fleet & PM Stack — Priority, Work-Item Orchestration, and Direct Dispatch

Integrated execution charter · v1.1 · 6 November 2026  
Repository: developing-today/code · Status: Proposed design, not evidence of implementation  
Companion: Fleet v0.3 Summit Planning & Granular Issue Decomposition Handoff (separate scope)

# 1. Mandate and intended outcome

Upgrade the existing Fleet multicall binary and PM Stack, not a replacement system, into the shared control plane for queue-driven agents. Agents should be able to discover work, understand its priority and dependencies, claim it safely, hydrate GitHub context into files, execute work, report notes and comments, handle new activity, park or resume, and complete linked PR/issue/queue states directly and verifiably. GitHub, GitHub Project #4, the Fleet queue, and PM Stack must present coherent and explainable views.

This is two coordinated designs and potentially two implementation tracks: (A) urgency/triage/PM visibility/direct dispatch and (B) work-item orchestration/context hydration/lifecycle/outbox. These must be reconciled with the existing Queue Table Improvements effort before shared schema changes. The coordinator should not treat the design as authorization for a broad unreviewed production migration.

# 2. Work for the coordinating agent — execute, do not merely propose

1. Inspect the current developing-today/code repository, especially pkgs/fleet/, pkgs/fleet/crates/fleet-queue/, pm-stack/, pm-stack/opencode-claimant/, pm-stack/dispatch/, pm-stack/aggregator/, pm-stack/projector/, repository AGENTS.md, queue schemas/migrations, source-of-truth conventions, and the Queue Table Improvements issue/design. Pin the inspected commit.
2. Produce and commit two repository-grounded design documents (use established paths if different): docs/design/fleet-priority-urgency-pm-stack.md and docs/design/fleet-work-item-orchestration.md. Treat this charter as the requirements and the existing full Work-Item Orchestration draft as supporting detail. Preserve all user outcomes; replace weak mechanisms with documented better alternatives and reasons.
3. Reuse existing queue brokers, source-of-truth APIs, GitHub Project mappings, dispatch/claimant paths and outbox. Do not introduce a parallel queue, scheduler, secret resolver, or PM database.
4. Locate and update an existing matching design issue, or create one central actionable design issue “Fleet: urgency, triage, PM visibility, and agent workflow design”. Link both documents, the Queue Table Improvements design, relevant PRs, and supporting architectural records; ensure project membership and native relationships are correct.
5. Set intrinsic urgency 450 (High) and Do Now as a *separate one-shot dispatch directive*. This design is high because it unblocks multiple interfaces; it is not an emergency. Do not inflate numeric urgency merely to bypass scheduling.
6. Immediately dispatch a design/architecture agent through the currently supported authorized execution mechanism. Bind the issue/work item to the task and OpenCode session where applicable. Confirm a real dispatch receipt and, if possible, a live/accepted session—not just priority, a label, or a queue position. Avoid double-claiming with ordinary workers.
7. Record in the issue the source links, dispatch method, lane/route, task/session identifiers, accepted/started status, and next steps. If dispatch cannot be verified, report precisely which step failed; do not claim success.
8. Return concise evidence: commit/PR, document URLs, issue ID, numeric urgency, dependency links, task/session IDs, and any blockers. The handoff is not complete without verified artifacts.

Source-of-truth observation: Current repository documentation describes GitHub Project Developing Today #4 as a portfolio map: Project Kind=Epic is canonical, the epic label is a maintained mirror/gate, native sub-issue parent links are hierarchy, and native blocking links are dependencies. The orchestrator is the Project's single writer; dashboard and agents should not bypass it. Confirm this remains accurate at execution time.

# 3. Shared mental model: never conflate the four axes

- Intrinsic urgency: durable numeric score, lower = sooner; default Standard = 1000.
- Importance/impact: expected value, strategy, risk and scope; separate field or documented provisional dimension.
- Effective scheduling urgency: dynamic derived pressure from hard blockers, deadlines, critical-path position, opportunities, resource availability and directive policy. Do not blindly overwrite intrinsic score.
- Workflow/ownership: triage, backlog, ready, claimed, in_progress, review, parked, blocked, completing, completed, cancelled; distinguish assignment, active execution, lease and GitHub issue/PR states.

A separate dispatch directive such as normal, expedite, do-now, direct-session, or hold controls operational handling. do-now does not mean Critical urgency and does not remove hard blockers.

# 4. Initial urgency bands and stable total order

| Intrinsic score | Band | Typical meaning |
|---|---|---|
| 0–99 | Emergency | Severe active incident or extraordinary immediate need |
| 100–199 | Supercritical | Severe critical-path or near-immediate operational work |
| 200–399 | Critical | Major blockers, near-term required work or hard deadlines |
| 400–799 | High | Ahead of normal actionable work |
| 800–1199 | Standard | Ordinary actionable work; new-item default 1000 |
| 1200–1599 | Low | Useful but not time-sensitive |
| 1600–1999 | Someday | Speculative or deferred |
| 2000+ | Cold backlog | Retained, searchable, scheduled for occasional reconsideration |

Validate migration against existing queue values; do not reinterpret them silently. Preserve authorized exact-score edits, band-default moves, move before/after, and additive adjustments (e.g. ±10, ±25). Equal scores are legitimate. Maintain a deterministic secondary rank and then oldest-first and stable-ID tie breaking; do not force unique scores for every ticket. The cold backlog must remain searchable and eligible for review rather than becoming a 9999-point purgatory.

# 5. Dynamic triage without inflation or oscillation

Regular triage considers age, freshness of evidence, deadlines, opportunity windows, strategic pivots, current epics/projects, blockers, work size, recent effort, and user direction. A medium item may rise after a week if still valuable and now time-sensitive; an obsolete High item may fall after a pivot. Age alone is not an automatic route to Critical.

Triage comparisons may classify each item as much more urgent, slightly more urgent, roughly equal, slightly less urgent, much less urgent, custom, or needs research. Minor automatic adjustments can default to ±10; larger jumps require evidence. Large clusters should be progressively refined (50 → rough stacks of 25/10/10/5 → smaller cohorts), not deterministically flattened. At most one ordinary automatic priority movement per item per sweep, except a new high-confidence event. Preserve reason, previous/new score, evidence, confidence and event ID.

Introduce configurable boundary hysteresis for low-confidence automatic score changes near the Standard↔High and other band boundaries, with a bounded buffer, confidence requirement and cooldown. This is resistance, not a permanent barrier: manual moves, verified deadlines, incidents and high-confidence hard blockers bypass it. Prevent repeated ping-pong changes and unbounded cumulative drift.

Handle temporary opportunity windows with deadline, temporary_boost, and expiry_action=retriage; expiration removes the boost and reassesses the task instead of abruptly burying it. A low-value quick fix blocking a high-value item should acquire effective scheduling pressure while its intrinsic importance/urgency remain independently explainable. Propagation must be cycle-safe, limited to true hard dependencies, and sensitive to effort and alternatives.

# 6. GitHub labels, Projects and PM Stack projections

Fleet numeric urgency is the canonical score; GitHub priority labels derive from bands (reuse existing taxonomy if different). An authorized external priority-label addition is an input command: translate to the configured band default or an appropriate retained intra-band score; remove mutually exclusive labels and reproject. If the final priority label is removed, preserve numeric score and expedite triage, rather than silently resetting it or treating it as emergency. Conflicts or untrusted changes should be held for triage. A distinct needs-triage label moves it to triage, not implementation.

Board columns should distinguish Inbox/Triage, Backlog, Ready/Todo, In Progress, Review, Blocked, Parked, Done. Ready includes executable High-and-above, explicitly prepared work and appropriate owner-assigned tasks; lower urgency generally remains Backlog unless state overrides. Claimed items must show agent/session and an In Progress Project status. Parked/blocked work once picked up must retain last worker, checkpoint, reason, blockers, and an actionable next-agent note instead of disappearing into Backlog.

The orchestrator remains the single writer of GitHub Project fields. The aggregator/projector/board are projections and authorized control surfaces, not new owners of the same fields. Keep native GitHub blockedBy distinct from sub-issue parent/epic links. Use PM Stack's existing Kaneo, Pad, Grist, aggregator, projector, and dispatch boundaries rather than merging service databases by fiat.

# 7. Mobile-first interactive operations

From a phone or browser, the authorized user should be able to search/filter the Fleet queue, read summaries and linked issues/PRs, add notes and comments, reprioritize, change permitted state, request triage or direct agent dispatch, inspect dependencies, request a status report, find an active session and send a live instruction. Show operation accepted, running, pending external sync, blocked, conflicted, and verified complete separately. Use accessible touch-sized controls, a compact responsive layout, keyboard navigation, and confirmations for consequential actions.

UI actions call the same validated Fleet command/service layer; do not embed a second ranking or authorization engine in client JavaScript. Support provider-specific session adapters (e.g., OpenCode and approved Muse/Slack routes), report actual capabilities, and never mistake posting a message to Slack for session acknowledgment.

# 8. Immediate commands, comments and event routing

Support a small authenticated slash-command grammar, with names adjusted to existing conventions: /fleet status, triage, start, resume, retry, park, unblock, priority high, priority 575, do-now, merge, and agent status. Also accept authorized plain-language requests on live or parked items when policy permits. An actionable live-issue comment routes to the current session and records delivery/acknowledgment. An authorized actionable parked-item comment may directly launch a new session or triage it; it must not merely rise in the ordinary queue.

Event handling: verify webhook signature → authorize actor → resolve item → deduplicate → find live session → send to session or direct-dispatch → bind/record receipt → update board/comment. Use allowlists, RBAC, rate limits, explicit confirmations for merges or irreversible actions, replay protection and audit IDs. Repeated event deliveries must not double-launch an agent. do-now bypasses waiting behind normal selection not leases, auth, hard dependencies or concurrency caps.

# 9. Queue selection and bounded exploration

Normal lanes should select the highest eligible effective scheduling urgency, subject to capability, ownership, dependency and fairness rules. When there is no eligible Emergency/Supercritical/Critical/Do Now work requiring that lane, allow a configurable low-probability exploration draw (initial suggestion 4%). Draw three distinct candidate items, but first persist all three dispositions: select for execution, triage, defer/reprioritize, or release. Do not claim three exclusive leases merely because they were sampled. If the exploratory agent terminates, durable hints survive and a triage worker can apply bounded suggestions (e.g. selected -20, triage -10, defer +10). Prefer hints and verification over mechanically moving priorities for random selection alone.

Default weighted sampling over eligible items:

    weight_i = r^(-(u_i - u_ref)/d) * count(bucket_i)^(-alpha)
    P(i) = weight_i / sum_j weight_j

    r = 1.25  (relative preference per d urgency points)
    d = 200   (point distance)
    alpha = 0.50 (cluster-density exponent)
    bucket_width = 200
    optional uniform-mixture epsilon = 0.05

Lower scores are favored. Bucket correction reduces, but does not eliminate, large-cluster advantage. Sample without replacement; compute density over the eligible population; make all parameters configurable per lane and expose seeded simulations and --explain. When separate importance scores exist, evaluate a tunable urgency-versus-importance axis mixture; do not fabricate importance from urgency.

# 10. Work-item lifecycle primitives and high-level sugar

Expose a cohesive agent-facing CLI, with exact naming reconciled to existing Fleet conventions:

    fleet agent bootstrap --scenario queue-worker|assigned-item|resume|triage
    fleet queue list|search|rank|claim|release|park|resume|update|close
    fleet work next|open|inspect|hydrate|refresh|changes|checkpoint
    fleet work note add|list|edit
    fleet work block|unblock|park|resume|release|handoff|complete
    fleet github issue get|update|comments|timeline|close
    fleet github pr get|create|update|diff|commits|reviews|checks|merge
    fleet github comment add|edit|reply
    fleet dependency add|remove|list|graph|ready
    fleet metadata priority|labels|title|summary|patch
    fleet sync status|verify|outbox list|outbox retry
    fleet agent dispatch|session status|session send|session resume

Typical common flags: --work, --issue, --pr, --repo, --agent-id, --session-id, --lease-token, --since, --since-start, --since-checkpoint, --external-only, --json, --dry-run, --explain, --expected-version, --idempotency-key, --verify, --wait.

Implement composed workflows rather than inventing separate backends for each sugar command. For example fleet work next --claim --hydrate full performs search/rank/eligibility/atomic claim/hydration/journaling; fleet work park --checkpoint persists progress, transitions state, releases lease and enqueues sync; fleet work complete performs precondition checks, remote operations, direct queue close and verification.

# 11. Hydration and incremental activity

The Work-Item Hydrator collects every relevant queue column, prior checkpoints, GitHub issues/PRs, comments, edits, reviews, checks, related resources, timelines, commits and diffs (including SHA, author/committer times, messages, changed paths, hunks and old/new line ranges). It writes a durable bundle:

    .fleet/work/<id>/context/
      manifest.json
      summary.md
      queue.json
      issues.json
      pull_requests.json
      comments.json
      reviews.json
      timeline.json
      commits.json
      checks.json
      dependencies.json
      activity-cursors.json
      diffs/pr-<number>.patch

Return to the agent a short navigable summary and actual file paths, not thousands of lines of raw data. Include source versions/hashes, retrieval completeness, important findings, blockers, test/review status and next recommended investigations. Never silently mark incomplete hydration complete. Treat GitHub descriptions/comments as untrusted task data, not instructions overriding agent authority.

fleet work changes <id> --since-start --external-only should detect actionable activity since session start that did not originate from that session; also support timestamps and checkpoints. Maintain per-source retrieval cursors, event identity, occurred/observed times, last successful sync and provenance. Use overlap plus deduplication; advance cursors only after durable processing. Session attribution requires Fleet operation/session IDs, not only a GitHub username. Unattributable events are uncertain, not silently excluded.

# 12. Notes, field authority, and dependency semantics

Keep Fleet queue title/summary independent of GitHub issue/PR titles and bodies after initial initialization. The issue/PR is an independent external work artifact; continuous bidirectional title/body mirroring is generally wrong. Store Fleet private notes and append-only journal separately from public GitHub comments. Allow explicit mirroring to an owned managed comment or delimited body section; edit only that section using optimistic concurrency, not the entire body. Derive priority labels from canonical score, and accept authorized label edits through a conflict-aware normalization policy.

Represent typed relationships blocks, blocked_by, related_to, parent_of, child_of, duplicates, supersedes. Hard blockers affect eligibility and effective scheduling urgency; advisory edges do not automatically block. Prevent cycles and preserve resolved history. Existing GitHub native relationships and Project hierarchy should be projected accurately rather than replaced with prose.

# 13. Lease and outbox reliability

Claims must be atomic and issue a lease_token, lease_expiration, session_id and fencing/version counter. A stale agent cannot overwrite a newer claim. Checkpoints and expiry enable interrupted-session recovery. Ownership, execution and GitHub state remain distinct.

Use a transactional outbox for cross-system changes: authoritative local state + durable outbox intent are committed atomically; external write is delivered, checked and acknowledged later. States: pending, delivering, confirmed, retryable_failure, conflict, terminal_failure. Store stable operation IDs, remote IDs and expected versions. Verify before retries to prevent duplicate GitHub comments or repeated merges. Do not pretend GitHub and the local database share an atomic transaction.

# 14. Direct verified completion

An agent should be able to run a high-level workflow such as:

    fleet work complete 842 \
      --merge-pr developing-today/code#231 \
      --close-issues developing-today/code#203 \
      --close-queue --require-clean-reconciliation --verify --wait

Fleet validates lease/auth, dependencies, last-minute external activity, PR head SHA, checks/reviews and merge eligibility. It moves to completing, persists outbox operations, merges the authorized PR, verifies merge, closes designated issues, verifies closure, directly closes the queue item, records outcome and releases lease. Do not require a subsequent webhook or sweeper to finish Fleet-initiated completion. The webhook/sweeper remains necessary for external changes and crash recovery. If merge succeeds but closure fails, retain a recoverable completing state and report partial completion precisely.

# 15. End-to-end reference scenarios

Scenario A — standard agent: fleet agent bootstrap --scenario queue-worker → fleet work next --ready --claim --hydrate full → read returned summary.md and selective diffs → inspect code/tests → fleet work note add → edit/commit/push using normal Git → fleet work changes --since-start --external-only → respond to external comments through Fleet → refresh review/check state → fleet work complete --verify --wait.

Scenario B — blocked, parked, resumed: after hydration, discover hard dependency #761 → fleet dependency add 842 --blocked-by fleet:761 --hard → label/journal through Fleet → fleet work park 842 --checkpoint --release-lease → later session checks --since-checkpoint, observes dependency resolved, calls fleet work unblock, fleet work resume --claim --hydrate incremental, completes implementation and merges when verified.

Scenario C — new live comment: authorized user comments on an active labeled issue from mobile → webhook verifies/normalizes → Fleet finds bound OpenCode session → routes instruction and records acknowledgment → agent handles new edge case, pushes fix and replies via Fleet; no ordinary queue wait.

Scenario D — near-timeout: agent has inadequate remaining time → checkpoint with branch/commit, tests run/pending, activity cursors and next steps → park/release safely. Do not start a risky merge to beat the deadline. Future workers can resume. Workers may iterate tasks until their remaining time budget becomes insufficient, but the examples assume one item each.

# 16. Implementation sequencing and acceptance

Phases: (1) verified repository census, data ownership and Queue Table Improvements compatibility; (2) shared identity, queue claims and hydration; (3) typed dependencies, notes, lifecycle/checkpoints; (4) GitHub adapter, managed content and outbox; (5) incremental activity and session attribution; (6) direct completion/recovery; (7) priority/triage and PM Stack mobile/direct dispatch; (8) simulations and production migration. Sequence according to actual dependencies, not this document's ordering alone.

Acceptance must cover atomic claims, stale lease fencing, full/partial hydration, per-source change cursors, external comment handling, managed-content conflicts, triage feedback, weighted exploration statistics, boundary hysteresis, PM Stack projection fidelity, mobile operations, authenticated slash commands, direct Summit/worker session receipt, duplicate-webhook resistance, crash recovery after remote mutation, PR merge verification, and immediate direct queue completion.

# 17. Execution evidence and reporting

The design agent should provide: current architecture map, gaps, decisions/ADRs, integrated schema/API/CLI contracts, safe migration plan, phased implementation issues with acceptance tests, risks and go/no-go gates, actual created documents/issue links, dispatch receipts, and verified status. Distinguish implemented, tested, scheduled, proposed, blocked, and unverified. An issue labeled Do Now is not evidence of an executing agent, and a test passing on mocks is not evidence of production synchronization.

# 18. Source references

- Existing full work-item-orchestration draft (Google Drive): https://docs.google.com/document/d/15md8Zff2BTQjZ5ZJE7k3QyAXm_d258IN_CB2ojsV1LA/edit
- Repository: https://github.com/developing-today/code
- PM Stack deployment/source-of-truth: pm-stack/README.md, pm-stack/SHARED-SCHEMA-MAP.md
- OpenCode claimant: pm-stack/opencode-claimant/prompt.md, pm-stack/opencode-claimant/claimant.py
- Related Fleet v0.3 Summit planning effort: see the companion Summit handoff; it is separate, and should share interface/issue conventions without conflating scope.

Final principle: preserve clear source-of-truth boundaries, visible execution status, durable recovery, and agents' ability to act without unnecessary permission loops. Prefer improving the architecture over blindly transcribing a numeric rule, but never silently drop a required user outcome.

Companion Google Doc — Fleet v0.3 Summit design and issue-decomposition handoff:
https://docs.google.com/document/d/1AKDACXObRcUwmJIeCfLaGv6bxcWC00e3rYjOztk4qiY/edit

