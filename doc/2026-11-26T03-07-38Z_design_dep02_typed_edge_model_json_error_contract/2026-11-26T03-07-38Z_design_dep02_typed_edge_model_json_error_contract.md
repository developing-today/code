# DEP-02 — Typed edge model, canonical direction, and JSON/error contract

Issue: #9503 (slice DEP-02 of #9208). Status: design record. It is not
an authorization to implement. It fixes the pure types in
`fleet-queue/src/work/relations.rs`:

- how the seven relation spellings become one canonical edge;
- how the two ends of an edge are named;
- the edge record and its before/after revision;
- the JSON views and the one error vocabulary that DEP-03, DEP-04,
  CLI-02 and CLI-13 already point at.

## 0. Governing decisions consumed

| Source                             | What this record takes from it                                                                                                                                           |
| ---------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| #9503 slice text                   | Owned file; red tests `pmo_dep_aliases_normalize_to_one_canonical_edge` and `pmo_dep_edge_wire_preserves_before_after`; done command; five negatives; no storage change. |
| #9208 §2.1–§2.4                    | Seven spellings, five canonical relations, canonical `source → target`; the edge record; external refs never a number alone; unresolved hard refs block by default.      |
| `work/contracts.rs` (CLI-01, main) | `WorkRef {authority_id, work_id}`, its `Ord`, `qualified_uri` = `fleet://<authority>/work/<id>`; canonical decimal helpers; `ResultError`; `fleet.result/v1`.            |
| CLI-02 record                      | The parser rejects a bare number (`ambiguous_target_form`); malformed values are `malformed_value`; resolution is behind the seam.                                       |
| CLI-13 record                      | Work refs take the forms this record defines; `--blocked-by` and `--blocks` are exclusive and one is required.                                                           |
| DEP-03 record                      | `dep_edges` columns; target is always Fleet work; `source_key` matches `dep_key`; transitions; override; the refusal names it hands to this record.                      |
| DEP-04 record                      | Pair ordering for `related_to`/`duplicates`; `role` values to ratify; which end is Fleet work; the five codes it hands to this record.                                   |
| CEN-04 record §3.5                 | `tasks.blocked_deps` is the operational subset; free-text block reasons are never parsed into edges; any new persisted field enters through CEN-04 §5.                   |

## 1. Verified state on main `71afc269`

- `work/relations.rs` does not exist. `work/mod.rs` declares
  `authority`, `completion`, `contracts`, `degraded`, `journal`,
  `lifecycle`, `operations`, `priority`, `routing`, `selection` and
  `sqlite_replica`.
- `work/contracts.rs` has `WorkRef`, canonical decimal compare and
  validate helpers, `ContractError` with stable `code()` strings
  (snake_case, except the two `OWNER_POLICY_*` codes),
  `ResultError {code, message}`, `Effect {field, requested,
  observed, verified}` and `CommandResult` (`fleet.result/v1`).
  `CommandResult` has no free payload field.
- `work/operations.rs` has `ResourceRef {repo, number}` parsed from
  `owner/repo#N`. It has no provider id and no kind.
- `turso_queue/core.rs::dep_key` names legacy deps as
  `pr:owner/repo#N`, `issue:owner/repo#N`, `ci:owner/repo#RUN` and
  `task:<id>`. `DepObs::Broken` (unknown reference) releases a
  legacy dep.
- No typed edge, edge JSON or relation error code exists in code.
  DEP-03 and DEP-04 are records only.

## 2. Intent

Every way of stating a relationship turns into exactly one canonical
edge, so the inverse statement of an edge is the same fact and not a
second one. Every edge end is named by a stable identity that cannot
collide across authorities or repositories. Every change to an edge
is a revision that keeps both its before and after. Every refusal has
one stable code. All of it is pure: no I/O, no storage, no clock.

## 3. Ownership

- **Owned:** new `pkgs/fleet/crates/fleet-queue/src/work/relations.rs`
  and its `#[cfg(test)] mod tests`.
- **Needed edit outside the owned list, named:** one line
  `pub mod relations;` in `work/mod.rs`.
- **Read-only:** `work/contracts.rs`, `turso_queue/core.rs::dep_key`
  (for key compatibility only).
- **Not touched:** the broker, its schema, `blocked_deps`, the
  client, `main.rs`, the registry.
