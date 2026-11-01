# worker-poc — plaza worlds in a Worker isolate (#2271)

PoC for developing-today/code#2271 (from the #2057 design): run one example
world **unmodified** in a Worker isolate (workerd / Durable Object with a
SQLite journal) and prove **journal equality** against the native wasmtime
host (`pkgs/id/src/sandbox.rs`): per-step `(view, records, snapshot)` +
snapshot hash over an identical scripted input sequence, including a forced
cold start mid-sequence.

Pre-registered semantic differences (findings note on #2271, posted before
implementation): no fuel metering in Workers (CPU-time bound instead), and
the hosted loader refuses over-ceiling / `portability=native` manifests up
front where the native host traps at run time. Equality is claimed on state
evolution only, never on execution-budget behaviour.

## Layout

- `src/guest.ts` — `PlazaGuest`: drives only the `plaza_*` exports, with the
  native host's alloc/free and output discipline (see the ABI table in the
  file header). Bytes path (Node) and module path (workerd) both supported.
- `src/manifest.ts` — hosted loader: refuses `portability=native` and
  declared memory over the 64MB hosted ceiling (#2057), naming the reason.
- `src/worker.ts` — one Durable Object per world; its SQLite `journal` table
  is the source of truth. `POST /step` applies + journals (snapshot, view,
  records, hash); `POST /cold-start` drops the in-memory guest so the next
  request restores from the journal via `plaza_restore`.
- `smoke/counter-smoke.mjs` — Node smoke over the prebuilt
  `../examples/roc-counter/counter.wasm`: seed + `inc`×3 with per-step
  view/records/snapshot-hash, a guest-level cold start (restore the step-2
  snapshot into a fresh instance, hash must match the uninterrupted run),
  and both loader-refusal assertions. Run: `npm run smoke`
  (Node ≥ 22, no install needed for the smoke).

## Status vs #2271 acceptance

- [x] Scaffold: TS isolate host instantiating the prebuilt counter guest,
      driving only the `plaza_*` ABI (this change).
- [x] Counter pipeline smoke green on the isolate side (2026-11-01, laptop
      Node 24): seed 0 + `inc`×3 → `count=0..3` in view and records, per-step
      snapshot hashes recorded in the run; guest-level cold start (restore
      step-2 snapshot, apply `inc`) reproduces the step-3 snapshot hash
      `4e07408562…` exactly.
- [ ] Counter sequence compared against the native reference runner
      (native runner over the same scripted sequence via the existing
      sandbox/world_fixture machinery — next increment).
- [ ] tic-tac-toe scripted sequence (0,4,1,4-occupied,9-not-a-move,8,
      2-X-wins,3-after-win) equal on isolate and native hosts per step.
- [ ] Durable Object forced cold start mid-sequence under workerd
      (`wrangler dev`), equality across it (guest-level restore is proven
      by the smoke; DO-level is the next increment).
- [x] Hosted-loader refusals: `portability=native` and >64MB declared
      memory, each with the reason named (asserted in the smoke).
- [x] Findings note with the pre-registered differences posted on #2271.

Nothing here changes the native sandbox, the guest examples, or the plaza
ABI, and the native `WorldManifest` portability field is a separate Rust
follow-up that does not gate this PoC.
