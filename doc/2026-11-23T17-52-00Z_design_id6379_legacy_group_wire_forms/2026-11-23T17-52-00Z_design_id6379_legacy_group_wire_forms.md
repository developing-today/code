# ID groups: legacy wire forms and their migration path (#6379 F6)

- Design record · Issue: developing-today/code#13533 (finding F6 of the
  #6379 stage-1 audit, task 14441; routed by the 2026-11-17 triage,
  task 18481) · Status: dispositions recorded; this record changes no
  code. Verified against `origin/main` `074d39603ac` on 2026-11-23.
- This record satisfies #6379's "migration path for deprecated
  features" acceptance item: every read-compat legacy form in the
  groups event surface is named below with the modern form that
  replaces it and a disposition — a removal condition, or retention
  with its reason — and the silent `cascade` default is stated as a
  decision (§4).

## 1. Scope and method

The groups event surface of `pkgs/id`: the fold and policy in
`crates/id-events/src/groups/` and the log and actions in
`crates/id-groups/src/groups/`. The inventory was taken by reading
both modules and grepping them for `legacy`, serde aliases,
`untagged`, `#[serde(other)]` and `#[serde(default)]`. The complete
read-compat surface is three constructs, all in `id-events`;
`id-groups` declares none of its own — it consumes the same
`MemberOp` type, and its log test pins the legacy mapping (§2).

The stage-1 citations were pre-split paths. Re-located in the split
tree: the `successor` wire form lives in
`crates/id-events/src/groups/mod.rs` (the test that pins it is in
`crates/id-groups/src/groups/log.rs`), and the delegation default in
`crates/id-events/src/groups/policy.rs`.

## 2. L1 — `Remove` event with `successor` and no `mode` (read-compat only)

- **Where:** `MemberOpWire::Remove` and the hand-written
  `Deserialize for MemberOp` in
  `pkgs/id/crates/id-events/src/groups/mod.rs`.
- **Legacy form:** `"Remove": { "member": <key>, "successor": null | <key> }`
  with no `mode` field.
- **Deterministic mapping:** `mode` absent and `successor` absent or
  null → `RemoveMode::Revoke`; `mode` absent and `successor` a key →
  `RemoveMode::Transfer(TransferOpts { successor: RemoveSuccessor::Member(key) })`.
  When `mode` is present it governs and `successor` is ignored.
- **Modern form:** every recorded `Remove` carries a concrete `mode`
  (`Revoke`, `Transfer` with successor `Member` / `Admin` /
  `Designated` / `Actor`, `KeepPrior`, `ReassignToGroup`).
  `ServerDefault` is resolved before recording and never appears in
  a replicated event. The serializer never emits `successor`; both
  mapping branches and the no-`successor` encoding are pinned by
  `test_legacy_remove_events_map_successor_to_a_deterministic_mode`
  in `pkgs/id/crates/id-groups/src/groups/log.rs`. Note the mapping
  is narrower than the modern form: a legacy transfer can only name
  a concrete member key, so no legacy event can be misread as one of
  the successor categories that have no legacy encoding.
- **Disposition: retained indefinitely, read-only.** Group event
  logs are append-only replicated history; any node must replay a
  log written by an older build, including logs held by peers that
  have not upgraded. The mapping is total and deterministic, so
  retention costs one decoder branch and creates no ambiguity.
  Removal condition: only inside a future log-format version change
  that rewrites stored logs to the modern form (a snapshot or
  compaction migration that re-encodes every historical `Remove`
  with its mapped mode); until such a migration exists and has run
  over every replica, the decoder branch stays. The write side of
  this migration is already complete — nothing emits the legacy
  form.

## 3. L2 — group-wide delegation preference (`Policy.delegation`)

- **Where:** `pub delegation: Option<Delegation>` on `Policy` in
  `pkgs/id/crates/id-events/src/groups/policy.rs`; `Delegation` is
  `KeepPrior` / `Cascade` / `Transfer`, each documented in code as a
  legacy preference.
