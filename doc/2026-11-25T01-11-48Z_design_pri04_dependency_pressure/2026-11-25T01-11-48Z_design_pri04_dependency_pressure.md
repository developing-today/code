# PRI-04 — Cycle-safe hard-dependency pressure

Issue: [#9480](https://github.com/developing-today/code/issues/9480)
(slice `PRI-04` of [#9199](https://github.com/developing-today/code/issues/9199),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the dependency-pressure slice of the
merged design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the derivation, bounds, cycle, budget and
revision contract a separately scoped implementation issue can
be filed against. Component #9199 states that hard-dependency
pressure is effective-only, cycle-safe and fail-closed; this
record _decides_ the points it leaves open — the value rule,
which edges and dependents count, the caps, how cycles and
quarantined edges behave during evaluation, what a budget
overrun returns, and what the slice persists (nothing). It
authorizes no code, schema, migration, deployment, or live
acceptance; those remain separately gated stages (see §9).

Governing decisions consumed, not re-opened:

- **Component #9199, "Dependency pressure, cycle safety and
  resource bounds"** and its `PRI-04` slice row: only true
  hard dependency edges gate eligibility or contribute
  inheritance; hierarchy and advisory/related/duplicate edges
  never block or inherit; blocker intrinsic urgency is
  preserved and pressure is computed from downstream urgency
  and critical path, bounded by effort, actionability,
  probability and alternatives; the explanation names source
  item, edge type, graph revision, path, values/cap and
  effort/alternative evidence; a large speculative blocker is
  never promoted to Emergency; canonical direction is
  **blocker → dependent**; inserting `u→v` creates a cycle iff
  `u == v` or a path `v→u` already exists; the opposite-edge
  race must not commit both edges; imported cycles are
  quarantined; propagation is `O(V+E)`; selection pins a graph
  revision and retries or rejects on change; budget exhaustion
  returns `graph_evaluation_incomplete` and fails closed. This
  record fixes the rule and constants that text defers and
  changes none of its semantics.
- **Component #9208 §2** (typed relationship contract): the
  edge record (`relation`, `hardness`, `status`, `certainty`,
  `revision`, provenance), the seven user-facing spellings
  normalised to one canonical edge, and the rule that only
  active hard `blocks` edges determine eligibility. #9208 §2.3
  states that dependency pressure is outside that component;
  this record is where it lives.
- **FDR-PMO-005** (typed dependencies, journal and field
  authority; accepted as a local design contract 2026-11-13):
  option 1 — one typed relation fact — and its graph-model
  oracle (parent edge never blocks; opposite concurrent inserts
  produce one accepted edge and one cycle conflict; an
  unresolved hard external ref is not assumed satisfied).
- **FDR-PMO-002** (priority profiles, ordering and
  provenance): intrinsic values are canonical, effective
  pressure is derived, and #7030's ratified `expedite=15` /
  `next-up=25` bands are consumed, never renumbered.
- **The package `architecture.md`**, "Cycle-safe
  hard-dependency pressure is effective-only" and the
  selection-order object: `graph_revision` is already a key of
  `fleet.selection-order/v1`, and the claim transaction
  revalidates graph revision with the other snapshot inputs.
  This record supplies what PRI-04 contributes to that object
  (§6.2) and adds no key.
- **The shared reason-code registry already on main**
  (`pkgs/fleet/crates/fleet-queue/src/work/contracts.rs`):
  `ReasonCode::DependencyPressure` ("hard-edge path / graph
  revision pressure") is registered with effect permissions
  `base: false, derived: true, hint: false`. PRI-04 is the
  producer of that code and stays inside those permissions: it
  never accepts a base mutation and never raises a hint.
- **PRI-02** ([#9479](https://github.com/developing-today/code/issues/9479))
  owns the dimension and explanation types (intrinsic urgency,
  nullable importance, effective pressure); **PRI-03**
  ([#9511](https://github.com/developing-today/code/issues/9511),
  record `doc/2026-11-23T15-34-52Z_design_pri03_peer_ordering_rank_rebalance/`)
  owns peer rank and the comparator, and consumes effective
  urgency as a pinned input. This record defines how the
  dependency component of that effective urgency is derived,
  and nothing about rank.
- **DEP-02** ([#9503](https://github.com/developing-today/code/issues/9503))
  and **DEP-03** ([#9443](https://github.com/developing-today/code/issues/9443))
  own the edge types, edge mutation, the insertion cycle check,
  and readiness. PRI-04 is a reader of their graph, not a
  second writer (§2).

Verified state at writing (2026-11-25, origin/main `5acf8c22`):

- No dated PRI-04 record exists under `doc/`; the only PRI-04
  text on main is the #9225 package (component slice row,
  census, preserved issue body). This record is the first.
  No DEP-02 or DEP-03 record exists either.
- `pkgs/fleet/crates/fleet-queue/src/work/relations.rs` and
  `.../work/priority.rs` **do not exist**. `work/mod.rs`
  declares `authority`, `completion`, `contracts`, `degraded`,
  `journal`, `lifecycle`, `operations`, `routing`,
  `selection` and (broker builds) `sqlite_replica`; none of
  them derives pressure or walks a dependency graph.
  `work/journal.rs` carries `BlockerEdgeRef { edge_id, target,
hard }` on checkpoints as an opaque reference "so dependency
  semantics stay with their owner".
- The only dependency mechanism on main is **#8276's typed
  block**: a lane blocks its task with a `blocked_deps` JSON
  array of `task`, `issue`, `pr` or `ci` refs
  (`turso_queue/core.rs`, `parse_block_deps`), and the broker
  rechecks them on a clamped interval (`DEP_RECHECK_*`,
  300–3600 s, default 900 s) until every dep clears, a ref is
  **broken** (404 / unknown task → released with the breakage
  named), or the block's max TTL expires (`DEP_TTL_*`, 60 s –
  7 days, default 3 days). That is a per-row wait list, not an
  edge store: there is no edge identity, no revision, no
  hardness/certainty, no inverse lookup, and **no cycle
  check** — two tasks blocked on each other's `task:` ref both
  stay blocked until their TTLs release them. Nothing on main
  turns any of this into urgency or pressure.
- Gate state for this slice: PR #9225 MERGED 2026-11-13.
  #9199 (design parent) and #9208 OPEN. DEP-02 #9503, DEP-03
  #9443, DSP-04 #9446, PRI-02 #9479, PRI-03 #9511 and DST-05
  #9455 OPEN. #9178 OPEN. Per the sibling pattern (PRI-03,
  REL-06, HYD-05/06/07, CEN-04 all landed as records while
  their implementation prerequisites stood open), these gate
  the implementation issue (§9), not this record.

## 1. Intent

A small fix that blocks a high-value item should be picked up
sooner, because the high-value item cannot move until it is
done — while the record of what the small fix is worth on its
own stays exactly as it was (charter C-090 / C-R064). This
slice is the rule that does that, and the safety around it:

1. a pure evaluator that, given a pinned hard-dependency graph
   and pinned urgencies, computes each blocker's **inherited
   urgency** and the effective urgency the comparator uses;
2. bounds that keep inherited urgency from inflating —
   no amplification, a band cap, hop decay, an effort clamp,
   certainty filtering (§3);
3. cycle behaviour that never loops, never invents pressure,
   and never quietly "repairs" a cycle (§4);
4. a budget whose overrun is reported, never truncated into a
   plausible answer (§5);
5. revision pinning so a claim never acts on pressure computed
   from a graph that has since changed (§6).

## 2. Ownership: who writes what

| Concern                                                              | Owner             | PRI-04's relation                                                            |
| -------------------------------------------------------------------- | ----------------- | ---------------------------------------------------------------------------- |
| Edge types, seven spellings, canonical `blocker → dependent` record  | DEP-02 (#9503)    | Consumes the types; defines none.                                            |
| Edge insertion, `would_create_cycle(u, v)`, revision CAS, readiness  | DEP-03 (#9443)    | Consumes the committed graph at a revision; never inserts edges.             |
| Native GitHub links, resolved history                                | DEP-04 (#9444)    | Consumes resolved/active status only.                                        |
| Dimension and explanation types                                      | PRI-02 (#9479)    | Fills the dependency component of the explanation.                           |
| Comparator, peer rank, selection-order digest contribution for rank  | PRI-03 (#9511)    | Supplies effective urgency as the comparator's first key.                    |
| Aging, hysteresis, temporary boosts, triage                          | PRI-05 (#9481–4)  | Independent derived components; combination rule in §3.6.                    |
| **Inherited-urgency derivation, its bounds, cycle and budget rules** | **PRI-04 (this)** | Owns `work/relations.rs` read-side evaluator and the `priority.rs` consumer. |

Two consequences. First, the cycle **predicate** for insertion
(`u→v` rejected iff `u == v` or a path `v→u` exists) has
exactly one implementation, DEP-03's. PRI-04's tests call it;
if PRI-04's implementation slice is ready before DEP-03's, it
waits (§9) rather than writing a second predicate. Second,
PRI-04 opens no write path: no edge, urgency, rank or label
is ever mutated by this slice. Its output is an observation.

## 3. The value rule

Notation: urgency is the current profile's integer, **lower is
sooner** (PRI-01). For a work item `x`: `intrinsic(x)` is its
stored urgency (possibly null = untriaged); `effective(x)` is
what the comparator uses; `inherited(x)` is the dependency
component this slice derives (possibly none).

### 3.1 Which edges count

An edge contributes to pressure only if, at the pinned graph
revision, all of these hold:

- relation is `blocks` (canonical `blocker → dependent`);
  `hierarchy`, `related_to`, `duplicates`, `supersedes` never
  contribute, whatever their direction;
- `hardness = hard`;
- `status = active` (`resolved`, `removed` and `quarantined`
  contribute nothing);
- `certainty ∈ {verified, asserted}`; an `uncertain` edge, or
  one with `reason_code = inferred_pending_review`, contributes
  nothing. Such an edge may still **gate** eligibility under
  DEP-03's conservative rule — gating and pressure are
  deliberately asymmetric: doubt keeps work blocked but never
  makes anything more urgent (component: "do not promote
  speculative blockers blindly").
- both endpoints are Fleet work at the evaluating authority.
  An external target (a GitHub issue that is not Fleet work)
  can block a Fleet item under DEP-03, but it has no urgency
  of its own in this store and receives none; a Fleet item
  that blocks only external work inherits nothing through
  that edge.

### 3.2 Which dependents are sources

A dependent `d` is a pressure source only if it is
non-terminal and not under `hold`. `manual_only` (#9178) does
not disqualify a source: it constrains who may claim `d`, not
whether `d` is wanted. A `do_now` directive on `d` does not
travel: directives are not urgency (C-057 / C-R045), and only
`effective(d)` propagates. A dependent whose own effective
urgency is null (untriaged) is not a source.

### 3.3 Inheritance along one path

For a path `b = p0 → p1 → … → pk = d` (k ≥ 1 hops) of
qualifying edges, the value it offers `b` is

```text
offer(path) = max( effective(d) + hop_step × (k − 1),  cap(b, d) )
```

- **No amplification.** `offer ≥ effective(d)` always: a
  blocker can become as urgent as what it blocks, never more.
- **Hop decay.** Each hop beyond the first adds the profile
  constant `hop_step` (lower urgency). A direct blocker gets
  the dependent's full urgency; distant ancestors get less.
  Decay is linear and bounded by the cap and by the item's
  intrinsic value (§3.5), so there is no exponent to calibrate.
- **Band cap.** `cap(b, d)` is the profile constant
  `inherited_floor` — the most urgent value inheritance alone
  may produce (proposed default: the profile's `high` band).
  This is the slice's "never promote to Emergency" rule. The
  one exception: if `d`'s own effective urgency rests on a
  `DeadlineBoost` or incident component with verified evidence
  (PRI-05's reason codes), `cap(b, d) = effective(d)` — a
  verified deadline carries its full weight down the critical
  path, which is the charter's case for pressure in the first
  place.
- **Effort clamp.** If `b` carries an effort estimate at or
  above the profile constant `effort_clamp_at` (proposed:
  `effort/l`), `offer` is additionally clamped to no better
  than the profile's `normal` band, unless the verified-deadline
  exception above applies. A large blocker is not a quick fix;
  pressure on it is a triage prompt, not an override. An item
  with no effort estimate is treated as unclamped and the
  explanation says the estimate was missing.

The constants `hop_step`, `inherited_floor`, `effort_clamp_at`
are **named PriorityProfile parameters** (PRI-01's contract),
versioned with the profile and recorded by `policy_ref`. This
record proposes defaults expressed as band names, not integers,
because the integer scale is PRI-01's to fix; it renumbers
nothing.

### 3.4 Many paths

`inherited(b) = min over all qualifying paths of offer(path)`
— the most urgent offer wins, which is the critical-path rule.
Because `offer` is monotone in `effective(d)` and in `k`, the
minimum is found by one reverse-topological pass that keeps,
per node, the best `(value, source, hops)` — no path
enumeration (§4.2).

Ties among equal offers are broken, for the **explanation
only**, by the comparator's own order on the source (PRI-03
§3.1: effective urgency, peer rank, created time, canonical
identity), so the named source is deterministic. The value is
unaffected by which tied source is named; the explanation also
records how many sources tied.

### 3.5 Combination with intrinsic urgency

```text
effective_dep(b) = min( intrinsic(b), inherited(b) )   # when both exist
effective_dep(b) = intrinsic(b)                        # no qualifying path
effective_dep(b) = inherited(b)                        # intrinsic null
```

- Pressure only ever moves an item **sooner**; it never makes
  an item later than its own intrinsic urgency.
- `intrinsic(b)` is never rewritten (O-R002, S-R006). An
  untriaged blocker of an important item becomes eligible to
  be picked by inheritance and remains untriaged — the
  explanation says so, and PRI-05's triage still owes it a
  review.
- The `do_now`/`expedite` distinction is untouched: inherited
  pressure can never produce the ratified `15`/`25` band
  semantics, because it produces a derived value with
  `ReasonCode::DependencyPressure`, not a stored band.

### 3.6 Combination with other derived components

PRI-05's temporary components (deadline boost, opportunity
boost) are computed per item first; the dependency pass then
reads each dependent's resulting effective urgency. The final
effective urgency is the minimum of all applicable derived
components and intrinsic urgency, each recorded separately in
the explanation. The evaluation order is fixed — per-item
components, then one dependency pass — so a boost on `d` flows
to its blockers but pressure never feeds back into the
components it was computed from within one evaluation.

### 3.7 Explanation

For each item whose effective urgency came from inheritance,
the evaluator emits (shape filled into PRI-02's explanation
type, wire keys snake_case, integers as canonical decimal
strings):

```json
{
  "reason_code": "dependency_pressure",
  "graph_revision": "57",
  "policy_ref": "profile:v3",
  "as_of_utc": "2026-11-25T01:30:00Z",
  "intrinsic": "70",
  "inherited": "20",
  "effective": "20",
  "source": { "authority_id": "local", "work_id": "842" },
  "source_effective": "20",
  "hops": "1",
  "path_edge_ids": ["edge:1931"],
  "path_edge_revisions": ["3"],
  "cap_applied": null,
  "decay_applied": "0",
  "effort_clamp": { "applied": false, "effort": null, "estimate_missing": true },
  "tied_sources": "0"
}
```

`cap_applied` names the constant that bound the value
(`inherited_floor`, `effort_clamp_at`) or is null. Items whose
inheritance was considered and lost to their own intrinsic
urgency record nothing, so the explanation set stays
proportional to what actually changed.

## 4. Cycles

### 4.1 Insertion (consumed from DEP-03)

DEP-03's insertion rule is the only line of defence against new
cycles: under one serialized graph-revision transaction (or a
revision CAS covering the reachability check and the insert),
`u→v` is rejected iff `u == v` or the committed graph has a
path `v→u`. The opposite-edge race — `A→B` and `B→A` each
checked against the same empty revision — resolves to exactly
one commit; the loser re-checks at the new revision, finds the
path, and is rejected with a cycle conflict. PRI-04's first
failing test (§8) drives this through DEP-03's interface and
then asserts what the pressure evaluator sees afterwards.

### 4.2 Evaluation must still be cycle-safe

A committed graph can still contain cycles, from three
sources: imported legacy edges (DEP-01/DEP-04 import), edges
DEP-03 quarantined, and any future bug. The evaluator must
therefore never assume a DAG:

1. Build the qualifying subgraph (§3.1). Edges with
   `status = quarantined` are already excluded by §3.1.
2. Compute strongly connected components (Tarjan or Kosaraju,
   iterative, `O(V+E)`, visited set — no recursion depth tied
   to graph shape).
3. Every node in a component of size > 1, and every node with
   a self-edge, is **cycle-isolated**: it neither gives nor
   receives inherited pressure, and every edge into or out of
   the component is skipped for pressure. Its effective urgency
   is its intrinsic urgency plus non-dependency components. The
   explanation records `dependency_cycle_isolated` with the
   sorted member identities and the edge ids.
4. Contract the remaining graph (now a DAG by construction)
   and run the reverse-topological pass of §3.4.

Isolation is the conservative choice in both directions: a
cycle cannot inflate anything, and an item caught in one is
not made less urgent than it was. Eligibility is not this
slice's call — DEP-03 keeps cycle members blocked until an
authorised repair — so isolated items usually are not
claimable anyway; the rule matters for ordering the repair
work itself, which must not be outranked by phantom pressure.

PRI-04 never removes, resolves or quarantines an edge. An
evaluation that finds a cycle among `active` edges (which
DEP-03 should have made impossible) reports it in the
evaluation result as `unexpected_active_cycle`, so the
broker's existing health surfaces can raise it; it does not
fix it.

### 4.3 The #8276 compatibility case

Until DEP-02/DEP-03 edges exist, the implementation must not
read #8276's `blocked_deps` as a pressure graph: those records
have no revision to pin, no hardness or certainty, and V1
already admits mutual `task:` blocks. Whether and how they are
imported as edges is DEP-01's crosswalk decision. If they are
imported, they arrive with `reason_code = imported` and are
subject to §3.1 and §4.2 like any other edge — an imported
mutual block is isolated, not traversed.

## 5. Budget

The evaluator accepts a budget `{max_nodes, max_edges}` (profile
constants, proposed defaults sized to the whole queue population
plus edges with headroom; the exact numbers are an
implementation-issue measurement, not a design guess). It either
finishes the whole qualifying subgraph within budget or returns:

```json
{ "status": "graph_evaluation_incomplete", "graph_revision": "57", "visited_nodes": "…", "visited_edges": "…", "budget": { "max_nodes": "…", "max_edges": "…" } }
```

and **no per-item pressure values at all**. Partial pressure is
never published: an item halfway down a long chain cannot be
told it inherits from the half that was seen.

What consumers do with that is fixed here so it cannot drift:

- **Ordering** falls back to effective urgency **without** the
  dependency component for every item, and the selection-order
  object records the pressure state as unavailable in its
  `policy_ref`-scoped observation (§6.2). That is a correct,
  complete order under a weaker policy, and it says so; it is
  not an apparently-valid pressure order.
- **Eligibility** is untouched by PRI-04's budget and remains
  DEP-03's: its own traversal overrun returns
  `graph_evaluation_incomplete` and fails closed for the
  dependent decisions it was asked about. This record does not
  relax that.
- A budget overrun is an operational signal, emitted once per
  graph revision, not per claim.

## 6. Revisions and claims

### 6.1 Pinning

An evaluation is a pure function of `(graph_revision,
urgency snapshot ref, policy_ref, as_of_utc)`. The evaluator
reads nothing else and keeps no state between calls. A cache,
if the implementation adds one, is keyed by that full tuple and
invalidated wholesale when any part changes; no incremental
upstream/downstream invalidation is designed in this slice.
(The component's post-review note — a dependent's urgency change
must invalidate its blockers' inherited values — is satisfied
trivially by wholesale keying; an incremental cache is a later,
separately tested optimisation.)

### 6.2 Contribution to the selection-order object

`fleet.selection-order/v1` already carries `graph_revision` and
`policy_ref`, and each `ordered_items` entry carries
`effective_urgency`. PRI-04 adds **no key**. It contributes:

- the `graph_revision` the evaluation used (the same value
  DEP-03's readiness used, or the order is rejected as
  internally inconsistent);
- `effective_urgency` values that include the dependency
  component;
- under `policy_ref`, the profile constants of §3.3 and the
  pressure state (`complete` / `unavailable`) — so two
  orders with different pressure states never share a
  fingerprint.

### 6.3 Claim

The claim transaction already revalidates graph revision
(architecture.md, "Claim revalidates current predicates"). If
the revision moved between selection and claim, the claim does
not use the stale pressure: it recomputes and reselects once,
then returns the existing typed stale result if the revision
moved again. PRI-04 adds no new refusal code for this; it relies
on the claim path's own.

## 7. Persisted fields

**None.** Inherited urgency, effective urgency, and the
explanation are derived observations, recomputed from a pinned
snapshot; storing them would create a second source of truth
that drifts from the graph. CEN-04 §5's field-admission gate is
therefore not engaged by this slice, and no amendment to the
CEN-04 matrix is required. The profile constants of §3.3 live
in PRI-01's versioned profile, which CEN-04 already covers as
policy, not as per-task fields. If a later slice wants to
persist a pressure snapshot (for audit or for the PM
projection), that slice brings its own admission entry.

## 8. Acceptance oracle, specified

All tests live in the owning modules (`work/relations.rs`,
`work/priority.rs`) and run with the slice body's command
form. Each test name must first appear in the `-- --list`
output of its exact command; a filter that matches zero tests
is not acceptance. Builds and test runs go through remote
dispatch (`fleet queue dispatch`), per the repository's
remote-build rule; no local compile is an acceptance record.

| Test                                                   | Oracle                                                                                                                                                                                                                                                                                      |
| ------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_dependency_cycle_opposite_edge_pair` (first, red) | Empty graph at rev r0; propose `A→B` and `B→A`, both checked against r0, committed through DEP-03's revision CAS in both orders. Exactly one commits; the other is rejected as a cycle at r1. Pressure at r1 flows only along the committed edge. Same final state for either commit order. |
| `pmo_dependency_cycle_predicate_truth_table`           | For generated graphs up to 7 nodes: DEP-03's predicate rejects `u→v` iff `u == v` or a brute-force path `v→u` exists. Asserted against an independent BFS in the test, not the evaluator.                                                                                                   |
| `pmo_dependency_pressure_matches_reference`            | For generated DAGs up to 9 nodes with random urgencies, efforts and certainties: the evaluator's inherited values equal a brute-force all-paths `min(offer)` reference (§3.3–§3.4).                                                                                                         |
| `pmo_dependency_pressure_non_hard_edges_inert`         | Adding any `hierarchy`, `related_to`, `duplicates`, `supersedes`, soft, resolved, removed, quarantined, uncertain or pending-review edge changes no effective urgency.                                                                                                                      |
| `pmo_dependency_pressure_preserves_intrinsic`          | Intrinsic urgency of every item is byte-identical before and after evaluation; null stays null; no evaluation output is a base mutation (effect class `DerivedPressure` only).                                                                                                              |
| `pmo_dependency_pressure_low_value_quick_fix`          | C-090: a `low` quick fix blocking a `high` item gets effective `high` and keeps intrinsic `low`; the explanation names the dependent, hop count 1, and the edge.                                                                                                                            |
| `pmo_dependency_pressure_caps_and_decay`               | No offer below `inherited_floor` without verified-deadline evidence; with it, the dependent's value passes through; each hop adds `hop_step`; effort at or above `effort_clamp_at` clamps to `normal`; missing effort is recorded, not guessed.                                             |
| `pmo_dependency_pressure_cycle_isolated`               | An imported 2-cycle and a 3-cycle with tails in and out: members neither give nor receive pressure; nodes outside the cycles are evaluated normally; the evaluation terminates; `dependency_cycle_isolated` lists sorted members.                                                           |
| `pmo_dependency_pressure_graph_revision_retry`         | Selection at rev r, edge added at r+1 before claim: the claim recomputes and reselects once; a second move returns the existing stale result; no claim uses r's pressure.                                                                                                                   |
| `pmo_dependency_pressure_budget_fails_closed`          | A chain longer than `max_nodes`: result is `graph_evaluation_incomplete` with no per-item values; ordering falls back to the no-dependency policy and the order object's pressure state is `unavailable`; the fingerprint differs from a complete evaluation's.                             |
| `pmo_dependency_pressure_directive_does_not_travel`    | A `do_now` dependent with `normal` effective urgency gives its blocker `normal`, not Critical; a `hold` dependent gives nothing; a `manual_only` dependent gives its effective urgency.                                                                                                     |

Negative coverage from the slice body, mapped: stale input →
`graph_revision_retry`; duplicate/replay → evaluation is pure,
same inputs give the same output and fingerprint
(`matches_reference` re-runs each case twice); ambiguous
resource → external targets inherit nothing (§3.1, in
`non_hard_edges_inert`); unauthorised actor → no write path
exists to authorise; partial remote effect → not applicable,
the slice has no remote effects; unknown version → an unknown
`policy_ref` or edge schema version fails the evaluation with a
typed error and no values; no false verified/completed result →
`budget_fails_closed`.

## 9. Gates and sequencing

Implementation against this record starts only when **all** of
the following hold:

1. This record is merged.
2. A separately scoped implementation issue is filed against
   it, carrying §8's oracles as its acceptance. #9480 itself
   authorises no code; it stays open as the slice tracker.
3. **DEP-03** (#9443) has landed the insertion predicate and
   revision protocol PRI-04 consumes (§2, §4.1), on top of
   **DEP-02** (#9503)'s edge types. This is the slice body's
   hard blocker (normalised from #9208 → DEP-03).
4. The shared-file sequencing edges from the slice body are
   honoured: **PRI-03** (#9511) lands its `work/priority.rs`
   changes first, and **DSP-04** (#9446) lands its
   `work/relations.rs` changes first.
5. **PRI-01/PRI-02** have fixed the profile constants' home and
   the explanation type that §3.3 and §3.7 fill in.
6. #9178's claim semantics are verified before the claim-path
   integration of §6.3 (the slice body's runtime gate; the pure
   evaluator and its offline oracles may precede it).

Wiring the evaluator into the live claim order is not part of
the implementation issue: it is PRI-07's shadow report first,
then a separately approved cutover. Production rollout and live
acceptance remain separate stages.

**Schedules:** this record creates, changes, and retires no
schedule on any surface.

## 10. Requirement trace (design-level)

Full per-occurrence text for every allocated ID is in
`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/census/requirements-crosswalk.jsonl`;
this table records which part of this design speaks to each
group — design coverage, not implementation.

| IDs                    | Where addressed                                                                                                  |
| ---------------------- | ---------------------------------------------------------------------------------------------------------------- |
| C-R041                 | §3 — dependency pressure as one input to effective scheduling urgency, beside deadlines and directives           |
| C-R045                 | §3.2, §3.5, `directive_does_not_travel` — `do_now` is not Critical urgency and removes no hard blocker           |
| C-R052, O-R006         | §3.6 — dependency pressure combined with PRI-05's triage components, which still owe untriaged blockers a review |
| C-R061                 | §3.3 — caps are resistance, not a barrier: verified deadlines pass through                                       |
| C-R064                 | §3.3–§3.5, `low_value_quick_fix` — a quick fix inherits the blocked item's urgency, intrinsic stays explainable  |
| H-R004, O-R002, S-R006 | §3.5, §7 — intrinsic, effective and directive kept distinct; nothing rewritten or persisted                      |
| H-R007, O-R008         | §3.1, §3.3, §4 — hard-only edges, cycle safety, effort and certainty safeguards                                  |
| H-R030                 | §3.3, §3.7 — bounded, explainable pressure; no inflation beyond the source                                       |
| O-R017                 | §3.4 — critical-path minimum; unrelated items (hierarchy children) get nothing                                   |

## 11. Non-goals

- No edge storage, edge mutation, insertion cycle check or
  readiness evaluation (DEP-02/DEP-03/DEP-04). PRI-04 reads a
  committed graph.
- No profile values, urgency scale, aging, hysteresis,
  temporary boosts or triage (PRI-01/PRI-02/PRI-05); no peer
  rank or comparator (PRI-03); no PM projection (PRI-06); no
  shadow report or cutover (PRI-07).
- No "alternatives" semantics. The component lists
  alternatives among the bounds, but DEP-02's vocabulary has no
  any-of edge; until it does, alternative evidence is recorded
  in the explanation when present and changes no value. Adding
  an any-of relation is DEP-02's decision, not this slice's.
- No probability weighting beyond certainty filtering (§3.1).
  A numeric probability on edges would need a field DEP-02 does
  not define.
- No new queue, database, scheduler, service, resolver,
  executable, or GitHub client, and no persisted field (§7).
- No change to #8276's dependency blocks, their recheck sweep,
  or their release rules; their import is DEP-01's question.
- No change to #6533 or its live acceptance.
