# ID/Atlas delivery preparation

**Readiness boundary: docs/preparation only.** This worker made no GitHub, queue, project, or comment writes and made no Git commits. The design-set commit `a1a6a64c25` and its PR-review-ready gate are separate from executable-issue publication. `prepare_delivery.py` is offline-only; any publication is a separately reviewed path requiring explicit GO after merged-design SHA and manifest digest approval.

The existing component issues #9110–#9148 remain the component homes. Every slice issue belongs natively under its existing component issue, with `blockedBy` reserved for hard dependencies. #9112 is separate sequencing/meta work, not a slice parent or self-parent; the manifest currently needs a separate orchestration/meta record under #9112.

## Verified publication mechanics (not executed)

- `gh issue create --help` supports `--parent` and `--blocked-by`; `gh issue edit --help` supports `--parent` and `--add-blocked-by`. Read back with `gh issue view <url> --json url,state,body,blockedBy,parent`; exact parent, body marker, and blocker set must match. Hierarchy and dependencies are different edges (`pm-stack/README.md`; `pm-stack/aggregator/src/projects.ts`). Select no labels unless verified live immediately before filing.
- Use stable marker `<!-- id-atlas-delivery: #9106:<sliceID> -->`, independent of the manifest digest. Put the approved digest in the body as provenance, not in the dedupe marker.
- Filing doctrine requires issue + queue coverage in the same pass (`AGENTS.md`, queue-reconcile README). CLI source shows `fleet queue enqueue <packet.json> --queue <pilot-queue>`; accepted enqueue options are `--queue`, `--platform`, `--dry-run`, `--shadow-intent`, with **no worker flag**. Packet schema and accepted values are defined in `scripts/fleet/queue-reconcile/check_coverage.py` and `pkgs/fleet/crates/fleet-queue/src/validate.rs`. Do not claim coverage until enqueue result and `repo-queue list --queue <queue> --task <id>` verify a live `queued|claimed|parked|blocked` row.
- Task dependency handling is supported separately from enqueue: `repo-queue block <task> --worker <worker> --reason-class blocked --blocker <text> --blocked-by-issue owner/repo#N --blocked-by-task <id>`; comma-separated refs per type are accepted. `repo-queue dep-recheck --task <id>` exists. Enqueue has no atomic dependency field, so a future publisher must disclose/handle the enqueue-then-block window; do not assert atomic gating. Here the conservative publication precondition is a merged design commit proven on `main`, not a new queue dependency on parent task 15354.
- `repo-queue --help` / `repo-queue enqueue --help` are not valid help invocations in this build; the Rust client parser and existing reconciler call are the verified sources. No queue command was run.

## Offline gate / report interface

Command:

```sh
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/prepare_delivery.py
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/test_prepare_delivery.py
```

Canonical issue fields and a row-valid fixture are in `delivery/schema.md`. The focused test script covers structured/reviewed oracles, `Option<Bytes>`, hard-null and exact dependency refs, pairwise/transitive collisions, path normalization and hard-link alias refusal, digest report output, ATX heading identity/rendering, and original-30 plus generic research-matrix reports (CRDT and unmeasured benchmark plan). Latest targeted run: **25 tests passed**. The full seven-manifest aggregate gate was deliberately not rerun in this normalization pass; parent will run it once after manifest writers finish.

Parent's first aggregate run after the separate #9112 research row: **126 records; 38/38 component homes; 384/384 statements; 176 errors** (full report in approved temp at `/home/user/.local/share/opencode/tmp/opencode/9106-delivery-report.json`). Parent identified 106 heading-normalization false positives, now fixed with exact ATX-prefix parsing and raw/rendered exact match. Other 70 errors remain delivery metadata/ownership/research issues assigned to their manifest owners. The parent will rerun once after those writers finish; this worker did not run the full aggregate gate.

Material blockers from the offline run:

1. Publication readiness remains blocked by issue-ready metadata in the manifest snapshot: rollback, test paths/commands, precise ownership and evidence/oracles; some rows still lack separately authored goal/enables/version fields. The validator does not copy one field into another.
2. Four file records are not exact path + existing/new declarations. `pkgs/id/crates/id-core/src/lib.rs` is shared across slices without distinct file owners and an explicit hard edge; the validator reports the collision and does not linearize it.
3. Dependency refs must be exact IDs with descriptions separately represented; prose-glued refs in the last snapshot included `C9133-01 result schema`, `C9138-01 decision`, and `C9133-CLOUD capacity evidence`. `requires`/`hard_dependencies` map to hard edges; `after`/`soft_dependencies` stay out of the hard DAG and native blocker list. Pairwise collisions require transitive hard reachability for every file-sharing pair.
4. Required standalone research scope tags/owners/triggers remain required for four CRDT evaluations plus the common matrix, real encoding bakeoff, cloud benchmark, producer-contract gap, and original-30 provenance recovery. Extra syntactically valid research scope tags are allowed; each required scope still needs exactly one row. The separate `delivery/research-slices.json` row makes provenance contingent under #9112; the accepted result may be “unrecovered after bounded source scan,” never a statement-derived reconstruction mislabeled as original.
5. The A receipt seam and URI correction are present in the current design/manifests: #9115 depends on B19's V2 shared receipt, not B11 (Diamond Types research), and unreserved URI octets remain literal. Preserve these when later repairing dependency edges.

## Validator choices and limitations (not a pass)

- `Option<Bytes>` is valid text; placeholder detection is limited to explicit placeholder phrases and does not interpret `<...>` as a template.
- Some crosscut manifests use `enables` where the gate requires a separate `goal`; that remains a schema gap, not permission to copy or invent a goal.
- Oracles are structured (`kind`, `expected`, `verifier`) or require an explicit fresh-judge review record. The checker validates shape, not semantic quality; research-matrix validation checks completeness/traceability, not factual truth or performance. Issue completion still needs fresh source review.
- GitHub anchor behavior was checked against `github-slugger` source: punctuation filtering then one replacement per literal space. Regression case `9113 — Foo` → `9113--foo` is tested.
- `--report PATH` writes a machine-readable JSON envelope with `manifest_sha256`, validity/errors, normalized `entries` in input order, and normalized #9112 `orchestration`. Stdout is bounded to summary plus at most twelve errors. It is explicit; without `--report` or digest-approved `--out`, the validator writes nothing.

These limitations and validation errors keep draft generation closed. Do not relax a gate merely to obtain a green count. Any next helper change should come after manifests settle; a separate publisher must be reviewed and implemented only with the merged SHA, approved digest, explicit GO, external marker/readback, queue validation/enqueue/readback, and a journal outside the repo.
