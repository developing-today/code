# Survey dump — id-worker-poc (`pkgs/id/worker-poc`)

## wrangler.toml (verbatim)

```toml
name = "id-worker-poc"
main = "src/worker.ts"
compatibility_date = "2024-11-01"

# The prebuilt, unmodified example guest, compiled by wrangler's CompiledWasm
# rule and imported in src/worker.ts.
[[rules]]
type = "CompiledWasm"
globs = ["**/*.wasm"]
fallthrough = true

[[durable_objects.bindings]]
name = "WORLD"
class_name = "WorldObject"

[[migrations]]
tag = "v1-sqlite"
new_sqlite_classes = ["WorldObject"]
```

No routes, no workers_dev setting, no vars, no secrets, no other bindings.

## package.json (verbatim)

```json
{
  "description": "PoC (#2271): plaza_* example worlds running unmodified in a Worker isolate, journal equality vs the native wasmtime host.",
  "devDependencies": {
    "typescript": "^5.6.0",
    "wrangler": "^4.0.0"
  },
  "name": "id-worker-poc",
  "private": true,
  "scripts": {
    "dev": "wrangler dev",
    "smoke": "node smoke/counter-smoke.mjs && node smoke/participant-clamp.mjs && node smoke/post-update-failure.mjs && node smoke/participant-validation.mjs",
    "typecheck": "tsc --noEmit",
    "compare": "node compare/compare.mjs",
    "do-cold-start": "node compare/do-cold-start.mjs",
    "participant-clamp": "node smoke/participant-clamp.mjs",
    "participant-validation": "node smoke/participant-validation.mjs",
    "post-update-failure": "node smoke/post-update-failure.mjs"
  },
  "type": "module"
}
```

## Tree (find, excluding node_modules)

`wrangler.toml`, `README.md`, `package.json`, `tsconfig.json`, `.gitignore`,
`src/worker.ts`, `src/guest.ts`, `src/manifest.ts`,
`compare/isolate-runner.mjs`, `compare/compare.mjs`, `compare/do-cold-start.mjs`,
`compare/native-reference.jsonl`,
`smoke/counter-smoke.mjs`, `smoke/compiled-wasm-loader.mjs`,
`smoke/post-update-failure.mjs`, `smoke/participant-validation.mjs`,
`smoke/participant-clamp.mjs`, and the `d1-journal-poc/` subfolder (separate
survey file).

## README.md — the acceptance record (read in full)

- Title: "worker-poc — plaza worlds in a Worker isolate (#2271)".
- Purpose: PoC for code#2271 (from the #2057 design): run one example world
  unmodified in a Worker isolate (workerd / Durable Object with SQLite
  journal) and prove journal equality against the native wasmtime host
  (`pkgs/id/src/sandbox.rs`): per-step `(view, records, snapshot)` + snapshot
  hash over an identical scripted input sequence, including a forced cold
  start mid-sequence.
- Pre-registered semantic differences (findings note on #2271, posted before
  implementation): no fuel metering in Workers (CPU-time bound instead);
  hosted loader refuses over-ceiling / `portability=native` manifests up
  front where the native host traps at run time. Equality claimed on state
  evolution only, never execution-budget behaviour.
