# MOB-01 — Versioned work read model with per-fact provenance

Issue: [#9468](https://github.com/developing-today/code/issues/9468)
(slice `MOB-01` of [#9201](https://github.com/developing-today/code/issues/9201),
program [#9196](https://github.com/developing-today/code/issues/9196)).
Status: **design proposal — not an authorization to implement.**
This record elaborates the read-model slice of the merged design
package (`doc/2026-11-13T03-20-24Z_design_pm-stack-fleet-orchestration/`,
PR #9225), component #9201 §3.1 and §3.5, into a typed fact envelope,
the inputs the projection reads, the conflict codes it emits, the
golden fixture set and the test matrix a separately scoped
implementation issue can be filed against. It authorizes no code,
endpoint, schema, migration, deployment or live acceptance; those
remain separately gated stages (§10).

Governing decisions consumed, not re-opened:

- **Component #9201 §3.1** — one versioned read operation over the
  existing sources, no PM database; each fact carries source/ref,
  observed-at, revision and freshness, or an explicit
  absent/invalid/error state; absent is never zero; board column is a
  projection with explanation, not a writable lifecycle enum; never
  `Done` from one facet; a claim is shown as active only with a fresh
  authoritative claim read and a verified session binding.
- **Component #9201 §3.5** — a stale or unavailable source renders
  facet-level stale/error; a lossy Project column shows facets plus
  explanation; no silent collapse.
- **Component #9201 §3.6** — the owner of each fact; the projection
  reads and never writes. Project #4 stays single-writer.
- **`architecture.md`** — shared `(authority_id, work_id)` identity,
  `snake_case` wire, canonical decimal strings for identifiers and
  revisions that may exceed the safe-integer range.
- **#9200** — canonical urgency and label interpretation. Until it
  lands, the projection shows the source label and score and marks the
  dimension `unmapped`; it never guesses.

## 1. What exists at main `20292100`

- `pm-stack/aggregator/src/model.ts` has `GhItem`/`GhState`
  (`open|closed|merged`), Project-pool items, `RefKey`
  (`owner/repo#n` display string, no stable provider id),
  `Provenance` (a per-page string map) and `ProvenanceFacet` with
  `state: live | partial-fixtures | fixtures | stale | absent |
invalid | error` (#1138). That facet vocabulary is page-level
  (board, items, quota…), not per work item.
- `src/work-items.ts` (#1206) assembles the canonical _orchestration
  section_ work list for the existing pages: identity
  `orch:<goal>/<title>`, one collapsed `status`, queue rank. It
  folds tiers into one status by design. It is the page list, not a
  per-fact read model, and MOB-01 does not change it.
- `src/work-action-contract.ts` (#9472, MOB-05) is a pure module (no
  I/O, injected clock) for the preview/apply envelope. It needs an
  `expected_revision` to preview against; nothing yet produces one per
  work item.
- `src/queue-view.ts` and `pm-stack/projector` produce queue
  _snapshots_; neither is a live claim truth source (#9201 V1).
- The Fleet broker has no `work_version` column yet (REL-02 owns it).
  The only fleet-side revision available today is the snapshot's own
  observation time.
- `.github/workflows/pm-aggregator-tests.yml` runs `bun test` and is
  triggered only by `workflow_dispatch` (#6126 ruling). It is not a
  required check.

Gap: no module turns the independent GitHub, Project, queue and
session readings of _one_ work item into a single view that keeps
each reading apart with its provenance and names their disagreements.
Every consumer (cards, detail, MOB-05 previews, MOB-02 columns) would
otherwise re-derive that, which is how #1206's divergence started.

## 2. Decisions

### D1. A pure projection function, no I/O, no endpoint in this slice

`work-projection.ts` exports one function:

```ts
projectWorkView(input: WorkViewInput, opts: WorkViewOptions): PmWorkView
```

- `input` holds the already-read source snapshots (§3). The function
  fetches nothing, reads no clock, and has no module state. `opts.now`
  (ISO-8601) is required, and `opts.stale_after_ms` gives a bound per
  source kind. The caller supplies both. There are no hidden defaults,
  so a test fixture and production differ only in what they pass.
- `server.ts` is not owned by MOB-01, so this slice adds **no HTTP
  route**. The first consumer that wires a read endpoint (MOB-03's
  view, or a small follow-up in the server's owner slice) passes the
  snapshots it already gathers. #9201 says "read integration only
  where existing source APIs already expose the fields". This slice
  meets that by adding only `from*` adapters (§3.2) over shapes that
  `model.ts` already has.
- It follows the same pattern as `work-action-contract.ts`: pure,
  deterministic, unit-testable with `bun test`, with no network or
  credentials in tests.

### D2. Wire schema `pm_work_view/v1`

The issue oracle spells the schema as both `pm_work_view/v1` and
`pm-work-view/v1`. The wire value is `pm_work_view/v1`, following the
`snake_case` convention in `architecture.md` and the §3.1 example.
`pm-work-view` is used only as a fixture directory name. The top-level
keys are exactly those in §3.1: `schema_version`, `generated_at`,
`work`, `source_refs`, `revisions`, `facts`, `projection`,
`available_actions` and `freshness`. Adding a key requires `v2`.
Unknown keys in _input_ snapshots are ignored. An input snapshot with
an unknown `snapshot_version` becomes that facet's `invalid` state
(D4), never a guess.

### D3. Identity: the locator is not the identity

- `work` is `{authority_id, work_id}` with `work_id` as a canonical
  decimal string. It is accepted only from a fleet snapshot, or from
  the caller's explicit, already-authenticated authority context.
  `fleet://work/842` is never parsed out of free text.
- Each `source_refs[]` entry is
  `{kind, provider, repository_id, resource_id, locator}`.
  - `kind` is one of `github_issue | github_pr | project_item |
fleet_work | session`.
  - `locator` is the display string, e.g. `owner/repo#12`.
  - `repository_id` and `resource_id` are stable provider ids (GitHub
    node ids), or `null` when the source did not supply them. Today's
    `RefKey` does not supply them.
- Two refs with the same `locator` and different non-null
  `repository_id` are two resources. They produce conflict
  `identity_ambiguous` and are never merged. A ref with a null
  `repository_id` adds reason code `identity_locator_only` to the
  projection. That reason does not block the read, but MOB-05 must
  refuse a write that targets such a ref (`identity_unverified`).

### D4. The fact envelope

Every entry under `facts` is one of the five tagged states below.
None of them has a default numeric value.

```ts
type Fact<T> = { state: "present"; value: T; source: SourceKind; source_ref: number | null; revision: Revision; observed_at: string; freshness: "live" | "stale" | "fixtures" } | { state: "absent"; source: SourceKind | null; reason: AbsentReason } | { state: "invalid"; source: SourceKind; observed_at: string; reason: string } | { state: "error"; source: SourceKind; observed_at: string | null; error_code: string } | { state: "policy_undefined"; policy: "9199" | "9200"; source_value?: SourceValue };

type Revision = { kind: "work_version" | "updated_at" | "etag" | "item_revision" | "observed_at"; value: string };
```

- `source_ref` indexes `source_refs[]`. It is `null` only for facts
  that come from fleet policy, not a resource.
- **Absent** means the source answered and has no value, or no source
  for this dimension is connected. `reason` is one of
  `not_sourced | source_has_no_value | not_applicable`. An absent
  urgency is _absent_, not `"0"`, and it is not taken from a label.
- **Stale** facts keep their last value with `freshness: "stale"`. A
  stale value is never zeroed, dropped or relabelled live.
  `observed_at` older than `now - stale_after_ms[source]` decides
  staleness. A source's own staleness flag (e.g. `ProvenanceFacet.state
=== "stale"` or the feed's freshness notice) also forces stale, and
  the stricter of the two wins.
- **Fixtures** maps `ProvenanceFacet` `fixtures | partial-fixtures`.
  A fixture value is never presented as live.
- **Error** carries no value. A source that failed to read cannot
  contribute a number, a status or a conflict side.
- **`policy_undefined`** replaces the §3.1 example's
  `not_yet_defined_by_9200`. It is the same meaning, typed with the
  owning policy. `source_value` keeps the raw label/score the source
  showed (e.g. a `priority:high` label, an `expedite=15` readback), so
  the UI can show what exists without mapping it. That covers
  `effective_pressure` (owned by #9199/#9200) and, until #9200 lands,
  urgency taken from labels.
- Fleet revision: until REL-02 adds `work_version`, the fleet facet's
  revision is `{kind: "observed_at"}`, the weakest kind. MOB-05 must
  treat a weak revision as `revision_unavailable` for consequential
  writes rather than as a version to compare. That makes the gap
  visible instead of letting a timestamp stand in for a version.
  GitHub uses `updated_at` (or `etag` where the reader has one).
  Project #4 uses the item's `updated_at`.

### D5. Facts and their single sources

| Fact                 | Source (sole)                                                                       | Notes                                                                                                                                                     |
| -------------------- | ----------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `intrinsic_urgency`  | fleet policy snapshot                                                               | canonical decimal string; label-only → `policy_undefined` with `source_value`                                                                             |
| `importance`         | fleet metadata (#9199)                                                              | `absent/not_sourced` until sourced                                                                                                                        |
| `effective_pressure` | #9199/#9200                                                                         | `policy_undefined` until defined                                                                                                                          |
| `dispatch_directive` | fleet                                                                               | `normal \| do_now \| …`; `expedite=15` stays a priority readback, not `do_now`                                                                            |
| `workflow_state`     | fleet lifecycle                                                                     | `open \| parked \| blocked \| completed \| …` verbatim                                                                                                    |
| `eligibility`        | fleet broker read                                                                   | `eligible \| ineligible \| manual_only`, plus `hard_blocker_refs` and reason codes as the broker returned them; never recomputed client-side, never OR-ed |
| `claim`              | fleet broker read                                                                   | `unclaimed \| claimed{lane, lease_expires_at}`; a queue row status alone is not a claim                                                                   |
| `session`            | provider adapter readback                                                           | `none \| bound{verified: bool, capabilities}`; a shell without a task binding is `bound{verified:false}`                                                  |
| `project`            | GitHub Project #4 snapshot                                                          | Status/Kind/Area/Horizon verbatim from `model.ts`; no second Status field                                                                                 |
| `github`             | GitHub issue and linked PR reads                                                    | `issue_state`, `pr_states[]` (`open\|closed\|merged`) per linked PR                                                                                       |
| `relations`          | DEP-03/DEP-04 edges when present, else `ProjectPoolItem.parentRef/blockedBy/blocks` | `parent_refs`, `hard_blocker_refs`, `advisory_refs`; provider hierarchy is advisory (DEP-04)                                                              |

`github` is added beside the §3.1 list. §3.1 says "separate underlying
GitHub issue/PR state", and its example omits a home for that state.
This is the only key added to §3.1's `facts`.

### D6. Conflicts are named disagreements, not a column

MOB-01 detects disagreements between facets and lists them in
`projection.conflicts`. It does **not** choose the board column.
Column precedence is MOB-02's owned policy (`work-lifecycle-policy.ts`).
To avoid two precedence rules:

- `opts.column_policy?: (view) => {column, reason_codes}` is an
  injection point. Without it (MOB-01 alone),
  `projection.column = null` and `reason_codes` contains
  `column_policy_not_installed`. A `null` column can never read as
  `Done` or `Ready`. MOB-02 installs its policy and updates the golden
  fixtures' `column` values, which is allowed because it co-owns
  `work-projection.ts`.
- Each conflict is `{code, facets: FactKey[], evidence: number[]}`,
  where `evidence` indexes `source_refs`. Conflicts are sorted by
  `code`, then `facets`. Only facets in the `present` state take part,
  so an error or absent facet can never manufacture a conflict.

| Code                       | Fires when                                                                                                                                         |
| -------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------- |
| `issue_open_pr_merged`     | issue `open` and a linked PR `merged`                                                                                                              |
| `issue_closed_work_open`   | issue `closed` and `workflow_state` not terminal                                                                                                   |
| `work_terminal_issue_open` | `workflow_state=completed` and issue `open`                                                                                                        |
| `project_status_disagrees` | Project Status ∈ {Todo, In Progress, Done} contradicts `workflow_state`/`eligibility` (e.g. Todo with blocked/parked, Done with non-terminal work) |
| `claimed_binding_unknown`  | `claim=claimed` and `session` is not `bound{verified:true}`                                                                                        |
| `binding_without_claim`    | `session=bound{verified:true}` and `claim=unclaimed`                                                                                               |
| `held_with_active_binding` | `workflow_state ∈ {parked, blocked}` and `session=bound{verified:true}`                                                                            |
| `identity_ambiguous`       | D3                                                                                                                                                 |

`Done` needs a verified completion receipt. That is the completion
policy's to supply, so MOB-01 adds reason code
`completion_receipt_absent` whenever any facet looks terminal and no
receipt input is present.

### D7. Freshness roll-up

`freshness.facets` maps each source kind to `live | stale | fixtures |
error | absent`. `freshness.state` is the worst state among _present or
error_ facets, in the order `live < fixtures < stale < error`. Absent
facets do not degrade it. A view with every facet absent is
`absent`. `mutation_blockers(view, facets)` is exported, so MOB-06 can
disable a control that depends on a stale or error facet (#9201 §3.5,
row 1) without re-deriving freshness.

### D8. Actions are not decided here

`available_actions` is emitted as `[]` with no other effect.
`opts.actions?: (view) => AvailableAction[]` is the injection point
for MOB-05/MOB-06. Their contract owns verbs, authorization and
expected revision. MOB-01 only supplies `revisions` to read from.

### D9. Determinism

The same input produces byte-identical JSON. Arrays coming from
sources (`source_refs`, `pr_states`, relation refs) are sorted by
`(kind, repository_id ?? locator, resource_id ?? locator)` before
indexing. Object keys are emitted in a fixed order. `generated_at` is
`opts.now`. All identifiers, numbers of record and revisions are
strings. No JavaScript number crosses the wire except `source_ref`
and `evidence` indices.

## 3. Inputs

### 3.1 `WorkViewInput`

```ts
interface WorkViewInput {
  authority: { authority_id: string; work_id: string } | null;
  fleet?: SourceRead<FleetWorkSnapshot>; // broker read: lifecycle, eligibility, claim, policy
  github_issue?: SourceRead<GhItemSnapshot>;
  github_prs?: SourceRead<GhItemSnapshot>[]; // linked PRs
  project?: SourceRead<ProjectItemSnapshot>; // Project #4 item
  session?: SourceRead<SessionBindingSnapshot>;
  relations?: SourceRead<RelationSnapshot>;
  completion_receipt?: SourceRead<CompletionReceiptRef>;
}
type SourceRead<T> = { ok: true; observed_at: string; provenance_state?: ProvenanceFacet["state"]; data: T } | { ok: false; observed_at: string | null; error_code: string };
```

An omitted key means that source is not connected (`absent/not_sourced`).
`ok:false` means the read failed (`error`). These two cases are kept
apart so that "we could not read GitHub" never looks like "there is no
issue".

### 3.2 Adapters (in `work-projection.ts`)

- `fromGhItem(GhItem)` maps `model.ts`'s existing `GhItem` to
  `GhItemSnapshot`. It sets `repository_id: null` because today's shape
  lacks the field (D3).
- `fromProjectPoolItem(ProjectPoolItem)` maps Project fields and native
  relations.
- No adapter for the fleet broker read or session binding is added
  here. Their snapshot types are defined, and the aggregator's existing
  fleet-feed/session-feed entries are _not_ mapped to `claim` or
  `session.bound{verified:true}`, because neither is an authoritative
  claim or binding readback (#9201 V1). Until a real reader exists,
  those facts stay `absent/not_sourced`. That is truthful, and MOB-02's
  "claimed + no session" row then shows correctly.

`model.ts` gains only the exported type declarations of §2/§3 (the
`PmWorkView`, `Fact`, `SourceRead`, snapshot interfaces). No existing
type changes.

## 4. Golden fixtures

They live in `pm-stack/aggregator/test/fixtures/pm-work-view/`, one
`<name>.input.json` and `<name>.view.json` pair per case. The fixtures
are the test data of the owned `test/work-projection.test.ts`. The
initial set:

1. `conflicting-facets`: issue open, PR merged, queue blocked, Project
   Todo stale, session bound verified. The view keeps all facets, has
   `freshness.facets.project = "stale"`, conflicts
   `held_with_active_binding`, `issue_open_pr_merged`,
   `project_status_disagrees` and `binding_without_claim` (if
   unclaimed), and `column: null`.
2. `all-live-eligible`: every source live and consistent, no conflicts.
3. `github-error`: issue read failed. `github` is an error, no
   conflict mentions it, and freshness is `error`.
4. `nothing-sourced`: only `authority`. Every fact is absent, and the
   view is `absent`, not zero.
5. `label-only-urgency`: `priority:high` and `expedite=15` labels,
   with no fleet policy value. Urgency is `policy_undefined` with
   `source_value`, and `dispatch_directive` is not `do_now`.
6. `claimed-unbound`: `claimed_binding_unknown`.
7. `renamed-repo-same-number`: `identity_ambiguous`.
8. `locator-only`: reason `identity_locator_only`.
9. `terminal-no-receipt`: queue completed and issue closed. No
   conflict, but reason `completion_receipt_absent`.
10. `fixture-source`: a Project from fixtures maps to
    `freshness: "fixtures"`, never `live`.
11. `big-ids`: a `work_id` and revision above 2^53 stay decimal
    strings and round-trip unchanged.

## 5. Tests (`pm-stack/aggregator/test/work-projection.test.ts`)

1. `work_projection_preserves_source_state_under_conflicting_facets`
   (the named red test): fixture 1. Every facet is present with its
   own source, revision and freshness, the conflicts are exactly the
   expected set, and `projection.column` is neither `Done` nor
   `Ready`.
2. `work_projection_matches_golden_fixtures`: every pair in §4
   deep-equals.
3. `work_projection_matrix_invariants`: model-based. It runs the full
   product of independent states (issue {absent, open, closed, error}
   × PR {none, open, merged, closed} × Project {absent, Todo,
   In Progress, Done, stale} × queue {absent, eligible, blocked,
   parked, claimed, completed, stale} × session {absent, none,
   bound-unverified, bound-verified, error}). It asserts that every
   `present` fact has `source`, `revision` and `freshness`; that no
   fact is numeric zero unless the source said zero; that error and
   absent facets appear in no conflict; that with the default policy
   the column is always `null`; and that the conflicts are a pure
   function of present facets (the table in D6).
4. `work_projection_absent_is_not_zero`
5. `work_projection_stale_keeps_value`: a value past the bound is
   kept, marked stale, and makes `mutation_blockers` non-empty.
6. `work_projection_error_has_no_value`
7. `work_projection_locator_is_not_identity`
8. `work_projection_claim_requires_verified_binding`
9. `work_projection_label_is_not_urgency`
10. `work_projection_is_deterministic`: shuffled input arrays and key
    order give byte-identical output. The test also asserts that the
    module never calls `Date.now` or `new Date()` without an argument;
    `now` must be injected.
11. `work_projection_unknown_snapshot_version_is_invalid`

The issue lists negative cases, and each one maps as follows:

- Stale input: tests 5 and 3.
- Duplicate or replay: test 10. A read is idempotent, so replay is
  determinism.
- Ambiguous resource: test 7.
- Unknown version: test 11.
- No false verified/completed: tests 1, 8 and fixture 9.
- Unauthorized actor: not applicable to a pure function. The read
  route's Access gate belongs to whichever slice adds the route (D1).
- Partial remote effect: not applicable to a read. `pending_sync` is
  MOB-09's desired-versus-observed view.

Run: `cd pm-stack/aggregator && bun test test/work-projection.test.ts`.
List the test names first (`bun test --test-name-pattern work_projection`
must report 11 tests). Zero tests is not a pass. Per Drew's Nov 23
remote-builds rule, the run happens remotely. Use a `workflow_dispatch`
of `pm-aggregator-tests.yml` on the implementation branch, or `fleet
queue dispatch` on a GitHub-hosted runner, never a laptop or sandbox.

## 6. Boundaries and ownership

- **Owned:** `src/model.ts` (additive types only), new
  `src/work-projection.ts`, new `test/work-projection.test.ts`, and
  new `test/fixtures/pm-work-view/`.
- **Not touched:** `work-items.ts` (#1206), `queue-view.ts`,
  `actions.ts`, `server.ts`, the projector and Fleet crates.
- **Added nowhere:** no database, store, cache, queue, endpoint,
  scheduler, GitHub client or writer.
- **MOB-02:** installs `column_policy` and owns column precedence.
  MOB-02's hard dependency on MOB-01 is this module's exported types
  and injection point.
- **MOB-05/06:** install `actions`, read `revisions`, and refuse
  writes against `observed_at` revisions or locator-only refs.
- **MOB-03:** renders views as filters over `PmWorkView[]`.

## 7. Compatibility and reversal

The change is additive. No existing export, page, route or fixture
changes. Removing the module removes a type set and an unused
function. The existing `expedite=15` / `next-up=25` spellings are shown
verbatim as `source_value`. They are not normalized and not folded
into `do_now`.

## 8. Prerequisites

- Hard blockers #9070, #9062, #9073 and #9076 are closed.
- Parent #9201 is open.
- No schema-coupled field is persisted, so the QTI crosswalk (#9241)
  does not gate this slice.

## 9. Open questions (not blocking)

- The stable GitHub node ids (`repository_id`, `resource_id`) need a
  reader that requests them. Today's pool reader does not. Until then
  every GitHub ref is locator-only (D3). This is a follow-up for the
  pool reader's owner, and it does not change this contract.
- The fleet broker read used for `claim` and `eligibility` will come
  from the CLI-13 read command or REL-02's versioned read, whichever
  lands first. The snapshot type in §3 is the seam either fills.

## 10. Gates

Implementation starts only on a separately scoped issue after this
record merges. Wiring a live read route, any persisted field, and any
deployed or live acceptance each need their own evidence and approval.
