# Review verification and evidence limits

Run from the repository root. These are small standalone design checks, not
production implementations or a substitute for the future conformance suites.
No Rust/Nix build, deployment, credentialed cloud benchmark or service change was
performed for this documentation set.

```sh
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/inventory/verify_inventory_regressions.py --source doc/2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-addressing/uri_codec_reference.py self-test
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-a-graph/graph-encoding-toy.py --check-oracles
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/verify_crosscut.py
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/federation_model.py
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/benchmark-workload-oracle.py
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-c/synthetic-oracle.py
python3 -I scripts/research_workflow.py check --repo-root .
git diff --check
```

## Observed results at the integrated checkpoint

| Check | Result | What it establishes |
|---|---|---|
| Numbered inventory + negative cases | 14 cases pass; 384 unique IDs, zero unmapped homes | Declared/fixed inventory and strict map/CLI rejection, not adequacy of a design |
| URI reference | 6 array vectors + 1 content golden, 17 malformed cases, 10,000 array + 10,000 content round trips pass | Typed slot/encoding/null/empty/context behavior of the reference, not production parsers |
| Toy triple encodings | Fixed hashes/round trips and arbitrary non-UTF-8 bytes pass | This finite corpus/codec behavior; SQLite JSONB bytes are specific to local SQLite 3.53.3 |
| Crosscut | 39 issue slots, 2 research slices, 17 implementation slices; DAG/file/config oracles pass | The crosscut model and its own manifest, not every cross-part dependency |
| B accounting model | Fixed checks and 2,000 seeded traces pass | Scoped reservation/retry/expiry abstraction, not cryptography, fairness or global Byzantine enforcement |
| C reference store | 16 executed transitions, computed SHA-256 fixture digest and 64 capability labels pass | Finite model/plan consistency, not provider or concurrent production GC correctness |
| C cost arithmetic | Synthetic totals reproduce; `winner: null` | Arithmetic only; no measured costs or procurement recommendation |
| B/C strict CLI | Three undeclared-argument cases exit 2, no normal output | Extra flags are not silently ignored before checks/fixture reads |
| Repository workflow check | `ok: true` | Existing workflow-package validation; this design-mill folder is not an S0–S8 research package |
| Whitespace | Clean | Formatting hygiene only |

Part B's allocation model and Part C's synthetic/reference checks are described in
their component documents. Their returned judges specifically distinguish
abstract-model evidence from cryptography, fairness, cloud measurements and real
adapter semantics. Repair dispositions preserve those limits; a later model pass
does not erase an earlier finding or imply full program acceptance.

## Separate publication gate

`delivery/prepare_delivery.py` validates/drafts offline and must report any
remaining concrete ownership, statement, tier or dependency gaps. A successful
component model or review-ready PR is not permission to publish defective issue
bodies. Native parents/blockers and **live** queue coverage require remote
readback after actual issue creation; no such readback is inferred from JSON
drafts. Task 15354 remains parked under its bound-owner handoff until its full
acceptance evidence exists and the launching lane reconciles it.

## Final delivery checkpoint

The canonical delivery gate passes 126 rows, all 38 component homes and all 384 statements with zero exemptions. Delivery regressions: 28; publisher regressions: 14. A research issue may create its exact owned domain validator; golden/reference oracles are valid, not forced into a generic matrix schema. Domain research acceptance is not established by generic structural validation. GitHub Actions remain queued under the existing runner-capacity issue #2605; the exact three proxy/basic workflow scripts were also executed locally and passed. No queued check is described as green.
