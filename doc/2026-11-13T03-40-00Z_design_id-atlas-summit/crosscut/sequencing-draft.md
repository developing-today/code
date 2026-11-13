# Cross-cutting sequencing draft for #9112

The machine-readable per-component map in [`sequencing.json`](sequencing.json) is authoritative. It covers all 38 design components (#9110–#9111 and #9113–#9148) and #9112 as the living integration owner: every issue has explicit research, design, and implementation slots; an implementation wave; hard dependency IDs; kind; reason; and (for later items) a review date. Research/design readiness can precede a dependent implementation output. [`research-slices.json`](research-slices.json) holds earned standalone crosscut research records; `implementation-slices.json` holds granular build/test slices. `verify_crosscut.py` checks inventory coverage, acyclicity, dependency-to-wave ordering, each later deferral's reason/review date, slice file collisions, and config invariants/examples.

## Readiness waves

Waves describe **implementation readiness**. A hard edge names the minimum output/interface needed, not completion of an entire sibling program; research and design have separately recorded slots and can proceed in parallel while a prerequisite interface is being settled. The map uses these layers:

| Wave | Readiness |
|---|---|
| 0 | Living #9112 integration plus primitive boundary, identity profiles, namespace/address cores, triple representation, and benchmark harness. |
| 1 | Direct consumers of wave-0 contracts: sidecars, URI, shorthand, graph/attribute representation, benchmark-informed choices, acceptance gate, and duplicate/request identity. |
| 2 | Location index, derived address families, config integration, GC, conflict reports, and routing validation. |
| 3 | Social model after versioning; moderation after GC; identity/delegation; search after location contract; enabled fan-out after duplicate safety; signed worlds/evidence extensions. |
| 5 | Review-dated horizon items: signed worlds, recommendation runtime/objects, CRDT research/adoption, advanced takeover evidence. |
| 6 | Advanced reputation synthesis after comparable evidence semantics. |

No `wave 4` is currently needed; gaps in wave numbers are labels for ordering, not missing work. #9145's stable request/dedup identity is a hard prerequisite to enabling #9144 fan-out; protocol design can proceed concurrently against that contract, but fan-out release cannot. #9146's routing-state research/design needs authenticated peer updates but not an already-implemented fan-out. #9148's advanced takeover work is later; minimum key disable/revocation and replay-window controls are V2 work in #9141.

## Hard prerequisites versus preferences

Only `hard_dependencies` in the JSON are DAG edges. These name minimum outputs, not whole-program completion. For example, #9144 waits for #9145 request identity, whereas #9146 does not wait for #9144 runtime. Benchmark work informs #9134–#9136 but does not block the independent primitive contract. Sibling-owned SSO (#9077), tenancy (#9083), content plane (#9084), hosted transport (#9085), Atlas embedder (#9092), and VFS (#9100) decisions are consumed by interface; do not restate or settle them here. The #9084 reference-vs-copy decision remains open with that owner.

Soft preferences remain in the component issue `reason` and the design: secure ingest before remote exposure; conflict reports before merge adoption; benchmark evidence before cache/placement choices; triple encoding before graph-language selection. CRDT research can start early and does not block V2 conflict-first sync.

## Deferred horizon

Every later component in `sequencing.json` has its own `review_date` of 2027-02-12 plus a component-specific deferral reason. The date is a review checkpoint, not a silent drop or automatic start. Earlier triggers are recorded per component where applicable. The Atlas `content_hash` producer contract and legacy artifact signature-preimage compatibility work are explicit V2 slices because identity interoperability and safe migration cannot be deferred behind general long-tail breadth.

## Verification

Shared Rust roots have two **serial** owners, never concurrent writers. `9110-02` owns phase 1: common crate-root/Cargo interfaces and private module-root skeletons, with no declarations of absent leaf modules. Consumer slices own only distinct leaf modules and tests; tests use explicit direct-path imports while leaves are not publicly wired. After required V2 leaves merge, `9110-09` hard-depends on them and owns phase 2: the sole public module/export wiring and cross-runtime gate. Every phase-1/phase-2 collision is declared by exact path and enforced by a hard edge. `9110-R1` and `9110-11` own distinct research and Rust test files; no consumer shares a root file.

Run `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/crosscut/verify_crosscut.py`. The stdlib verifier checks the per-issue DAG, per-slice DAG, owned-file collision graph, config override/security examples, and emits the component/research/implementation-slice counts. Update the JSON and rerun the verifier whenever a sibling design changes the minimum prerequisites.
