# DEP-04 — Seven relation types, native GitHub links, and resolved history

Issue: [#9444](https://github.com/developing-today/code/issues/9444)
(slice `DEP-04` of [#9208](https://github.com/developing-today/code/issues/9208),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the relation-mapping slice of the merged
design package
(`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225) into the mapping, storage, import, removal and
history contract a separately scoped implementation issue can be
filed against. Component #9208 §2.1 names the seven spellings
and their execution effects; §5 says an epic closure never
implies its children are done and that native `blockedBy` and
sub-issue hierarchy map to their own types. This record _decides_
how each of the seven lands in the `dep_edges` table DEP-03
defines, how GitHub's native links are imported without being
confused with each other, what a removal leaves behind, and why
no relation ever closes, completes or unblocks anything by
itself. It authorizes no code, schema, migration, deployment or
live acceptance; those remain separately gated stages (§10).

Governing decisions consumed, not re-opened:

- **Component #9208 §2.1–§2.2, §5, §7** — canonical
  `source → target` storage, the seven user-facing spellings,
  "parent-child and GitHub native sub-issue hierarchy are not
  hard dependencies", "existing GitHub native `blockedBy`
  remains a blocker relationship", no auto-close, no implicit
  merge, removal as a tombstone, external refs carry provider,
  repository and stable remote id, never an issue number alone.
- **FDR-PMO-005** — one typed relation fact; only active hard
  `blocks` edges affect readiness; ambiguous or cyclic imports
  are quarantined, never repaired by deletion.
- **DEP-02** ([#9503](https://github.com/developing-today/code/issues/9503))
  owns the relation enum, alias normalisation and the JSON and
  error contract in `work/relations.rs`. This record uses those
  names; where it needs a value DEP-02 has not fixed (the
  `role` values in §3.2) it proposes one for DEP-02 to ratify.
- **DEP-03** ([#9443](https://github.com/developing-today/code/issues/9443),
  record `doc/2026-11-25T01-14-01Z_design_dep03_hard_dependency_dag/`)
  owns `dep_edges` and `dep_graph`, the cycle-safe insert, the
  graph revision, the readiness clause and the quarantine
  procedure for hard `blocks`. Its §2 hands DEP-04 "native
  GitHub links, all seven relations' projection", and its
  §5 leaves hierarchy cycles to DEP-04. This record adds rows
  of other relations to DEP-03's table and one nullable column;
  it changes none of DEP-03's rules for `blocks`.
- **CEN-04** ([#9434](https://github.com/developing-today/code/issues/9434))
  §5 field admission. §8 below supplies the entry for the one
  new column.
- **#10626 / #14268 staleness check**
  (`turso_queue/staleness.rs`) retires a task whose _own_
  linked issue already closed or PR already merged. That is a
  task's identity, not a relation, and this record leaves it
  untouched (§6.3).

## 1. Intent

Fleet work and GitHub resources are connected in seven
different ways, and only one of them means "do not start this
yet". The failure this slice exists to prevent is the quiet
promotion of one meaning into another:

1. A sub-issue's parent closing is read as the child being done,
   or a parent being open is read as the child being blocked.
2. A duplicate or superseded issue closing cascades a close or
   a completion onto the other end.
3. Issue `#42` in one repository is matched to `#42` in another.
4. A native link disappearing on GitHub silently deletes Fleet's
   record that it ever existed.

The contract below makes each relation's effect explicit,
stores every relation in the one table DEP-03 already guards,
and keeps the history of every link from first sight to removal.

## 2. Ownership

| Concern                                                         | Owner                                                                                                               |
| --------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------- |
| Relation enum, aliases, JSON and error envelope                 | DEP-02 (#9503)                                                                                                      |
| `dep_edges`/`dep_graph`, `blocks` insert, readiness, quarantine | DEP-03 (#9443)                                                                                                      |
| Effect of the other relations; hierarchy/supersedes cycles      | **DEP-04 (this)**                                                                                                   |
| Native GitHub link import, provider removal, user suppression   | **DEP-04 (this)**                                                                                                   |
| `dependency.graph`, `dependency.remove`, `--depth`              | **DEP-04 (this)**                                                                                                   |
| Reading native links from GitHub                                | existing `fleet gh` adapter (`pkgs/fleet/crates/fleet/src/gh.rs`, read-only consumer), #9073/#9074 metadata surface |
| Writing native links back to GitHub                             | not in scope (§12)                                                                                                  |
| Dependency pressure from the graph                              | PRI-04 (#9480)                                                                                                      |

Owned files when implemented, unchanged from the slice:
`pkgs/fleet/crates/fleet-queue/src/work/relations.rs` and
`pkgs/fleet/crates/fleet-queue/tests/dependency_github_relations.rs`.
`relations.rs` is also DEP-03's file, so DEP-04 lands after
DEP-03 (the package's shared-file sequencing edge). No new
queue, database, scheduler, service, executable or GitHub client
is added; the GitHub read goes through the existing adapter.

## 3. The seven relations

### 3.1 Mapping and effect

Each spelling normalises to one stored fact. The "Fleet end"
column says which side of the stored edge must be Fleet work
(§3.3).

| User spelling    | Stored `relation` | Stored direction              | `role`                                        | Execution effect                                | Resolvable? |
| ---------------- | ----------------- | ----------------------------- | --------------------------------------------- | ----------------------------------------------- | ----------- |
| `A blocks B`     | `blocks`          | `A → B` (blocker → dependent) | NULL                                          | If hard and active, B is not ready (DEP-03 §6). | yes         |
| `B blocked_by A` | `blocks`          | `A → B` — the same row        | NULL                                          | Same edge; the spelling is an alias.            | yes         |
| `A related_to B` | `related_to`      | ordered pair (§3.3)           | NULL                                          | None. Advisory, display only.                   | no          |
| `P parent_of C`  | `hierarchy`       | `P → C` (parent → child)      | NULL                                          | None on readiness, state or completion.         | no          |
| `C child_of P`   | `hierarchy`       | `P → C` — the same row        | NULL                                          | Same edge; alias.                               | no          |
| `A duplicates B` | `duplicates`      | ordered pair (§3.3)           | `source_is_duplicate` / `target_is_duplicate` | None. No close, no merge, no completion.        | no          |
| `N supersedes O` | `supersedes`      | `N → O` (newer → superseded)  | NULL                                          | None. Historical link only.                     | no          |

Rules that hold for every row:

- **Only `blocks` gates.** DEP-03's claim clause already filters
  `relation = 'blocks' AND hardness = 'hard'`; every other
  relation is excluded by construction, not by a second check.
  A `hierarchy`, `related_to`, `duplicates` or `supersedes` edge
  is always stored `hardness = 'soft'`, and an add that asks for
  `--hard` on one of them is refused with `relation_not_gating`
  rather than silently downgraded.
- **Only `blocks` resolves.** The other four are facts about how
  two pieces of work relate, not conditions that become true.
  They stay `active` whatever state either end reaches, until a
  person or the importer removes them (§5). A closed parent, a
  merged superseding PR or a closed duplicate changes nothing
  stored; it is visible by reading the other end's state.
- **No relation writes state.** No edge of any relation causes
  a close, a reopen, a completion, a cancellation, a priority
  change or a label change on either end, in Fleet or on GitHub.
  This is the "never auto-close" rule, and it is enforced by the
  absence of any such code path, checked by the oracle in §11.

### 3.2 The `role` column and why `duplicates` needs it

`duplicates` is stored as an ordered pair so that "A duplicates
B" and "B duplicates A" cannot both be active as separate facts
(#9208 §2.1: "normalize stable pair ordering"). But the two
statements mean different things — which one is the canonical
copy — and normalising the pair would lose that. The proposed
additive nullable column `role` records it:
`source_is_duplicate`, `target_is_duplicate`, or NULL when the
statement came without a side (an import that only knows the two
are duplicates). DEP-02 ratifies the spellings. `role` is NULL
for every other relation; hierarchy's "typed role" from #9208 §2.1
is carried by direction (`parent → child`) and needs no column.
A `CHECK` keeps `role` NULL outside `duplicates`.

Because the partial unique index is on
`(relation, source_key, target_task_id)`, a second statement of
the same pair with a different side is a conflict, not a new
row: the add returns `relation_role_conflict` with the current
row, and changing the side is an explicit remove-and-add with a
reason.

### 3.3 Which end is Fleet work

DEP-03 fixes `target_task_id NOT NULL`: the stored target is
always Fleet work, because only Fleet work can be held back by
Fleet's claim path. For `blocks` that is natural (the dependent
is Fleet work). For the other relations this record decides:

- **Directional relations** (`hierarchy`, `supersedes`) are
  stored when their canonical target is Fleet work. "External
  parent P of Fleet task C" stores `P → C`. "Fleet task P is
  parent of external issue X with no Fleet work" stores nothing
  as an edge; it stays a provider fact shown in hydration (§4.4)
  and becomes an edge the moment X gains Fleet work.
- **Pair relations** (`related_to`, `duplicates`) put the Fleet
  end on the target side when only one end is Fleet work. When
  both ends are Fleet work in the same authority, the lower task
  id is the source. That ordering is the "stable pair ordering"
  and is what makes the unique index catch the reversed
  statement.
- **Neither end is Fleet work.** Not stored. Fleet has nothing to
  schedule on either side; the link is GitHub's fact and stays
  there.

This keeps every DEP-04 row inside DEP-03's table, indexes and
readiness clause with no change to `target_task_id`, and it means
"the edges of task T" is always one indexed read on target plus
one on source.

### 3.4 Identity: never a number alone

An external end is DEP-02's canonical ref: provider, repository
full name, resource kind, number **and** the provider's stable
node id, in `source_ref`. `source_key` keeps DEP-03's
`issue:owner/repo#N` / `pr:owner/repo#N` shape so the #8276 probe
cache still serves both. Two consequences:

- `#42` in `a/x` and `#42` in `a/y` have different keys and can
  never collide (negative case "same number cross-repo").
- A repository rename or issue transfer changes the key but not
  the node id. The importer detects that from the node id and
  writes a `removed` row (`resolution.reason = provider_moved`)
  plus a new `active` row carrying the new key, in one
  transaction, with `resolution.successor_edge_id` linking them.
  History follows the resource, not the number.

A bare `#42` on the command line is accepted only when the work's
own linked repository makes it unambiguous; otherwise the add
returns `ambiguous_ref` and asks for the qualified ref (#9208
§2.4).

### 3.5 Cycles outside `blocks`

| Relation                   | Cycle meaning           | Rule                                                                                                              |
| -------------------------- | ----------------------- | ----------------------------------------------------------------------------------------------------------------- |
| `blocks`                   | deadlock                | DEP-03 §5 (refuse on add, quarantine on import)                                                                   |
| `hierarchy`                | A is its own ancestor   | same check as DEP-03 §5, run over `relation = 'hierarchy'`; refuse `hierarchy_cycle` on add, quarantine on import |
| `supersedes`               | A replaces B replaces A | same check over `relation = 'supersedes'`; refuse `supersedes_cycle`, quarantine on import                        |
| `related_to`, `duplicates` | not meaningful (pairs)  | no check; the unique index stops the reversed duplicate                                                           |

The hierarchy and supersedes checks reuse DEP-03's recursive CTE
with the relation filter changed, inside the same `BEGIN
IMMEDIATE` and the same `dep_graph` revision CAS, with the same
`max_reach` bound and the same fail-closed
`graph_evaluation_incomplete` refusal. A quarantined hierarchy or
supersedes edge blocks nothing (these relations never gate); it
is quarantined so that graph walks and `dependency.graph` cannot
loop, and it surfaces in the same `unexpected_active_cycle`
health signal DEP-03 defines. Repair is DEP-03 §7's procedure.

Every mutation of every relation bumps `dep_graph.revision`, as
DEP-03 §4 already states. The cost is that a hierarchy edit
invalidates an in-flight selection snapshot pinned to the old
revision; the claim path then re-reads, which is the existing
`VERSION_CONFLICT` retry and not new behaviour. A separate
per-relation revision was considered and rejected: two counters
that both feed `fleet.selection-order/v1` would need their own
CAS ordering, for a cost (a rare extra re-read) that has not been
measured as a problem.

## 4. Importing native GitHub links

### 4.1 What is imported

The existing `fleet gh` adapter reads, for an issue or PR that a
Fleet task links to, the native relations GitHub exposes. This
record names the facts it needs; the adapter slice verifies the
exact fields and endpoints (that verification is a gate, §10):

| GitHub fact                                                                                                      | Imported as                                                                               |
| ---------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------- |
| issue X is blocked by issue B (native dependency)                                                                | `blocks`, `B → T` for every Fleet task T linked to X                                      |
| issue X blocks issue D                                                                                           | nothing from X's side; D's own import writes `X → T_D` if D has Fleet work                |
| issue X has parent P (native sub-issue)                                                                          | `hierarchy`, `P → T`                                                                      |
| issue X has sub-issue C                                                                                          | nothing from X's side; C's own import writes `X → T_C` if C has Fleet work                |
| X closed as duplicate of Y, with Y named                                                                         | `duplicates`, pair per §3.3, `role` naming X as the duplicate                             |
| X closed as duplicate with no target named                                                                       | nothing; a `dep_import_unmapped` event records it                                         |
| Project item "parent" field or tracked-by prose, task-list checkboxes, "blocked by #N" text in a body or comment | **nothing.** Prose is never parsed into an edge (#9208 §6: "Reject implicit resolution"). |

Each relation is imported from exactly one side (the side whose
Fleet task becomes the edge's target), so a link is never written
twice by two imports racing; if they do race, the unique index
makes the second an idempotent replay.

Every imported row carries `reason_code = imported`,
`certainty = verified` (GitHub's own native fact, not an
inference), and a `provenance.source_ref` naming the provider,
the resource read and the read time.

### 4.2 Import mode: shadow before enforce

Importing a native `blockedBy` as a hard edge changes which tasks
can be claimed the moment it lands. That is the point, but
switching it on across every repository at once could strand
work that lanes are mid-way through. Import therefore runs per
repository in one of three modes, set by configuration the
implementation slice names:

- `off` — nothing imported (the default until enabled).
- `shadow` — native blockers are written as `blocks` with
  `hardness = 'soft'` and `resolution` empty. They gate nothing.
  `dependency.blockers --explain` shows them as "would block",
  and an import report counts tasks that `enforce` would hold.
  Hierarchy, duplicates and supersedes are written as normal
  (they never gate in any mode).
- `enforce` — native blockers are written `hardness = 'hard'`
  through DEP-03's insert check. Switching a repository from
  `shadow` to `enforce` upgrades its existing imported soft
  `blocks` rows to hard, each through the same check, so a cycle
  among imports is quarantined at that moment rather than
  committed.

The mode never touches edges a person added: `reason_code =
explicit` rows keep the hardness they were given.

### 4.3 Recheck, not webhook

Imports run on the existing #8276 recheck sweep's cadence for
tasks whose linked resource is in an importing repository, using
the same probe batching and cache, so no new schedule is
created. A `Failed` read (rate limit, auth, network) changes
nothing and reschedules, exactly as DEP-03 §6.3. A `Broken` read
(404) of the task's own linked resource stops importing for that
task and raises one `dep_import_source_broken` event; it does not
remove existing imported edges (§5.2).

### 4.4 Links that are not edges

Native links whose canonical target has no Fleet work (§3.3) are
still useful context. The hydrator (#9207) records them in the
task's hydration manifest as provider evidence — kind, other
end's ref, read time — read-only and versioned with the rest of
the snapshot. They are not rows in `dep_edges`, they do not bump
the graph revision, and nothing reads them for readiness.

## 5. Removal, suppression and history

### 5.1 Removal is a tombstone

`dependency.remove` (allocated to this slice) moves an edge of any
relation `active → removed`, increments the edge revision and
`dep_graph.revision`, fills `resolution` with
`{reason, actor, operation_id, at}` and writes one
`dep_edge_removed` event — all in one transaction, with the
edge-level and graph-level CAS DEP-03 §4 defines. The row is never
deleted. `--expected-version` is required for interactive use;
a stale version returns `VERSION_CONFLICT` with the current row.
Removing a hard `blocks` edge is a high-impact transition under
#9208 §4 and needs the AUTH-01 capability for it.

### 5.2 Provider removal

When GitHub no longer reports a native link that an imported row
recorded, the importer writes `removed` with
`resolution.reason = provider_unlinked` and the read that showed
it gone. Only rows with `reason_code = imported` and a GitHub
`provenance.source_ref` are eligible: an import never removes or
edits a row a person added, even if it states the same link. A
`Failed` or `Broken` read is never evidence of removal; only a
successful read that lists the resource's relations and omits
this one counts.

### 5.3 User suppression of an imported link

If a person removes an imported edge while the native link is
still on GitHub, the next import would re-add it. Instead the
importer looks for a `removed` row with the same
`(relation, source_key, target_task_id)` whose
`resolution.reason = user_removed` and whose recorded provider
read matches the current one. While it matches, the importer
does not re-add and raises one `dep_import_suppressed` event per
import that sees it, so the divergence between Fleet and GitHub
is visible and not repeated silently. If GitHub's link is removed
and later re-added (a new provider fact), the suppression no
longer matches and the link imports normally.

### 5.4 What history can answer

Because rows are never deleted and every transition writes an
event, the following are answerable from stored data alone,
which is #9208's "preserve tombstone/resolution history":

- every relation a task has ever had, with who added it, from
  where, and when (`dependency.list --include-resolved`);
- for each removed or resolved edge, why and on whose evidence;
- the chain of a moved resource (`successor_edge_id`);
- every import that was suppressed or unmapped (events).

## 6. Readiness and completion: the negative cases

### 6.1 Hierarchy never blocks

A child whose parent is open is ready if it has no unsatisfied
hard `blocks` edge. A parent whose children are open is ready
likewise. The first failing oracle,
`pmo_dep_github_hierarchy_is_not_hard_blocking`, builds exactly
this from a native fixture: parent P with sub-issues C1, C2;
C2 natively blocked by issue B. Expected: C1 ready, C2 not ready
with B named as its only blocker, P ready; then close P on the
fixture and re-import: C1 and C2's readiness and states are
unchanged and no event names a completion.

### 6.2 Closing an end completes nothing

- Parent/epic closes: children stay in whatever state they were
  (#9208 §7 "do not cascade child completed states").
- Duplicate target closes, or the duplicate closes: neither end's
  Fleet task changes state.
- Superseding work merges: the superseded task is not completed
  or cancelled; whether to cancel it is a person's call.
- A `blocks` source closes as completed: only that one edge
  resolves (DEP-03 §6.3); other edges on the dependent are
  untouched.

### 6.3 Coexistence with the staleness check

The #10626 claim-time staleness check retires a task whose own
linked issue closed or PR merged, with the #14268 exemptions.
That is the task's identity link, not a relation, and DEP-04
does not change it. What DEP-04 adds is a boundary: **no relation
edge is ever an input to the staleness check.** A task whose
parent epic, duplicate peer or superseding work closed is not
"already done" by that fact. The implementation slice keeps
`staleness.rs` reading only the packet's own linked work, and the
oracle in §11 asserts a related-end close leaves the task
claimable.

## 7. Commands allocated to this slice

Allocated per `census/command-slice-map.jsonl`:
`dependency.graph`, `dependency.remove`, `flag.depth`. All use the
shared Command/Result envelope (CLI-02), are parsed strictly
before side effects, and register through CLI-13; no shortcut
around `fleet queue`'s parser.

```text
fleet dependency graph <work-id> [--relation R]... [--direction up|down|both]
                                 [--depth N] [--include-resolved]
                                 [--cursor TOKEN] [--json]
fleet dependency remove <work-id> --edge <edge-id> --reason CODE
                                  --expected-version N [--dry-run] [--json]
```

- `graph` walks from the work along the chosen relations
  (default all) at one pinned `dep_graph.revision`, returning
  nodes, edges (with relation, role, hardness, status, certainty,
  provenance) and that revision. `--depth` bounds the walk
  (default 3, maximum set by the implementation and reported).
  A walk cut short by `--depth`, by the node budget or by
  pagination returns `"complete": false` with the reason and a
  continuation cursor — never a silently truncated graph (#9208
  §2.3). A cursor taken at an older revision returns
  `VERSION_CONFLICT` rather than mixing two snapshots.
  Quarantined edges are included and marked; they stop the walk
  along that edge so a quarantined cycle cannot loop it.
- `remove` is §5.1. `--dry-run` is read-only: it reports the
  edge that would be removed and the resulting readiness of the
  target, and writes no edge, event or revision (#9208 §4).

## 8. Persisted field — CEN-04 §5 admission entry

One column is added to DEP-03's `dep_edges`:

```sql
ALTER TABLE dep_edges ADD COLUMN role TEXT
  CHECK (role IS NULL OR (relation = 'duplicates'
         AND role IN ('source_is_duplicate', 'target_is_duplicate')));
```

| CEN-04 §5 item      | Entry                                                                     |
| ------------------- | ------------------------------------------------------------------------- |
| Owner               | #9208 typed model (DEP-04); broker store holds it (CEN-04 §3.5 "extend")  |
| Purpose             | Which end of a `duplicates` pair is the duplicate (§3.2)                  |
| Writers             | `dependency.add` for `duplicates`; the importer (§4.1)                    |
| Readers             | `dependency.list`, `dependency.graph`, hydration                          |
| Nullability/default | NULL; NULL for every relation but `duplicates`, and for side-less imports |
| Compatibility       | Additive; existing rows read NULL; no reader of DEP-03 columns changes    |
| Rollback            | Column left in place and ignored; no destructive drop (CEN-04 §7)         |
| Privacy             | Not private; no note or comment text                                      |

No other new persisted field is introduced: the import mode is
configuration, suppression and moves live in the existing
`resolution` JSON, and events use the existing `events` table.

## 9. Failure and recovery

| Scenario                                            | Behaviour                                                                                        |
| --------------------------------------------------- | ------------------------------------------------------------------------------------------------ |
| Native read rate-limited / auth fails               | Nothing imported, nothing removed; recheck rescheduled                                           |
| Task's linked issue 404s                            | Import stops for that task; existing imported rows kept; one `dep_import_source_broken` event    |
| Native blockers form a cycle under `enforce`        | DEP-03 quarantine; dependents stay not ready until repaired                                      |
| Native sub-issue cycle                              | Hierarchy edge quarantined; nothing blocked; health signal raised                                |
| Same number, two repositories                       | Different keys; never matched                                                                    |
| Issue transferred / repo renamed                    | `removed` (`provider_moved`) + new row, linked by `successor_edge_id`                            |
| User removed an imported link that GitHub still has | Suppressed on re-import; `dep_import_suppressed` event                                           |
| Duplicate stated both ways                          | Second is `relation_role_conflict`; change via remove and add                                    |
| `--hard` asked on a non-`blocks` relation           | Refused, `relation_not_gating`                                                                   |
| Import and a person add the same link concurrently  | Unique index: one row; the loser is an idempotent replay; person's row keeps `explicit` if first |
| Epic closes while children open                     | No state change; readiness unchanged                                                             |
| Repository switched `shadow → enforce`              | Imported soft blockers upgraded through the insert check, one at a time; cycles quarantined      |
| Rollback of the slice                               | Import set to `off`; rows and history kept; `blocks` rows keep gating per DEP-03                 |

## 10. Gates and sequencing

Implementation of this slice starts only after all of:

- PR #9225 merged (done).
- DEP-02 (#9503): relation enum, aliases, `role` spellings,
  error codes (`relation_not_gating`, `relation_role_conflict`,
  `hierarchy_cycle`, `supersedes_cycle`, `ambiguous_ref`).
- DEP-03 (#9443) implemented: the table, insert check, revision
  and quarantine this slice extends. Shared file
  `work/relations.rs` sequences DEP-04 after DEP-03.
- CLI-02 (#9438): the shared command contract and strict parser.
- CEN-04 (#9434): the §5 amendment admitting `role` merged before
  the first write.
- #9073/#9074: the metadata surface, if the adapter's native-link
  read is routed through it.
- An adapter verification that the native dependency,
  sub-issue and duplicate facts in §4.1 are readable through the
  existing `fleet gh` client, with recorded fixtures. If any fact
  is not readable, that row of §4.1 is dropped for the slice and
  recorded as unsupported, not approximated from prose.

Separately authorized later: enabling `shadow` or `enforce` on a
live repository, any migration of existing #8276 `blocked_deps`
into typed edges (DEP-01's crosswalk), and any write of native
links back to GitHub.

## 11. Acceptance oracle, specified

All are future tests, written failing first, listed by exact
name with `-- --list` before running (a zero-test filter is not
acceptance), under
`cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib --features client,queue-broker <name>`
unless noted; the GitHub fixtures live in
`tests/dependency_github_relations.rs`.

1. `pmo_dep_github_hierarchy_is_not_hard_blocking` — §6.1, the
   slice's first failing test.
2. `pmo_dep_seven_spellings_normalise_to_five_relations` —
   each spelling and its inverse land on one row; the reversed
   statement of a pair relation hits the unique index.
3. `pmo_dep_non_blocks_relation_refuses_hard` — `--hard` on
   each non-`blocks` relation returns `relation_not_gating`.
4. `pmo_dep_no_relation_writes_state` — for every relation, close
   / merge / complete / cancel either end; assert no state,
   label or priority write on the other end and no completion
   event.
5. `pmo_dep_closed_related_end_is_not_staleness` — a task whose
   parent, duplicate peer or superseder closed stays claimable
   through the default pick.
6. `pmo_dep_same_number_cross_repo_never_matches` — `#42` in two
   repositories yields two keys and two independent readiness
   answers.
7. `pmo_dep_provider_move_keeps_history` — transfer fixture;
   one `removed` row with `provider_moved`, one active successor,
   linked.
8. `pmo_dep_provider_unlink_tombstones_import_only` — native link
   removed; the imported row is `removed`, a person's row for the
   same link is untouched; a `Failed` read removes nothing.
9. `pmo_dep_user_removal_suppresses_reimport` — §5.3, including
   the re-add-on-GitHub case importing again.
10. `pmo_dep_hierarchy_cycle_refused_and_quarantined` — add
    refuses; import quarantines; nothing becomes not ready.
11. `pmo_dep_shadow_mode_never_gates` — native blocker in
    `shadow` leaves the dependent claimable and appears in
    `--explain` as "would block"; switching to `enforce` makes it
    not ready, and a cyclic pair is quarantined at the switch.
12. `pmo_dep_graph_partial_is_never_complete` — `--depth`,
    node budget and pagination each yield `complete: false` with
    a cursor; a cursor across a revision bump is
    `VERSION_CONFLICT`.

## 12. Non-goals

- Writing native links, sub-issues or duplicate marks back to
  GitHub. Fleet → GitHub link mirroring would be an outbox
  operation under #9209/AUTH and needs its own design.
- Parsing prose ("blocked by #N", task lists, Project parent
  fields) into edges.
- Any automatic close, reopen, merge, completion, cancellation
  or priority change driven by a relation.
- Pressure or urgency from non-`blocks` relations (PRI-04: they
  contribute none).
- Migrating #8276 `blocked_deps` rows (DEP-01).
- Hierarchy rollups (a parent's percent done, child counts). A
  read model may compute them later from `dependency.graph`; none
  is stored here.

## 13. Requirement trace (design-level)

| Requirement                                                                    | Where addressed                                                                |
| ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------ |
| C-R129                                                                         | §3.1 all seven spellings                                                       |
| C-R130                                                                         | §3.1 only hard `blocks` gates; others refuse `--hard`                          |
| C-R131                                                                         | §3.5 cycles; §5 tombstones and history                                         |
| C-R132                                                                         | §4 native links projected, prose never parsed                                  |
| C-R124, C-R125                                                                 | §3.1 no relation writes title, body or state                                   |
| O-R008                                                                         | §3.1, §12 non-gating relations add no pressure                                 |
| O-R033                                                                         | §5.4 resolved history retained; unblocking erases nothing                      |
| C-R107, C-R108, H-R017, O-R023                                                 | §7 commands and flags through CLI-02/CLI-13                                    |
| H-R024, H-R031                                                                 | §9 failure matrix, §11 negative tests                                          |
| P-R009, P-R013, P-R019, P-R027, S-R006, C-R005, C-R009, C-R123, H-R001, O-R022 | consumed via the component trace; no further design-level change in this slice |

Implementation, parser tests, deployment and live acceptance for
each remain open evidence, as the slice body states.
