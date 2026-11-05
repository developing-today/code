# Atlas — schema-driven filter surface from the classification manifest

Design doc — 2026-11-05. Companion to the Atlas binary+service design
(`doc/2026-11-04T08-07-13Z_design_atlas_binary_service/`, epic #5350) and
the scaler-era revision
(`doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision/`). This note is
the deliverable for #5410 (post-cutover design): whether atlas-core's
filter surface should be built from a **classification manifest** carried
with the corpus, instead of being hand-coded in four places — and if so,
exactly how. Design only; no implementation is authorized by this note.
The go/no-go is §7.

State verified at `origin/main` `04f78696ab` (2026-11-05), the same main
the #5410 triage (task 10035) checked:

- `pkgs/atlas-search/crates/atlas-core/src/filters.rs` defines
  `Filters { kinds, topics, projects, clients, max_sensitivity, since,
  until, until_exclusive }` in code. Dimensions are hand-built in both
  `Filters::where_clause` and `Filters::session_clauses`, with two
  different storage shapes: scalar columns matched by `IN (...)`
  (`l.kind`, `s.client`) and JSON-list text columns matched by `LIKE`
  (`l.topics`, `l.projects`); dates are a range over `s.created_at`.
- `pkgs/atlas-search/crates/atlas-proto/src/lib.rs` repeats the surface
  as `FilterSpec { kind, topic, project, client, since, until }` (the
  ceiling is deliberately a sibling on `SearchRequest`, not a member).
- The CLI (`crates/atlas-search/src/main.rs`) maps the four CSV flags
  into both shapes at two call sites.
- Serve mode dedupes on `CacheKey` (`crates/atlas-search/src/server.rs`):
  the effective request verbatim — query, mode, …, `kind[]`, `topic[]`,
  `project[]`, `client[]`, `since`, `until`, `effective_ceiling`.
- No manifest exists: a grep for `manifest` across
  `pkgs/atlas-search/crates` finds nothing.
- The sensitivity ladder is code: `SENS_ORDER` in `filters.rs`, the
  ceiling clamped server-side, filter-before-rank enforced inside the
  branch SQL. #5410's hard constraint stands and this design keeps it:
  **sensitivity and ceiling semantics are never data-defined.**

---

## 1. Problem

The filter *vocabulary* is already data-driven — filters match whatever
labels the classification pass wrote onto chunks (kinds, topics,
projects, clients), so a new label value is filterable immediately. The
*dimensions and their semantics* are not: adding one classification
dimension (say a source-app label) today means a coordinated code change
in atlas-core's SQL builders, the CLI flag surface, atlas-proto's
request types, and serve-mode validation — plus the dedupe key. Match
semantics are likewise frozen in code, and one of them is wrong: topics/
projects matching is a raw `LIKE` interpolation, which is #5375
(unescaped `%`/`_` act as wildcards; a `"` in a value crosses JSON
element boundaries).

The cost is not the four edits; it is that a *reclassification* — a data
event — cannot change what searches can slice by without a binary
release, and every new dimension re-opens the semantics question in
four places at once.

## 2. Proposal in one paragraph

The classification/publish pass writes a **classification manifest**
into the corpus database: a small, versioned declaration of the
filterable dimensions — name, storage shape, match semantics — for
non-sensitivity fields only. atlas-core loads the manifest at backend
open and builds its filter surface from it; the CLI and the service
inherit the same surface through atlas-proto. The code-defined surface
of today becomes the **built-in manifest**: the fallback when no
manifest is present, and the reference the loaded manifest is validated
against. §3–§6 settle the four open questions from #5410 in order.

## 3. Manifest home: one table, both modes

**Decision: a `classification_manifest` table in the corpus database —
not publish-file metadata alone, not a sidecar file.**

- Local mode already reads SQLite publish files (`atlas-publish-*.db`
  beside the DB; labels/sensitivity are attached from the publish copy,
  `crates/atlas-core/src/local.rs`). A table in that same DB travels
  with the corpus by construction.
- Turso mode reads the hosted DB, where labels live in
  `session_labels`. A table in the same database is the only home both
  modes read identically; publish-file metadata cannot describe the
  Turso corpus, and a sidecar file has no Turso analogue.
- One row per dimension plus a header row (or a single JSON document
  row — implementation choice, but one atomic read): `manifest_version`
  (monotonic integer), `content_hash`, and per dimension:
  `name`, `shape` (§4), `semantics` (§5), `label_column` (§6's
  allowlist binding).
- The classification/publish pass is the only writer. It writes the
  manifest in the same publish transaction as the labels it describes,
  so a corpus can never carry labels its manifest doesn't declare, or
  a manifest describing labels that aren't there.

## 4. Storage shapes are declared, not inferred

Today's two shapes, plus the date range, become the manifest's closed
shape vocabulary — the manifest names one of these, never a column or
SQL fragment of its own:

| shape | storage | today's example |
|---|---|---|
| `scalar_in` | one value per row in a label column | `l.kind`, `s.client` |
| `json_list` | JSON list text in a label column | `l.topics`, `l.projects` |
| `timestamp_range` | timestamp column, `since`/`until` pair | `s.created_at` |

A new dimension is a new manifest row of an existing shape. A new
*shape* is a code change — deliberately: shapes are where SQL is born,
and SQL stays code (§6).

## 5. Match semantics, and the #5375 interaction

Each dimension declares `exact` or `pattern`:

- **`exact` is the default and the only semantics a manifest may assume
  silently.** For `scalar_in`, `IN (...)` as today. For `json_list`,
  element equality: the value is matched as one JSON element, with
  `LIKE` metacharacters (`%`, `_`, `\`) escaped and an explicit
  `ESCAPE` clause — i.e. exactly the fix #5375 specifies. The `"`
  boundary-crossing case in #5375 is closed by construction, because
  the escaped pattern can only match a whole quoted element.
- **`pattern` is an explicit per-dimension opt-in** for dimensions
  whose values are meant to be globbed. Pattern values are translated
  (glob `*`/`?` → `LIKE`, everything else escaped) — a raw user `LIKE`
  pattern is never interpolated. No existing dimension is `pattern`;
  topics/projects stay `exact`.

**Sequencing with #5375.** #5375 is a live correctness bug on today's
surface, independent of this design. Recommendation: land the #5375
escaping fix FIRST, as a standalone patch implementing §5's `exact`
semantics for `json_list` — the semantics are defined once, here, and
the patch is that definition applied to the hand-coded builder. The
manifest work (§8 phase 3) then generalizes the already-correct builder
instead of re-deciding semantics, and #5375 closes with its own PR
rather than waiting on the manifest programme. What must not happen is
the inverse: pattern support bolted onto the unescaped construction.

## 6. The safety invariant: what a manifest can never do

The manifest is data, and data is what a mislabeled or corrupted corpus
could carry. The design holds #5410's hard constraint by construction:

1. **No sensitivity keys exist in the manifest schema.** `SENS_ORDER`,
   `Sensitivity`, the ceiling clamp, and `allowed()` stay in
   `filters.rs`, untouched. A manifest row naming a sensitivity or
   ceiling dimension is a validation error (§7), not a feature.
2. **Dimensions bind to an allowlist, in code.** `label_column` in a
   manifest row must name one of the code-known label columns/tables
   for its shape (today: `kind`, `client`, `topics`, `projects`,
   `created_at`, and future label columns added by code). An unknown
   binding fails validation. The manifest can declare a dimension the
   code doesn't know how to store *only* by being rejected.
3. **A manifest can narrow or extend the non-sensitivity surface; it
   can never widen visibility.** The ceiling clause is emitted first,
   from code, in both branch builders, exactly as today
   (filter-before-rank). Manifest-driven clauses are additional `AND`
   fragments after it. There is no manifest input that removes,
   reorders, or weakens a code-emitted clause.
4. **Unknown request dimensions are errors, never ignored.** A request
   filtering on a dimension the effective surface doesn't declare fails
   validation loudly — the failure mode #5375 taught: a filter that
   silently becomes a no-op is worse than no filter.

## 7. Versioning and failure mode: fall back, loudly, never widen

- The manifest carries `manifest_version` + `content_hash`. atlas-core
  records the loaded pair in the backend's source description (the
  pattern `local.rs` already uses for sensitivity provenance) and in
  serve-mode job metadata.
- **Manifest absent** → the built-in manifest (today's surface,
  expressed in the manifest's own schema) applies. This is the normal
  state for every corpus published before the manifest writer ships.
- **Manifest unreadable, malformed, or failing §6 validation** →
  fall back to the built-in manifest AND log at warning level with the
  reason, on every backend open, until fixed. Never a silent widening:
  the built-in surface filters a subset of what a valid manifest could
  declare, and the ceiling is code either way. A corpus whose manifest
  *newer than the binary understands* (`manifest_version` above the
  supported maximum) is the same case: built-in + loud log, because a
  newer manifest can only add non-sensitivity dimensions.
- **Dedupe-key versioning.** Serve mode's `CacheKey` gains the
  effective surface identity: `(manifest_version, content_hash)` of
  the loaded manifest, or a fixed `builtin` sentinel. A manifest change
  therefore versions into the key exactly as #5410 requires: no
  computation computed under one surface can be served under another,
  and the effective-ceiling protection already in the key is
  unchanged. Requests also canonicalize: dimension names sorted, so
  the same effective filter tuple keys identically regardless of the
  client's field order once `FilterSpec` goes generic (§8 phase 3).

## 8. Delivery phases (for the implementation issues, not this note)

1. **#5375 escaping patch** — §5 `exact` semantics in the current
  hand-coded builder; parity fixtures updated deliberately.
2. **Manifest schema + loader in atlas-core**, with today's surface
  expressed as the built-in manifest. Acceptance test: SQL emitted
  under the built-in manifest is byte-identical to today's (the parity
  fixtures are the oracle).
3. **Generic dimensions end-to-end:** atlas-proto `FilterSpec` gains a
  dimension map (the four named fields remain as aliases for one
  release), the CLI generates its flags from the loaded surface
  instead of the two hand-mapped call sites, serve-mode validation
  checks requests against the effective surface.
4. **CacheKey versioning** per §7, plus the surface identity in job
  metadata.
5. **Manifest writer** in the classification/publish pass, same
  transaction as the labels (§3); first real consumer: one new
  dimension (e.g. source-app) declared by manifest only, no binary
  change — the acceptance demonstration for the whole design.

Each phase is independently shippable and independently revertible;
phases 2–4 change no visible behaviour with a built-in manifest in
effect.

## 9. Alternatives considered and rejected

- **Publish-file metadata only** — fails Turso mode (§3); two homes
  for one fact is how local/serve surfaces drift.
- **Dimensions in atlas-proto only (a registry release)** — keeps the
  release coupling this design exists to remove; reclassification
  still can't change the surface.
- **Fully dynamic SQL from the manifest** (data names columns and
  operators) — rejected outright by §6: it makes corpus data into a
  SQL author, one corrupted publish away from a visibility bug.
- **Do nothing** — the four-place hand-coded surface is correct today
  except #5375; the honest cost is that every future dimension pays
  the four-place change and re-litigates semantics. §10 weighs this.

## 10. Recommendation: GO, with the §8 phasing

**GO.** The design is small because the hard parts already exist: the
vocabulary is data-driven, the ceiling is code and stays code, the
parity fixtures give phase 2 a byte-exact oracle, and the dedupe key
already versions on the effective request — the manifest just joins
it. The manifest's blast radius is bounded by §6: worst case of a bad
manifest is the built-in surface and a loud log.

Conditions on the GO: phase 1 (#5375) lands first or as the manifest
programme's first milestone — never after pattern semantics; §6's
allowlist and no-sensitivity-keys rules are schema-level, enforced by
the loader's tests, not by convention; and phase 5's manifest-only
dimension is the acceptance gate — if a new dimension still needs a
binary change after phase 4, the programme stops and this note is
revisited instead of declaring victory.

**NO-GO trigger, named in advance:** if the classification/publish
pass cannot write the manifest in the same transaction as the labels
in *both* modes (local publish file and Turso), §3's atomicity premise
fails and the home decision must be re-taken before any loader work.

---

*Evidence index: #5410 (issue + task 10035 triage, 2026-11-05); #5375
(LIKE-escaping probes, verified on `a65000aeabf`); `filters.rs`,
`atlas-proto/src/lib.rs`, `atlas-search/src/main.rs`,
`atlas-search/src/server.rs` (`CacheKey`), `atlas-core/src/local.rs`
(publish-copy attach + provenance strings) at `04f78696ab`.*