- **Added nowhere:** no table, column, store, resolver, GitHub
  client or I/O. Resolution of a ref against GitHub or an authority
  is the caller's job; this module takes the resolved facts as
  inputs.

## 4. Decisions

### D1. Seven spellings, five relations, one direction

`RelationSpelling` is the closed input enum. `Relation` is the closed
canonical enum. `normalize(subject, spelling, object)` returns one
`CanonicalEdge {relation, source, target, role}`:

| Statement        | Canonical edge           | `role`                |
| ---------------- | ------------------------ | --------------------- |
| `A blocks B`     | `blocks`: A → B          | none                  |
| `A blocked_by B` | `blocks`: B → A          | none                  |
| `A parent_of B`  | `hierarchy`: A → B       | none                  |
| `A child_of B`   | `hierarchy`: B → A       | none                  |
| `A supersedes B` | `supersedes`: A → B      | none                  |
| `A related_to B` | `related_to`: pair order | none                  |
| `A duplicates B` | `duplicates`: pair order | which end is the copy |

- **Pair order** (DEP-04): when one end is Fleet work it is the
  target. When both are, the smaller `WorkRef` by its `Ord`
  (authority bytewise, then `work_id` numerically) is the source.
- **`role`** takes the spellings DEP-04 proposed, ratified here:
  `source_is_duplicate`, `target_is_duplicate`, or absent when an
  import only knows the two are duplicates. "A duplicates B" means A
  is the copy, so the role follows A after ordering. `role` is
  absent for every other relation.
- **No other spelling is accepted.** `superseded_by`, `depends_on`,
  `blocking`, a capitalised form or any other text is
  `unknown_relation`. Nothing is aliased by guess.

### D2. Edge ends

```rust
enum EdgeEnd { FleetWork(WorkRef), External(ExternalRef) }

struct ExternalRef {
    provider: Provider,          // github (closed enum)
    repository: String,          // provider's canonical owner/repo
    resource_kind: ResourceKind, // issue | pull_request | actions_run
    number: String,              // canonical decimal string
    node_id: Option<String>,     // provider's stable id
}
```

- **The target is always Fleet work** (DEP-03). A canonical edge
  whose target is external is refused `target_not_fleet_work`. A
  statement whose two ends are both external has nothing for Fleet
  to schedule and is refused the same way.
- **Identity key.** `EdgeEnd::key()` returns the string that
  DEP-03's `source_key` stores. It is byte-identical to
  `dep_key` for the legacy shapes, so one probe cache serves both:
  `issue:owner/repo#N`, `pr:owner/repo#N`, `ci:owner/repo#RUN`, and
  `task:<work_id>` for Fleet work in the local authority. Fleet work
  in another authority is `work:<authority_id>/<work_id>`.
- **Never a number alone.** `#42` in `a/x` and `#42` in `a/y` have
  different keys. Equality and the duplicate check use the key and
  the relation, never the number.
- **`node_id` is the identity, the name is the label.** The
  repository string is the provider's canonical name as the caller
  resolved it. A rename or transfer changes the key but not the
  `node_id`; DEP-04 handles that as remove-and-add with
  `provider_moved`.

### D3. Ref input forms

`parse_ref(text) -> RefInput` is pure syntax. `bind_ref(input,
&BindContext) -> Result<EdgeEnd, RelationError>` turns it into an
end, given the facts the caller resolved: the authenticated
authority (if exactly one), the work item's linked repositories,
and the provider's answer for an external lookup.

| Form                                                         | Meaning                                                                   |
| ------------------------------------------------------------ | ------------------------------------------------------------------------- |
| `fleet://<authority_id>/work/<work_id>`                      | Qualified Fleet work. Always accepted.                                    |
| `fleet://work/<work_id>`, `fleet:<work_id>`                  | Local shorthand. Bound only when the authority is unambiguous.            |
| `owner/repo#N`                                               | A GitHub issue or PR; the caller's lookup decides which.                  |
| `issue:owner/repo#N`, `pr:owner/repo#N`, `ci:owner/repo#RUN` | Kind given explicitly; the same spellings as `fleet queue block --dep`.   |
| `#N`                                                         | Bound only when the work item has exactly one linked repository (DEP-04). |
| `N` alone                                                    | Never a ref. The CLI-02 parser refuses it as `ambiguous_target_form`.     |

