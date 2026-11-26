# SEL-02 — Typed claim revalidation: a stale selection never becomes a lease

Issue: [#9492](https://github.com/developing-today/code/issues/9492)
(slice `SEL-02` of [#9204](https://github.com/developing-today/code/issues/9204),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the claim-revalidation slice of the merged
design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the claim shapes, the inputs revalidated at commit,
the typed outcomes, and the compatibility contract a separately
scoped implementation issue can be filed against. Component
#9204 V2 says "claim atomically rechecks work version, both
predicates, eligibility, graph/policy version, lease and lane
constraints; if the snapshot changed, recompute or return
stale/retry, never take stale head"; `architecture.md` says "a
projection or sampled candidate cannot grant a lease". This record
_decides_ which of those inputs are re-evaluated live, which are
compared to the chooser's snapshot, what the broker returns when
they disagree, and how that lands in the existing
`turso_queue` claim path without a second claim mechanism. It
authorizes no code, schema, migration, deployment or live
acceptance; those remain separately gated stages (§10).

Governing decisions consumed, not re-opened:

- **Component #9204 V2** — the immutable `SelectionInput`
  snapshot; both predicates (`TaskRequirements AND
ClaimerFilter`); hard gates are exclusions, not weights;
  `do_now`/direct-session does not bypass auth, hard
  dependencies, lease, concurrency cap or claim guards; an
  exploration draw does not claim, and a stale selected item is
  marked `stale` and returned to normal selection with "no
  speculative extra claims".
- **`architecture.md`, "Ordering and distributed priority"** —
  the consistency boundary is the broker's claim transaction;
  validate item version, graph revision, policy/profile
  revision, ownership and eligibility there; a changed input
  means recomputation or a typed stale result; an adapter that
  cannot validate atomically cannot advertise the capability.
- **Component #9209 / REL-02** ([#9487](https://github.com/developing-today/code/issues/9487))
  — each claim or takeover increments `fence_generation`, the
  claim transaction writes holder, lease, generation and work
  version, and "atomic claim revalidates current status,
  eligibility, dependency, lane requirements and lease at
  commit, not just earlier selection". REL-02 owns those
  columns; this record consumes them.
- **SEL-01** ([#9491](https://github.com/developing-today/code/issues/9491))
  — the pure reference selector in `work/selection.rs`, whose
  eligibility verdict and reason codes this slice re-runs at
  commit.
- **PRI-03** (record
  `doc/2026-11-23T15-34-52Z_design_pri03_peer_ordering_rank_rebalance/`)
  — the one comparator and the claim/list order parity
  (`... created_at ASC, id ASC`); the slice's "claim/list tie
  fixed by PRI-CMP-01" is PRI-03.
- **DEP-03** (record
  `doc/2026-11-25T01-14-01Z_design_dep03_hard_dependency_dag/`)
  — the hard-dependency `NOT EXISTS` clause added to the claim
  predicate and `dep_graph.revision`.
- **#9178** — the existing claim, `manual_only`, takeover and
  resume semantics. A non-blocking evidence gate for this slice:
  offline models may proceed, mutation and cutover wait for it.

## 1. Intent

A chooser — the broker's own pick, an exploration draw, a person
pressing "start" on a board, a directive, a lane resuming its
parked work — looks at the queue at one moment and claims at a
later one. In between, the item can be edited, claimed by someone
else, blocked by a new hard edge, made `manual_only`, or outranked
by newly urgent work. The failure this slice prevents is a lease
granted on what the chooser saw rather than on what is true at
commit.

The decision in one line: **eligibility is always evaluated live
inside the claim's own statement; the chooser's snapshot is
compared only for the inputs that justified the choice; any
disagreement is a typed result, never a different claim.**

## 2. The V1 baseline this builds on

Verified at the current `main` (source, not deployment proof):

- `claim_repo_task_turso` (`turso_queue/lifecycle.rs`) already
  picks first and claims by id: a `SELECT id ... ORDER BY ...
LIMIT 1` over the pick predicate, then
  `claim_update_sql(&pick_predicate)` — a conditional
  `UPDATE ... WHERE id = ? AND (pick predicate) AND
platform-limit clause AND scope`. If the row changed in
  between, the `UPDATE` affects 0 rows and the loop re-picks
  (bounded by a 32 race budget, #9609). This is the #7617 fix,
  and it is the right shape: the conditional `UPDATE` _is_ the
  revalidation for the broker's own pick.
- `claim_scope` puts the caller's filter (`filter_sql`) and the
  task's requirements (`TASK_REQUIREMENTS_CLAUSE`) into both the
  pick and the `UPDATE`, so both predicates are already
  rechecked at commit (#7068).
- After the `UPDATE`, two non-SQL gates run before any lease is
  written: `claim_guard_reason` quarantines a defective row, and
  the #10626 staleness check retires a row whose own linked
  issue or PR already landed.
- The pick order lacks a final `id` tie (PRI-03 repairs it).
- There is no `work_version` or `fence_generation` column yet
  (REL-02), no `dep_graph` (DEP-03), no exploration (SEL-03/04),
  and the by-id claim path names its row deliberately and checks
  requirements in Rust (`task_requirements_unmet`), not inside
  its `UPDATE`.

## 3. Two claim shapes, one claim clause

### 3.1 The claim clause

**Decision.** There is exactly one SQL expression that decides
"this row may be claimed by this claimer now": the **claim
clause**, the conjunction of

1. the pick predicate (`default_pick_predicate()` or
   `HANDOFF_SQL` by mode: status, attempts cap, `not_before`,
   parked-term exclusion, budget gate);
2. `PLATFORM_LIMIT_PICK_CLAUSE`;
3. the claim scope (queue, caller filter, task requirements);
4. DEP-03's hard-blocker `NOT EXISTS`;
5. the #9178 `manual_only` exclusion;
6. REL-02's lease predicate (no live lease held by another
   holder, judged by the store clock).

Every claim path — broker pick, by-id, handoff, resume — runs
its `UPDATE ... WHERE id = ? AND <claim clause>` with the same
clause built by the same function. A gate added to eligibility
is added once, to the clause, and every path inherits it. The
existing source-level test that the claim path builds its
`UPDATE` from the same predicate as its pick (in
`turso_queue.rs`) is widened to assert every claim path uses the
one builder.

The by-id path keeps its deliberate override of _ordering_ (it
names its row) but loses its separate Rust-side requirement
check as the only gate: the clause's requirement and filter terms
now bind it too. A by-id claim that the clause refuses returns
`ineligible` with reasons (§5); it does not quarantine or retry.

### 3.2 Shape A — broker pick

`queue claim`, `work next` and the lane's default pick. The
broker chooses and claims in the same call: the "snapshot" is the
pick `SELECT`, and the conditional `UPDATE` revalidates it. This
record changes only what the `UPDATE` re-checks (§3.1) and what
it writes (REL-02's generation and version bump); the
pick → `UPDATE` → re-pick loop, the race budget, the quarantine
budget and the staleness retirement stay as they are.

**Normal selection is always a broker pick.** No client, board,
mobile view or hosted surface may claim "the head of the queue"
by sending the id it saw at the head. A surface that wants the
next normal item calls the broker pick and receives whatever is
the head at commit. This is how "never take stale head" holds by
construction: there is no path on which a remembered head is
claimed.

### 3.3 Shape B — claim of a chosen item (`ClaimIntent`)

Every other claim names a specific item that someone else chose.
It carries the shared command envelope (`fleet.command/v1`,
`work/contracts.rs`) with:

| Field                   | Meaning                                                                                                |
| ----------------------- | ------------------------------------------------------------------------------------------------------ |
| `work`                  | `(authority_id, work_id)` pair; never a bare number at another authority                               |
| `expected_work_version` | REL-02 work version the chooser saw (canonical decimal string)                                         |
| `observed_snapshot_ref` | the chooser's snapshot: queue snapshot, `graph_revision`, `policy_ref`, `comparator_ref`, lane context |
| `lease_generation`      | for `work resume` only: the fence generation the resumer last held                                     |
| `reason.code`           | the intent's **source**: `exploration`, `directive`, `operator_pick`, `resume`, `handoff`              |
| `operation_id`          | idempotency key (§4.4)                                                                                 |

The source decides which snapshot inputs are compared (§4.2).
Shape B never chooses a substitute: when revalidation fails it
returns a typed result and the caller decides again (#9204: "no
speculative extra claims").

## 4. What is revalidated, and how

### 4.1 Live inputs — evaluated at commit, never compared

These decide _whether_ the item may be claimed at all. They are
read inside the claim's own `UPDATE` through the claim clause and
are never taken from the chooser's snapshot, so they cannot be
stale:

- status, attempts, timers, park terms, budget;
- both predicates (requirements and filter);
- hard dependencies (DEP-03 clause, which reads the current
  graph, so a mismatched `graph_revision` cannot leak a blocked
  item through);
- `manual_only`, platform limit;
- the lease and fence (REL-02).

Because these are live, a changed `graph_revision` alone is
**not** a reason to refuse a Shape B claim: if the item is still
not blocked under the current graph, the clause passes. This
matters at 12+ concurrent lanes, where some edge mutation happens
between almost any read and any claim; refusing on every graph
bump would turn a correct check into a livelock.

### 4.2 Choice inputs — compared to the snapshot, by source

These decide _why this item_ was chosen. They matter only for the
sources whose choice depended on them:

| Source          | Compared at commit                                                                                         | On mismatch                       |
| --------------- | ---------------------------------------------------------------------------------------------------------- | --------------------------------- |
| any             | `expected_work_version` = current REL-02 work version                                                      | `stale_item` (`VERSION_CONFLICT`) |
| `exploration`   | `policy_ref` and exploration profile version = current                                                     | `stale_policy`                    |
| `exploration`   | urgent gate (§4.3) recomputed for this lane                                                                | `preempted`                       |
| `directive`     | directive still active and its issuer still authorized (AUTH-01, protected-owner policy)                   | `stale_directive`                 |
| `resume`        | `lease_generation` equals the item's current fence generation (no takeover since the resumer last held it) | `fenced`                          |
| `operator_pick` | nothing beyond version: a person chose this item on purpose                                                | —                                 |
| `handoff`       | the sink row is still addressed to this claimant                                                           | `stale_item`                      |

`comparator_ref` and `graph_revision` are never compared for
Shape B: Shape B items are not chosen by order, and order-based
choice is Shape A only (§3.2).

The work-version check is exact. A version the store has never
issued (greater than current, malformed, or from another
authority) is `VERSION_CONFLICT` with the current version, never
treated as "newer is fine". A version bump caused by a field the
chooser did not look at is still a conflict: the retry is one
re-read, and a narrower "relevant fields" rule would need every
chooser to declare its field set, which none does.

### 4.3 The urgent gate for exploration

#9204: exploration may proceed only if no qualifying urgent work
that this lane could claim would be delayed. At commit, inside
the same `BEGIN IMMEDIATE` as the claim, the broker runs one
`SELECT id ... WHERE <claim clause for this lane> AND <urgent
class predicate> ORDER BY <PRI-03 order> LIMIT 1`. If it returns
a row other than the intent's item, the claim is refused
`preempted` with that row's ref, and the caller does a Shape A
pick (which will take the urgent row). The urgent classes come
from the active ratified policy profile by `policy_ref`; this
record defines no thresholds and does not reuse the historical
450/1000 categories.

### 4.4 Idempotency and replay

A Shape B claim is idempotent by `operation_id`:

- Replayed after success while the same holder still holds the
  lease at the generation the first call wrote: returns the
  first call's `claimed` result unchanged; no second
  generation bump, event or lease write.
- Replayed after the lease was lost or taken over: returns
  `held` or `fenced` with the current holder kind and
  generation — never re-claims under an old operation id.
- Replayed with a different intent body under the same
  `operation_id`: `idempotency_conflict`, nothing written.

The record of `operation_id → outcome` lives in the existing
command/operation journal the shared envelope already assumes
(JRN-01); this slice adds no table for it.

### 4.5 One transaction

A Shape B claim runs, in one `BEGIN IMMEDIATE` (the pattern
`reprioritize_task` already uses):

1. authorization of the authenticated principal for this source
   and item (AUTH-01; `actor_ref` is attribution, not proof);
2. identity: the `work` pair resolves to one row at this
   authority, else `ambiguous_ref` / `wrong_authority`;
3. idempotency lookup (§4.4);
4. the choice comparisons of §4.2, including the urgent gate;
5. `UPDATE tasks SET <claim fields, REL-02 generation + version
bump> WHERE id = ? AND <claim clause> AND work_version = ?`;
6. if 0 rows: the diagnosis in §5.2, then `COMMIT` of nothing but
   the refusal event;
7. if 1 row: lease row, binding, `claimed` event with the
   intent's source and refs, `COMMIT`.

The post-`UPDATE` guards of Shape A (`claim_guard_reason`,
staleness retirement) apply to Shape B as they do to a by-id
claim today: the staleness check stays default-pick-only (#10626
exempts deliberate by-id claims), and the quarantine guard runs
on every path.

No remote effect happens inside the transaction. GitHub
assignment, Project fields and comments that follow a claim go
through the outbox after commit (#9209); if they fail, the claim
stands and the result says the remote effect is `pending`, never
`verified`.

## 5. Outcomes

### 5.1 The typed result

Every claim returns the shared `fleet.result/v1` envelope with
one of these outcomes:

| Outcome                                                                    | Meaning                                                          | Carries                                                                     |
| -------------------------------------------------------------------------- | ---------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `claimed`                                                                  | lease granted                                                    | `work_version`, `fence_generation`, `lease_until`, remote effects `pending` |
| `stale_item`                                                               | work version moved since the chooser looked (`VERSION_CONFLICT`) | current version                                                             |
| `stale_policy`                                                             | exploration profile or policy changed                            | current `policy_ref`                                                        |
| `stale_directive`                                                          | directive withdrawn or issuer no longer authorized               | directive state                                                             |
| `preempted`                                                                | urgent work for this lane exists                                 | the urgent item's ref                                                       |
| `ineligible`                                                               | the claim clause refuses the item                                | reason codes (§5.2)                                                         |
| `held`                                                                     | another holder has a live lease                                  | holder kind (lane/session/person), `lease_until`                            |
| `fenced`                                                                   | resume after a takeover                                          | current generation                                                          |
| `nothing_claimable`                                                        | Shape A found no row (today's `None`)                            | —                                                                           |
| `unsupported`                                                              | this store or adapter cannot validate atomically                 | missing capability                                                          |
| `unauthorized`, `ambiguous_ref`, `wrong_authority`, `idempotency_conflict` | refused before any read of the item's state                      | —                                                                           |

None of these is a `claimed` with a caveat: every non-`claimed`
outcome wrote no lease, no generation bump and no `claimed` event.

### 5.2 Explaining a refusal

When the Shape B `UPDATE` affects 0 rows, the broker evaluates the
claim clause's conjuncts one by one on the same row inside the
same transaction and returns every failing one as a SEL-01 reason
code (`not_queued`, `timer_pending`, `requirements_unmet`,
`filter_mismatch`, `hard_blocked`, `manual_only`,
`platform_limited`, `lease_held`, `budget_short`, ...). The
diagnosis reads the same row the `UPDATE` just refused, so the
reasons describe the refusal, not a later state. The clause
itself is not changed to produce reasons; the diagnosis is a
second, read-only query.

### 5.3 Stale is not an error to retry blindly

A caller that receives `stale_*` or `preempted` re-reads and
decides again; the broker never loops on its behalf for Shape B.
Callers bound their own reselection (the exploration run marks
the candidate `stale` and returns to normal selection, as #9204
specifies). Shape A keeps its existing internal re-pick loop
because there the broker is the chooser.

## 6. Commands allocated to this slice

Per `census/command-slice-map.jsonl`: `current.claim.filter`,
`flag.claim`, `queue.claim`, `queue.fallback.claim`, `work.next`,
`work.open`, `work.resume`. Final names are reconciled by CLI-02 /
CLI-13; this record fixes only which shape each one uses:

| Command                                    | Shape | Source / note                                                                                                            |
| ------------------------------------------ | ----- | ------------------------------------------------------------------------------------------------------------------------ |
| `queue claim`, `work next`                 | A     | broker pick; existing flags unchanged                                                                                    |
| `queue fallback claim`                     | A     | the existing fallback/handoff pick mode (`HANDOFF_SQL`)                                                                  |
| `work open <work>`                         | B     | `operator_pick`; requires `--expected-version` (the version the operator's view showed)                                  |
| `work resume <work>`                       | B     | `resume`; carries the last `lease_generation`                                                                            |
| `--claim` on a read command (`flag.claim`) | B     | the read's returned version becomes `expected_work_version`, so "show then claim" is one intent                          |
| `current claim filter`                     | —     | read-only: prints the claimer's effective scope (filter AST and requirement args) exactly as the claim clause binds them |

The legacy by-id claim without a version stays available for
compatibility (§9) and reports `version_checked: false` in its
result, so a caller can tell which guarantee it got.

## 7. Persisted fields

**None new.** This slice consumes REL-02's `work_version` and
`fence_generation`, DEP-03's `dep_graph.revision`, PRI-03's
`peer_rank`, and the existing `events` table; refusal and
`claimed` events record the source, refs and outcome in their
existing JSON `detail`. Exploration runs and dispositions are
SEL-04's. No CEN-04 §5 admission entry is needed.

## 8. Failure and recovery

| Scenario                                                                                     | Outcome                                                         |
| -------------------------------------------------------------------------------------------- | --------------------------------------------------------------- |
| Two lanes claim the same item at once                                                        | One `claimed`; the other `held` (Shape B) or re-picks (Shape A) |
| Item edited between view and `work open`                                                     | `stale_item` with the current version                           |
| Hard edge added after the draw                                                               | `ineligible` / `hard_blocked`                                   |
| Graph changed elsewhere, item still unblocked                                                | `claimed` (graph is live, not compared)                         |
| Urgent item for this lane arrives after an exploration draw                                  | `preempted` with its ref                                        |
| Directive withdrawn before its claim                                                         | `stale_directive`                                               |
| Item made `manual_only` after a draw                                                         | `ineligible` / `manual_only`                                    |
| Old holder resumes after a takeover                                                          | `fenced`                                                        |
| Replay of a successful claim                                                                 | the original `claimed`, no second bump                          |
| Version from another authority                                                               | `wrong_authority`                                               |
| Forged `actor_ref` claiming to be the owner                                                  | `unauthorized`                                                  |
| Store lacks `work_version` (pre-REL-02) or a hosted adapter cannot do the conditional update | `unsupported`; no claim                                         |
| Outbox fails after commit                                                                    | claim stands; remote effect `pending`                           |
| Crash between `UPDATE` and lease write                                                       | impossible: one transaction; rollback leaves the row unclaimed  |

## 9. Compatibility and rollout

- Shape A's observable behaviour is unchanged except that rows
  newly excluded by DEP-03's clause and REL-02's lease predicate
  are no longer handed out, and order gains PRI-03's tie-break.
- The by-id path gains the clause's requirement/filter terms in
  its `UPDATE`. A by-id claim that succeeds today on a row whose
  requirements it does not meet would start returning
  `ineligible`; today's Rust-side check already refuses that
  case, so the visible change is the typed result, not the
  verdict.
- The legacy unversioned by-id claim remains for one compatibility
  window, reporting `version_checked: false`; removing it is a
  separate decision, not this slice.
- Rollback: turn off Shape B's comparisons (keep live clause);
  claims fall back to the clause alone, which is still at least as
  strict as V1. No data is migrated, so nothing is reverted.
- Hosted parity (#9079/#9081): a target that cannot run the
  conditional update with the work-version term returns
  `unsupported` for Shape B and must not advertise claim parity
  (`architecture.md`).

## 10. Gates and sequencing

Implementation of this slice starts only after:

- PR #9225 merged (done).
- SEL-01 (#9491): the pure selector and its reason codes.
- REL-02 (#9487): `work_version`, `fence_generation` and the lease
  predicate in the claim transaction.
- Shared-file sequencing from the package: REL-04 before this
  slice on `turso_queue.rs`, CLI-04 before it on
  `work/selection.rs`.
- DEP-03 implemented, for the dependency conjunct (the clause can
  ship without it and gain it when DEP-03 lands; the conjunct is
  additive).
- PRI-03 implemented, for the tie-break in Shape A (likewise
  additive).
- #9178 claim/manual/takeover/resume semantics verified before
  any mutation or cutover (runtime evidence gate; offline model
  tests may proceed first).

## 11. Acceptance oracle, specified

All are future tests, written failing first, listed by exact name
with `-- --list` before running (a zero-test filter is not
acceptance), under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`.

1. `pmo_selection_stale_claim_revalidates` — the slice's first
   failing test. A chooser reads item X at version v; X is edited
   (v+1); a Shape B claim with `expected_work_version = v`
   returns `stale_item` with v+1, writes no lease, bumps no
   generation and emits no `claimed` event; the same claim with
   v+1 succeeds.
2. `pmo_selection_one_clause_every_path` — source-level: every
   claim path's `UPDATE` is built from the one claim-clause
   builder.
3. `pmo_selection_concurrent_claims_one_winner` — two claimers on
   one item in parallel transactions: exactly one `claimed`, one
   `held`; one lease row; generation bumped once.
4. `pmo_selection_graph_bump_does_not_refuse` — unrelated edge
   added between read and claim; claim succeeds.
5. `pmo_selection_new_hard_edge_refuses` — hard edge onto X
   between read and claim; `ineligible` with `hard_blocked`.
6. `pmo_selection_exploration_preempted_by_urgent` — urgent item
   for the lane arrives after the draw; `preempted` with its ref;
   the following Shape A pick takes the urgent item.
7. `pmo_selection_directive_no_bypass` — a directive on a
   `manual_only` or hard-blocked item returns `ineligible`;
   directives skip order, never the clause.
8. `pmo_selection_replay_is_idempotent` — replay after success
   returns the original result; replay with a different body is
   `idempotency_conflict`.
9. `pmo_selection_resume_after_takeover_is_fenced` — old holder's
   `work resume` with its old generation returns `fenced`.
10. `pmo_selection_refusal_reasons_match_clause` — for each
    conjunct, a row failing only that conjunct returns exactly
    that reason code.
11. `pmo_selection_unknown_version_conflicts` — a version greater
    than current, malformed, or from another authority never
    claims.
12. `pmo_selection_no_remote_effect_in_claim` — the claim
    transaction performs no adapter call; remote effects appear
    only as `pending` outbox intents.

## 12. Requirement trace (design-level)

The slice's 30 allocated requirement IDs are carried by the
component trace in the issue body. At design level this record
addresses the slice's acceptance line from #9204: "at most one
concurrent claim" (§4.5, test 3); "stale graph/item/policy
rejected/recomputed" (§4.1–§4.2, tests 1, 4–6); "no bypass for
directive or exploration" (§4.2, tests 6–7); "claim/list tie fixed
by PRI-CMP-01" (§3.2, consumed from PRI-03); and the issue's
negative coverage — stale input (tests 1, 5), duplicate/replay
(test 8), ambiguous resource (§4.5 step 2), unauthorized actor
(§4.5 step 1), partial remote effect (test 12), unknown version
(test 11), no false verified/completed result (§5.1). Parser
tests, deployment and live acceptance remain open evidence.

## 13. Non-goals

- The selector itself, its reason codes and the urgent predicate
  (SEL-01), exploration sampling (SEL-03), draw persistence and
  dispositions (SEL-04), fairness (SEL-06).
- The fence and work-version columns and their bump rules
  (REL-02).
- A second claim mechanism, queue, scheduler or client: both
  shapes run in the existing `turso_queue` claim path.
- Changing which items the staleness check retires (#10626).
- Any remote write inside the claim transaction.
