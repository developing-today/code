# Atlas recommendation objects, search history, and retention classes — design record (#9140)

Status: design only, written 2026-11-24 for developing-today/code#9140
(parent epic #9109, Part C #9106 program). Queue design slice 1 for task
21566 (lane code-h). No implementation is authorized by this record.
Companions: content versioning and retention
(`doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`),
the retention rule engine (`doc/2026-11-17_design_atlas_retention_rule_engine.md`),
and the retrieval stats feedback record
(`doc/2026-11-10T20-41-01Z_design_atlas_retrieval_stats_feedback.md`).

## 1. Problem and context

The source ask (#9140, §6 of the extensible-primitives program) has three
parts, none of which has a shape yet:

- **Recommendations** are "next 10" style outputs. Today they are computed
  on read and discarded, so there is no object to compare, audit, or
  retire. The epic #9109 Part C already names a fan-out worker and a
  Durable Object that accumulates recommendations, but not what a
  recommendation *is*.
- **Search history** exists only as transient query traffic. The retrieval
  stats feedback record already needs query-side inputs, but no record
  defines search history as an object with an identity.
- **Retention** is set per content version by the retention rule engine
  (#6041), which covers versioned Atlas content. Recommendations, search
  history, and hot-data snapshots are not content versions, so the engine
  has no class to apply to them. The "what's hot" intermediate data has no
  short-retention class at all, and admins cannot compare state before and
  after a change because nothing is versioned or timestamped.

What breaks today: a recommendation cannot be reproduced after the fact,
search history cannot be reported on without a second pipeline, and
short-lived hot data either persists forever or is dropped without a rule.

## 2. Goals

- G1. Recommendations are stored objects with an identity, a timestamp, and
  the inputs that produced them.
- G2. Search history is stored as objects that can be read as inputs to
  recommendations and to retrieval stats.
- G3. Every stored object of these kinds is a versioned, timestamped
  snapshot, so an admin can compare before and after a change.
- G4. Retention is declared per data class, and a short-retention class
  exists for intermediate "what's hot" data.
- G5. Reuse the retention rule engine's invariants (conjunction, narrowing
  only, floors, report before delete, cold before delete) rather than
  writing a second engine.

## 3. Non-goals

- NG1. No change to content versioning or the content retention policy
  (#6026 §4). Those records govern content versions and stay as they are.
- NG2. No recommendation model or ranking algorithm. This record fixes the
  object shape, not the scoring.
- NG3. No user-facing surface. Admin comparison is an API and report, not
  a UI.
- NG4. No change to the D1 / R2 placement decisions in epic #9106 Part C.
- NG5. No retention of raw query text beyond what a class declares. Query
  text is personal data until a class says otherwise.

## 4. Alternatives considered

A. **Do nothing.** Recommendations stay computed on read, search history
   stays transient, and retention stays content-only. Cost: none now. Cost
   later: the stats feedback and recommendation work each invent their own
   history, and the admin before/after need is unmet.

B. **Store recommendations as rows in the existing content tables.** Reuses
   code, but mixes derived outputs with content versions, so the content
   retention policy would retire recommendations and content by the same
   rules. Rejected: it breaks NG1.

C. **Separate typed object families, one shared snapshot and retention
   envelope.** Recommendations, search history, and hot-data snapshots are
   each a typed family. All three share one envelope: `object_id`,
   `class`, `captured_at`, `version`, `inputs_ref`, and a `payload`. Chosen.

D. **One generic event log for everything.** Simple to write, but makes the
   retention classes and admin comparison a query over an undifferentiated
   log, and gives no typed contract for consumers. Rejected.

Chosen: C.

## 5. Approach

### 5.1 Object envelope

Every recommendation, search-history entry, and hot-data snapshot has:

| field | meaning |
| --- | --- |
| `object_id` | stable id of the logical object (a recommendation set, a search session, a hot-data key) |
| `family` | `recommendation`, `search_history`, or `hot_snapshot` |
| `class` | a retention class from §5.3 |
| `captured_at` | RFC 3339 time the snapshot was taken |
| `version` | monotonically increasing per `object_id`; the same ordering the snapshot comparison uses |
| `inputs_ref` | pointer to the inputs (query ids, retrieval-stats version id) that produced it, so the output is reproducible |
| `payload` | typed body for the family |

A new version is appended, never overwritten. "Current" is the highest
`version` for an `object_id`.

### 5.2 Families

- **Recommendation**: payload is an ordered list of item refs with a score
  and the rule or model id that produced them. Fan-out worker writes one
  version per recompute.
- **Search history**: payload is a query, its filters, the result refs
  returned, and the session it belongs to. Query text is stored only under a
  class that allows it (NG5).
- **Hot snapshot**: payload is a bounded window of the "what's hot" data
  (for example the last-N accumulated items), captured on the Durable
  Object cadence.

### 5.3 Retention classes

Retention is declared per class and reuses the rule engine's schema from
#6041. Each class names its windows and the keep-newest-N floor.

| class | holds | default window | default floor | notes |
| --- | --- | --- | --- | --- |
| `hot-short` | hot snapshots | 24 h | keep newest 2 | intermediate "what's hot" data |
| `recommendation` | recommendation versions | 30 d | keep newest 10 | enough history for before/after |
| `search-history` | search history | 90 d | keep newest 20 per session | query text only if the class says so |
| `snapshot-audit` | any family, admin comparison set | 365 d | keep newest 50 | longest class; admin before/after |

Classes follow the rule engine's invariants: a version is retired only when
its class rule permits and a trigger fires; a class may lengthen a window or
raise a floor but not shorten it below the default; reports run before any
delete; and a version passes through cold storage before deletion.

### 5.4 Admin before/after comparison

The `snapshot-audit` class is the comparison set. An admin compares two
versions of one `object_id` by `version`, and the report reads only the
envelope and payload. No read path depends on a non-audit class being
present.

## 6. Rollout and rollback

- Rollout is additive. The envelope and families land first with no reader.
  Writers switch on per family, one family at a time, behind a per-class
  enable flag that defaults to off.
- Rollback: turn the family's flag off. Stored versions remain and fall to
  their class retention; nothing is deleted on rollback.
- Retention rules start in report-only mode (rule engine §5), and a class
  goes live only after its dry-run report is reviewed.

## 7. Verification plan

- A property test that a version is never retired without a firing trigger
  and a permitting class (rule engine invariant 1).
- A property test that a class rule never lengthens below its default floor
  (narrowing check, rule engine invariant 2).
- A model-based test of the envelope: append-only versions, `current` is the
  highest version, and reproduction from `inputs_ref`.
- A live proof on the hot-data path: write three hot snapshots, confirm the
  short class retires the oldest and keeps the floor.

## 8. Build slices (dependency-ordered)

Each slice is sized for one lane session and has its own acceptance.

1. **Envelope and classes.** Define the envelope and the four classes as
   schema, with the narrowing check. Acceptance: schema tests pass; a class
   that shortens below default is rejected at write time.
2. **Recommendation family.** Writer from the fan-out worker, one version per
   recompute. Depends on 1. Acceptance: reproduction from `inputs_ref` matches
   the stored payload.
3. **Search-history family.** Writer from the query path, with the query text
   gated by class. Depends on 1. Acceptance: text is absent under a class that
   does not allow it.
4. **Hot-snapshot family and `hot-short` class.** Writer from the Durable
   Object cadence. Depends on 1. Acceptance: the live proof in §7.
5. **Admin comparison report.** Reads `snapshot-audit`. Depends on 2 to 4.
   Acceptance: before/after report for one `object_id` across two versions.
6. **Retention wiring to the rule engine.** Registers the four classes with
   #6041 dry-run reporting. Depends on 1 and the rule engine. Acceptance:
   dry-run report lists candidate versions per class with no deletes.

## 9. Open questions (each with a default and a decider)

- Q1. Does query text belong in `search-history` at all? **Default:** store
  it only in a class that opts in, and the initial `search-history` class
  opts out. **Decider:** the program owner for #9106 (Drew).
- Q2. Are recommendation scores comparable across model or rule versions?
  **Default:** not assumed; each version carries its producing rule id, and
  comparisons are within one id. **Decider:** the recommendation owner on
  #9109 Part C.
- Q3. Is the `snapshot-audit` floor of 50 right? **Default:** 50, tuned from
  dry-run reports under rule engine §7. **Decider:** the retention owner
  under #6041.
- Q4. Where does the envelope table live, D1 or the laptop store? **Default:**
  follow the placement decision in #9106 Part C, which is not yet made; until
  then the envelope is written to the same store as the object it describes.
  **Decider:** the storage owner under #9082.

## 10. Recorded tracking

- Source issue: developing-today/code#9140 (link this record there).
- Parent epic: developing-today/code#9109; program #9106.
- Queue design slice: task 21566, owned path `doc/` only.
