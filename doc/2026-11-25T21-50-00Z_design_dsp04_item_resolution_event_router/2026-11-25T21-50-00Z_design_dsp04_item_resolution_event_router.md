# DSP-04 — Item resolution, event router and policy decision table

Issue: [#9446](https://github.com/developing-today/code/issues/9446)
(slice `DSP-04` of [#9203](https://github.com/developing-today/code/issues/9203),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the DSP-04 row of the merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225). The row asks to "resolve canonical issue/PR/task and route
to status/triage/active-session/direct-start/blocked actions without
confounding hierarchy and dependencies", and names six cases that
must each give "a deterministic explainable result": an ambiguous
cross-repo id, a PR vs issue mismatch, no task, duplicate tasks,
hard vs advisory dependencies, and a hidden manual-only item. It
never gives the resolution steps, the decision table, or what
"explainable" has to contain. This record _decides_ them. It
authorizes no code, schema, deployment or live routing; those remain
separately gated stages (see §9).

Governing decisions consumed, not re-opened:

- **Component #9203 §3.2 processing order**: … → strict command parse
  → canonical target resolve (issue vs PR, repo, linked Fleet work,
  parent vs blocker relations) → authorize this principal for this
  verb and resource → idempotency lookup scoped to that authorization
  (never disclose a prior receipt before auth) → fresh eligibility,
  lease, dependency and capacity checks → reservation → effect. "A
  resolution failure or ambiguous link returns shared `Result` as
  `rejected` with `ambiguous_target` reason and candidate refs; do not
  choose nearest issue." "Never infer a cross-repo target from a bare
  number."
- **Component #9203 §3.6 and §1 boundaries**: `do-now` bypasses
  waiting only and "never rewrites numeric urgency or priority
  label"; hard-blocked work answers `rejected` with
  `ineligible_blocker` and blocker refs; `manual_only` is an
  eligibility gate, and an explicit owner direct start may use the
  #9178 manual-claim mode without clearing it; a parked task needs an
  explicit authorized start or resume under #9178/#9179; no implicit
  redirection of another item onto a live session.
- **Shared contracts** (`work/contracts.rs`, read-only here):
  `WorkRef (authority_id, work_id)`, `CommandResult`, the eight
  `Outcome`s, and the #9200 reason-code registry, whose
  `stale_snapshot` and `unauthorized_input` codes are reused as is.
- **DSP-05 record §4**: the existing-binding lookup (`BindingView`,
  derived, never stored) is how a live session is found.
- **DSP-06 record §4–§5**: the gates are re-read at the atomic claim;
  DSP-04's decision is the first read, never the last.
- **Queue broker as it stands** (`pkgs/fleet/book/reference/`):
  typed dependency blocks in `blocked_deps`
  (`pr | issue | task | ci`), `dep_blocked` state, broker-run
  recheck; `manual_only` as a top-level task flag shown as
  `(manual-only)` in listings; park, block and lease semantics.

Verified state on `origin/main` (source facts, not deployment proof):
`work/relations.rs` does **not** exist yet; DEP-04 is its shared-file
predecessor and creates it. `work/operations.rs` holds DSP-01's
`ResourceRef { repo, number }`, which carries no kind (issue or PR)
and no authority; DSP-04 widens resolution, not that struct's role
as the command's locator. No routing decision table exists anywhere.

## 1. Intent

Every inbound command reaches exactly one of seven actions, or is
refused, by a pure function over one pinned read of the world, and
the answer says which target it chose, which rule fired and what it
read. Two requests over the same snapshot get the same answer. No
row of the table changes priority, and no refused row has any
effect.

## 2. Ownership

| Concern                                                           | Owner                | DSP-04 role                            |
| ----------------------------------------------------------------- | -------------------- | -------------------------------------- |
| Resolution steps, snapshot, router, decision table, route reasons | `work/operations.rs` | writes (with DSP-05/06/07 on the file) |
| Relation read contract (`RelationView`)                           | `work/relations.rs`  | writes the read side, after DEP-04     |
| Typed edge kinds and canonical direction                          | DEP-02 / DEP-04      | consumes                               |
| `WorkRef`, `CommandResult`, #9200 reason registry                 | `work/contracts.rs`  | reads only                             |
| Signature, actor identity                                         | DSP-01               | consumes                               |
| Closed verb grammar                                               | DSP-02               | consumes                               |
| Idempotency ledger                                                | DSP-03 / REL-03      | runs after DSP-04's authorize step     |
| Existing binding and send                                         | DSP-05               | routes to it                           |
| Reservation, launch, bind                                         | DSP-06               | routes to it                           |
| Claim, manual-claim mode, takeover; park and resume               | #9178, #9179         | routes to their explicit operations    |
| Genealogy read model (ancestors, sentence)                        | #9070 (closed)       | read for display only, never a gate    |

## 3. Input

All three sources (verified webhook comment, PM Stack action,
authenticated CLI or service call) arrive as one shape:

```text
RouteRequest {
  source:        "github_comment" | "pm_action" | "cli",
  principal:     authenticated principal (DSP-01),
  verb:          closed DSP-02 verb,
  locator:       the target as the caller wrote it,
  origin_repo:   the repository the event came from (comments only),
  event_ref:     delivery id, comment node id and version (comments only),
}
```

Accepted locator forms, nothing else:

| Form                              | Meaning                                                                            |
| --------------------------------- | ---------------------------------------------------------------------------------- |
| `owner/repo#N`                    | qualified GitHub number; kind (issue or PR) is read, never assumed                 |
| implicit (the comment's own item) | a slash command in a comment targets the issue or PR it is on                      |
| `#N`                              | only inside a comment, and only in `origin_repo`; elsewhere `ambiguous_target`     |
| `fleet://<authority>/work/<id>`   | the canonical `WorkRef`; `fleet://work/<id>` only when one authority is configured |
| `task:<id>`                       | a queue task id, resolved back to its work                                         |

A bare `N` or a `#N` from a CLI or PM action is never resolved
against a default repository (§3.1 "never infer a cross-repo target
from a bare number"). It answers `ambiguous_target` with no lookup.

## 4. Resolution

Resolution reads; it never writes. Its reads are gathered into one
`ResolutionSnapshot` with a version (the queue read's high-water mark
plus the provider read times), so the router in §5 is a pure
function of `(RouteRequest, ResolutionSnapshot)`.

| Step | Reads                                                                                                | Can end with                                                                                                           |
| ---- | ---------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- |
| R1   | locator syntax                                                                                       | `ambiguous_target` (bare number), `invalid_locator`                                                                    |
| R2   | provider item: exists, kind (issue or PR), state, repository id                                      | `not_found` (also when the principal cannot see it, §4.1); kind recorded                                               |
| R3   | canonical work: the queue tasks whose canonical key is this item, live and terminal                  | zero live tasks → `no_task` (routed, not refused, §5); more than one live task → `duplicate_tasks` with all candidates |
| R4   | relations (§4.2): parents and children; hard blockers; advisory links                                | recorded; never ends resolution                                                                                        |
| R5   | task state: status, `manual_only`, lease holder and lane, `blocked_deps`, park reason; `BindingView` | recorded; never ends resolution                                                                                        |

**PR vs issue.** The kind read at R2 wins over the caller's guess.
When the verb needs the other kind, the answer is `kind_mismatch`,
with the linked items of the right kind as candidates (closing
references and the PR's linked issue). A verb that works on both
kinds (status, triage, park) resolves the item it was given. A
`/fleet start` on a PR resolves to the PR's own work when it has a
live task; otherwise it answers `kind_mismatch` with the closing
issues as candidates. It never picks one.

**Duplicates.** Two live tasks for one item is a data defect, not a
choice: the answer is `duplicate_tasks` with both task ids, their
states and holders. Terminal tasks are listed as history and never
count as duplicates.

### 4.1 Visibility before disclosure

Resolution reads with the service's access, but the answer is
filtered by what the principal may read: candidates the principal
cannot see are dropped, and when every candidate is invisible the
answer is `not_found`, the same as for a missing item. This keeps
§3.2's rule that nothing about an item, a receipt or a candidate is
disclosed before authorization. Authorization for the verb itself
still runs after resolution, as §3.2 orders it.

### 4.2 Hierarchy is not dependency

`RelationView` is read-only and keeps three lists apart:

| List            | Sources                                                                       | Effect on routing                                      |
| --------------- | ----------------------------------------------------------------------------- | ------------------------------------------------------ |
| `hierarchy`     | native parent and sub-issues; epic membership                                 | **never** blocks; shown for context                    |
| `hard_blockers` | native `blockedBy` on GitHub; queue `blocked_deps` entries; DEP-02 hard edges | an open hard blocker makes start and resume ineligible |
| `advisory`      | text mentions ("depends on", "related"), labels, DEP-02 advisory edges        | **never** blocks; listed as advisory in the answer     |

A parent that is open does not block its child; a child that is open
does not block its parent; neither is ever promoted to a blocker by
the router. A hard blocker that cannot be read (provider error) is
not treated as cleared: the item is `ineligible_blocker` with the
blocker marked `unknown`. The edge kinds and their canonical
direction are DEP-02's; DSP-04 needs only these three lists.

## 5. The decision table

Actions: `status` (read and report), `triage` (hand to triage with
the reason), `send_to_session` (DSP-05), `direct_start` (DSP-06),
`resume` (#9179's explicit resume, then DSP-06), `blocked_answer`
(explain, change nothing), `reject`.

Rows are checked top to bottom; the first match decides. "Any" means
every verb not matched by an earlier row.

| #   | Condition                                                | Verb                                                  | Action            | Reason / outcome                                                                    |
| --- | -------------------------------------------------------- | ----------------------------------------------------- | ----------------- | ----------------------------------------------------------------------------------- |
| 1   | R1/R2 failed                                             | any                                                   | `reject`          | `ambiguous_target`, `invalid_locator` or `not_found`; `rejected`                    |
| 2   | `kind_mismatch`                                          | any                                                   | `reject`          | `kind_mismatch` with candidates; `rejected`                                         |
| 3   | `duplicate_tasks`                                        | any but `status`                                      | `reject`          | `duplicate_tasks` with candidates; `rejected`                                       |
| 4   | principal not authorized for verb and resource           | any                                                   | `reject`          | `unauthorized_input`; `rejected`, no candidate detail                               |
| 5   | any                                                      | `status`, `agent status`                              | `status`          | report state, relations and binding; `verified_complete` for the read               |
| 6   | live bound session (`BindingView` live and capable)      | `start`, `resume`, `retry`, `do-now`, actionable text | `send_to_session` | instruction goes to the session; never a second launch (C-R089, O-R051)             |
| 7   | open or `unknown` hard blocker                           | `start`, `resume`, `retry`, `do-now`                  | `blocked_answer`  | `ineligible_blocker` with blocker refs; `rejected`                                  |
| 8   | `manual_only`, no manual-claim mode named                | `start`, `retry`, `do-now`                            | `blocked_answer`  | `manual_only`; `rejected`; the flag and the hidden status are reported, not cleared |
| 9   | `manual_only`, manual-claim mode named and authorized    | `start`                                               | `direct_start`    | via #9178 manual-claim mode; flag left set; override recorded                       |
| 10  | parked                                                   | `resume`                                              | `resume`          | #9179 explicit resume; park reason and checkpoint carried                           |
| 11  | parked                                                   | `start`, `retry`, `do-now`                            | `blocked_answer`  | `parked`; names `resume` as the operation to use                                    |
| 12  | lease held by another worker or session                  | `start`, `retry`, `do-now`                            | `blocked_answer`  | `lease_held` with the holder; `conflict`; never a takeover                          |
| 13  | task terminal                                            | `start`, `retry`, `do-now`                            | `blocked_answer`  | `terminal`; names `retry` only where #9178 allows it                                |
| 14  | `no_task`                                                | `start`, `do-now`                                     | `direct_start`    | DSP-06 creates the task under the operation's dedupe key                            |
| 15  | `no_task`                                                | `triage`, actionable text                             | `triage`          | `no_task`; triage creates nothing by itself                                         |
| 16  | eligible, queued                                         | `start`, `do-now`                                     | `direct_start`    | DSP-06; `do-now` consumed once                                                      |
| 17  | eligible, queued                                         | `triage`                                              | `triage`          | —                                                                                   |
| 18  | any                                                      | `priority`                                            | `reject` here     | `routed_elsewhere`: priority is #9200's operation; DSP-04 never writes it           |
| 19  | any                                                      | `park`, `unblock`, `merge`                            | `reject` here     | `routed_elsewhere`: their own owners (#9179, DEP, merge policy)                     |
| 20  | plain-language text that the classifier could not settle | —                                                     | `triage`          | `low_confidence`; no direct effect (§3.7)                                           |

Rows 18–19 make the table total without pretending DSP-04 owns those
verbs: the router names the owning operation and does nothing else.

**`do-now` never mutates priority.** No row emits a priority or label
effect. `do-now` changes only which action a row picks (rows 6 and 16) and is recorded as the one-shot directive; the oracle checks
every `do-now` row for the absence of any priority effect.

**Re-read at effect time.** The decision carries the snapshot
version. DSP-06 and DSP-05 re-read before their first effect; when
the state behind the chosen row has changed, the operation stops
with `stale_snapshot` and is routed again, never carried through on
the old row.

## 6. What "explainable" contains

```text
RouteDecision {
  action:            one of the seven,
  target:            WorkRef and locator, or null,
  row:               the table row number that fired,
  reason:            RouteReason (§6.1),
  candidates:        [ {locator, kind, work, state} ] filtered by visibility,
  relations:         RelationView summary (hierarchy, hard, advisory),
  manual_only:       bool,
  binding:           BindingView summary or null,
  snapshot_version:  string,
  evidence_refs:     the reads behind each condition the row tested,
}
```

The answer is returned as the shared `CommandResult` with this
decision in its effects (field `route`) and `next_action` naming the
operation to use when the row refused (for example `resume` for row
11).

### 6.1 Route reasons

DSP-04 adds a closed `RouteReason` set in `operations.rs` and does
not widen the #9200 priority registry, whose codes govern priority
effects: `ambiguous_target`, `invalid_locator`, `not_found`,
`kind_mismatch`, `duplicate_tasks`, `no_task`, `ineligible_blocker`,
`manual_only`, `parked`, `lease_held`, `terminal`,
`routed_elsewhere`, `low_confidence`. Two registry codes are reused
by their existing wire strings, not redefined: `unauthorized_input`
and `stale_snapshot`.

## 7. Commands

The router serves `agent.dispatch`, `agent.slash.start` and the
status and triage verbs of the DSP-02 grammar; CLI-13 registers the
CLI spellings. A `--explain` flag (C-R108's common flag) prints the
`RouteDecision` without taking the action, and a `--dry-run` routes
without reserving.

## 8. Failure modes

| Case                                      | Answer                                                                                    |
| ----------------------------------------- | ----------------------------------------------------------------------------------------- |
| provider unreachable at R2                | `uncertain`, `next_action` names the read; no guess from cache                            |
| queue unreachable at R3/R5                | `uncertain`; never "no task" from a failed read                                           |
| relation read fails at R4                 | the item is ineligible for start (`unknown` blocker), status still answers                |
| item moved or transferred to another repo | the provider's redirect is followed once and recorded; a second hop is `ambiguous_target` |
| comment edited after routing              | a new versioned candidate (§3.3); the earlier decision stands in its receipt              |

## 9. Persisted fields, acceptance and gates

Persisted fields: none. The snapshot and the decision live in the
operation's receipt (REL-03); `RelationView` and `BindingView` are
derived reads.

Tests (first red is the slice's starting test), all over generated
snapshots with no network:

| Test                                                                | Proves                                                                        |
| ------------------------------------------------------------------- | ----------------------------------------------------------------------------- |
| `pmo_dsp_04_ambiguous_work_reference_rejects_before_effect` (first) | ambiguous cross-repo id rejects with candidates and no effect                 |
| `pmo_dsp_04_bare_number_never_crosses_repo`                         | `N`/`#N` outside a comment never resolves                                     |
| `pmo_dsp_04_pr_issue_mismatch_explained`                            | `kind_mismatch` with the linked items as candidates                           |
| `pmo_dsp_04_no_task_routes_by_verb`                                 | rows 14 and 15                                                                |
| `pmo_dsp_04_duplicate_tasks_reject_with_candidates`                 | row 3; terminal tasks are not duplicates                                      |
| `pmo_dsp_04_hierarchy_never_blocks`                                 | an open parent or child never makes start ineligible                          |
| `pmo_dsp_04_hard_blocks_advisory_does_not`                          | row 7 for hard and `unknown`; advisory listed only                            |
| `pmo_dsp_04_hidden_manual_only_explained`                           | rows 8 and 9; the flag is reported and left set                               |
| `pmo_dsp_04_do_now_never_mutates_priority`                          | no `do-now` row emits a priority or label effect                              |
| `pmo_dsp_04_candidates_filtered_by_visibility`                      | invisible candidates dropped; all invisible → `not_found`                     |
| `pmo_dsp_04_live_session_gets_send_not_launch`                      | row 6                                                                         |
| `pmo_dsp_04_table_is_total`                                         | every verb and state reaches exactly one row                                  |
| `pmo_dsp_04_decision_is_deterministic`                              | same request and snapshot, same decision                                      |
| `pmo_dsp_04_refused_rows_have_no_effect`                            | model-based: every `reject` and `blocked_answer` leaves the doubles unchanged |
| `pmo_dsp_04_stale_snapshot_reroutes`                                | a changed state between route and effect stops with `stale_snapshot`          |

Command:
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker pmo_dsp_04_ambiguous_work_reference_rejects_before_effect`,
after proving with `-- --list` that the `pmo_dsp_04` tests are listed
(a zero-test filter is not a pass).

Gates:

- **Hard blockers:** DSP-01 (principal), #9070 (closed; genealogy
  read model, display only), PRI-01 (priority operation that rows
  18 routes to), CLI-02.
- **Shared-file predecessor:** DEP-04 on `work/relations.rs`.
- **Runtime gates:** #9178 and #9179 semantics verified before any
  mutation path uses rows 9 and 10; offline tables proceed now.
- **Stages each needing their own evidence and approval:** wiring
  the router into the projector forwarder and pm-dispatch,
  deployment, and live routing.
- **Rollback:** disable the router; the existing direct verbs keep
  working as today. No history or state is removed.

## 10. Requirement trace (design-level)

| ID                                     | Outcome (crosswalk, abridged)                                                                      | Where                  |
| -------------------------------------- | -------------------------------------------------------------------------------------------------- | ---------------------- |
| C-R005, C-R107, P-R009, H-R017, O-R023 | Cohesive Fleet CLI; names reconciled; no parallel dispatch mechanism                               | §7                     |
| C-R009                                 | Agents discover work, understand priority and dependencies, claim safely                           | §4, §5                 |
| C-R087                                 | Small authenticated slash grammar                                                                  | §3, §5                 |
| C-R089                                 | An actionable live-issue comment routes to the current session                                     | §5 row 6               |
| C-R091                                 | Verify → authorize → resolve → dedupe → find live session → send or dispatch                       | §4, §5                 |
| C-R108                                 | Common flags including `--explain`, `--dry-run`                                                    | §7                     |
| C-R152                                 | Scenario C: a live comment routes to the bound session                                             | §5 row 6               |
| H-R016                                 | Fresh work, parked resume, late comment, duplicate webhook, stale lease, conflict                  | §5, §8                 |
| H-R021                                 | Verify ids and records; reuse an active compatible session                                         | §4, §5 row 6           |
| H-R024, H-R031                         | Original commands, errors and failure cases stay in coverage                                       | §5, §8                 |
| H-R038, H-R051                         | Sanctioned issue and task creation paths                                                           | §5 row 14 (via DSP-06) |
| H-R039, H-R052                         | Verify created ids and task records; pending intents are not evidence                              | §4 R3                  |
| H-R040, H-R053                         | Discover the actual route; never invent or silently switch                                         | §5 rows 18–19          |
| O-R011                                 | do-now is a one-shot request that preserves blockers                                               | §5 rows 7, 16          |
| O-R014                                 | Authenticated commands; replay and audit controls; visible failures                                | §4.1, §6, §8           |
| O-R051                                 | Resolve a phone or comment request, route to a live session, do not queue when the session can act | §5 row 6               |
| S-R004                                 | Authorized bot, webhook and phone controls; comments route to a live session                       | §3, §5                 |

## 11. Non-goals

- No edge model and no dependency writes (DEP-02/DEP-04).
- No priority writes (PRI-01, #9200); no park, unblock or merge
  semantics (their owners).
- No new reason codes in the #9200 registry.
- No nearest-match, title-similarity or default-repository guess.
- No schema change and no stored routing state.