- Layout bullets: `src/guest.ts` (PlazaGuest, plaza_* exports only, native
  alloc/free + output discipline; bytes path Node and module path workerd);
  `src/manifest.ts` (hosted loader refusals); `src/worker.ts` (one DO per
  world, SQLite journal table is source of truth; POST /step applies +
  journals; POST /cold-start drops in-memory guest so next request restores
  via plaza_restore); `compare/` harness; `smoke/counter-smoke.mjs`;
  `smoke/post-update-failure.mjs`; `smoke/participant-validation.mjs` (#4074
  pin); `smoke/participant-clamp.mjs` (PR #3726 pin).
  NOTE: the layout bullet for worker.ts does NOT mention GET /state — see
  disagreements POC-1.
- Compare regeneration recipe (verbatim from README):
  `cd pkgs/id && WORKER_POC_REFERENCE_OUT=worker-poc/compare/native-reference.jsonl cargo test --test worker_poc_reference -- --nocapture`
  then `cd worker-poc && npm run compare`.
- Acceptance checklist — ALL boxes checked, dated 2026-11-01:
  - Scaffold: TS isolate host instantiating prebuilt counter guest, plaza_* ABI only.
  - Counter pipeline smoke green on isolate side (2026-11-01, laptop Node 24):
    seed 0 + inc×3 → count=0..3 in view and records, per-step snapshot hashes;
    guest-level cold start (restore step-2 snapshot, apply inc) reproduces
    step-3 snapshot hash `4e07408562…` exactly.
  - DO host hardened per Codex review on the PR (2026-11-01): SQLite BLOB
    snapshots normalized to Uint8Array before plaza_restore; all request
    handling serialized (shared lazy-init promise; update → hash → journal
    insert is one unit); step inputs validated before any guest mutation;
    guest discarded on any post-update failure before journal commit; guest
    output pointers normalized unsigned; wrangler declared in
    nix-common.nix. Smoke PASS + tsc clean after fixes.
  - Counter sequence compared against native reference runner (2026-11-01):
    all steps equal in (view, records, snapshot sha256).
  - Tic-tac-toe scripted sequence (0,4,1,4-occupied,9-not-a-move,8,2-X-wins,
    3-after-win) equal on isolate and native hosts per step (2026-11-01):
    final records `{"board":"XXX-O---O","plays":5,"winner":"X"}` on both;
    each host asserts its own forced cold start mid-sequence (native after
    step 5, isolate after step 5) changes nothing.
  - DO forced cold start mid-sequence under workerd (2026-11-01):
    compare/do-cold-start.mjs drives the counter through the real DO host —
    /cold-start after step 2 restores step 2 from the SQLite journal, a full
    wrangler dev restart (fresh DO instance) restores it again, step 3's
    snapshot hash equals the native reference hash. Full harness run:
    `compare: PASS (13 steps equal)`, `do-cold-start: PASS`.
  - Hosted-loader refusals: portability=native and >64MB declared memory,
    each with the reason named (asserted in the smoke).
  - Findings note with the pre-registered differences posted on #2271.
- Closing note: "Nothing here changes the native sandbox, the guest examples,
  or the plaza ABI, and the native WorldManifest portability field is a
  separate Rust follow-up that does not gate this PoC."

## src/worker.ts — the Durable Object host (read in full)

Header comment (paraphrase-tight): Worker isolate host for plaza_* worlds
(#2271 PoC). One Durable Object per world. The DO's SQLite storage is the
journal and source of truth: every applied event is journaled with the
resulting snapshot, so a forced cold start restores from the latest journaled
snapshot via plaza_restore and state evolution continues identically. The
guest module is imported as a compiled Wasm binding (COUNTER_WASM) — the
prebuilt, unmodified `pkgs/id/examples/roc-counter/counter.wasm`.
Routes listed in the header: POST /step, GET /state, POST /cold-start.
Concurrency note: DOs interleave at every await, so all request handling is
serialized through a per-object promise chain; lazy init shared through one
promise; a /step runs update → hash → journal-insert as one serialized unit.

Code facts:

- Imports `PlazaGuest` from `./guest.ts`, `loadHostedModule` +
  `HostedManifest` type from `./manifest.ts`, and COUNTER_WASM from
  `../../examples/roc-counter/counter.wasm` (with a @ts-expect-error comment:
  wrangler supplies a WebAssembly.Module).
- `MANIFEST = { name: "roc-counter", portability: "portable" }`, `SEED = 0`.
- Constructor: `CREATE TABLE IF NOT EXISTS journal (step INTEGER PRIMARY
KEY, participant INTEGER NOT NULL, event TEXT NOT NULL, snapshot BLOB NOT
NULL, view TEXT NOT NULL, records TEXT, snapshot_hash TEXT NOT NULL)`.
  Unversioned schema — no version constant.
- `serialized(fn)`: promise-chain queue; runs fn after all previously queued
  requests, on both fulfilment and rejection paths.
- `ensureGuest()`: if guest cached, return it; else shared promise: read
  latest journal row (`SELECT step, snapshot FROM journal ORDER BY step DESC
LIMIT 1`), `loadHostedModule(MANIFEST, COUNTER_WASM, SEED)`, if a row
  exists `guest.restore(asBytes(rows[0].snapshot))` and set `this.step`;
  a failed init clears the cached promise so it does not poison later
  requests. `asBytes` accepts Uint8Array/ArrayBuffer, throws otherwise
  ("journal snapshot is not a byte buffer") — workerd returns ArrayBuffer,
  the Node sqlite driver returns Uint8Array.
- `handle(req)`:
  - POST /cold-start → dropGuest(); answer `{cold: true, journaledStep}`
    (latest journaled step or 0).
  - POST /step → parse JSON body (null on failure); validate
    `participant` is a number, integer, ≥ 0 and `event` a non-empty string,
    else 400 `{error: "step requires a non-negative integer participant and
a non-empty string event"}` — validated BEFORE touching the guest.
    Then ensureGuest, nextStep = step+1; in try: guest.update(participant,
    encoded event); view = guest.view(); records = guest.records();
    snapshot = guest.snapshot() (throw if none: "guest exports no snapshot;
    journaling requires the snapshot/restore pair"); snapshotHash =
    sha256Hex(snapshot); INSERT the journal row. On any error: dropGuest()
    and rethrow (a failure before journal commit may leave the guest
    advanced without a matching row; next request restores from journal).
    On success: step = nextStep; answer `{step, view, records,
snapshotHash}` (records decoded or null).
  - Otherwise: `const guest = await this.ensureGuest()` runs FIRST (so an
    unmatched request still instantiates/restores the guest — survey
    observation recorded in the chapter); then GET /state → `{step, view,
records, snapshotHash}` with snapshotHash null if the guest exports no
    snapshot; anything else → plain `new Response("not found", {status: 404})`
    (plain text, not JSON).
- Front door (default export): `name = url.searchParams.get("world") ??
"default"`; `env.WORLD.get(env.WORLD.idFromName(name))`; forward the
  request unchanged.

## src/manifest.ts — hosted loader (read in full)

- Header: pre-registered difference — hosted loader refuses up front, naming
  the reason; native host traps at run time. (#2271, #2057.)
- `HOSTED_MEMORY_CEILING_BYTES = 64 * 1024 * 1024` (64MB, #2057 ceiling).
- `HostedManifest { name: string; portability?: "portable" | "native";
wasm_memory_bytes?: number }`.
- `assertHostable`: portability === "native" → throw
  `manifest ${name}: portability=native is not hostable in a Worker isolate`;
  wasm_memory_bytes > ceiling → throw naming declared bytes and the ceiling.
- `loadHostedGuest(manifest, bytes, seed, limits = DEFAULT_LIMITS)` and
  `loadHostedModule(manifest, module, seed, limits)` both assertHostable
  first, then instantiate via PlazaGuest.

## src/guest.ts — PlazaGuest (header + interface read)

- Header: TypeScript host for plaza_* ABI guests; `pkgs/id/src/sandbox.rs`
  is the native reference. The guest receives NO imports; the host drives
  only the plaza_* exports and keeps the model pointer inside guest memory,
  exactly like the native WorldInstance.
- ABI table (from the file header, citing sandbox.rs:3-7, 199-235):
  plaza_init(i64) -> i32 (model ptr); plaza_update(i32,i32,i32,i32) -> i32
  (model, participant, event ptr, event len); plaza_view(i32) -> i32 (output
  ptr, length via plaza_out_len()); plaza_records(i32) -> i32 (optional);
  plaza_snapshot(i32) -> i32 (optional, paired with restore);
  plaza_restore(i32,i32) -> i32 (snapshot ptr, len) -> model;
  plaza_alloc(i32) -> i32 / plaza_free(i32,i32). Input discipline mirrors
  copy_input/free_input: allocate max(len,1), pass logical length, free with
  allocation length.
- `GuestLimits { messageBytes: number }`; `DEFAULT_LIMITS = { messageBytes:
1024 * 1024 }`.
- `PlazaGuest.instantiate(bytes, seed, limits)` (compile + fromModule) and
  `PlazaGuest.fromModule(module, seed, limits)` (the workerd path: wrangler's
  CompiledWasm rule hands a module, not bytes).

## Existing book mentions (grep, pre-PR)

- concepts/hosted.md §"worker-poc: a completed proof, not a service";
  reference/hosted-worker.md §"worker-poc: a completed proof, not live"
  (routes table, journal, loader refusals — accurate summary treatment);
  concepts/architecture.md; getting-started/shape.md; design/worlds-runtime.md;
  design/hosted-editions.md; development/layout.md (tree row);
  development/release-and-rollout.md; operations/services.md (one paragraph);
  reference/data-stores.md (journal row in the store table).
- NONE of these used the exact Worker name `id-worker-poc`, and there was no
  SUMMARY chapter for it — census GAP confirmed as a name/coverage gap, not
  a total absence of prose.
