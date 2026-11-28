## Problem and goal
Determine whether the original thirty Part B candidate boundaries can be recovered from an authoritative source, separately from the current statement-derived reconstruction.

## Version and design basis
Version: V4 provenance investigation. Settled design: https://github.com/developing-today/code/blob/main/doc/2026-11-13T03-40-00Z_design_id-atlas-summit/main.md#4-research-and-verification-discipline (section: **4. Research and verification discipline**).
Tier: **contingent**. Start trigger: Proceed if the original intake/source record becomes accessible; otherwise complete a bounded source scan and report an unrecovered result without reconstructing a fictional original list.
Statement IDs: research provenance scope; no statement ownership asserted
Enables: A provenance-backed comparison or an explicit unrecovered disposition that prevents the reconstruction from being mislabeled as the original list.

## Scope and ownership
Parent: #9112 (native component/meta parent).
- `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/original-30-provenance-report.json` (new; owner: meta-9112-R-original-30-provenance)
Hard dependencies (native blockers): none
Soft sequencing (not blockers): none

## Test / evidence first
Create or validate `doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/original-30-provenance-report.json` first. Proposed command: `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/validate_research_report.py --input doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/original-30-provenance-report.json`
Oracle: **research-matrix** — expected `Either 30 unique source-backed original records, or status unrecovered with a completed bounded search log and no fabricated item list.`; verifier: `The standard-library artifact validator checks status-dependent shape, uniqueness, search log presence, and source references.`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: inventory-live/candidate-provenance.md; part-b/implementation-slices.json provenance status; authoritative source records searched by the research owner.
Rollback: Remove only the provenance report; never replace it with inferred titles or treat the statement-derived reconstruction as recovered source.

Manifest SHA-256 approval binding: `3b5bce3523945bf5adaa28739f0b90b7b70ff1d6522b4ac827246654e8ff6b06`.
<!-- id-atlas-delivery: #9106:meta-9112-R-original-30-provenance -->
