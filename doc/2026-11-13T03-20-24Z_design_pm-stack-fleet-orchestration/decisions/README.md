# PM/Fleet decision record index

**Package:** PM Stack & Fleet orchestration, task 15381 / #9196  
**Status:** accepted for this package's local design contract after independent technical review and Summit-owner adoption, 2026-11-13; not production authorization.
**Package record home:** this directory, selected for this public-safe design package. **Canonical fleet-wide home:** #9180 recommends `doc/decisions/README.md` and `doc/decisions/`; this package does not claim that convention has been merged or adopted.  
**Namespace:** `FDR-PMO-NNN` is proposed for these package records, pending reconciliation with the canonical #9180 index. It does not allocate or reserve fleet-wide IDs. The existing `FDR-0001`–`FDR-0010` seeds in #9180 are a distinct namespace and are not reused here.

## How to use this index

These eight records consolidate related candidate decisions already described in the architecture and component designs. Their IDs are now the stable local references for this package; earlier component-level candidate IDs were suggestions, not allocations. Component designs should resolve their candidates to these records when their owner review happens. This index and records are not implementation issues, do not establish deployed behavior, and do not authorize schema, dispatch, migration, or production changes.

The Summit design owner (task 15381) adopts these eight records for local design scope following `../reviews/component-contracts.md`; each record carries the date and limitation. This is not adoption of new live queue semantics, numeric thresholds or production policy. Canonical fleet-wide registration/naming through #9180 remains separate and does not block independent offline contract work. Production changes still require granular implementation issues, source-owner/schema compatibility review, tests, rollback evidence and genuine operational/security approvals. Existing #7030 `expedite=15` / `next-up=25` authority is consumed, not re-ratified.

The #9180 convention requires each record to state ID, title, date, status, owner, decision maker, context, source links, options including do-nothing where genuine, operational decision, rationale/evidence with fact/inference separation, consequences and migration, tests, revisit triggers, supersession links, and implementation/research links. Each record below follows that checklist. No supersession is asserted unless a link is explicitly added.

## Records

| ID          | Record                                                                                                        | Status                 | Domain / current links            |
| ----------- | ------------------------------------------------------------------------------------------------------------- | ---------------------- | --------------------------------- |
| FDR-PMO-001 | [Canonical broker and distributed command seam](FDR-PMO-001-canonical-broker-command-seam.md)                 | accepted (design only) | #9200, #9203, #9028, #9198, #9241 |
| FDR-PMO-002 | [Versioned priority profiles, ordering, and provenance](FDR-PMO-002-priority-profiles-ordering-provenance.md) | accepted (design only) | #9199, #9200, #9204, #7030        |
| FDR-PMO-003 | [Lifecycle, claim compatibility, and lease fencing](FDR-PMO-003-lifecycle-claim-fencing.md)                   | accepted (design only) | #9206, #9209, #9178, #9179        |
| FDR-PMO-004 | [Immutable hydration, cursors, and attribution](FDR-PMO-004-hydration-cursors-attribution.md)                 | accepted (design only) | #9207, #9206, #9209               |
| FDR-PMO-005 | [Typed dependencies, private journal, and field authority](FDR-PMO-005-dependencies-journal-authority.md)     | accepted (design only) | #9208, #9199, #9209               |
| FDR-PMO-006 | [Transactional outbox and direct verified completion](FDR-PMO-006-outbox-direct-completion.md)                | accepted (design only) | #9209, #9207, #9208, #9198        |
| FDR-PMO-007 | [Direct dispatch and durable launch receipts](FDR-PMO-007-direct-dispatch-receipts.md)                        | accepted (design only) | #9203, #9201, #9028, #6533        |
| FDR-PMO-008 | [Bounded exploration, fairness, and calibration](FDR-PMO-008-exploration-fairness-calibration.md)             | accepted (design only) | #9204, #9199, #8097               |

## Cross-record source and compatibility anchors

- Governing packet: [handoff source 01](../sources/01_handoff_pm_stack_fleet_orchestration.md), [charter v1.1 source 02](../sources/02_charter_v1_1.md), [full orchestration proposal source 03](../sources/03_full_orchestration_proposal.md), [recovered original source 04](../sources/04_recovered_original_combined_design.md), and [supplement source 05](../sources/05_supplemental_raw_note.md). Source 04 is flattened in recovery; the packet's source inventory and component trace preserve that limitation.
- Shared current design: [architecture.md](../architecture.md). Component proposals: [#9199](../components/9199-distributed-priority.md), [#9200](../components/9200-priority-distribution.md), [#9201](../components/9201-pm-projection-mobile-control.md), [#9203](../components/9203-direct-dispatch-slash-commands-event-routing.md), [#9204](../components/9204-queue-selection-bounded-exploration.md), [#9206](../components/9206-work-item-lifecycle-fleet-subcommands.md), [#9207](../components/9207-context-hydration-incremental-activity.md), [#9208](../components/9208-typed-dependencies-notes-journal-field-authority.md), [#9209](../components/9209-leases-fencing-outbox-direct-completion.md).
- Evidence and limitations: [Track A census](../census/track-a.md), [Track B census](../census/track-b.md), [Track A prior art](../research/track-a-state-of-art.md), and [Track B prior art](../research/track-b-state-of-art.md). Census facts are pinned-source facts, not deployment proof; research sources support only their stated claims.
- Schema compatibility: Queue Table Improvements (QTI) remains unlocated/unverified. Research issue #9241 is the current recovery/crosswalk work; #3424, #8575, #8579 and #8602 are compatibility evidence/candidates, not presumed QTI equivalents. Schema-coupled changes remain gated on the documented compatibility disposition.
- Fleet-wide record process: [#9180](https://github.com/developing-today/code/issues/9180) is the live decision-record convention issue and reserves `FDR-0001`–`FDR-0010` for its seed set. The package-level `FDR-PMO-*` namespace remains proposed until the canonical index owner adopts a convention.

## Adoption and maintenance

Independent design review should verify source traceability, genuine alternatives, internal consistency, and proposed status before owner adoption. Adoption does not convert proposal evidence into V1 implementation facts. When adopted, link the canonical copies from #9180 and retain the package copies or move/link them only under an explicit owner-approved policy; do not rewrite historical decision files or reuse IDs. Future implementation issues and stable semantic requirement IDs will be overlaid by the parent after allocation; this index intentionally invents neither.
