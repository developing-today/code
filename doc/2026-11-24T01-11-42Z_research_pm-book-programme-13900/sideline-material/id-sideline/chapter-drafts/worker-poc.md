# id-worker-poc: the World Durable Object proof of concept

Source: https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/wrangler.toml, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/README.md, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/src/worker.ts, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/src/manifest.ts, https://github.com/developing-today/code/blob/main/pkgs/id/worker-poc/src/guest.ts

`id-worker-poc` is a **completed proof of concept, not a product and not a production path**. This chapter exists so the Worker is covered by its exact name — the name in its `wrangler.toml` and in the `name` field of its `package.json` — and so its result is on the record in one place. [The hosted edition](../concepts/hosted.md) and [The hosted Worker API](hosted-worker.md) already state its status in passing; what follows is the whole of it.

| Item               | Value                                                                                                                                                     |
| ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Worker name        | `id-worker-poc`                                                                                                                                           |
| Source             | `pkgs/id/worker-poc/` — entry point `src/worker.ts`                                                                                                       |
| Compatibility date | `2024-11-01`                                                                                                                                              |
| Tracking issue     | #2271, from the #2057 design                                                                                                                              |
| Status             | Complete: every acceptance box in its README is checked, dated 2026-11-01. Nothing in the package is deployed or run outside its own tests and harnesses. |

## What it was for, and what it proved

The question was narrow: can one example ID world run **unmodified** in a Cloudflare Worker isolate — workerd, one Durable Object per world, a SQLite journal — and evolve its state **identically** to the same world on the native wasmtime host (`pkgs/id/src/sandbox.rs`)?

The answer it proved is yes, on state evolution. The harness compares per-step `(view, records, snapshot)` plus the snapshot hash over identical scripted input sequences, on two guests:

- the counter sequence (seed 0, then three increments), and
- a tic-tac-toe sequence including an occupied-cell move, an out-of-range move, the winning move and a move after the win — final records `{"board":"XXX-O---O","plays":5,"winner":"X"}` on both hosts.

Each sequence also runs with a forced cold start mid-sequence on both hosts, and the Durable Object harness (`compare/do-cold-start.mjs`) drives the real object under `wrangler dev`: a `/cold-start` after step 2 restores step 2 from the SQLite journal, a full `wrangler dev` restart restores it again, and step 3's snapshot hash equals the native reference hash. The README's harness summary is `compare: PASS (13 steps equal)`, `do-cold-start: PASS`.

Two differences were registered **before** implementation, in the findings note on #2271, and equality was never claimed over them:

- Workers have no fuel metering; the isolate host is bounded by CPU time where the native host meters fuel.
- The hosted loader refuses an unhostable manifest **up front, naming the reason**, where the native host traps at run time.

Equality is therefore claimed on state evolution only, never on execution-budget behaviour.

## The surface

Bindings, from `wrangler.toml`:

| Binding / rule    | Value                                                                                                                                                                                  |
| ----------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `WORLD`           | Durable Object namespace → class `WorldObject`. Migration tag `v1-sqlite`, `new_sqlite_classes = ["WorldObject"]`.                                                                     |
| CompiledWasm rule | `type = "CompiledWasm"`, globs `**/*.wasm`, `fallthrough = true` — it compiles the prebuilt guest the entry point imports: the unmodified `pkgs/id/examples/roc-counter/counter.wasm`. |

There are no routes, no `workers.dev` setting, no secrets, no environment variables and no other bindings in the configuration. The front-door `fetch` handler forwards every request to one `WorldObject`, selected by the `?world=` query parameter (default `default`): the object is named by that parameter's value via `idFromName`.

The object's routes, from `src/worker.ts`:

| Method | Path          | Purpose                                                                                                                                                                                                                                                                              |
| ------ | ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| POST   | `/step`       | Apply one step. Body `{participant, event}`: `participant` must be a non-negative integer and `event` a non-empty string, validated before the guest is touched; a malformed body is `400` with an error naming that requirement. On success, `{step, view, records, snapshotHash}`. |
| POST   | `/cold-start` | Drop the in-memory guest, so the next request restores from the journal. Answers `{cold: true, journaledStep}`.                                                                                                                                                                      |
| GET    | `/state`      | `{step, view, records, snapshotHash}` for the current state; `snapshotHash` is `null` when the guest exports no snapshot.                                                                                                                                                            |

Any other method/path combination is a plain-text `404` with body `not found` — and, by the order of the handler, an unmatched request still instantiates (or restores) the guest before answering, because only `/step` and `/cold-start` are matched before the guest is loaded.