- A shorthand under zero or several authorities, or `#N` under zero
  or several linked repositories, is `ambiguous_ref`, with the
  qualified form named in the message.
- Text matching no form is `malformed_value` when it arrives through
  the CLI-02 parser, and `invalid_schema` when it arrives in JSON.
- The CLI-02 parser calls `parse_ref` for ref-typed values, so the
  syntax lives in one place.

### D4. The edge record

```json
{
  "schema": "fleet.dependency_edge/v1",
  "edge_id": "7",
  "revision": "3",
  "relation": "blocks",
  "role": null,
  "source": { "kind": "external", "provider": "github",
              "repository": "developing-today/code", "resource_kind": "issue",
              "number": "761", "node_id": "I_kwDO..." },
  "target": { "kind": "fleet_work", "work": { "authority_id": "local", "work_id": "842" } },
  "source_key": "issue:developing-today/code#761",
  "hardness": "hard",
  "status": "resolved",
  "reason_code": "explicit",
  "certainty": "verified",
  "provenance": { "principal_ref": "...", "actor_ref": "...", "session_ref": "...",
                  "operation_id": "...", "source_ref": null },
  "created_at": "2026-11-26T03:00:00Z",
  "updated_at": "2026-11-26T04:00:00Z",
  "resolution": { "reason": "blocker_satisfied", "evidence_ref": "...", "override": false,
                  "expires_at": null, "successor_edge_id": null }
}
```

Field rules, each checked by `Edge::validate`:

- `edge_id` and `revision` are canonical decimal strings, never
  numbers on the wire.
- `relation`, `hardness` (`hard`, `soft`), `status` (`active`,
  `resolved`, `removed`, `quarantined`), `reason_code` (`explicit`,
  `imported`, `migrated`, `inferred_pending_review`) and `certainty`
  (`verified`, `asserted`, `uncertain`) are closed enums. An unknown
  value is `invalid_schema` naming the field; it is never mapped to a
  near value.
- `reason_code` here is the edge's origin. Its Rust type is
  `EdgeOrigin`, so it is never confused with `contracts.rs`'s
  `ReasonCode` registry, which is about commands.
- Only `blocks` may be `hard`. Any other relation is always `soft`;
  asking for `hard` on it is `relation_not_gating`.
- `role` is present only on `duplicates`.
- `source` ≠ `target` by key, or `self_edge`.
- An external end without `node_id` is allowed only with
  `reason_code` `imported` or `migrated` and `certainty`
  `uncertain`. An explicit add whose external end the caller could
  not resolve is `unresolved_ref`. This keeps a number-only edge out
  of any explicit write while still letting DEP-03's importer keep
  legacy data as quarantined or uncertain.
- `certainty` `verified` needs a `node_id`.
- `resolution` is null when `status` is `active`, and present with a
  `reason` otherwise. `override: true` is allowed only with
  `status: resolved`.
- `provenance.principal_ref` is the authenticated principal the
  caller passes in. It is never read from a client flag.
- Unknown JSON fields are refused (`deny_unknown_fields`).

Field names match DEP-03's `dep_edges` columns, so this record
changes no storage: `source_key`, `resolution` and the enums map one
to one; `edge_id` and `revision` are the integers rendered as
decimal strings.

### D5. Status transitions and before/after

`transition(from, to)` is a closed table (DEP-03 §4, §6.4, §7):

| From          | To            | When                                                     |
| ------------- | ------------- | -------------------------------------------------------- |
| (none)        | `active`      | add                                                      |
| (none)        | `quarantined` | import of a cycle or an uncertain legacy dep             |
| `active`      | `resolved`    | resolve with evidence, automatic resolution, or override |
| `active`      | `removed`     | remove, with a reason                                    |
| `active`      | `quarantined` | import repair                                            |
| `quarantined` | `active`      | restore, through DEP-03's insert check                   |
| `quarantined` | `removed`     | repair that drops the edge, with a reason                |
| `resolved`    | `active`      | an override expires, through the same insert check       |

Any other pair is `invalid_transition`. `removed` is final; bringing
a removed fact back is a new add with a new `edge_id`.

Every change is one `EdgeRevision`:

