# PM Stack & Fleet orchestration — package guide

**Package:** #9196 / task 15381 · **Status:** integrated design package in progress; no implementation or production authorization.  
**Baseline:** source census pinned to `a038f084dbeb525494d7bb928ad91f597db2ac51`; dated GitHub/PR evidence is a separate layer. Source observations do not prove deployment.  
**Entry point:** [`architecture.md`](architecture.md). **Requirements/census:** [`components/9198-requirement-crosswalk-current-state-census.md`](components/9198-requirement-crosswalk-current-state-census.md). **Design records:** [`decisions/README.md`](decisions/README.md). **Issue index:** `issues/index.json` is pending the filing/batch owner; no URL or issue mapping is invented here.

## What this package decides

The charter calls for two coordinated designs, not one monolithic rewrite:

- **Track A — Priority, urgency, PM visibility and dispatch:** [`fleet-priority-urgency-pm-stack.md`](fleet-priority-urgency-pm-stack.md). Detailed contracts: [priority model #9199](components/9199-distributed-priority.md), [distribution/provenance #9200](components/9200-priority-distribution.md), [PM/mobile #9201](components/9201-pm-projection-mobile-control.md), [direct dispatch/events #9203](components/9203-direct-dispatch-slash-commands-event-routing.md), and [selection/exploration #9204](components/9204-queue-selection-bounded-exploration.md).
- **Track B — Work-item lifecycle, hydration, dependencies, journal, reliability and direct completion:** [`fleet-work-item-orchestration.md`](fleet-work-item-orchestration.md). Detailed contracts: [lifecycle/CLI #9206](components/9206-work-item-lifecycle-fleet-subcommands.md), [hydration/activity #9207](components/9207-context-hydration-incremental-activity.md), [relations/authority #9208](components/9208-typed-dependencies-notes-journal-field-authority.md), and [leases/outbox/completion #9209](components/9209-leases-fencing-outbox-direct-completion.md).

Shared authority, identity, command/result, ordering, cross-system reliability, locality and gates live once in [`architecture.md`](architecture.md); the track documents link to it rather than redefining shared contracts. These are reviewed design contracts, not V1 functionality or production policy. The Summit owner adopted eight package FDRs for local design scope on 2026-11-13 after independent technical agreement; fleet-wide canonical registration/naming through #9180 remains separate. No implementation or rollout is authorized by that adoption.

## Evidence layers and current known state

1. **V1 — observed source:** pinned repository code/tests and dated issue/PR evidence, explicitly scoped by [Track A census](census/track-a.md) and [Track B census](census/track-b.md). Neither is deployment proof. The queue broker has one-queue matching and atomic claim source evidence; final claim/list tie behavior differs. Proposal lifecycle commands are not inferred from adjacent queue verbs.
2. **V2 — contracts:** architecture, component designs, crosswalk and eight locally adopted FDRs. Independent source-semantic and technical review agree; final issue/publication coverage is still separate. Design defaults guide offline fixtures and planning; routine choices are not all human blockers.
3. **V3 — implementation:** only granular linked issues, each with its own failing test/oracle, compatibility and rollback acceptance. The live issue/PR index and exact IDs are pending issue filing/reconciliation, not assumed from titles or this roadmap.
4. **V4 — hosted parity:** #9079/#9081 consume the common semantics and fixtures; separately prove deployment and live reads/writes. Their closed parent #9078 does not close component work.

Known constraints: #7030’s `expedite=15` / `next-up=25` is already ratified; the current dispatch adapter snaps them to 10/20 and must be fixed without re-ratification or renumbering. `expedite=15` is not Do Now; persistent `manual_only` is eligibility, not a directive. The PM dispatch PostgREST producer bypass is tracked by #9028; the canonical broker is target architecture, not a claim all existing producers comply. The PM projector already has bounded HMAC intake/delivery dedup and disposable hot-projection checkpoints, but no canonical Fleet operation acceptance or remote mutation authority; its projection ACK is not launch/command proof. #8187 is existing launch-on-handoff behavior; #6533’s live launch-and-bind proof remains open. #9016/#8887 are the existing `fleet gh` boundary. #9178/#9179 govern claim/resume/manual-only and park constraints; #9180 governs decision-record convention. QTI remains unlocated/unverified; #9241/task 15480 is the bounded recovery/crosswalk work. #3424/#8575/#8579/#8580–#8585 are compatibility evidence, not presumed QTI equivalence. No overlapping schema change precedes compatibility disposition.

Known package-level artifacts at this writing: PR #9225 carries the orchestration design-buildout; research/crosswalk issue #9241 exists; full current issue map and batch-filed implementation links remain pending. No production/database/schema/dispatch/live-session change is authorized by this package.

## Reading order

1. Read this guide, then [`architecture.md`](architecture.md) and the exact scoped census/crosswalk.
2. Read the relevant track design for end-to-end workflow; use component sections for detailed contracts, scenarios, test oracles and implementation slices.
3. Read `decisions/README.md` and the relevant FDR when a contract is disputed. Local design adoption is not a ratified change to live fleet behavior; unresolved upstream rulings and real implementation/rollout gates remain.
4. Use `delivery-plan.md` for dependency order. Local slice keys are not GitHub issues; resolve them against `issues/index.json` after batch filing.
5. Before schema work, verify QTI research #9241, #3424/#8575/#8579 ownership and migration compatibility. Offline models/conformance can proceed without that schema gate.

## Completion boundary

This package is design evidence, not a claim of whole-program completion. Report separately what is designed, issued, implemented, tested, merged, deployed, live-verified, blocked, or still pending. Source evidence and mocks do not prove deployment; merged code does not prove live behavior; queued work does not prove execution. Final integration remains subject to fresh independent design review and real issue/acceptance links.
