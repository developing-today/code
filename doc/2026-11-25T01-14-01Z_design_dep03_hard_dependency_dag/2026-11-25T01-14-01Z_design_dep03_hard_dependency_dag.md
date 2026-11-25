# DEP-03 — Hard-dependency DAG: cycle-safe mutation and complete readiness

Issue: [#9443](https://github.com/developing-today/code/issues/9443)
(slice `DEP-03` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the hard-dependency slice of the merged
design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the storage, transaction, readiness, quarantine
and compatibility contract a separately scoped implementation
issue can be filed against. Component #9208 §2.3 states the
rules — only active hard `blocks` edges gate eligibility, `u→v`
is a cycle iff `u == v` or a path `v→u` exists, opposite inserts
must not both commit, imported cycles are quarantined, an
incomplete traversal is never `ready`. This record _decides_
how those rules are enforced in the broker's existing store: the
two additive tables, the single-transaction insert with an
in-SQL reachability check, the graph revision, the claim-path
readiness clause, what satisfies an edge, and the quarantine
repair procedure. It authorizes no code, schema, migration,
deployment, or live acceptance; those remain separately gated
stages (see §10).

Governing decisions consumed, not re-opened:

- **Component #9208 §2** — canonical `source → target` storage
  (for hard blocks, **blocker → dependent**), the seven
  user-facing spellings normalised to one edge, the edge record
  (`edge_id`, `revision`, `relation`, `hardness`, `status`,
  `reason_code`, `provenance`, `certainty`, resolution fields),
  append-only removal and resolution, and "a hard unresolved
  external ref blocks by default".
- **FDR-PMO-005** (accepted as a local design contract
  2026-11-13): one typed relation fact; active hard `blocks`
  edges alone affect readiness; serialized revision validation
  and insertion; incomplete evaluation is partial/unknown and
  fails closed for claim and completion; ambiguous or cyclic
  imports are quarantined, never repaired by deletion.
- **The package `architecture.md`**: graph revision validation
  and edge insertion share one transaction; the claim
  transaction revalidates `graph_revision` with the other
  snapshot inputs; `fleet.selection-order/v1` carries
  `graph_revision`.
- **CEN-04** ([#9434](https://github.com/developing-today/code/issues/9434),
  record `doc/2026-11-23T13-51-45Z_design_cen04_schema_owner_migration_compatibility/`):
  §3.5 already names the broker as owner of the operational edge
  set and #9208 as owner of the typed model, with disposition
  **extend** and "any new persisted edge field enters via §5";
  §5's field-admission gate; §7's additive, reversible move
  shape. §8 below supplies the admission entries.
- **DEP-02** ([#9503](https://github.com/developing-today/code/issues/9503))
  owns the edge types, the alias normalisation and the JSON and
  error contract in `work/relations.rs`. This record consumes
  those types and specifies their storage and mutation; it
  defines no new spelling.
- **PRI-04** ([#9480](https://github.com/developing-today/code/issues/9480),
  record `doc/2026-11-25T01-11-48Z_design_pri04_dependency_pressure/`)
  consumes this slice's insertion predicate and graph revision
  and reads the committed graph for pressure. This record is the
  single owner of the predicate PRI-04 §2 refers to.

Verified state at writing (2026-11-25, origin/main `a9e68397`):

- No dated DEP-03 or DEP-02 record exists under `doc/`; this is
  the first. `work/relations.rs` does not exist.
- The broker store's `SCHEMA` (`turso_queue/core.rs`) has no
  edge table and no graph revision. Two dependency-shaped
  mechanisms exist today, both per-row:
  - **#8276 typed blocks**: a `blocked` task carries
    `blocked_deps` (JSON array of `task` / `issue` / `pr` / `ci`
    refs) plus `dep_next_recheck_at`, `dep_blocked_until` and
    `dep_recheck_seconds`. The broker's recheck sweep probes each
    ref through the `DepProbe` trait and unblocks the row when
    every dep clears, when a ref is **broken** (404 / unknown
    task — released with the breakage named), or when the
    block's max TTL (default 3 days, max 7) expires. A `task`
    dep clears on `done` and also releases on `cancelled` /
    `failed`. There is no cycle check: two tasks each blocked on
    the other's `task:` ref hold until TTL.
  - **Parked ready conditions**: a parked task can wait on
    `ready_condition.kind = task_terminal` (and `task_status`,
    `event_seen`, `person_answer`); the claim predicate
    (`turso_queue/claim.rs`, `PARKED_TERM_EXCLUSION`) keeps such
    rows out of timer-based claim.
- The claim pick predicate is one SQL fragment,
  `default_pick_predicate()` in `turso_queue/claim.rs`
  (`status = 'queued' …` or eligible `parked` rows, plus the
  budget gate and handoff clause). Readiness in this record is
  one more clause in that fragment (§6.2).
- Atomic multi-statement writes already use one Turso pipeline
  transaction: `BEGIN IMMEDIATE`, guarded `UPDATE`/`INSERT`
  statements chained on `changes()`, `COMMIT` (for example
  `reprioritize_task` in `turso_queue/lifecycle.rs`). §5 uses
  exactly that pattern; it needs no new transaction machinery.
- Gate state: PR #9225 MERGED 2026-11-13. #9208 (design parent),
  DEP-02 #9503, #9178 and #9179 OPEN; #9241 (QTI disposition)
  CLOSED with CEN-04 §6's bounded verdict. Per the sibling
  pattern these gate the implementation issue (§10), not this
  record.

## 1. Intent

When a worker discovers that task 842 cannot proceed until
task 761 lands, Scenario B of the charter (C-151) has it run
`fleet dependency add 842 --blocked-by fleet:761 --hard`. After
that, three things must be true without anyone re-checking by
hand:

1. 842 is not claimable while 761 is unfinished, and becomes
   claimable when — and only when — every one of its active
   hard blockers is satisfied (§6).
2. No sequence of such commands, however concurrent, can
   produce a cycle of active hard edges (§5).
3. Every edge's life — added, resolved, removed, quarantined —
   is a recorded fact with provenance, never a silent edit
   (§4, §7).

#8276's per-row blocks keep working unchanged beside this; §9
says how the two coexist.

## 2. Ownership

| Concern                                                   | Owner             |
| --------------------------------------------------------- | ----------------- |
| Edge types, aliases, JSON and error contract              | DEP-02 (#9503)    |
| **Edge storage, revision, cycle-safe mutation, readiness** | **DEP-03 (this)** |
| Native GitHub links, all seven relations' projection      | DEP-04 (#9444)    |
| Dependency-derived pressure                               | PRI-04 (#9480)    |
| Command spelling / registration                           | CLI-02 / CLI-13   |
| Claim and manual-only semantics                           | #9178             |
| Park/resume safety                                        | #9179             |

DEP-03 writes `work/relations.rs` (the pure readiness and
reachability model, after DEP-02's types) and the broker
integration in `turso_queue.rs` and its submodules (storage,
transaction, claim clause, verbs) — the slice body's owned
files. It adds no service, resolver, database or GitHub client:
external observation reuses #8276's `DepProbe` (§6.3).

## 3. Storage: two additive tables in the broker store

Both live in the existing broker `SCHEMA`, created
`IF NOT EXISTS` like every other table, and are additive: no
existing column changes.

### 3.1 `dep_edges`

```sql
CREATE TABLE IF NOT EXISTS dep_edges (
  edge_id            INTEGER PRIMARY KEY,
  revision           INTEGER NOT NULL DEFAULT 1,
  relation           TEXT NOT NULL,      -- DEP-02 canonical: blocks | related_to | hierarchy | duplicates | supersedes
  hardness           TEXT NOT NULL,      -- hard | soft
  status             TEXT NOT NULL,      -- active | resolved | removed | quarantined
  certainty          TEXT NOT NULL,      -- verified | asserted | uncertain
  reason_code        TEXT NOT NULL,      -- explicit | imported | migrated | inferred_pending_review
  source_task_id     INTEGER,            -- Fleet source (blocker, for blocks); NULL when external
  source_ref         TEXT,               -- canonical external ref JSON when source is not Fleet work
  source_key         TEXT NOT NULL,      -- normalized identity: 'task:<id>' or the external ref key
  target_task_id     INTEGER NOT NULL,   -- Fleet target (dependent, for blocks)
  provenance         TEXT NOT NULL,      -- JSON: principal_ref, actor_ref, session_ref, operation_id, source_ref
  idempotency_key    TEXT,
  created_at         TEXT NOT NULL,
  updated_at         TEXT NOT NULL,
  resolved_at        TEXT,
  resolution         TEXT                -- JSON evidence for resolved/removed/quarantined
);
CREATE UNIQUE INDEX IF NOT EXISTS dep_edges_active_unique
  ON dep_edges (relation, source_key, target_task_id) WHERE status = 'active';
CREATE INDEX IF NOT EXISTS dep_edges_by_source
  ON dep_edges (source_task_id, relation, hardness, status);
CREATE INDEX IF NOT EXISTS dep_edges_by_target
  ON dep_edges (target_task_id, relation, hardness, status);
```

Decisions behind the shape:

- **Target is always Fleet work.** Only Fleet work can be held
  back by Fleet's claim path, so the dependent side is a task
  id. The source (blocker) may be Fleet work or an external
  resource (GitHub issue, PR, Actions run, or a task at another
  authority), stored as DEP-02's canonical external ref with a
  normalized `source_key` (`pr:owner/repo#12` etc., the same key
  shape #8276's `dep_key` already uses, so one probe cache can
  serve both).
- **One active fact per edge.** The partial unique index is
  #9208's "unique active normalized edge". Inverse spellings
  (`blocked_by`) normalise before insert, so they hit the same
  index row.
- **History in place, not in a second table.** `status` moves
  `active → resolved | removed | quarantined` with `revision`
  incremented and `resolution` filled; rows are never deleted.
  Every transition also writes an `events` row (§7), which is
  the append-only trail; the edge row is current state.
- **No per-row TTL.** Unlike #8276, a typed hard edge never
  expires on a timer: expiry is a silent unblock, which
  FDR-PMO-005 rules out. The escape hatch is an explicit
  override (§6.4).

### 3.2 `dep_graph`

```sql
CREATE TABLE IF NOT EXISTS dep_graph (
  authority_id TEXT PRIMARY KEY,
  revision     INTEGER NOT NULL,
  updated_at   TEXT NOT NULL
);
```

One row per authority (in practice one row: the broker's own).
`revision` increments by exactly one in the same transaction as
**every** edge mutation of any relation or status. It is the
`graph_revision` of the selection-order object and of PRI-04's
evaluation, and the CAS token for `--expected-version` on
dependency commands. Task state changes (a blocker reaching
`done`) do not bump it; they change readiness, not the graph.
A blocker's resolution _does_ bump it, because resolution is an
edge mutation (§6.3).

## 4. Mutations

| Operation             | Effect on the edge                                        | Revision |
| --------------------- | --------------------------------------------------------- | -------- |
| `dependency.add`      | insert `active` (§5 for hard `blocks`; §5.4 for others)   | +1       |
| `dependency.resolve`  | `active → resolved`, evidence required                    | +1       |
| `dependency.remove`   | `active → removed`, reason required (tombstone)           | +1       |
| quarantine (§7)       | `active → quarantined` (import or repair only)            | +1       |
| `dependency.restore`  | `quarantined → active` via the §5 insert check            | +1       |
| automatic resolution  | `active → resolved` when the blocker is satisfied (§6.3)  | +1       |

Every mutation is one pipeline transaction that updates the edge
row with `WHERE status = ? AND revision = ?` (edge-level CAS),
bumps `dep_graph.revision` gated on `changes() = 1`, and inserts
its event gated the same way — the `reprioritize_task` pattern.
A zero-row first statement means the edge moved underneath the
caller: the verb returns `VERSION_CONFLICT` with the current
edge, and nothing else in the transaction took effect.

`work.unblock` / `queue.unblock` (allocated command keys) act on
**one named edge** or on the legacy #8276 block; there is no
"unblock everything" form. Resolving or removing one edge never
touches another, so a task with two hard blockers stays blocked
until both are satisfied (P-R019: "never unblock past another
hard blocker").

## 5. Cycle-safe insertion

### 5.1 The rule

For a proposed active hard `blocks` edge `u → v` (u blocks v),
the insert is rejected with `dependency_cycle` iff `u == v` or
the committed graph has a path `v → u` over active hard
`blocks` edges. Soft, non-`blocks`, resolved, removed and
quarantined edges are not part of the path. An edge whose
source is external (`source_task_id IS NULL`) cannot close a
cycle and skips the check.

### 5.2 One transaction, check in SQL

The reachability check runs **inside** the insert's transaction,
as a recursive CTE in the `INSERT … SELECT … WHERE NOT EXISTS`,
so there is no window between "checked" and "written":

```sql
BEGIN IMMEDIATE;
INSERT INTO dep_edges (relation, hardness, status, certainty, reason_code,
                       source_task_id, source_key, target_task_id, provenance,
                       idempotency_key, created_at, updated_at)
SELECT 'blocks', 'hard', 'active', :certainty, :reason_code,
       :u, 'task:' || :u, :v, :provenance, :idem, :now, :now
WHERE :u <> :v
  AND (SELECT revision FROM dep_graph WHERE authority_id = :auth) = :expected_revision
  AND NOT EXISTS (
    WITH RECURSIVE reach(n) AS (
      SELECT :v
      UNION                                   -- UNION, not UNION ALL: terminates on any graph
      SELECT e.target_task_id FROM dep_edges e JOIN reach r ON e.source_task_id = r.n
      WHERE e.relation = 'blocks' AND e.hardness = 'hard' AND e.status = 'active'
      LIMIT :max_reach                        -- §5.3
    )
    SELECT 1 FROM reach WHERE n = :u
  )
  AND (WITH RECURSIVE reach(n) AS (… same …) SELECT count(*) FROM reach) < :max_reach;
UPDATE dep_graph SET revision = revision + 1, updated_at = :now
  WHERE authority_id = :auth AND changes() = 1;   -- written as a guarded statement per the existing pattern
INSERT INTO events (…) SELECT … WHERE changes() = 1;
COMMIT;
```

(The sketch shows intent; the implementation follows the
existing pipeline idiom of one guarded statement per step.)

Why this is race-free: `BEGIN IMMEDIATE` takes the database
write lock before the first read, so two writers cannot both
evaluate reachability against the same committed state. The
opposite-edge race — `A→B` and `B→A` submitted together — is
serialised by the lock: whichever transaction runs second sees
the first's committed edge, finds the path, and inserts nothing.
The `expected_revision` term is the interactive CAS from #9208
§2.4 (`--expected-version`); callers that do not pass it bind
the current revision read in the same transaction, so the lock
alone provides safety and the CAS only adds staleness detection
for a user who looked at the graph first.

`UNION` (set semantics) makes the CTE terminate even if the
committed graph already contains a cycle (§7), because a node is
emitted at most once.

### 5.3 Bound and fail-closed

`:max_reach` (a broker constant, default to be measured in the
implementation issue against the live task count, with headroom)
bounds the walk. If the reachable set from `v` reaches the bound,
the insert is refused with `graph_evaluation_incomplete` rather
than accepted on a partial walk: an edge is never admitted on
the strength of a check that did not finish. The refusal names
the bound and the visited count, so an operator can raise it.

### 5.4 Classifying a refusal

A zero-row insert does not say why. After the transaction, the
verb classifies with read-only queries against the now-current
revision, in this order, and returns the first that applies:
`VERSION_CONFLICT` (revision moved), `duplicate_edge` (an active
row with the same `(relation, source_key, target_task_id)` — and
if its `idempotency_key` matches, this is a replay: return the
existing edge as success), `self_edge`, `dependency_cycle` with
the path `v → … → u` (a bounded BFS over the same edge set,
which is evidence for the user, not the safety check), or
`graph_evaluation_incomplete`. Because classification runs after
commit, a concurrent change can make the reported reason stale;
it is advisory, and the caller retries against the new revision.

Edges that are not hard `blocks` (soft blocks, `related_to`,
hierarchy, `duplicates`, `supersedes`) use the same transaction
minus the reachability term; they cannot create an eligibility
cycle. Hierarchy cycles (A parent of B parent of A) are DEP-04's
validation concern, not an eligibility one.

## 6. Readiness

### 6.1 Definition

Task `t` is **dependency-ready** iff every edge with
`target_task_id = t`, `relation = 'blocks'`, `hardness = 'hard'`
and `status ∈ {active, quarantined}` is satisfied. An `active`
edge is satisfied when its blocker is satisfied (§6.3); a
`quarantined` edge is **never** satisfied (conservative,
FDR-PMO-005). Soft and non-`blocks` edges never affect
readiness.

Readiness looks only at **direct** hard blockers. Transitivity
is carried by the blockers themselves: a blocker is not `done`
while its own blockers hold it back. So readiness is `O(degree)`
per task, needs no graph walk, and cannot be "incomplete". The
`graph_evaluation_incomplete` result exists for walks — the
insert check (§5.3) and the explanatory `graph` / `blockers
--explain` reads — and those reads never return `ready = true`
from a truncated walk.

### 6.2 The claim clause

The pick predicate gains one conjunct, a correlated
`NOT EXISTS` over `dep_edges_by_target`:

```sql
AND NOT EXISTS (
  SELECT 1 FROM dep_edges e
  WHERE e.target_task_id = tasks.id
    AND e.relation = 'blocks' AND e.hardness = 'hard'
    AND e.status IN ('active', 'quarantined'))
```

Because satisfied edges are moved to `resolved` (§6.3), "active
or quarantined" is exactly "unsatisfied" — the claim path does
no probing, joins nothing external, and stays inside the existing
claim transaction. #9178 owns the predicate; this clause is
handed to it as one more eligibility gate, alongside
`manual_only`, and does not change its ordering.

`dependency.ready <work>` returns the same verdict through the
same model, listing every unsatisfied hard edge with its
blocker state and certainty — the completion and claim paths
share one evaluator (#9208 §2.3), and "ready" in the CLI means
exactly "not excluded by this clause".

### 6.3 What satisfies a blocker, and who records it

| Blocker                   | Satisfied when                          | Recorded by                                                        |
| ------------------------- | --------------------------------------- | ------------------------------------------------------------------ |
| Fleet task                | it reaches `done`                       | the `complete` verb, in its own transaction: resolve every active hard edge it sources (+1 revision) |
| Fleet task                | it reaches `cancelled` / `failed`       | **not satisfied.** Edge stays active; a `dep_blocker_terminal_unsuccessful` event is raised once. |
| GitHub issue              | CLOSED as completed                     | the existing #8276 recheck sweep, extended to typed edges          |
| GitHub issue              | CLOSED as not planned                   | not satisfied; event raised, as for a failed task                  |
| GitHub PR                 | MERGED                                  | recheck sweep                                                      |
| GitHub PR                 | CLOSED unmerged                         | not satisfied; event raised                                        |
| Actions run               | completed with `success`                | recheck sweep                                                      |
| Actions run               | completed otherwise                     | not satisfied; event raised                                        |
| any external              | probe `Broken` (404, unknown)           | not satisfied; `certainty → uncertain`, event raised              |
| any external              | probe `Failed` (rate limit, auth)       | nothing inferred; recheck rescheduled                              |

This deliberately differs from #8276, which releases on failure,
breakage and TTL so that free-form blocks never strand a task.
A **typed hard** edge is a stronger statement — "this cannot
proceed until that lands" — and #9208 §2.2 says an unresolved
hard ref "must not be silently treated satisfied". The cost is
that a failed blocker strands its dependent until someone acts;
the event is what makes that visible, and §6.4 is how it is
cleared. #8276 blocks keep their own lenient rules (§9).

The recheck sweep already batches probes per pass with a shared
cache keyed by `dep_key`; typed edges are probed through the
same `DepProbe` with the same cache, so one GitHub read serves a
legacy dep and a typed edge on the same PR. Edges are rechecked
on the sweep's existing cadence; no new schedule is created.

### 6.4 Override

An authorized principal may resolve an unsatisfied edge with
`dependency.resolve --override --reason CODE --evidence REF`
(optionally `--expires-at`). It is an ordinary resolution with
`resolution.override = true`, its own event, and the principal
recorded; it is visible in `blockers --explain` forever. An
expiring override re-activates the edge at expiry through the
§5 insert check (as a restore), so it can never re-activate into
a cycle. Authorization is AUTH-01's capability model; an
`actor_ref` flag alone does not authorize (architecture.md).

## 7. Quarantine and repair

**Where cycles can come from.** §5 makes new active hard cycles
impossible through the broker. A cycle can still exist if edges
are imported (DEP-01 crosswalk, DEP-04 native-link import) or
restored from backup. Imports therefore go through the same §5
insert, one edge at a time in a stable order (by source key,
then target id); an edge the check refuses is written directly
as `status = quarantined` with `reason_code = imported` and
`resolution.cycle_path` filled, instead of being dropped. The
import never deletes or rewrites the raw source it read.

**Effect.** A quarantined edge keeps its dependent unready
(§6.1) and contributes nothing to pressure (PRI-04 §3.1). Each
quarantine raises one `dep_edge_quarantined` event naming every
edge on the detected cycle, so the whole loop is visible, not
just the edge that lost the ordering.

**Repair procedure** (the slice's "Docs" deliverable, carried
into the implementation issue's runbook):

1. `fleet dependency graph <work> --json` on any member lists
   the cycle's edges and which are quarantined.
2. Decide which dependency is false. Remove it:
   `dependency.remove --edge <id> --reason CODE` (a tombstone
   with provenance; the edge may be active or quarantined).
3. Restore the others: `dependency.restore --edge <id>` runs the
   §5 check at the current revision; it succeeds only if the
   cycle is now broken.
4. Nothing in this procedure is automatic. The broker never
   chooses which edge of a cycle is wrong.

**Health signal.** If an evaluation ever finds a cycle among
**active** hard edges — which §5 should make impossible — the
broker reports `unexpected_active_cycle` through its existing
health surface and quarantines nothing on its own: an invariant
breach is a bug report, not a repair trigger.

## 8. Persisted fields — CEN-04 §5 admission entries

These amend CEN-04 §3.5 ("Blocked dependencies"), which already
assigns the operational edge set to the broker and says new edge
fields enter through §5.

### 8.1 `dep_edges` (all columns of §3.1)

1. **Fact:** one typed relationship between a source (Fleet task
   or canonical external ref) and a Fleet target task, with
   relation, hardness, status, certainty, reason, provenance,
   revision, and resolution evidence.
2. **Owner:** #9208 (typed model, via DEP-02 types); the broker
   (operational storage and mutation, via DEP-03).
3. **Home:** the existing broker store, new table in `SCHEMA`.
4. **Write API:** `dependency.add / resolve / remove / restore`,
   `work.unblock` / `queue.unblock` on a named edge, `complete`
   (auto-resolution), the recheck sweep (auto-resolution and
   certainty downgrade), import (quarantine). **Read API:**
   the claim predicate clause, `dependency.ready / blockers /
   graph / list`, `show`, PRI-04's evaluator.
5. **Legacy mapping:** `tasks.blocked_deps` and its recheck
   fields are **not** this fact: they are #8276's per-row block
   with its own release rules and stay authoritative for the
   rows that carry them (§9). Free-text block reasons in
   `progress` / `last_error` are not edges (CEN-04 §3.5's
   standing refusal). `ready_condition.kind = task_terminal` on
   parked rows is a park wake-up condition, not an edge.
   `journal.rs`'s `BlockerEdgeRef` is a checkpoint's reference
   **to** an edge (its `edge_id` is `dep_edges.edge_id`).
6. **Disposition:** extend. **Rollback:** disable the claim
   clause and the dependency verbs; the table stays, unread.
   Dependents become eligible under their legacy gates only;
   no edge row is deleted.

### 8.2 `dep_graph.revision`

1. **Fact:** the monotonic revision of an authority's dependency
   graph.
2. **Owner:** the broker (DEP-03).
3. **Home:** existing broker store, new single-row-per-authority
   table.
4. **Write API:** every edge mutation, in its transaction.
   **Read API:** dependency command results, the
   selection-order object's `graph_revision`, PRI-04, the
   claim-time revalidation.
5. **Legacy mapping:** none exists; `tasks.updated_at` and
   per-task `version` are not graph revisions.
6. **Disposition:** extend. **Rollback:** as §8.1.

## 9. Coexistence with #8276

- Both mechanisms gate claim: a task is eligible only if its
  legacy state allows it (not `blocked`, or released by the
  #8276 sweep) **and** the §6.2 clause holds. Neither clears the
  other: #8276's sweep never touches `dep_edges`; resolving a
  typed edge never touches `blocked_deps` or `status`.
- New hard dependencies should be filed as typed edges. The
  existing `fleet queue block --dep …` verb keeps working with
  its documented semantics; nothing is renamed or redirected.
- Converting existing `blocked_deps` rows into typed edges is a
  DEP-01 crosswalk decision. If it is done, it is an import
  (§7) with `reason_code = migrated`, the original row is left
  as it was, and the converted edges start `certainty =
  asserted`.

## 10. Gates and sequencing

Implementation against this record starts only when **all** of
the following hold:

1. This record is merged.
2. A separately scoped implementation issue is filed against
   it, carrying §11's oracles as its acceptance. #9443 itself
   authorizes no code; it stays open as the slice tracker.
3. **DEP-02** (#9503) has landed its types in
   `work/relations.rs` (the slice's hard blocker and its
   shared-file sequencing edge).
4. The CEN-04 amendment carrying §8's two entries is merged
   **before the first edge write** (CEN-04 §5).
5. #9178's claim semantics are verified before the §6.2 clause
   enters the live pick predicate, and #9179's park semantics
   before any park-transition interaction; offline models and
   fixtures may precede both (the slice body's runtime gates).
6. The schema step is executed under the program's migration
   gates (CEN-04 §7: expand → dual-read → cutover), with its own
   evidence. Production rollout and live acceptance remain
   separate stages.

**Schedules:** this record creates, changes, and retires no
schedule on any surface; typed-edge rechecks ride the existing
#8276 sweep (§6.3).

## 11. Acceptance oracle, specified

Tests live in the owning modules and run with the slice body's
command form; each name must first appear in the `-- --list`
output of its exact command. Builds and test runs go through
remote dispatch (`fleet queue dispatch`), per the repository's
remote-build rule.

| Test                                                     | Oracle                                                                                                                                                                                               |
| -------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_dep_opposite_concurrent_edges_cannot_both_commit` (first, red) | Two writers on separate connections submit `A→B` and `B→A` against revision r0, released together, many iterations. Exactly one edge commits each time; the other returns `dependency_cycle`; revision is r0+1. |
| `pmo_dep_cycle_rule_matches_bfs`                         | Generated graphs up to 8 nodes: the in-SQL check rejects `u→v` iff `u == v` or an independent in-test BFS finds `v→u` over active hard `blocks` edges only.                                            |
| `pmo_dep_reach_bound_fails_closed`                       | A chain longer than `max_reach`: insert refused with `graph_evaluation_incomplete`; no edge, no revision bump.                                                                                     |
| `pmo_dep_ready_requires_all_hard_blockers`               | Two hard blockers; resolving one leaves the task unclaimable; resolving both makes it claimable; `unblock` on one edge never clears the other.                                                       |
| `pmo_dep_non_hard_edges_do_not_gate`                     | Soft blocks, `related_to`, hierarchy, `duplicates`, `supersedes` and resolved/removed edges never exclude a task from the pick predicate.                                                             |
| `pmo_dep_unsuccessful_blocker_does_not_release`          | Blocker task `failed`; closed-not-planned issue; closed-unmerged PR; probe `Broken`: the dependent stays unready and one event is raised each; probe `Failed` changes nothing.                       |
| `pmo_dep_done_blocker_resolves_in_complete_txn`          | `complete` on a blocker resolves its outgoing hard edges and bumps revision in the same transaction; a crash between is impossible to observe (single pipeline).                                     |
| `pmo_dep_imported_cycle_quarantined`                     | Importing a 3-cycle: two edges active, one quarantined with its path; the quarantined edge keeps its dependent unready; the CTE terminates.                                                           |
| `pmo_dep_restore_rechecks_cycle`                         | Restoring a quarantined edge while the cycle stands is refused; after removing another cycle edge it succeeds.                                                                                     |
| `pmo_dep_edge_mutation_is_cas`                           | Stale `--expected-version` returns `VERSION_CONFLICT` with the current revision and changes nothing; an idempotent replay of an add returns the existing edge.                                       |
| `pmo_dep_legacy_block_coexists`                          | A task with both a #8276 block and a typed edge needs both released; the #8276 TTL release does not clear the typed edge, and resolving the edge does not change `status`.                            |

Negative coverage from the slice body, mapped: imported cycles
quarantined → `imported_cycle_quarantined`; incomplete traversal
cannot return ready → `reach_bound_fails_closed` and §6.1
(readiness walks no graph); unresolved external hard ref blocks →
`unsuccessful_blocker_does_not_release`; hierarchy/parent does not
block → `non_hard_edges_do_not_gate`.

## 12. Requirement trace (design-level)

Full per-occurrence text is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table claims design coverage, not implementation.

| IDs                                   | Where addressed                                                                                         |
| ------------------------------------- | ------------------------------------------------------------------------------------------------------- |
| C-R129, C-R130, P-R019                | §3.1, §6 — typed relations stored once; hard blockers gate eligibility, advisory edges do not          |
| C-R131, O-R033                        | §4, §5, §7 — cycles prevented; resolved/removed history kept in place with events; no silent unblock   |
| C-R151                                | §1, §6.3 — Scenario B's `--blocked-by fleet:761 --hard` and its release on 761's completion            |
| C-R005, C-R009, C-R107, C-R108, H-R017, O-R023, P-R009 | §4, §6.2 — allocated dependency command keys and `--expected-version`; spelling stays CLI-02's |
| C-R087                                | §4 — `agent.slash.unblock` acts on one named edge, never all                                            |
| C-R123, C-R124, C-R125, C-R132        | §2, §9 — dependency semantics separate from titles/bodies; native links are DEP-04's projection        |
| H-R001, O-R022                        | §2 — dependency resolver responsibility inside the existing broker, no new service                     |
| H-R016, H-R024, H-R031, P-R027, P-R052, P-R067 | §11 — blocked/resume, conflict, cycle and history cases carried into acceptance               |
| O-R008, S-R006                        | §6.1, §7 — hard/soft separation; pressure left to PRI-04, which reads this graph                        |
| P-R013                                | §3 — dependencies as part of the logical work-item record, in the existing store                       |

## 13. Non-goals

- No new edge spellings, JSON shapes or error codes beyond
  those DEP-02 defines; the refusal names used here
  (`dependency_cycle`, `duplicate_edge`, `self_edge`,
  `graph_evaluation_incomplete`, `VERSION_CONFLICT`) are
  #9208 §2's and are handed to DEP-02 for its error contract.
- No native GitHub link projection or hierarchy validation
  (DEP-04); no pressure (PRI-04); no journal schema (JRN-01) —
  edge events use the existing `events` table.
- No change to #8276's blocks, their sweep cadence, TTLs or
  release rules; no change to parked ready conditions.
- No automatic cycle repair, no TTL on typed edges, no
  whole-task "unblock all".
- No new queue, database, scheduler, service, resolver,
  executable, or GitHub client.
- No change to #6533 or its live acceptance.