All request handling is serialised through a per-object promise chain, lazy guest initialisation is shared through one promise, and a step's update → hash → journal insert runs as one unit, so journal order always matches guest evolution order. A failure after the guest has advanced but before the journal insert commits discards the guest; the next request restores from the journal.

## The journal and the guest

The object's SQLite `journal` table is the source of truth, created in the constructor:

| Column          | Content                                              |
| --------------- | ---------------------------------------------------- |
| `step`          | Integer primary key.                                 |
| `participant`   | The step's participant.                              |
| `event`         | The step's event text.                               |
| `snapshot`      | The guest snapshot bytes (BLOB) after the step.      |
| `view`          | The guest view after the step, as text.              |
| `records`       | The guest records after the step, as text, nullable. |
| `snapshot_hash` | SHA-256 hex of the snapshot.                         |

The schema is unversioned — there is no schema-version constant in this package, unlike the hosted Worker's `UserDO` (see [Data stores and schemas](data-stores.md), which also lists this journal). Restoring takes the latest row's snapshot and hands it to the guest's `plaza_restore`.

The guest driver (`src/guest.ts`, class `PlazaGuest`) drives only the `plaza_*` exports, with the native host's alloc/free and output discipline, and receives no imports. The manifest baked into `src/worker.ts` is `{name: "roc-counter", portability: "portable"}` with seed `0`.

The hosted loader (`src/manifest.ts`) enforces the two up-front refusals, each throwing an error that names the manifest and the reason:

- `portability=native` is not hostable in a Worker isolate;
- a declared memory (`wasm_memory_bytes`) over the hosted ceiling `HOSTED_MEMORY_CEILING_BYTES` — 64 MiB, the #2057 ceiling — is refused.

## Running its proofs

From the package (`package.json` scripts; Node 22 or newer for the smokes, no install needed for them):

| Command                 | What it runs                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| ----------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `npm run smoke`         | The Node smokes: `counter-smoke` (seed + three increments, per-step hashes, a guest-level cold start), `participant-clamp` (the PR #3726 clamp-not-wrap pin for participant IDs above `i32::MAX`), `post-update-failure` (a one-shot post-update view failure leaves only the successful event in the journal and in cold-start restore) and `participant-validation` (the #4074 pin: participant values the native u64 host can never produce — NaN, negative infinity, negative or fractional values — are rejected, not coerced). |
| `npm run compare`       | Regenerates nothing itself: it diffs the isolate runner's per-step lines against the native reference JSONL, which is regenerated from `pkgs/id` with `WORKER_POC_REFERENCE_OUT=worker-poc/compare/native-reference.jsonl cargo test --test worker_poc_reference -- --nocapture`.                                                                                                                                                                                                                                                    |
| `npm run do-cold-start` | The Durable Object cold-start harness under `wrangler dev` described above.                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `npm run typecheck`     | `tsc --noEmit`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `npm run dev`           | `wrangler dev` — a local harness entry point, not a deployment.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |

## Where its ideas landed, and what it is not

- The proof is an input to the hosted edition's design, not its implementation. The package that carries the hosted data plane forward is `pkgs/id/hosted-worker` ([The hosted Worker API](hosted-worker.md)): a different Worker (`id-hosted-worker`), a different object (`UserDO`, one per bound user), a different store (per-user key/value, blob references and a signed-object bridge store) and a local-only scaffold status of its own. No code in `worker-poc` is the hosted Worker, and the hosted Worker does not run worlds through this object's `/step` route. What carried over is the established fact the design stands on — an unmodified `plaza_*` guest evolves identically in an isolate when its journal is the source of truth — plus the loader-refusal shape (refuse up front, name the reason), which the design records treat in [Design: hosted and serverless editions](../design/hosted-editions.md).
- The world-on-Worker question it answered is the #2271 slice of [Worlds and the Roc guest runtime](../concepts/worlds.md); the native sandbox, the guest examples and the plaza ABI were explicitly unchanged by the PoC, and the native `WorldManifest` portability field was recorded in its README as a separate Rust follow-up that did not gate it.
- It is not referenced by any production configuration: no NixOS module, host configuration or workflow deploys or invokes it. Its subfolder probe, `id-d1-journal-poc`, is covered separately in [id-d1-journal-poc: the D1 journal proof of concept](d1-journal-poc.md).

One documentation gap in the package itself: the README's layout bullet for `src/worker.ts` names `POST /step` and `POST /cold-start` but not `GET /state`, which the source's own header comment and handler both implement. The table above follows the source.