- **What it still does:** exactly one live read. A removal never
  consults it — a removal's concrete `RemoveMode` is validated by
  `remove_policy` and is not overridden by this setting. The one
  consumer is the fold's demotion rule
  (`pkgs/id/crates/id-events/src/groups/delegation.rs`): a
  non-removal role change (a demotion) carries no `RemoveMode`, so
  under `Cascade` a delegated link stands only while the grantor
  still holds the scope at `after`; under any other preference the
  link stands until a removal disposes of it. `id-cli` displays the
  effective value (`crates/id-cli/src/commands/group.rs`), and the
  `id-groups` action layer compares an explicitly requested mode
  against it (`crates/id-groups/src/groups/actions.rs`).
- **Modern form:** per-removal concrete `RemoveMode` plus
  `RemovePolicy` (allowed modes, allowed successor categories,
  default mode) on the same record.
- **Disposition: retained indefinitely as a policy field.** It is
  not a wire-compat shim but the only recorded statement of demotion
  semantics for the groups whose records carry it; deleting the
  field would change fold results for those groups. It stops being
  load-bearing only if demotion semantics are re-specified to a
  single rule — a design change for the #6379 owner to make
  explicitly, not a cleanup. New code must not extend its reach:
  removals stay governed by `RemoveMode` / `remove_policy`.

## 4. Decision — a missing delegation preference reads as `cascade`

- **Where:** `Policy::delegation_mode()` in
  `pkgs/id/crates/id-events/src/groups/policy.rs`: "the record's, or
  cascade when it names none." `Policy::default()` records
  `delegation: None`, so every policy record that names no
  preference folds as `Cascade`.
- **Decision: the silent default is intended and is retained.**
  Three reasons. (1) Determinism of replay: the fold must produce
  the same links from the same log on every replica, so an absent
  field needs one fixed reading — and `Cascade` is the reading the
  pre-`RemoveMode` fold always applied, so old logs keep their
  historical meaning. (2) Direction: for the one rule the preference
  drives (§3), `Cascade` is the least-authority reading — a demoted
  grantor's delegated links fall; the alternatives let them stand.
  (3) The default is not hidden in effect: `id-cli` surfaces the
  effective preference via `delegation_mode()` in group output, so
  an operator sees `Cascade` even when the record names none.
- **Consequence for writers:** a group that wants `KeepPrior` or
  `Transfer` demotion semantics must record the preference
  explicitly; relying on the default is relying on `Cascade`.

## 5. L3 — `Policy.remove_policy` absent-field default (read-compat only)

- **Where:** `#[serde(default)] pub remove_policy: RemovePolicy` on
  `Policy` in `pkgs/id/crates/id-events/src/groups/policy.rs`.
- **Legacy form:** a stored policy record with no `remove_policy`
  field, written before the field existed. It decodes with
  `RemovePolicy::default()`: all four concrete modes allowed, all
  four successor categories allowed, `default_mode: Revoke`.
- **Modern form:** records written now carry `remove_policy` in
  full.
- **Disposition: retained indefinitely, read-only default.** Stored
  policy records are replicated state and must decode identically
  everywhere. The default is the permissive superset with the
  least-authority default mode, so an old record gains no power it
  did not have: its removals were subject to no `remove_policy`
  before, and the default admits exactly the settled modes.
  Removal condition: the same future state-format migration as L1,
  re-encoding stored policy records in full.

## 6. What is not legacy

- `RemoveMode::ServerDefault` is a request-side resolver directive:
  encoding refuses it (`crates/id-events/src/groups/event.rs`) and
  a replicated event carrying it is a fold error. It is never a
  stored form.
- No serde aliases, untagged enums, or `#[serde(other)]` fallbacks
  exist anywhere in the two groups modules (grep-verified
  2026-11-23). The three constructs above are the complete
  read-compat surface.

## 7. Acceptance mapping (#13533)

- Every legacy form that is read-compat only, with the modern form
  that replaces it: L1 (§2), L3 (§5); the retained policy field is
  L2 (§3).
- Each form dispositioned — removal condition or retained with a
  reason: §2, §3, §5.
- The `cascade` default stated as a decision: §4.
