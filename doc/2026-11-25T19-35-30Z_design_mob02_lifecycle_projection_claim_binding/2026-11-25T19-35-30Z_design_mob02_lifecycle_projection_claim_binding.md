# MOB-02: Lifecycle projection, column precedence, and truthful claim binding

Issue: [#9469](https://github.com/developing-today/code/issues/9469)
(slice `MOB-02` of [#9201](https://github.com/developing-today/code/issues/9201),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal (V3), not an authorization to implement.**
This record elaborates the lifecycle-projection slice of the merged
design package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225), component #9201 §3.1 and §3.5, into the lifecycle facets
the board reads, the one precedence rule that turns them into a
column, the evidence that alone may show a claim as `In Progress`,
the typed contract of the pure policy function, the golden oracle
and the named tests a separately scoped implementation issue can be
filed against. It authorizes no code, endpoint, schema, migration,
Project write, deployment or live acceptance; those remain separately
gated stages (§12).

It builds on the MOB-01 record
(`doc/2026-11-25T01-21-26Z_design_mob01_work_read_model/2026-11-25T01-21-26Z_design_mob01_work_read_model.md`,
"the MOB-01 record") and on the MOB-01 implementation slice now open
as PR #15188 (`xaltha/9468-work-projection`, head `273cf215`). Where
this record names a type, function or reason code that PR already
introduces, it reuses that spelling. Where PR #15188 and the MOB-01
record disagree, D9 lists the disagreement and says which side this
record takes.

Governing decisions consumed, not re-opened:

- **Component #9201 §3.1** (`components/9201-pm-projection-mobile-control.md:67-71`):
  board column is a projection with explanation, not a writable
  lifecycle enum (line 67); a claimed task is `In Progress` only when
  a fresh authoritative claim read says so, and a session is shown
  bound only when readback confirms the task/session binding; a
  `work/assigned` label, dispatch intent, queue row status or session
  shell alone is not proof of execution (line 68); workflow,
  eligibility and board projection are separate facets (line 69);
  blocked or parked keeps its state, urgency and owner assignment
  cannot promote it, `manual_only` is a hard eligibility property,
  `do_now` cannot bypass it, and Ready is eligible/unblocked plus
  executable High-and-above, explicitly prepared, or explicitly
  owner-assigned, everything else actionable is Backlog (line 70);
  `Done` needs the completion policy's verified terminal receipt and
  required reconciliation, otherwise conflict/pending (line 71).
- **Component #9201 §3.5** (lines 122, 125, 126, 130): a stale source
  shows facet-level stale and disables dependent mutations; a claimed
  item without a verified session shows "claimed / binding unknown",
  never "working"; a parked/blocked item with an active binding keeps
  parked/blocked plus the discrepancy; a lossy Project column shows
  facets and an explanation.
- **Charter §6** (`sources/02_charter_v1_1.md:70,72`): a distinct
  `needs-triage` label moves work to triage, not implementation; the
  board distinguishes Inbox/Triage, Backlog, Ready/Todo, In Progress,
  Review, Blocked, Parked, Done; parked/blocked work once picked up
  keeps its last worker, checkpoint, reason and blockers instead of
  disappearing into Backlog.
- **Original §12.1 to §12.4** (`sources/04_recovered_original_combined_design.md:13`,
  one flattened line): the column list; Review is "implementation
  completed but awaiting review, checks, or approved merge"; In
  Progress is "actively claimed/executing work" plus "a verified bound
  agent session"; Done is "verified completed work"; "a blocked or
  parked status should take precedence over a simple urgency-based
  Ready classification"; previously worked blocked items must not
  return to "a generic untouched Backlog column".
- **FDR-PMO-003** (`decisions/FDR-PMO-003-lifecycle-claim-fencing.md:31,35,58`):
  four distinct dimensions (lifecycle, readiness, ownership/execution,
  provider-native remote state); `completed` needs a verified
  terminal receipt; broker time, not client clock, decides expiry, and
  clock uncertainty fails conservatively.
- **#9206 §3** (`components/9206-work-item-lifecycle-fleet-subcommands.md:56-66`):
  the same four dimensions; "a lease is not proof" of execution.
- **The MOB-01 record D6** (lines 192-225): MOB-01 names conflicts
  and does not choose the column; column precedence is MOB-02's owned
  policy in `work-lifecycle-policy.ts`, installed through an
  injection point.

## 1. What exists at main `5bacbbf2` (2026-11-25)

Nothing below is changed by this record. Each item is either a
source MOB-02 reads, or an existing rule MOB-02 deliberately does not
reuse.

### 1.1 Fleet queue (the authority for claim, lease, park, block)

- **Stored task statuses.** `tasks.status` takes eight values:
  `queued`, `claimed`, `parked`, `blocked`, `handed_off`, `done`,
  `cancelled`, `failed`
  (`pkgs/fleet/crates/fleet-queue/src/work/lifecycle.rs:26-43`,
  spellings at `:59-70`). `from_stored` returns `None` for any other
  spelling, so an unratified value such as `in_progress` is never
  silently renamed (`:72-81`). The client treats `done`, `cancelled`,
  `failed` as terminal (`client/transport.rs:17`).
- **Loss-aware workflow crosswalk (CLI-01, #9501).**
  `map_queue_state` (`work/lifecycle.rs:364-452`) maps `claimed` to
  workflow `active` as _lossy_, with the declared loss "a claim is
  ownership, not proof a session started" (`:394-400`); `done` maps to
  `completed` only with `verified_completion_receipt`, and is
  explicitly unmapped without it (`:423-448`). `ExecutionState`
  (`:262-271`) names `not_started … terminal` and says "execution
  evidence does not follow from a live lease".
- **Claim eligibility model (REL-01, #9512).** `ClaimRefusal`
  (`work/lifecycle.rs:617-648`) includes `ExpiredAwaitingReclaim`
  ("the lease lapsed but the sweep has not reclaimed the row yet"),
  `AttemptCap`, `ManualOnly`, `Blocked`, `ParkedNeedsOverride`,
  `HandedOffBound`. `claim_eligibility` (`:677-760`) refuses blocked
  and terminal rows on every door and manual-only rows on the
  automatic door regardless of Do Now.
- **Session binding.** A binding is written in two places: the
  `repo_task_lane_ledger.session_id` column and
  `tasks.progress.session_id` (+ `launched_by`)
  (`turso_queue/lifecycle.rs:1505-1524`). `verb_bind` refuses unless
  the task is `claimed` with `lease_until > now` (`:1477-1490`) and
  refuses a second, different session (`:1496-1503`). A claim that
  names its session binds it in the same step (`:588-594`, `:616-617`);
  a fresh claim never inherits a binding (`:582-584`).
- **Binding clearing.** `clear_binding` strips `session_id` and
  `launched_by` and stamps `session_cleared {session_id, cleared_at,
via}` (`turso_queue/core.rs:2497-2518`). It runs on park
  (`turso_queue/lifecycle.rs:2026-2029`), release (`:2207`), lease
  expiry (`core.rs:2602`), parked revalidation (`core.rs:2068`), fail
  (`admin.rs:1206`) and claim (`claim.rs:1283`). The terminal verbs
  `complete` and `cancel` keep the binding on the row "as the
  execution record" and emit `session_unbound` instead
  (`claim.rs:658-677`, callers `admin.rs:1155,1650`). So a `done` row
  that still carries `progress.session_id` is a historical record,
  not a live binding.
- **Lease expiry.** `expire_leases` selects `status = 'claimed' AND
lease_until < now` (`core.rs:2582-2592`); it returns rows to
  `queued`, or to `failed` with `last_error = "lease expired at
attempt cap (n/m)"` when `attempts >= max_attempts`
  (`core.rs:2563-2569`, `:2610-2625`), or to `handed_off` for sink
  rows (`:2625-2640`). Between expiry and the sweep the row still
  reads `claimed`.
- **Park and wait kinds.** `park_kind` is `absolute` or `relative`
  (`turso_queue/lifecycle.rs:1975-1981`, `:1912-1928`). `wait_kind`
  is one of `timer`, `triage`, `ci`, `review`, `capacity`,
  `external` (`:1878-1890`); a `person_answer` condition maps to
  `review`.
- **Waiting on a person.** `PERSON_WAIT_CLASSES` =
  `escalated-to-user`, `drew_decision`, `person-answer`;
  `PERSON_WAIT_CONDITION_KINDS` = `person_answer`
  (`person_wait.rs:23-27`).
- **Blocked rows.** A lane block writes `status = 'blocked'` with
  `blocked_deps`, `dep_next_recheck_at`, `dep_blocked_until`
  (`turso_queue/admin.rs:89-100`).
- **Priority profile (PRI-01, #9478, closed).**
  `PriorityProfile::queue_v1()` (`work/priority.rs:344-377`) and
  `queue_v1_golden()` (`:827-839`) carry `critical=10`,
  `expedite=15`, `high=20`, `next-up=25`, `normal=40`, `low=70` and
  the other scale members. No "High-and-above" predicate is defined
  there; #9200 (open) owns that interpretation.

### 1.2 PM aggregator (what the board shows today)

- **No lifecycle policy module.** `src/work-lifecycle-policy.ts` and
  `test/work-lifecycle-policy.test.ts` do not exist on main, and
  neither does `src/work-projection.ts` (it exists only on PR #15188).
- **`queue-view.ts` In Progress is a union of single signals.**
  `reconciledInProgress` (`pm-stack/aggregator/src/queue-view.ts:122-200`)
  adds a ref to In Progress on any one of `project-status`
  (Project Status = In Progress), `active-session` (an orchestration
  lined-up task whose _status text_ is `in-progress`) or `open-pr`
  (`:104`, `:154-191`); the page-level `inProgress` list also admits
  a `status/in-progress` or `status/in-review` label (`:305-345`).
  None of these is a verified task/session binding. #9201 V1 already
  says this view is not a Fleet queue truth source
  (`components/9201-pm-projection-mobile-control.md:22`).
- **`status-taxonomy.ts` folds facets.** Orchestration `blocked` and
  `stalled` map to canonical `In Progress` (`src/status-taxonomy.ts:75-82`);
  GitHub `merged pr`, `closed pr`, `closed issue` map to `Done`
  (`:89-91`). That is the #1198 page vocabulary; it violates §3.1
  lines 70-71 if reused as a board column, so MOB-02 does not reuse it.
- **Session feed carries no task binding.** `SessionFeedEntry`
  (`src/model.ts:536-568`) has `sessionId`, `pr`, `branch`, `title`,
  `lastActivityAt`, `awaitingQuestion`, `outcome`, but no field that
  names the bound fleet task. No adapter today returns a readback that
  could confirm a binding from the session side.
- **`work/assigned` is not ownership.** The lane guard treats only
  `held`, `user/owned`, `do-not-auto-edit` as held; "assignees and
  `work/assigned` never count as held"
  (`pm-stack/book/concepts/claimant.md:37`).
- **Test runner.** `package.json` declares `"test": "bun test"`;
  `.github/workflows/pm-aggregator-tests.yml` runs on
  `workflow_dispatch` only (line 21-22).

### 1.3 MOB-01 implementation (PR #15188, open, not merged)

`pm-stack/aggregator/src/work-projection.ts` at `273cf215`:

- Exports `projectWorkView(input: WorkProjectionInput, relations)`,
  `WorkView`, `BoardColumn` = `"Inbox/Triage" | "Backlog" | "Ready" |
"In Progress" | "Review" | "Blocked" | "Parked" | "Done"`
  (lines 110-118), and `ClaimInput {state, worker, lane, fresh,
session_binding: "verified" | "unknown"}` (lines 57-68).
- Computes the column inline (lines 233-280) in the order Done
  (verified receipt) > Parked > Blocked > In Progress (`claimFresh`)
  > terminal signal without receipt (Backlog + `completion_unverified`)
  > Ready > Inbox/Triage > Backlog.
- Reason codes it emits: `verified_terminal_receipt`,
  `parked_with_reason`, `hard_blockers_present`, `queue_blocked`,
  `fresh_authoritative_claim`, `completion_unverified`,
  `eligible_high_or_above`, `explicitly_prepared`, `owner_assigned`,
  `triage_required`, `not_ready`,
  `manual_only_excluded_from_automatic_claim`. Conflict codes:
  `claimed_binding_unknown`, `completion_unverified`.
- Its own comment on #9468 (2026-11-25T19:00Z) says verification is
  incomplete: the remote run failed and no aggregator test run is on
  the PR yet.

### 1.4 Gap

No module decides the board column from the separate facets under
one written precedence rule, and nothing today requires _both_ a live
broker claim _and_ a confirmed session binding before showing work as
active. The existing pages show In Progress from any one signal
(§1.2), and PR #15188 shows it from a fresh claim alone (D9, item
1). MOB-02 supplies that rule as a pure, table-tested policy.

## 2. Decisions

### D1. One pure policy module; the read model calls it

`work-lifecycle-policy.ts` exports two pure functions (§4):
`evaluateClaimBinding` and `projectLifecycleColumn`. They fetch
nothing, read no clock, hold no module state; `opts.now` and the
staleness bounds are injected, as in MOB-01 D1. `projectWorkView`
obtains `projection.column`, `projection.reason_codes` and the
lifecycle conflicts from `projectLifecycleColumn`, either through the
MOB-01 record's `opts.column_policy` injection point (MOB-01 D6) or by
a direct call that replaces PR #15188 lines 233-280. Either way there
is exactly one precedence implementation in the repository.

### D2. Lifecycle facets

The policy reads seven facets, each kept separate and each with its
own source and freshness. No facet is derived from another.

| Facet                 | Source (sole)                                    | Values the policy distinguishes                                                                                                     |
| --------------------- | ------------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| `queue_status`        | fleet broker task read                           | the eight stored spellings verbatim (§1.1); anything else is `unknown_queue_status`                                                 |
| `hold`                | fleet broker task read                           | park `{park_kind, wait_kind, reason, checkpoint_ref, not_before}`; block `{reason_class, blocked_deps, dep_blocked_until}`          |
| `eligibility`         | fleet broker read (never recomputed client-side) | `eligible`, `ineligible{reason_codes}`, `manual_only`; `hard_blocker_refs`                                                          |
| `claim`               | fleet broker task read                           | `worker_id`, `lease_until`, `attempts`, `max_attempts`, binding `progress_session_id`, `ledger_session_id`                          |
| `session`             | provider adapter readback                        | per session: `session_id`, `provider`, `bound_work` (what the adapter says it is bound to), `state`, `last_activity_at`             |
| `remote`              | GitHub issue/PR reads, #9070 genealogy           | own issue `open`/`closed`; linked PRs `open`/`closed`/`merged` with `draft`; spawned follow-ups `open`/`closed`                     |
| `completion`          | designated completion policy                     | `verified_terminal_receipt` and the reconciliations the receipt requires                                                            |
| `policy` (sub-facets) | fleet policy (#9200) and authorized owner policy | `high_or_above`, `explicitly_prepared`, `owner_assigned`, `triage_required`; each present/absent/error, never inferred from a label |

Project #4 Status is read only to report `project_status_disagrees`
(MOB-01 D6). It never decides the column, because §3.1 line 70 allows
an explicit Board/Project status to override only "through a
documented source authority/precedence rule", and Project #4 is the
orchestrator's single-writer projection
(`components/9201-pm-projection-mobile-control.md:13`), not a
lifecycle authority.

### D3. Column precedence (the one rule)

Rules are evaluated in order; the first that fires sets the column
and its primary reason codes. Two kinds of secondary reason are also
added, so the explanation shows what precedence overrode: when R2 or
R3 wins while any promoting input (`high_or_above`,
`owner_assigned`, `explicitly_prepared`, a `do_now` directive) is
present and true, `held_not_promoted` is added; and when R5 wins, any
R6 condition that also holds adds its R6 reason. Informational
reasons (`waiting_on_person`, `session_awaiting_input`,
`draft_pr_open`, `handed_off_unbound`, `follow_up_open_elsewhere`,
`held_state_stale`) are added wherever their condition holds.

| Rule | Column         | Fires when                                                                                                                                                                                                           | Primary reason code(s)                                                                                            |
| ---- | -------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------- |
| R0   | `null`         | the broker read is `error`; or `queue_status` is an unknown spelling; or every facet is absent                                                                                                                       | `lifecycle_source_error`, `unknown_queue_status`, `nothing_sourced`                                               |
| R1   | `Done`         | `completion.verified_terminal_receipt` is present and true, **and** no present facet contradicts it: own issue not `open`, no linked PR `open`, `queue_status` not one of `queued/claimed/parked/blocked/handed_off` | `verified_terminal_receipt` (plus `follow_up_open_elsewhere` when a spawned follow-up is open)                    |
| R2   | `Parked`       | `queue_status = parked`                                                                                                                                                                                              | `parked_with_reason`                                                                                              |
| R3   | `Blocked`      | `queue_status = blocked`, or `eligibility.hard_blocker_refs` non-empty                                                                                                                                               | `queue_blocked`, `hard_blockers_present`                                                                          |
| R4   | `In Progress`  | `evaluateClaimBinding` returns `fresh = true` and `session_binding = "verified"` (§3)                                                                                                                                | `fresh_authoritative_claim`, `verified_session_binding`                                                           |
| R5   | `Review`       | a linked PR is `open` and not `draft`; or a terminal signal is present (queue `done`, own issue `closed`, a linked PR `merged`) without R1 holding                                                                   | `pr_awaiting_review`, `completion_unverified`                                                                     |
| R6   | `Inbox/Triage` | `triage_required`; or `queue_status` is `failed` or `cancelled`; or `queued` with `attempts >= max_attempts`; or a claim is live but `session_binding = "unknown"` (§3.3)                                            | `triage_required`, `queue_failed`, `queue_cancelled`, `attempt_cap_reached`, `claim_binding_unverified`           |
| R7   | `Ready`        | `eligibility = eligible` from a fresh read, not `manual_only`, and at least one of `high_or_above`, `explicitly_prepared`, `owner_assigned` is present and true                                                      | `eligible_high_or_above`, `explicitly_prepared`, `owner_assigned`                                                 |
| R8   | `Backlog`      | none of the above                                                                                                                                                                                                    | `not_ready`, `manual_only_excluded_from_automatic_claim`, `no_fleet_task`, `claim_lease_expired_awaiting_reclaim` |

This satisfies the slice goal "blocked/parked > eligible Ready >
Backlog" (R2, R3 before R7 before R8) and the §3.1 line 70 rule that
urgency and owner assignment cannot promote held work: `high_or_above`
and `owner_assigned` are read only in R7.

Why R1 sits above R2/R3: a verified receipt with no contradicting
facet means the work is finished; a stale park on a finished item is
the contradiction R1 already checks (`queue_status = parked` blocks
R1). Why R6 sits above R7: charter §6 line 70, a `needs-triage` item
goes to triage, not implementation. Why R5 sits above R6: an open PR
or an unverified completion is the more specific explanation of
where the work stands; R6's reasons are still added.

### D4. Staleness can only restrict, never promote

A facet whose read is older than `opts.stale_after_ms[source]` (or
that its source marks stale, MOB-01 D4) keeps its value and is marked
stale. Then:

- It **may** still fire a restrictive rule: R2, R3, R6. A stale park
  or block stays Parked/Blocked with `held_state_stale` added, because
  dropping it would promote held work on old evidence.
- It **may not** fire a permissive or terminal rule: R4, R7. A stale
  claim read never shows In Progress (`claim_read_stale`); a stale
  eligibility read never shows Ready (`eligibility_stale`).
- R1 reads the receipt, which is an immutable record, so a stale
  receipt read still counts; but any present facet that contradicts
  it, stale or not, blocks R1.
- `error` facets contribute nothing, as in MOB-01 D4: no value, no
  rule, no conflict side.

### D5. Absent is not error

A work item with no fleet task (the broker answered and there is no
row, or no broker read is connected) has `queue_status` absent. It can
reach R1 (a receipt exists), R5, R6 or R8 (`no_fleet_task`), never R4
or R7, because both need broker evidence. A broker read that failed
is R0 (`null`), because the projection cannot tell whether the item
is held. This keeps "we could not read the queue" from looking like
"the queue has nothing for this item" (MOB-01 §3.1).

### D6. `null` is a column value

`BoardColumn | null` is the policy's output type. PR #15188 types
`projection.column` as non-null `BoardColumn`; the MOB-01 record D6
allows `null`. This record takes the MOB-01 record's side: R0 needs a
value that cannot read as Done, Ready or Backlog. MOB-03 renders
`null` as "Unknown" with the reason codes.

### D7. Conflicts reuse MOB-01 names and add only lifecycle ones

The policy emits conflicts as `{code, detail}` (PR #15188 shape,
lines 145-147), sorted by `code`. It reuses MOB-01 D6's
`claimed_binding_unknown`, `binding_without_claim`,
`held_with_active_binding`, `issue_open_pr_merged`,
`work_terminal_issue_open`, `issue_closed_work_open`,
`project_status_disagrees`, and PR #15188's `completion_unverified`.
It adds:

| Code                       | Fires when                                                                                                          |
| -------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| `claimed_lease_expired`    | `queue_status = claimed` and `lease_until <= broker read observed_at`                                               |
| `binding_session_mismatch` | `progress_session_id` and `ledger_session_id` are both present and differ, or the adapter names a different session |
| `session_shell_unbound`    | an adapter session names this work as `bound_work`, but the broker has no binding for it                            |
| `extra_session_for_work`   | more than one adapter session names this work; at most one matches the broker binding                               |
| `bound_session_terminal`   | the broker binding names a session the adapter reports `terminal`                                                   |
| `work_terminal_pr_open`    | queue `done` or a verified receipt, and a linked PR is still `open`                                                 |

`detail` is fixed text per code plus the refs involved; it never
copies free text from issues, comments or sessions.

### D8. Determinism and source spellings

Same input, byte-identical output (the MOB-01 record D9). `reason_codes` and
`conflicts` are sorted. Queue spellings, `wait_kind`, `park_kind`,
block `reason_class` and priority readbacks (`expedite=15`,
`next-up=25`) are carried verbatim; the policy never normalizes them
and never maps `expedite` to `do_now`.

### D9. Disagreements found, and the side this record takes

1. **In Progress without a session.** PR #15188 line 251 projects In
   Progress when `claim.fresh === true`, whatever `session_binding`
   is; its `claimed_binding_unknown` conflict fires only when a
   session object is present (lines 216-231). A fresh claim with no
   session therefore shows In Progress with no conflict. That
   contradicts §3.1 line 68, original §12.2 ("a verified bound agent
   session") and this slice's oracle row "claimed + no session". This
   record requires both (R4, §3).
2. **Terminal signal without receipt goes to Backlog.** PR #15188
   lines 254-262 put it in Backlog. Charter §6 line 72 and original
   §12.4 say previously worked items must not disappear into Backlog,
   and original §12.2 defines Review as completed-but-awaiting
   checks. This record uses Review + `completion_unverified` (R5).
3. **Triage below Ready.** PR #15188 evaluates Ready before
   Inbox/Triage (lines 263-276). Charter §6 line 70 sends
   `needs-triage` to triage, not implementation. This record puts R6
   above R7.
4. **Non-null column.** See D6.
5. **One `github_state` value.** PR #15188 carries a single
   `github_state` (`open|closed|merged`, line 88). R1, R5 and the
   oracle rows "PR merged + issue open" and "done but PR open" need
   the issue state and each linked PR (with `draft`) separately, as
   the MOB-01 record D5 specifies (`github: issue_state, pr_states[]`).
   MOB-02 needs that split in the read model's input; it does not
   add a second GitHub reader.
6. **Freshness is caller-asserted.** PR #15188's `ClaimInput.fresh`
   and `Sourced.freshness` are booleans/labels the caller sets; there
   is no `opts.now` or `stale_after_ms` (contrast MOB-01 D1, D4). This
   record derives `fresh` and `session_binding` inside the policy
   from timestamps (§3), and keeps the PR's field names as the
   output of `toClaimInput` (§4.3).
7. **File ownership.** The issue's normalized owned files are only
   `work-lifecycle-policy.ts` and its test; the component slice text
   (`components/9201-pm-projection-mobile-control.md:173`) and the
   MOB-01 record §6 also give MOB-02 `work-projection.ts`. Wiring D1
   needs a change in `work-projection.ts` (the injection point or the
   call). §11 lists this as an open question for the issue owner.

## 3. In Progress requires verified task and session evidence

### 3.1 What counts as evidence

`evaluateClaimBinding` returns `fresh = true` only when all of these
hold on the broker read:

1. `broker` is `present` (not absent, not error), and its
   `observed_at >= opts.now - opts.stale_after_ms.fleet_broker`.
2. `queue_status = claimed` and `worker_id` is non-null.
3. `lease_until > broker.observed_at`. The comparison uses the read's
   own observation time, not `opts.now`, so that a client clock ahead
   of the broker cannot expire a live lease and a client clock behind
   it cannot revive a dead one (FDR-PMO-003 clock oracle, line 58).
   This mirrors `verb_bind`'s own liveness test (`status = claimed`
   and `lease_until > now`, `turso_queue/lifecycle.rs:1477-1490`).

It returns `session_binding = "verified"` only when, in addition:

4. The broker names exactly one bound session: `progress_session_id`
   is non-null, and `ledger_session_id` is either equal to it or
   `"not_read"` (the read did not include the ledger). Unequal values
   give `binding_session_mismatch` and `unknown`.
5. `sessions` is `present` and fresh
   (`observed_at >= opts.now - opts.stale_after_ms.session_adapter`).
6. Exactly one adapter readback has `session_id` equal to the broker
   binding, `bound_work` equal to this `(authority_id, work_id)`, and
   `state` not `terminal`. `last_activity_at` age is reported, not
   required: the lease heartbeat is the liveness signal, and an idle
   agent with a live lease and a confirmed binding is still working.

Both sides are required because each side alone is a known false
positive: a broker binding survives as the execution record on
`done`/`cancel` (§1.1) and is only cleared on the next sweep after a
lease lapses; an adapter session can mention a task it is not bound
to (the "session shell" of §3.1 line 68).

### 3.2 What never counts

A `work/assigned` label; any `priority:*` label; Project #4 Status
`In Progress`; a `status/in-progress` label; an orchestration
lined-up status text of `in-progress`; an open or draft PR; a
dispatch intent or launch receipt without bind; a Slack post (a
`delivered-to-channel` receipt, §3.1 line 73); a `done` row's
retained `progress.session_id`; a `session_cleared` stamp. Each of
these is still shown as its own facet.

### 3.3 When the claim is live but the binding is not verified

The column is not In Progress. R6 fires with
`claim_binding_unverified` and the conflict
`claimed_binding_unknown`, and the claim facet still shows the worker
and lane. That is §3.5 line 125's "claimed / binding unknown", placed
in Inbox/Triage because the stated recovery is "request
reconciliation; don't dispatch another worker blindly", which is an
attention item, not untouched Backlog. If a linked PR is open, R5
wins (Review) and the R6 reason and the conflict are still listed.

Specific sub-cases:

- **No session at all** (`progress_session_id` null): reason
  `no_bound_session`.
- **Broker binding but no adapter readback** (today's normal case,
  §1.2): reason `session_readback_absent`.
- **Adapter read failed:** reason `session_read_error`.
- **Adapter read stale:** reason `session_read_stale`.
- **Bound session terminal:** conflict `bound_session_terminal`.
- **Adapter names another session:** conflict
  `binding_session_mismatch`.

### 3.4 Consequence with today's sources

No adapter returns `bound_work` (§1.2), so step 6 cannot hold today
and R4 never fires on live data. That is the truthful result the
MOB-01 record §3.2 already anticipates; In Progress appears once a
session adapter readback with task binding exists (MOB-07 and the
#9073 to #9076 session/ingestion foundations, consumed, not built
here). Fixtures exercise R4 now.

## 4. Typed contract

All types live in `work-lifecycle-policy.ts` except `BoardColumn`,
`ClaimInput` and `WorkView`, which are imported from
`work-projection.ts` (PR #15188 names). Identifiers and counts that
could exceed the safe-integer range are decimal strings
(`architecture.md`); `attempts`/`max_attempts` are small integers as
the broker stores them.

### 4.1 Inputs

```ts
export const LIFECYCLE_POLICY_VERSION = "pm_work_lifecycle_policy/v1";

export interface LifecyclePolicyOptions {
  now: string; // ISO-8601, injected; the module never calls Date.now()
  stale_after_ms: {
    fleet_broker: number;
    session_adapter: number;
    github: number;
    project: number;
    policy: number;
  };
}

export type Read<T> = { state: "present"; value: T; source: string; observed_at: string; freshness_hint?: "live" | "stale" | "fixtures" } | { state: "absent"; reason: "not_sourced" | "source_has_no_value" } | { state: "error"; source: string; observed_at: string | null; error_code: string };

export interface BrokerTaskRead {
  task_id: string;
  status: string; // stored spelling, verbatim; parsed against the eight known values
  worker_id: string | null;
  lane_id: string | null;
  lease_until: string | null;
  attempts: number;
  max_attempts: number;
  manual_only: boolean;
  park: {
    park_kind: string | null; // "absolute" | "relative" today
    wait_kind: string | null; // "timer" | "triage" | "ci" | "review" | "capacity" | "external" today
    reason: string | null;
    checkpoint_ref: string | null;
    not_before: string | null;
  } | null;
  block: { reason_class: string | null; blocked_deps: string[]; dep_blocked_until: string | null } | null;
  binding: { progress_session_id: string | null; ledger_session_id: string | null | "not_read" };
  eligibility: { state: "eligible" | "ineligible" | "manual_only"; reason_codes: string[]; hard_blocker_refs: string[] };
}

export interface SessionReadback {
  session_id: string; // opaque, never coerced
  provider: string;
  bound_work: { authority_id: string; work_id: string } | null;
  state: "active" | "idle" | "awaiting_input" | "terminal" | "unknown";
  last_activity_at: string | null;
}

export interface RemoteRead {
  issue_state: "open" | "closed" | null; // null: the work has no own issue
  prs: { locator: string; state: "open" | "closed" | "merged"; draft: boolean }[];
  follow_ups: { locator: string; state: "open" | "closed" }[]; // #9070 spawned-by children
}

export interface LifecycleEvidence {
  work: { authority_id: string; work_id: string };
  broker: Read<BrokerTaskRead | null>; // present with null value: broker answered, no task
  sessions: Read<SessionReadback[]>;
  remote: Read<RemoteRead>;
  project_status: Read<string>; // conflict reporting only (D2)
  completion: Read<{ verified_terminal_receipt: boolean; receipt_ref: string | null }>;
  policy: {
    high_or_above: Read<boolean>; // #9200-owned predicate; absent until #9200 defines it
    explicitly_prepared: Read<boolean>;
    owner_assigned: Read<boolean>;
    triage_required: Read<boolean>;
  };
}
```

`LifecycleEvidence` is built by an adapter in `work-projection.ts`
from the `WorkProjectionInput` the read model already receives, plus
the split GitHub facet (D9 item 5). It is not a second reader.

### 4.2 Outputs

```ts
export interface ClaimBindingVerdict {
  claim: "unclaimed" | "claimed_live" | "claimed_lease_expired" | "claimed_read_stale" | "unknown";
  fresh: boolean; // §3.1 steps 1-3
  session_binding: "verified" | "unknown"; // §3.1 steps 4-6
  bound_session_id: string | null;
  reason_codes: string[]; // sorted
}

export type RuleId = "R0" | "R1" | "R2" | "R3" | "R4" | "R5" | "R6" | "R7" | "R8";

export interface LifecycleColumnResult {
  policy_version: typeof LIFECYCLE_POLICY_VERSION;
  column: BoardColumn | null;
  rule: RuleId;
  reason_codes: string[]; // sorted; includes overridden rules' reasons
  conflicts: { code: string; detail: string }[]; // sorted by code
  stale_facets: string[]; // sorted facet keys read as stale
  claim_binding: ClaimBindingVerdict;
}

export function evaluateClaimBinding(e: LifecycleEvidence, o: LifecyclePolicyOptions): ClaimBindingVerdict;
export function projectLifecycleColumn(e: LifecycleEvidence, o: LifecyclePolicyOptions): LifecycleColumnResult;
```

### 4.3 Bridge to PR #15188 names

```ts
export function toClaimInput(v: ClaimBindingVerdict, worker?: string, lane?: string): ClaimInput;
// state: v.claim === "unclaimed" ? "unclaimed" : "claimed"
// fresh: v.fresh
// session_binding: v.session_binding
```

The read model's `facts.claim` and `facts.session` keep PR #15188's
shapes (`claimed` vs `claimed_unverified`, `binding_unknown`). The
only change MOB-02 needs in those shapes is that their inputs are
derived, not asserted.

### 4.4 Invariants

1. `column = "In Progress"` implies `claim_binding.fresh` and
   `claim_binding.session_binding = "verified"`.
2. `column = "Done"` implies a present, true receipt and no present
   contradicting facet (R1).
3. `column ∈ {"Ready", "Backlog", "Inbox/Triage", "Review", "In Progress"}` implies
   `queue_status ∉ {parked, blocked}` and `hard_blocker_refs` empty.
4. `column = "Ready"` implies a fresh `eligible` broker read and not
   `manual_only`.
5. A stale broker or session read never fires R4, and a stale
   eligibility or policy read never fires R7 (D4).
6. Changing `high_or_above`, `owner_assigned`, urgency or
   `dispatch_directive` alone never moves an item out of Parked or
   Blocked.
7. The function never returns a terminal column (`Done`) for an input
   in which every terminal signal is absent ("no terminal state
   fabricated").

## 5. Edge cases

| Case                                | Input sketch                                                                                | Column         | Rule | Reasons / conflicts                                                                             |
| ----------------------------------- | ------------------------------------------------------------------------------------------- | -------------- | ---- | ----------------------------------------------------------------------------------------------- |
| Claimed but unbound                 | `claimed`, lease live, `progress_session_id` null                                           | `Inbox/Triage` | R6   | `claim_binding_unverified`, `no_bound_session`; conflict `claimed_binding_unknown`              |
| Claimed, broker-bound, no readback  | as above with `progress_session_id = ses_a`, `sessions` absent                              | `Inbox/Triage` | R6   | `claim_binding_unverified`, `session_readback_absent`; conflict `claimed_binding_unknown`       |
| Lease expired, sweep not yet run    | `claimed`, `lease_until < observed_at`, attempts below cap                                  | `Backlog`      | R8   | `claim_lease_expired_awaiting_reclaim`; conflict `claimed_lease_expired`                        |
| Lease expired at attempt cap        | as above, `attempts >= max_attempts`                                                        | `Inbox/Triage` | R6   | `attempt_cap_reached`, `claim_lease_expired_awaiting_reclaim`; conflict `claimed_lease_expired` |
| Queued at attempt cap               | `queued`, `attempts >= max_attempts`, `high_or_above=true`; broker eligibility `ineligible` | `Inbox/Triage` | R6   | `attempt_cap_reached`                                                                           |
| Failed after cap                    | `failed`, `last_error` "lease expired at attempt cap"                                       | `Inbox/Triage` | R6   | `queue_failed`                                                                                  |
| Waiting on a person (block)         | `blocked`, `reason_class = escalated-to-user`                                               | `Blocked`      | R3   | `queue_blocked`, `waiting_on_person`                                                            |
| Waiting on a person (park)          | `parked`, `wait_kind = review`, condition `person_answer`                                   | `Parked`       | R2   | `parked_with_reason`, `waiting_on_person`                                                       |
| Working, session asks a question    | R4 evidence holds; adapter `state = awaiting_input`                                         | `In Progress`  | R4   | `fresh_authoritative_claim`, `verified_session_binding`, `session_awaiting_input`               |
| Done, PR still open                 | `done`, receipt verified, linked PR `open`                                                  | `Review`       | R5   | `pr_awaiting_review`, `completion_unverified`; conflict `work_terminal_pr_open`                 |
| Done, no receipt, issue open        | `done`, no receipt, issue `open`                                                            | `Review`       | R5   | `completion_unverified`; conflicts `completion_unverified`, `work_terminal_issue_open`          |
| Multiple bindings, broker disagrees | `claimed` live, `progress_session_id = ses_a`, `ledger_session_id = ses_b`                  | `Inbox/Triage` | R6   | `claim_binding_unverified`; conflicts `binding_session_mismatch`, `claimed_binding_unknown`     |
| Multiple adapter sessions           | R4 evidence holds for `ses_a`; a second readback `ses_c` also names this work               | `In Progress`  | R4   | as R4; conflict `extra_session_for_work`                                                        |
| Bound session ended                 | `claimed` live, bound `ses_a`, adapter `ses_a.state = terminal`                             | `Inbox/Triage` | R6   | `claim_binding_unverified`; conflicts `bound_session_terminal`, `claimed_binding_unknown`       |
| Parked with stale active binding    | `parked`, adapter `ses_a` active with `bound_work` = this work                              | `Parked`       | R2   | `parked_with_reason`; conflict `held_with_active_binding`                                       |
| Handed off to the sink              | `handed_off`, no worker, no binding, eligible                                               | `Backlog`      | R8   | `handed_off_unbound`, `not_ready`                                                               |
| Cancelled                           | `cancelled`, issue `closed`                                                                 | `Inbox/Triage` | R6   | `queue_cancelled` (see §11 Q4)                                                                  |
| No fleet task                       | broker present with `null` value; issue open; Project Todo                                  | `Backlog`      | R8   | `no_fleet_task`                                                                                 |
| Broker read failed                  | `broker` error                                                                              | `null`         | R0   | `lifecycle_source_error`                                                                        |
| Unknown stored spelling             | `status = in_progress`                                                                      | `null`         | R0   | `unknown_queue_status` (value kept verbatim in the facet)                                       |
| Manual-only, Do Now                 | `queued`, `manual_only`, eligible + High, directive `do_now`                                | `Backlog`      | R8   | `manual_only_excluded_from_automatic_claim`                                                     |
| Draft PR, no claim                  | `queued`, PR `open` `draft`, not High                                                       | `Backlog`      | R8   | `not_ready`, `draft_pr_open`                                                                    |

## 6. Golden oracle: the issue's pairwise conflicts

Each row is one fixture pair in
`pm-stack/aggregator/test/fixtures/work-lifecycle/<name>.evidence.json`
and `<name>.result.json`. `now = 2026-11-25T12:00:00Z`; all bounds
`stale_after_ms = 300000` in fixtures (the aggregator's existing
`FEED_STALE_MS`, `src/feed.ts:478`, chosen for fixtures only; the
production values are §11 Q1). The expected column and full
`reason_codes`/`conflicts` are exact.

| Fixture                        | Evidence                                                                                                               | Expected column | Expected reasons (sorted)                               | Expected conflicts                              |
| ------------------------------ | ---------------------------------------------------------------------------------------------------------------------- | --------------- | ------------------------------------------------------- | ----------------------------------------------- |
| `hard-blocked-high`            | `queued`, eligibility `ineligible` with `hard_blocker_refs=["fleet://work/9"]`, `high_or_above=true`                   | `Blocked`       | `hard_blockers_present`, `held_not_promoted`            | none                                            |
| `parked-owner`                 | `parked`, reason + `checkpoint_ref`, `owner_assigned=true`                                                             | `Parked`        | `held_not_promoted`, `parked_with_reason`               | none                                            |
| `ready-stale`                  | `queued`, `eligible`, `high_or_above=true`, broker `observed_at` 10 min before `now`                                   | `Backlog`       | `eligibility_stale`, `not_ready`                        | none                                            |
| `claimed-no-session`           | `claimed`, `lease_until` 10 min after `observed_at`, `progress_session_id=null`                                        | `Inbox/Triage`  | `claim_binding_unverified`, `no_bound_session`          | `claimed_binding_unknown`                       |
| `session-shell-unbound-task`   | `queued`, `eligible`, not High; adapter `ses_x` active with `bound_work` = this work; broker binding null              | `Backlog`       | `not_ready`                                             | `session_shell_unbound`                         |
| `pr-merged-issue-open`         | `queued` (no live claim), issue `open`, PR `merged`, no receipt                                                        | `Review`        | `completion_unverified`                                 | `completion_unverified`, `issue_open_pr_merged` |
| `task-done-followup-open`      | `done`, receipt verified, own issue `closed`, follow-up `owner/repo#2` `open`                                          | `Done`          | `follow_up_open_elsewhere`, `verified_terminal_receipt` | none                                            |
| `verified-binding-in-progress` | `claimed` live, `progress_session_id=ses_a`, ledger `ses_a`, adapter `ses_a` active bound to this work; Project `Todo` | `In Progress`   | `fresh_authoritative_claim`, `verified_session_binding` | `project_status_disagrees`                      |

The last row is the positive case the slice's failing-test line
requires ("verify active verified binding creates In Progress and
preserves source facets"). The `task-done-followup-open` row shows
that a follow-up is its own work item: it stays open in its own row
and does not stop this item's Done, because the receipt policy, not
MOB-02, says which reconciliations are required. If that policy names
the follow-up as required, R1 does not hold (§11 Q3).

## 7. Tests (`pm-stack/aggregator/test/work-lifecycle-policy.test.ts`)

All future tests, written failing first. The suite must list exactly
these 14 names before it counts as run; zero tests is not a pass.

1. `work_lifecycle_never_infers_done_from_one_facet` (the named red
   test). Table: each single terminal signal in isolation (queue
   `done`; issue `closed`; PR `merged`; Project `Done`; a
   `status:done` label) gives a column other than `Done`, and
   `completion_unverified` where a terminal signal is present.
2. `work_lifecycle_never_infers_in_progress_from_one_facet`. Table:
   `work/assigned` label; priority label; Project `In Progress`;
   `status/in-progress` label; lined-up `in-progress` text; open PR;
   fresh claim alone; broker binding alone; adapter session alone.
   None gives `In Progress`.
3. `work_lifecycle_verified_binding_projects_in_progress_and_preserves_facets`.
   Fixture `verified-binding-in-progress`; every input facet is
   present unchanged in the view and Project `Todo` is reported, not
   overwritten.
4. `work_lifecycle_pairwise_conflicts_match_golden_oracle`. Every row
   of §6 deep-equals its `.result.json`.
5. `work_lifecycle_edge_cases_match_golden_oracle`. Every row of §5.
6. `work_lifecycle_held_precedes_ready_and_backlog`. For every
   combination of `high_or_above`, `owner_assigned`,
   `explicitly_prepared`, `do_now` over `parked` and `blocked`, the
   column stays Parked/Blocked.
7. `work_lifecycle_stale_evidence_never_promotes`. For each facet,
   marking it stale never moves the column up the list Backlog <
   Ready, never into In Progress or Done, and never out of
   Parked/Blocked (D4).
8. `work_lifecycle_claim_binding_requires_both_sides`. The six §3.1
   conditions, each failed alone, give `session_binding = "unknown"`
   or `fresh = false` with the §3.3 reason.
9. `work_lifecycle_lease_expiry_uses_read_time_not_wall_clock`.
   `lease_until` between `observed_at` and `now`: not live; `now`
   earlier than `observed_at` with a live lease: still live.
10. `work_lifecycle_error_or_unknown_status_yields_null_column`.
    Broker error, unknown spelling, and nothing sourced each give
    `null` with the R0 reason.
11. `work_lifecycle_absent_fleet_task_is_not_ready`. No task: never
    Ready, never In Progress; `no_fleet_task`.
12. `work_lifecycle_preserves_source_spellings`. `expedite=15`,
    `next-up=25`, unknown `wait_kind`, unknown `reason_class` and
    unknown queue status are carried verbatim; nothing maps to
    `do_now`.
13. `work_lifecycle_matrix_invariants`. Model-based product of
    queue status {absent, error, the eight spellings, unknown} ×
    lease {none, live, expired} × binding {none, broker-only,
    verified, mismatch} × adapter {absent, error, stale, matching,
    other, terminal} × remote {none, issue open, issue closed, PR
    open, PR draft, PR merged} × receipt {absent, true} × policy bits
    {none, High, owner, prepared, triage} × freshness {live, stale}.
    Asserts the §4.4 invariants on every element.
14. `work_lifecycle_is_deterministic_and_pure`. Shuffled arrays and
    key order give byte-identical output; the module source contains
    no `Date.now(` and no argument-less `new Date(`.

Negative coverage named by the issue:

- Stale input: tests 7, 4 (`ready-stale`), 9.
- Duplicate/replay: test 14 (a pure read is idempotent).
- Ambiguous resource: tests 5 (multiple bindings rows) and 8; work
  identity ambiguity itself stays MOB-01's `identity_ambiguous`.
- Unknown version: test 10 (unknown queue spelling); an unknown input
  `snapshot_version` is MOB-01's `invalid` facet, which this policy
  treats as `error`.
- No false verified/completed result: tests 1, 2, 3, 13 (invariants
  1, 2, 7).
- Unauthorized actor: not applicable to a pure function; the read
  route's Access gate belongs to the slice that adds the route
  (MOB-01 record D1).
- Partial remote effect: not applicable to a read; MOB-09 owns
  desired-versus-observed.

Run: `cd pm-stack/aggregator && bun test test/work-lifecycle-policy.test.ts`.
List first with `bun test test/work-lifecycle-policy.test.ts
--test-name-pattern work_lifecycle`, which must report 14 tests. The
MOB-01 record §5 states that the run happens remotely (a
`workflow_dispatch` of `pm-aggregator-tests.yml` on the
implementation branch, or `fleet queue dispatch` on a GitHub-hosted
runner); this slice follows the same rule.

## 8. Requirement trace

| ID                        | Source (path:line)                                                                                   | What it asks                                                                                                                                     | Where this record answers                                                                                                              | Coverage                                                                                                      |
| ------------------------- | ---------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------- |
| C-R080 (C-108)            | `sources/02_charter_v1_1.md:76` (§7 heading); crosswalk row `census/requirements-crosswalk.jsonl:80` | Mobile-first interactive operations                                                                                                              | D1, D3, §3, §4: one explainable column per item for the mobile board                                                                   | Partial: the column and explanation only; controls, layout and receipts are MOB-03 to MOB-06                  |
| H-R011 (H-138)            | `sources/01_handoff_pm_stack_fleet_orchestration.md:103`; crosswalk `:178`                           | Project/epic lifecycle and strategic urgency change effective order, with manual intervention and owner escalation, not arithmetic sorting alone | D3 R2/R3/R7, invariant 6: urgency never overrides held state; `owner_assigned` and `triage_required` are explicit inputs, not computed | Partial: MOB-02 bounds where urgency may act; effective order itself is PRI-01 to PRI-04                      |
| O-R017 (O-688 to O-698)   | `sources/04_recovered_original_combined_design.md:13` (§17); crosswalk `:247`                        | Project/epic phase, deadlines and dependencies influence order selectively; board explains item and project urgency                              | D2 keeps urgency facets separate and unmodified; R3 makes hard dependencies a hold                                                     | Gap: project-level urgency display (O-697) is not produced here; it belongs to PRI-03/PRI-04 and MOB-03 views |
| C-6 (slice text)          | `sources/02_charter_v1_1.md:70,72`                                                                   | Columns, Ready definition, triage separation, parked/blocked retain history                                                                      | D3, D9 items 2-3, §5                                                                                                                   | Design-level complete for the column rule                                                                     |
| C-7 (slice text)          | `sources/02_charter_v1_1.md:78-80`                                                                   | Show accepted/running/blocked/conflicted/verified separately; no client policy                                                                   | D1 (policy in one module, no client recomputation), D7                                                                                 | Partial: operation outcomes are MOB-05                                                                        |
| H-SURVIVE-A / H-SCENARIOS | `sources/01_handoff_pm_stack_fleet_orchestration.md:51-59`                                           | Stale lease, blocked/parked resume, live launch/bind proof scenarios survive refinement                                                          | §3, §5 (lease expired, bind proof, parked with binding)                                                                                | Partial: scenario rows for the read side only                                                                 |
| O §12.2 to §12.4          | `sources/04_recovered_original_combined_design.md:13`                                                | Column definitions, precedence, picked-up and parked/blocked visibility                                                                          | D3, §3, §5                                                                                                                             | Design-level complete for projection; the Project write in §12.3 is the orchestrator's (out of scope)         |

The issue's curated trace marks all three IDs `design-only` with
missing evidence "exact implementation/parser tests for the full
behavior" and "deployment/live acceptance". This record specifies the
tests (§7) but supplies none of that evidence.

## 9. Ownership and boundaries

- **Owned (per the issue):** new `src/work-lifecycle-policy.ts`, new
  `test/work-lifecycle-policy.test.ts`, new fixtures under
  `test/fixtures/work-lifecycle/`.
- **Needed edit outside that list:** the D1 wiring in
  `src/work-projection.ts` (D9 item 7, §11 Q5).
- **Not touched:** `queue-view.ts`, `status-taxonomy.ts`,
  `work-items.ts`, `server.ts`, the projector, the Fleet crates. Their
  single-signal In Progress and facet-folding taxonomy stay as the
  existing pages' behavior; moving pages onto `PmWorkView` is MOB-03.
- **Added nowhere:** no database, queue, store, cache, endpoint,
  scheduler, GitHub client, session adapter, Project writer or
  creation-outbox producer.
- **Not decided here:** what a session adapter returns (MOB-07), the
  `high_or_above` predicate (#9200), what the completion receipt
  requires (the completion policy, #9209 domain), claim and park
  semantics (#9178, #9179).

## 10. Compatibility and reversal

Additive. The policy reads existing stored spellings and never
writes. Removing it returns `projection.column` to the MOB-01 default
(`null` with `column_policy_not_installed`, MOB-01 D6) or to PR
#15188's inline rule, whichever is on main. No queue status, Project
field, label or persisted row changes. `expedite=15` and
`next-up=25` stay distinct from `do_now`.

## 11. Open questions

1. **Staleness bounds.** Production values of
   `stale_after_ms.fleet_broker` and `.session_adapter` are not set
   here. The lease duration and heartbeat cadence of the claimant are
   the natural anchor, but this record does not assert their values.
2. **Claim-to-bind window.** The claimant flow binds after launch
   (`turso_queue/lifecycle.rs:1442`, "collect -> launch -> bind" in
   the #9046 comment at `:1525-1528`), so a just-claimed task shows
   Inbox/Triage until bind and readback both land. A grace window
   would need a measured bind latency; none is cited.
3. **Follow-up versus required reconciliation.** Whether an open
   spawned follow-up can block Done depends on the receipt policy's
   list of required reconciliations. MOB-02 treats only own issue,
   linked PRs and queue state as contradicting (R1).
4. **Cancelled and failed placement.** The charter's eight columns
   have no Cancelled or Failed column. This record sends both to
   Inbox/Triage (never Done, never Backlog). A view may filter
   `queue_cancelled` with a closed issue; whether the board should
   instead drop such items is for MOB-03 and the owner.
5. **Owned-file grant.** The issue's normalized ownership omits
   `work-projection.ts`, which the component slice (line 173) and the
   MOB-01 record §6 include. Either the implementation issue adds it,
   or MOB-01's PR #15188 lands the `column_policy` injection point so
   MOB-02 needs no edit there.
6. **`handed_off` rows.** CLI-01 maps `handed_off` to workflow
   `active` (lossy, `work/lifecycle.rs:413-422`). MOB-02 does not show
   it as In Progress without R4 evidence; whether an unbound sink row
   is Backlog or Ready depends only on the broker's eligibility read.
7. **Broker-stamped time.** §3.1 step 3 assumes the broker read's
   `observed_at` is stamped by the broker. If the reader stamps it
   client-side, the clock oracle of FDR-PMO-003 is only approximated;
   REL-02's versioned read is the expected fix.
8. **Split GitHub facet.** PR #15188's single `github_state` must
   become the MOB-01 record's `issue_state` plus `pr_states[]` (with
   `draft`) before R1/R5 can be exact (D9 item 5).

## 12. Gates and sequencing

- Design PR #9225: merged.
- Hard blockers named by the issue: MOB-01 (#9468, open; its
  implementation PR #15188 is open with incomplete verification),
  PRI-01 (#9478, closed; `work/priority.rs` on main), #9070 (closed).
- Runtime gates, not blockers for offline fixtures: #9178 and #9179
  (both open) before any mutation or cutover that depends on claim,
  takeover, resume or park semantics.
- Contract dependency: #9200 (open) for `high_or_above`. Until it
  defines the predicate, that input is absent and Ready can come only
  from `explicitly_prepared` or `owner_assigned`.
- Implementation starts only on a separately scoped issue after this
  record merges. Wiring a live read route, any persisted field,
  telemetry, and any deployed or live acceptance each need their own
  evidence and approval.

## 13. Out of scope

- Any write: Project #4 Status (the orchestrator is the single
  writer; original §12.3's "update the task-level Project status to
  In Progress" is its job), labels, comments, queue transitions,
  session binding.
- The read model's fact envelope, identity and freshness roll-up
  (MOB-01).
- Board views, filters, layout and accessibility (MOB-03, MOB-04).
- Actions, previews, receipts (MOB-05, MOB-06); session capability
  panel and adapter readbacks (MOB-07); desired/observed
  reconciliation (MOB-09).
- Urgency, effective pressure, project/epic urgency and ordering
  (#9199, #9200, PRI-02 to PRI-04).
- Claim, lease, fence and park semantics (#9178, #9179, REL-01,
  REL-02, SEL-02).
- Changing `queue-view.ts`, `status-taxonomy.ts` or any existing page.
