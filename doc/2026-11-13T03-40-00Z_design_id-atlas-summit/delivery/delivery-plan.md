# ID/Atlas delivery preparation

**Readiness boundary: docs/preparation only.** This worker made no GitHub, queue, project, or comment writes and made no Git commits. The design-set commit `a1a6a64c25` and its PR-review-ready gate are separate from executable-issue publication. Per parent direction, no publisher/executor is shipped; `prepare_delivery.py` is default dry-run/offline and cannot publish. Any eventual publication requires a separate explicit GO after merged-design SHA and manifest digest approval.

The existing component issues #9110–#9148 remain the component homes. Every slice issue belongs natively under its existing component issue, with `blockedBy` reserved for hard dependencies. #9112 is separate sequencing/meta work, not a slice parent or self-parent; the manifest currently needs a separate orchestration/meta record under #9112.

## Verified publication mechanics (not executed)

- `gh issue create --help` supports `--parent` and `--blocked-by`; `gh issue edit --help` supports `--parent` and `--add-blocked-by`. Read back with `gh issue view <url> --json url,state,body,blockedBy,parent`; exact parent, body marker, and blocker set must match. Hierarchy and dependencies are different edges (`pm-stack/README.md`; `pm-stack/aggregator/src/projects.ts`). Select no labels unless verified live immediately before filing.
- Use stable marker `<!-- id-atlas-delivery: #9106:<sliceID> -->`, independent of the manifest digest. Put the approved digest in the body as provenance, not in the dedupe marker.
- Filing doctrine requires issue + queue coverage in the same pass (`AGENTS.md`, queue-reconcile README). CLI source shows `fleet queue enqueue <packet.json> --queue <pilot-queue>`; accepted enqueue options are `--queue`, `--platform`, `--dry-run`, `--shadow-intent`, with **no worker flag**. Packet schema and accepted values are defined in `scripts/fleet/queue-reconcile/check_coverage.py` and `pkgs/fleet/crates/fleet-queue/src/validate.rs`. Do not claim coverage until enqueue result and `repo-queue list --queue <queue> --task <id>` verify a live `queued|claimed|parked|blocked` row.
- Task dependency handling is supported separately from enqueue: `repo-queue block <task> --worker <worker> --reason-class blocked --blocker <text> --blocked-by-issue owner/repo#N --blocked-by-task <id>`; comma-separated refs per type are accepted. `repo-queue dep-recheck --task <id>` exists. Enqueue has no atomic dependency field, so a future publisher must disclose/handle the enqueue-then-block window; do not assert atomic gating. Here the conservative publication precondition is a merged design commit proven on `main`, not a new queue dependency on parent task 15354.
- `repo-queue --help` / `repo-queue enqueue --help` are not valid help invocations in this build; the Rust client parser and existing reconciler call are the verified sources. No queue command was run.

## Offline gate snapshot

Command:

```sh
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/prepare_delivery.py
python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/test_prepare_delivery.py
```

Latest bounded snapshot digest: `ed8875df3eb774c5d192ca6e6ce26d8f4d8f9e24dc795ef200f1567436640e35` (recompute after manifest edits; not an approval). The gate reads the five component implementation manifests plus `crosscut/research-slices.json`: **124 records; all 38/38 required component homes represented; range-expanded statement mapping covers 384/384 IDs with no exemptions.** Parent's annotation repair supplies the last eight statement mappings in the existing 17 crosscut rows: S0.3/S0.4 in 9110-01, S0.10 in 9110-02 (Kafka-like test double, not Kafka implementation), S0.11/S7.7 in 9111-03, and S1.116/S1.117/S1.120 in the interop row.

Material blockers from the offline run:

1. Publication readiness remains blocked by missing issue-ready metadata: exact design-section anchors, rollback, test paths/commands, and evidence-backed decidable oracles; several crosscut and research records also lack separately stated goal/enables/version fields. The helper emits no drafts while any required field is missing.
2. Four file records are not exact path + existing/new declarations. `pkgs/id/crates/id-core/src/lib.rs` is shared across slices without distinct file owners and an explicit hard edge; the validator reports the collision and does not linearize it.
3. Four dependency refs are prose-bearing (`C9133-01 result schema`, `C9138-01 decision`, `C9133-CLOUD capacity evidence`) rather than exact slice ID plus separate description. Classify them as hard or soft explicitly. Existing `requires`/`hard_dependencies` map to hard edges; `after`/`soft_dependencies` remain soft and are not used in the hard DAG or native blocker list.
4. Required standalone research scope tags/owners/triggers are still absent for four CRDT evaluations plus the common matrix, real encoding bakeoff, cloud benchmark, producer-contract gap, and original-30 provenance recovery. `crosscut/research-slices.json` is parsed, but its records still need issue-ready ownership/design anchors and exact output/test fields. Part B explicitly says the original 30-item source list was not recovered; statement-derived reconstruction is not equivalent to recovery.
5. The A receipt seam and URI correction are present in the current design/manifests: #9115 depends on B19's V2 shared receipt, not B11 (Diamond Types research), and unreserved URI octets remain literal. Preserve these when later repairing dependency edges.

## Known validator limitations (not a pass)

- The placeholder regex can misread Rust-like `Option<Bytes>` as a template placeholder.
- Some crosscut manifests use `enables` where this gate requires a separately stated `goal`; this is an unresolved schema mismatch, not permission to copy `enables` into `goal` silently.
- Oracle decidability currently has a lexical heuristic. Its 66 failures require human/model review; keyword presence is not proof of a decisive oracle, and a valid oracle can be a false positive under the heuristic.
- The exact design heading check and current coverage/digest are local snapshots; final publication must revalidate merged design files and manifest digest.

These limitations and validation errors keep draft generation closed. Do not relax a gate merely to obtain a green count. Any next helper change should come after manifests settle; a separate publisher must be reviewed and implemented only with the merged SHA, approved digest, explicit GO, external marker/readback, queue validation/enqueue/readback, and a journal outside the repo.