```json
{
  "schema": "fleet.dependency_edge_revision/v1",
  "edge_id": "7", "revision": "3", "operation_id": "...",
  "before": { "status": "active", "hardness": "hard", "certainty": "verified",
              "role": null, "resolution": null },
  "after":  { "status": "resolved", "hardness": "hard", "certainty": "verified",
              "role": null, "resolution": { "reason": "blocker_satisfied", "...": "..." } }
}
```

- `before` is null only at revision `1`.
- `revision` is exactly one more than the previous revision.
- `relation`, `source`, `target` and `role` never change across
  revisions. Changing any of them is remove plus add.
- The revision is what DEP-03's `events` row carries and what
  JRN-01's journal reads; DEP-02 defines its shape only.

### D6. Perspective keeps the meaning

Storage keeps only canonical edges, but a reader asking about one
work item should see the relation as stated from that item.
`describe(edge, from) -> Perspective {spelling, other_end}`:

- For work X, a `blocks` edge with X as target reads `blocked_by`
  the source; with X as source it reads `blocks` the target.
- `hierarchy` reads `parent_of` or `child_of` the same way.
- `supersedes` reads `supersedes` from the newer end and
  `superseded_by` from the older one. `superseded_by` is a display
  word only; it is not an input spelling (D1).
- `duplicates` reads `duplicates` from the copy and `duplicated_by`
  from the kept item, or `duplicates` from both when `role` is
  absent. `duplicated_by` is display-only too.

### D7. The error vocabulary

One `RelationError` enum. Each variant has a stable `code()` and
maps into the shared `ResultError {code, message}`. Codes from other
slices are registered here once so there is a single list:

| Code                          | When                                                               | Raised by      |
| ----------------------------- | ------------------------------------------------------------------ | -------------- |
| `unknown_relation`            | Spelling not one of the seven.                                     | DEP-02         |
| `invalid_schema`              | Unknown field or enum value, malformed shape (shared code).        | DEP-02, CLI-01 |
| `self_edge`                   | Both ends have the same key.                                       | DEP-02         |
| `target_not_fleet_work`       | The canonical target is external, or neither end is Fleet work.    | DEP-02         |
| `relation_not_gating`         | `hard` asked on a relation other than `blocks`.                    | DEP-02         |
| `ambiguous_ref`               | A shorthand or `#N` that does not bind to exactly one end.         | DEP-02         |
| `unresolved_ref`              | An explicit add whose external end has no stable id.               | DEP-02         |
| `invalid_transition`          | A status move outside D5's table.                                  | DEP-02         |
| `duplicate_edge`              | An active edge with the same relation and keys exists.             | DEP-03         |
| `relation_role_conflict`      | Same `duplicates` pair, different `role`.                          | DEP-04         |
| `dependency_cycle`            | The add would close a `blocks` cycle.                              | DEP-03         |
| `hierarchy_cycle`             | The add would make an item its own ancestor.                       | DEP-04         |
| `supersedes_cycle`            | The add would close a `supersedes` cycle.                          | DEP-04         |
| `graph_evaluation_incomplete` | The check could not finish within its bound; refused, not assumed. | DEP-03         |
| `unexpected_active_cycle`     | An active cycle found on read; a repair signal.                    | DEP-03         |
| `VERSION_CONFLICT`            | `--expected-version` is stale.                                     | DEP-03         |

- `VERSION_CONFLICT` keeps the upper-case spelling that #9208,
  DEP-03 and DEP-04 already use, as `contracts.rs` does for
  `OWNER_POLICY_VERSION_CONFLICT`. Every other code is snake_case.
- `ambiguous_target_form` (bare number) and the other parse codes
  stay CLI-02's. They are never produced here.
- A refusal is outcome `rejected`, except `VERSION_CONFLICT` and
  `duplicate_edge`, which are `conflict`. Both conflicts return the
  current edge's `status` and `revision` as `effects` (`field`
  `edge:<id>.status` and `edge:<id>.revision`, `observed` set), since
  `fleet.result/v1` has no payload field.
- A replay with the same idempotency key and digest is not
  `duplicate_edge`; it returns the first result (shared contract).
- Adding a code later is compatible; renaming one is a breaking
  change.

### D8. Read views

Mutations answer with `fleet.result/v1` only. Reads return their own
documents, all snake_case with decimal-string versions:

