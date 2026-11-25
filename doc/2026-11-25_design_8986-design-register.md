# Design register governance and weekly verification

- **Date:** 2026-11-25
- **Status:** Design proposal; no implementation in this record
- **Tracking home:** [developing-today/code#8986](https://github.com/developing-today/code/issues/8986)
- **Lineage:** [PR #8980](https://github.com/developing-today/code/pull/8980), [lineage manifest](../pm-stack/lineage-manifest-2026-11-12.md)
- **Evidence:** [AUDIT-7 design-gaps #13873](https://github.com/developing-today/code/issues/13873), [AUDIT-7 umbrella #13871](https://github.com/developing-today/code/issues/13871)

## 1. Problem and context

Design work has escaped both a durable tracking home and its board. The
2026-11-12 lineage pass created #8986 as the home for the 27 `epic/design-register`
issues that had no epic; 26 were natively parented because some already had a
parent or were intentionally nested. The source epic now requires every child
to carry `epic/design-register` and be a native sub-issue (with documented
cross-epic exceptions), and every new issue in the effort to follow the same
rule. The epic is a register, not a deliverable that should close while its
children remain open.

The companion tracking-manifest pass found 37 tracking issues and 27 committed
doc files, but that inventory demonstrates why linkage must be maintained as
work arrives rather than repaired in occasional sweeps. AUDIT-7 Lane 6 then
found hundreds of design-demand issues and 111 short/missing designs in code;
its report explicitly queued #8986 as the P1 design-register gap. AUDIT-7's
later updates also show the need for fresh, auditable reconciliation: rows can
be newly dispositioned or corrected after a summary is written.

The repo already has the relevant surfaces. Project #4 has seven named design
views, including `Design Register`; `pm-stack/aggregator/src/design-boards.ts`
renders read-only cached snapshots, and the aggregator README documents
unavailable/stale behavior and the fact that the projection does not mutate
board state. The existing projection is not a second source of truth and
should not be replaced by a new tracker.

## 2. Goals and non-goals

### Goals

- Give each design one durable tracking issue and a dated, reviewable design
  record in `doc/` (or an explicit link to an already committed canonical
  design record).
- Keep the tracking issue, `design` and relevant epic labels, native parent
  relation, and Project #4 design-board membership consistent.
- Make exceptions explicit, with a reason and canonical parent/tracking home.
- Run a weekly reconciliation that finds drift, reports evidence, and routes
  corrections without silently changing ownership or closing the epic.
- Make the workflow idempotent, inspectable, and safe when GitHub/Project data
  is missing or stale.

### Non-goals

- Replace GitHub issues, native sub-issues, Project #4, or the existing cached
  aggregator projection with a new database or tracker.
- Automatically decide whether a design is good, sufficient, approved, or
  complete; the nine-point standard is evaluated by a person/review process.
- Automatically reparent cross-epic issues, apply labels, close issues, or
  change board Status fields without a separately authorized decision.
- Close #8986 merely because an audit finds no drift in one run.

## 3. Alternatives and tradeoffs

1. **Do nothing / rely on memory and manual spot checks.** No implementation
   cost, but it repeats the zero-carrier and missing-design failures and has no
   durable proof that a check happened.
2. **Use only the issue checklist.** Minimal and already present on #8986, but
   checkboxes drift from issue state and do not prove doc/board membership or
   later additions. Keep the checklist as a human-readable summary, not the
   audit oracle.
3. **Create a separate register database or custom board.** Could offer
   stronger structured queries, but duplicates GitHub issue/Project data,
   requires synchronization and access-control work, and conflicts with the
   existing Project #4 family and cached projection.
4. **Adopt a write-capable board sync bot.** Could repair drift automatically,
   but expands credentials and mutation risk, and a bot cannot safely resolve
   ambiguous ownership, multi-epic labels, or quality judgments.
5. **Chosen: GitHub as canonical record, existing Project #4 as mirror, and a
   read-only weekly reconciliation with explicit human-routed repairs.** This
   matches current surfaces, keeps authority clear, and makes uncertainty
   visible rather than pretending the board is complete.

## 4. Chosen approach and contracts

### Authority and record shape

- **Canonical tracking record:** the GitHub issue that names the design and
  links its dated design document. Existing open tracking homes are reused;
  do not create twins just to satisfy a mechanical count.
- **Canonical design document:** a committed `doc/` record that states scope,
  decisions, and evidence. A living document may be linked where the work is
  ongoing; a dated design record records the decision snapshot. Every issue
  must link to the canonical file, and the file must link back to its issue.
- **Epic membership:** `epic/design-register` plus native parent `#8986` for
  ordinary members. A cross-epic member records the exception on both relevant
  tracking surfaces and uses the existing one-parent rule; #8833 is the known
  exception, natively under #8988 (`epic/storage`).
- **Board mirror:** Project #4's `Design Register` view is selected by the
  existing `label:design` filter. Area views are separate named filters, not
  implicitly intersected with that label. Board membership is a discoverability
  mirror, not proof of completeness.
- **Current state:** issue status and Project Status remain human-owned. The
  weekly check reports differences and creates/routs a repair item; it does not
  silently resolve them.

### Weekly monitor contract

The monitor takes a dated snapshot of the candidate issue set and, for each
candidate, checks: issue is readable; canonical doc link resolves to a committed
file; reciprocal link exists; expected design/epic labels are present; parent
relation is correct or a recorded exception applies; and the issue appears in
the appropriate Project #4 view. It records run time, source query/scope,
counts, exceptions, per-finding evidence and follow-up issue/queue references.
It distinguishes **missing**, **mismatched**, **unavailable**, and **stale**;
an unavailable Project snapshot is never reported as an empty board. The report
is append-only/date-stamped and linked from #8986. Findings are deduplicated by
issue + invariant so reruns do not create duplicate repair tasks.

The monitor uses the existing project-token and cached board-feed path; the
aggregator is read-only. A future writer must be separately designed and
reviewed. Schedule creation or modification must also update
`doc/fleet-schedule-registry.md` in the same change, per the repository rule;
this proposal alone creates no schedule.

### Security and capability boundaries

Read-only issue and Project access is sufficient for detection. A missing or
expired token, API permission error, rate limit, incomplete pagination, or
stale snapshot produces an explicit incomplete run and preserves the last
successful evidence; it must not claim zero findings. Do not put credentials
or private issue bodies in the report. Mutating operations (labels, parent
links, project fields, closing) remain outside the monitor and require a
separately scoped authorized workflow. The existing renderer validates GitHub
URLs, rejects malformed snapshots, escapes displayed text, and reports stale
data; retain these boundaries.

## 5. Dependency-ordered build slices

Each slice is a separately reviewable one-session unit. Slices 1–3 establish
the governance contract and baseline before automation; slice 4 adds the
read-only monitor; slice 5 wires the recurring schedule and verifies operations.

### Slice 1 — Define the register schema and exception ledger

Document the required issue/doc/label/parent/board fields and the one-parent
exception format; normalize the existing #8986 member list and record #8833's
cross-epic treatment. No bulk reparenting or issue mutation.

**Acceptance:** a reviewer can map every listed member to a tracking issue,
canonical doc or explicit doc-gap, labels, parent, and exception reason; the
counts reconcile to the source issue and lineage manifest; no parent is
overwritten.

### Slice 2 — Establish the human intake and repair protocol

Specify the new-design checklist and repair-item format. New work is filed with
the source/design link, `design` and relevant epic labels, native parent, and
Project #4 membership; corrections go through an owned issue/queue packet.

**Acceptance:** a dry-run against one existing ordinary member and #8833
demonstrates both the ordinary and exception paths; duplicate tracking homes
are not created; the checklist is actionable and linked from #8986.

### Slice 3 — Build a read-only baseline/audit oracle

Use existing GitHub issue/Project clients and `designBoardsFromFeed` /
`boardFamilyPanels` contracts where applicable. Add a report model for
findings, run completeness, evidence links, and stable dedupe keys. Do not add
new persistence or writes.

**Acceptance:** fixtures cover correctly linked, missing doc, missing label,
wrong parent, documented exception, unsupported view filter, absent snapshot,
and stale snapshot; incomplete inputs cannot produce a clean bill of health.

### Slice 4 — Implement the weekly read-only reconciliation

Run the audit over the complete, paginated candidate set and publish a dated
report linked from #8986. Findings name the invariant, evidence, and suggested
owner/action; dedupe repeat findings. Detection does not write back to issues
or Projects.

**Acceptance:** a controlled fixture with injected drift produces exactly the
expected findings; a clean complete fixture reports zero findings; rerunning
the same input is idempotent; unavailable data yields `incomplete`, not
`clean`.

### Slice 5 — Schedule, observe, and rehearse recovery

Register the weekly schedule in `doc/fleet-schedule-registry.md`, add failure
visibility and a documented manual invocation, then rehearse a failed-token or
stale-board run and recovery. This slice follows the monitor implementation.

**Acceptance:** schedule appears in the registry and on its execution surface;
one successful run and one simulated incomplete run are inspectable; the
incomplete alert identifies the failed input; manual rerun does not duplicate
findings or repair packets.

## 6. Rollout and rollback

Roll out in shadow mode: first generate reports without creating repair tasks;
compare findings with a manual read of the source issue and Project #4. After
one complete weekly cycle, enable deduplicated repair-task routing while
keeping all mutations human-directed. Announce the report location in #8986.

Rollback means disable the schedule/repair routing and retain dated reports and
the last known-good snapshot. Do not delete the canonical issues, docs, or
historical results. If reports disagree with GitHub, stop interpreting that
run as clean, fix the query/data-coverage defect, and rerun. Any automation
that changes labels, parenting, board fields, or issue state is outside this
rollout and needs its own approval, tests, and rollback design.

## 7. Verification plan

- **Static contract:** verify every #8986 child and new intake record has the
  canonical issue/doc links, labels, parent or exception, and board route.
- **Unit/model tests:** assert the reconciliation invariants on a small model;
  property tests should establish that adding an unlinked candidate cannot
  decrease the finding count, duplicate input does not duplicate findings,
  and incomplete data never yields a clean state. Each test names its oracle.
- **Integration fixture:** exercise complete pagination, GitHub API errors,
  missing permissions, unsupported view filters, stale snapshots, and the
  #8833 exception. Verify stable finding identity across repeated runs.
- **Live proof:** perform a read-only spot check of the full member set against
  the epic's native sub-issues and Project #4's `Design Register` view; record
  query time, counts, and inaccessible coverage. Never infer absence from a
  truncated response.
- **Operations:** inspect one weekly successful report, one deliberately
  incomplete report, schedule-registry parity, alert delivery, and a manual
  recovery run. Keep the epic open while its work remains open.

Existing tests in `pm-stack/aggregator/test/design-boards.test.ts` already
assert named-panel count, filter semantics, stale/unavailable handling,
escaping, and no client-side GitHub fetch. New monitor tests should extend this
contract rather than weaken it.

## 8. Open questions and deciders

1. **Should all designs (including completed/historical designs) be required
   on Project #4, or only active design work?** Decider: Drew, as product/board
   policy owner. Default: include every tracked design; use status to separate
   completed work, unless he narrows scope.
2. **Where should dated weekly reports live: `doc/` or a generated operational
   artifact with only summaries committed?** Decider: PM-stack maintainer.
   Default: dated, compact `doc/` report with raw machine output retained only
   if it is needed to reproduce findings.
3. **Which scheduler/execution surface should own the monitor?** Decider:
   PM-stack maintainer with Fleet schedule owner. Default: reuse the existing
   registered weekly register-monitor surface if present; otherwise choose the
   smallest supported Fleet surface and update the unified schedule registry.
4. **May any future repair bot write labels or Project fields?** Decider: Drew
   for policy and the repository security reviewer for capability scope.
   Default: no writes; create or link a repair task for a human.
5. **How should the monitor handle designs whose tracking home is in another
   repository?** Decider: PM-stack maintainer. Default: include only when the
   canonical record is accessible and state repo/token coverage; otherwise
   report `unavailable`, never silently omit it.

## 9. Record and tracking linkage

This dated document is the design record for #8986. The source issue must link
here; this document links back to #8986. The issue remains the tracking home,
Project #4 remains the board mirror, and the queue item that requested this
design is `repo-health-code-pilot` task 21560 (feature slice 1). This record
does not close #8986 or authorize implementation.

## Change log

- 2026-11-25: Initial design proposal from #8986, the AUDIT-7 evidence, PR
  #8980 lineage, and the existing Project #4 aggregator surface.
