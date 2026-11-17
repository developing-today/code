# Atlas retention rule engine — design record (#6041)

Status: design decision record for issue #6041, written 2026-11-17.
Parent policy: `doc/2026-11-08T10-26-00Z_design_atlas_content_versioning_retention/`
(issue #6026), §4. This record parameterizes that policy; it does not
amend it. Where this record and §4 disagree, §4 governs and this record
is the defect.

## 1. What this decides, and what it does not

Decides: the rule schema the engine evaluates, the evaluation cadence,
the dry-run report format, and the path by which a rule is promoted
from dry-run to live. These are the four deliverables in #6041's
"Done looks like".

Does not decide: numeric thresholds beyond the §4.2 starting defaults
(they are parameters of the schema below, tuned from dry-run reports,
per §7 of the parent record); the stats/feedback pipeline's own schema
(sibling design under #6018 — this engine consumes only its §4.3
interface, version id in, surfaced-events out); the cold shard layout
and restore protocol (#6042); implementation language or module
placement (the implementation packet's call, against this record).

## 2. Invariants the schema must make unrepresentable

From §4.1, §4.4, and the #3125 reconciliation:

1. **Conjunction, always.** A version retires only when at least one
   trigger fires for it AND its class rule permits retirement. If
   either side says keep, the version is kept. There is no rule shape
   that retires a version no trigger fired on.
2. **Class rules only narrow.** A rule record may lengthen windows,
   raise keep-counts, or disable a trigger for its class. It may not
   shorten a window below the §4.2 default, lower a keep-count below
   the default floor, or enable a trigger the default row leaves
   ineligible for that column — narrowing is checked mechanically at
   rule-write time against the default row, not by reviewer vigilance.
3. **Floors are absolute.** The keep-newest-N floor binds the
   increments trigger regardless of the other triggers' state for the
   newest N versions individually (the other columns still apply to
   them only if they independently fire, per §4.2's reading). The head
   version of a live file is never retired by the increments trigger
   while it is still the head.
4. **Report before deletion, per rule.** No rule deletes anything
   until it has completed the promotion path in §5. A report alone
   never deletes (§4.4).
5. **Cold before delete, per version.** A version must spend at least
   one full retention cycle cold-tiered before it is deletion
   eligible, whatever its rule's mode.
6. **Every retirement is answerable.** Each retired version leaves the
   §4.4 tombstone (file id, version seq, content hash, trigger, rule,
   run id), so a query trace (#6027) referencing it resolves to
   "retired under policy", never a dangling reference.

## 3. Rule schema

Rules are data, evaluated identically by the reporter and (later) the
live runner. One record per (class, tier) pair:

| field                                     | meaning                                                                                                                                                                   |
| ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `class`                                   | classification-manifest vocabulary: `code`, `docs`, `session logs`, `chat logs`, `generated reports` (extensible by manifest amendment, not by rule edit)                 |
| `tier`                                    | `hot` or `cold` — the tier the rule's retirements move versions out of                                                                                                    |
| `date_days`                               | by-date trigger threshold (days since ingestion); `null` disables the trigger for this row                                                                                |
| `keep_newest_n`                           | increments floor: never retire any of the newest N versions via increments; also the increments trigger threshold (versions with ≥ N newer siblings are trigger-eligible) |
| `access_window_days`                      | by-access trigger: never-surfaced, or not surfaced within this many days; `null` disables                                                                                 |
| `override_owner`                          | who may write this rule record and promote it (§4.2's overrides column: repo owner, corpus owner, program owner, generating job's owner)                                  |
| `mode`                                    | `dry-run`, `live-cold`, or `live-delete` (§5); every new or changed rule starts at `dry-run`                                                                              |
| `version`, `effective_from`, `supersedes` | rule records are themselves versioned; evaluation names the exact rule-set version it used                                                                                |
| `promoted_from_reports`                   | the dry-run run ids that earned the current mode (§5); empty in `dry-run`                                                                                                 |

Semantics are fixed, not per-rule options: triggers combine by OR
(any one firing makes a version a candidate); the class row is the
permission; invariants 2–3 are applied after trigger evaluation and
cannot be configured away. The §4.2 defaults ship as the initial rule
set, version 1, all rows `dry-run`.

Worked reading (defaults, `code` class, hot tier): a non-head version
ingested 500 days ago, with 60 newer versions, never surfaced, is a
candidate on all three triggers; the row permits retirement; the
increments floor (50) does not protect it (60 > 50 newer); the report
lists it under each trigger that fired. A version 500 days old but
among the newest 50 is protected from the increments trigger only —
the date trigger still fires on it, and the report says so explicitly
rather than letting the floor read as a general keep.

## 4. Evaluation cadence

- **Cycle.** One evaluation cycle per week by default; the cycle
  length is a deployment parameter, and §5's "one full cycle" always
  means one complete cycle at the then-current length. A cycle
  evaluates every rule in the current rule-set version.
- **Deterministic runs.** A run is a pure function of three pinned
  inputs: the lineage-graph snapshot, the surfaced-events snapshot
  (the #6018 interface), and the rule-set version. Each run has a
  run id and records the three input pins, so any report can be
  re-derived byte-for-byte and two runs can be diffed meaningfully.
- **On-demand dry-runs.** Anyone may run a dry-run evaluation against
  a proposed rule set (e.g. a threshold change) at any time; on-demand
  runs produce reports in the same format but never count toward
  promotion (§5 counts only scheduled cycle reports under an
  unchanged rule).
- **No continuous evaluation.** Retention is a batch judgement over
  snapshots, not a streaming one: a version that becomes eligible
  mid-cycle waits for the next cycle. This is deliberate — it is what
  makes reports reviewable before anything moves.

## 5. Promotion path (per rule, never global)

A rule earns trust independently; promoting one class's rule says
nothing about another's.

1. `dry-run` → `live-cold`: the rule has appeared **unchanged** in at
   least **two consecutive scheduled cycle reports**, and its
   `override_owner` records the promotion, naming those run ids in
   `promoted_from_reports`. In `live-cold`, the runner performs only
   hot→cold moves for versions the rule selects; deletions still only
   appear in the report.
2. `live-cold` → `live-delete`: at least **one further full cycle**
   in `live-cold` with the rule unchanged, and a second recorded
   promotion by the same owner naming the `live-cold` run ids. Only
   now may the runner delete — and only versions that have already
   spent ≥1 full cycle cold (invariant 5), which the runner checks
   per version from cold-tier residency records, not from report
   history.
3. **Demotion is immediate and unilateral.** The override owner (or
   Drew) may drop a rule's mode at any time, effective the next run;
   demotion needs no earning period and no justification field,
   though the rule record keeps the demotion in its version history.
   Any edit to a rule's thresholds returns it to `dry-run` with
   `promoted_from_reports` cleared — a changed rule is a new rule.
4. **Defaults stewardship.** Until an override owner delegates
   otherwise, Drew is the promotion authority for the §4.2 default
   rows; this matches §4.4's "until Drew promotes a rule to live".

## 6. Dry-run report format

One report per run, in two renderings of the same data: a
machine-readable JSON document (the promotion artifact; diffs and
tooling consume it) and a human summary rendered from it.

Required contents:

- **Header:** run id, cycle timestamp, the three input pins
  (lineage snapshot id, stats as-of, rule-set version), engine
  version.
- **Per-rule sections**, one per rule evaluated, each stating: the
  rule record (all fields, including mode); candidate counts and
  total bytes, split by **would cold-tier** vs **would delete**;
  candidates grouped by the trigger(s) that fired, each group with
  count and bytes; explicit **protection notes** — how many versions
  a trigger fired on but a floor or the head rule protected, and
  which protection applied (this is what keeps the §4.2 floor from
  being misread as a general keep).
- **Version detail** (JSON only): every candidate version with file
  id, version seq, content hash, class, tier, triggers fired, and the
  action the rule's current mode would take (`report-only`,
  `cold-tier`, `delete`). In `dry-run` mode every action is
  `report-only`; the would-be action is still computed and shown.
- **Deltas vs the previous cycle's report** for the same rule set:
  newly eligible, no longer eligible, and mode changes — so a
  reviewer reads what changed, not the whole corpus.
- **No content.** Reports name versions by id and hash only; no file
  contents, no extracted text. A report is safe to attach to an issue.

Deletion runs (in `live-delete`) emit the same report for the run's
selections **plus** the tombstone records actually written, appended
as a run annex; the pre-deletion report and the annex share the run
id, so "what was here and why is it gone" is one lookup.

## 7. Governance encoding (#3125 vs deletion triggers)

The engine encodes the #3125 reconciliation exactly as §4.1 states
it, per tier and class, rather than as a global flag:

- On the **hot tier**, keep-ALL is the default posture: the only path
  out is the §4.1 conjunction under a promoted rule (§5). An
  unpromoted rule set — including the shipped defaults — moves
  nothing.
- **Byte-identical duplicates** are outside this engine entirely:
  #3125's dupe cleanup is a storage-layer property (content
  addressing), not a retention decision, and no rule record can
  express it.
- The schema-level narrowing check (invariant 2) is the mechanical
  form of "class rules may only narrow": a rule write that would
  widen retirement relative to the default row is rejected at write
  time with the default row cited.

## 8. Falsifiability

- A live run that retires a version no trigger fired on refutes
  invariant 1 and invalidates this design's core claim.
- A rule write that shortens a window below the §4.2 default being
  accepted refutes invariant 2.
- A version deleted without a prior full cycle cold-tiered refutes
  invariant 5.
- A report that cannot be re-derived from its three input pins
  refutes §4's determinism claim.
- A retired version with no tombstone, or a #6027 trace resolving a
  retired version to anything other than its tombstone, refutes
  invariant 6.

## 9. Follow-ons

Implementation of this design (schema storage, evaluator, reporter,
runner) is a separate packet, filed when the atlas program
prioritizes it; this record authorizes no implementation by itself.
Threshold tuning after the first cycles is rule-record editing under
§3/§5, not new design.