- `fleet.dependency_list/v1`: `work`, `graph_revision`, `complete`,
  `cursor`, `edges[]`. Each edge is a D4 edge plus its D6
  `perspective`.
- `fleet.dependency_blockers/v1`: `work`, `graph_revision`,
  `complete`, `ready`, `blockers[]`. Each blocker is `edge_id`,
  `perspective`, `hardness`, `state` (`unsatisfied`,
  `unresolved_external`, `quarantined`, `override_active`) and
  `evidence_ref`.
- `ready` is `true` only when `complete` is `true` and no active hard
  `blocks` edge targets the work. An incomplete evaluation reports
  `complete: false` and `ready: false`, never `true`.
- DEP-04 owns the graph document's traversal fields; it reuses the
  D4 edge and D6 perspective inside it.

### D9. Legacy `blocked_deps`

`from_legacy_dep(&serde_json::Value) -> Result<EdgeEnd,
RelationError>` maps the four legacy kinds (`task`, `issue`, `pr`,
`ci`) to ends whose `key()` equals `dep_key` for the same value.
The resulting ends carry no `node_id`, so an edge built from them is
`imported` and `uncertain` until DEP-03's importer resolves them.

The legacy path releases a dep whose reference is broken
(`DepObs::Broken`); a typed hard edge does not, because an
unresolved hard ref blocks by default (#9208 §2.2). This record only
names that difference. DEP-03 owns the importer and the readiness
rule.

## 5. Negative cases

| Negative (slice text)               | Result                                                                                                                          | Test                                                |
| ----------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------- |
| Same GitHub number across repos     | `issue:a/x#42` and `issue:a/y#42` are different keys and two edges.                                                             | `pmo_dep_same_number_across_repos_is_two_ends`      |
| Inverse duplicate                   | `B blocked_by A` after `A blocks B` produces the same canonical edge, so the store's unique key refuses it as `duplicate_edge`. | `pmo_dep_aliases_normalize_to_one_canonical_edge`   |
| Unknown enum                        | `unknown_relation` for a spelling; `invalid_schema` naming the field for any other enum.                                        | `pmo_dep_unknown_enum_is_refused_not_aliased`       |
| Shorthand under ambiguous authority | `fleet:842` with two authorities in context is `ambiguous_ref`.                                                                 | `pmo_dep_shorthand_needs_one_authority`             |
| Unresolved external target          | Explicit add: `unresolved_ref`. Import: allowed only as `imported` and `uncertain`.                                             | `pmo_dep_unresolved_external_ref_is_never_explicit` |
| `--hard` on an advisory relation    | `relation_not_gating`.                                                                                                          | `pmo_dep_only_blocks_can_be_hard`                   |
| External target                     | `target_not_fleet_work`.                                                                                                        | `pmo_dep_target_is_always_fleet_work`               |

## 6. Oracles and tests

All tests are unit tests in `work/relations.rs`, named with the
`pmo_dep_` prefix so the slice's done command selects them:

| Test                                                    | Asserts                                                                                                                                     |
| ------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------- |
| `pmo_dep_aliases_normalize_to_one_canonical_edge` (red) | Every statement and its inverse give equal `CanonicalEdge`s, for all seven spellings; `duplicates` keeps the stated copy in `role`.         |
| `pmo_dep_edge_wire_preserves_before_after` (red)        | An `EdgeRevision` round-trips through JSON byte-for-byte; `before`/`after` both survive; versions stay strings; unknown fields are refused. |
| `pmo_dep_same_number_across_repos_is_two_ends`          | Keys differ by repository; equal numbers never compare equal.                                                                               |
| `pmo_dep_unknown_enum_is_refused_not_aliased`           | Every closed enum refuses a near-miss value with the right code.                                                                            |
| `pmo_dep_shorthand_needs_one_authority`                 | Shorthand binds with one authority, refuses with zero or two; the qualified form always binds.                                              |
| `pmo_dep_unresolved_external_ref_is_never_explicit`     | Explicit without `node_id` is refused; imported without `node_id` validates only as `uncertain`.                                            |
| `pmo_dep_only_blocks_can_be_hard`                       | `hard` on each of the other four relations is `relation_not_gating`.                                                                        |
| `pmo_dep_target_is_always_fleet_work`                   | External target and external-to-external statements are refused.                                                                            |
| `pmo_dep_transitions_are_closed`                        | Exactly the eight D5 transitions are accepted; every other pair is `invalid_transition`.                                                    |
| `pmo_dep_identity_never_changes_across_revisions`       | A revision that changes relation, an end or role is refused.                                                                                |
| `pmo_dep_perspective_preserves_meaning`                 | `describe` gives the stated spelling from either end for every relation.                                                                    |
| `pmo_dep_legacy_keys_match_dep_key`                     | For each legacy kind, `from_legacy_dep(v).key() == dep_key(v)`.                                                                             |
| `pmo_dep_error_codes_are_stable`                        | The D7 table, code by code, against `RelationError::code()`.                                                                                |

A property test generates random pairs of ends and spellings and
checks that `normalize` of a statement and of its inverse are equal,
and that `describe(normalize(s), subject)` returns `s`'s spelling or
its D6 display inverse.

Done command, as the slice states, run remotely:

```text
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib \
  --features client,queue-broker pmo_dep_ -- --list
cargo test --manifest-path pkgs/fleet/Cargo.toml -p fleet-queue --lib \
  --features client,queue-broker pmo_dep_
```

The first must list each test above by exact name; a filter that
matches zero tests is not a pass. Both run through a
`fleet-rust-tests.yml` dispatch on the branch, never on a laptop or
sandbox. If that workflow's whole-workspace run does not enable both
features for `fleet-queue`, the implementation PR adds one step with
this exact command (a named edit to the workflow).

## 7. Compatibility and reversal

No storage, schema, wire or CLI change ships with this module: it is
types and pure functions that nothing calls until DEP-03 and CLI-13
land. `blocked_deps`, `dep_key`, `fleet queue block --dep` and the
recheck sweep are untouched. Reversal is deleting the file and its
`mod` line.

## 8. Requirement trace

| Requirement                                                           | How this record serves it                                                                                   |
| --------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| C-R129, P-R019                                                        | All seven spellings, normalised to five canonical relations (D1).                                           |
| C-R130, O-R008, S-R006                                                | Only `blocks` can be hard; every other relation is advisory by type (D4). Pressure stays with PRI slices.   |
| C-R131, O-R033                                                        | Resolved and removed are revisions that keep before/after, never deletes (D5); cycle codes registered (D7). |
| C-R132                                                                | Native GitHub ends carry provider, repo, kind, number and node id (D2); DEP-04 projects them.               |
| C-R123, C-R124, C-R125                                                | Edges are Fleet facts separate from GitHub titles and bodies; nothing here mirrors text.                    |
| O-R026, P-R013                                                        | Stable keys by authority or provider id; never a number alone (D2, D3).                                     |
| C-R005, C-R107, C-R108, O-R023, P-R009, P-R012                        | Ref forms and error codes the CLI registers (D3, D7) consumed by CLI-02 and CLI-13.                         |
| C-R009, O-R021, O-R022, P-R002                                        | Agents read dependencies through one typed model; the module is pure and the resolver stays separate (§3).  |
| C-R150–C-R153, O-R049, O-R050, O-R052, P-R035, P-R036, H-R016, P-R027 | Scenario B's `fleet:842 --blocked-by fleet:761 --hard` binds under D3; blockers and readiness views (D8).   |
| H-R001, H-R014, H-R017, H-R024, H-R031                                | Every spelling, error and shape from the sources is kept and tested (D1, D7, §6).                           |

## 9. Gates

1. This record merged.
2. `work/contracts.rs` identity and result types: on main (CLI-01,
   closed).
3. CEN-04 (#9434) disposition for `blocked_deps`: its record is on
   main. Nothing here persists, so no CEN-04 §5 amendment is needed
   for this slice; DEP-04's `role` column still needs one.
4. The module and its tests can then land on their own. DEP-03,
   DEP-04 and CLI-13 consume it.

## 10. Non-goals

- Storage, migrations, the importer, cycle checks, readiness
  evaluation, overrides (DEP-03).
- GitHub native link projection and hierarchy validation (DEP-04).
- CLI parsing and registration (CLI-02, CLI-13).
- Journal schema (JRN-01) and dependency pressure (PRI slices).
- Resolving refs against GitHub or an authority.
