# Add a Roc guest app

Source: `pkgs/id/examples/roc-world/platform.roc`, `pkgs/id/examples/roc-world/ABI.md`, `pkgs/id/crates/id-cli/src/cli/world.rs` and `pkgs/id/apps/tic-tac-toe/app.toml` in https://github.com/developing-today/code/blob/main/pkgs/id/examples/roc-world/platform.roc — provenance only; this chapter is complete without following it.

A guest app is a Roc program compiled to an import-free WebAssembly module and run by the host under the Plaza ABI v1 contract. This chapter covers writing one, checking it offline, and putting it in front of users through the program gallery. The runtime model behind it is [Worlds and the Roc guest runtime](../concepts/worlds.md); the host-side commands are in [World commands: administering and verifying](../reference/cli-worlds-admin.md).

## Where guest apps live

Worked examples live in `pkgs/id/examples/`. Each app is a directory holding its Roc source, a `README.md`, a build script, and — for most apps — a checked-in `.wasm` build of the guest, so the host conformance suites run without a Roc toolchain installed.

| Directory | What it demonstrates |
|---|---|
| `examples/roc-counter/` | The smallest complete world: guests send `inc`, all participants see the count, and `records` mirrors the count to the host. |
| `examples/roc-text-counter/` | The same counter on the legacy plain-text presentation, importing no platform modules. |
| `examples/roc-chat/` | Chat over the capability protocol: `chat.say` and `chat.history` through `RocCap`. |
| `examples/roc-grid/` | The `RocGrid` library: a 4×3 byte grid with bounds-checked reads. |
| `examples/roc-mind-map/` | Stable IDs with duplicate labels, a typed cross-link, refused cycles, preserved positions. |
| `examples/roc-screen/` | The `Screen` and `Key` libraries: a structured screen document instead of text. |
| `examples/lounge/` | The capability system end to end: subscriptions, arrival greetings through `chat.say`, and refusal counting. |
| `examples/territory/` | A complete multiplayer game: at most 8 seated players on a 16×8 grid, adjacency-only claiming, seats that persist across leave, rejoin and restart. |
| `examples/roc-cap-demo/` | A capability-dependent Roc library plus its app, with two compile-must-fail negative tests for the library import gate. |
| `examples/roc-import-policy/` | Not an app: the compile-fixture matrix for the import gate, executed by the `id-caps` integration test `roc_import_policy`. |
| `examples/rust-world/`, `examples/rust-counter/` | Not Roc: the Rust twin SDK (`plaza_sdk`) and its counter example, emitting the same Plaza exports. |
| `apps/tic-tac-toe/` | The one packaged app, outside `examples/`: an `app.toml` plus `program/main.roc`. Packaging is an `app.toml` declaring `[program] kind = "roc-world"` and the `[interfaces.web]` binding where the app takes a web surface. |

## Scaffold a new app

```sh
id world new <DIR>
```

`id world new` is a local command; it contacts no node. It creates `<DIR>` — which must be absent or empty — and writes three files into it:

- `main.roc` — the guest source, implementing the eight entry points below.
- `build.sh` — the build script for the guest module.
- `README.md` — the app's readme.

The `--name <NAME>` flag sets the world name in the scaffold; its default is `world`.

Start from the scaffold, or copy `examples/roc-counter/` when the new app is closer to a counter than to the scaffold. Keep the app's directory self-contained: source, build script, readme, fixtures and the checked-in `.wasm` build live together, as they do in every directory in the table above.

## The platform package: `examples/roc-world`

Every Roc guest builds against the one shared platform package in `examples/roc-world/`:

- `platform.roc` is the platform definition. It exposes exactly six pure-Roc modules to apps: `Screen`, `Key`, `RocJson`, `RocGrid`, `RocCap` and `RocText`, each a `.roc` file beside it in the same directory. A second platform file, `platform-no-caps.roc`, exposes the platform with no capability module; it exists so the import gate's negative tests have a platform under which a capability-dependent library must fail to compile.
- `host/` holds the freestanding Zig host adapter: `host/src/host.zig`, `host/src/native_worker.zig` and the generated `host/src/roc_platform_abi.zig`. `build-host.sh` and `build-native.sh` at the package root build it.
- `ABI.md` in the same directory is the written Plaza ABI v1 contract.
- The app readmes pin the Roc toolchain as `nightly-2026-10-04-130536d` and the host's Zig as `0.16`. The host-side compiler lookup honours the `ID_ROC_BIN` and `ID_ROC_PLATFORM` environment variables and the matching serve and world flags; their defaults and fallbacks are in [Configuration and environment](../reference/configuration.md).

A guest program provides all eight entry points the platform requires. Some may be stubs, but all eight must exist:

| Entry point | Signature, in words | Contract |
|---|---|---|
| `init` | seed → model | The seed is the `U64` the host starts the world with. |
| `update` | model, participant id, event text → model | Participant id `0` is reserved for host events, including capability results. |
| `view` | model → text | Legacy text, or a portable screen document. |
| `layout` | model, width, height → text | Viewport-sized rendering. |
| `records` | model → text | A JSON object projection; the host mirrors it and serves it to readers. |
| `wants` | model → text | Subscriptions and one-shot capability requests, as JSON. |
| `snapshot` | model → text | The whole state as text, for journal checkpoints. |
| `restore` | text → model | The inverse of `snapshot`. |

`update` and `view` are pure. The platform requires no `Task`, and the guest has no host functions to call: all effects go through the capability protocol below.

## The Plaza ABI v1 contract

The compiled module is import-free WebAssembly: no WASI, no filesystem, network, clock or randomness imports. The module exports `memory` plus the `plaza_*` exports defined in `ABI.md`. The required exports are `memory`, `plaza_alloc`, `plaza_free`, `plaza_init`, `plaza_update`, `plaza_view` and `plaza_out_len`. The optional exports are `plaza_abi_version` (returns `1`), `plaza_layout`, `plaza_records`, `plaza_wants`, `plaza_snapshot` and `plaza_restore` — the last two are a pair, both present or both absent — and `plaza_error_ptr` / `plaza_error_len` for diagnostics, at most 4096 bytes.

The host runner is `id_worlds::sandbox::{Sandbox, WorldInstance}` on wasmtime. Fuel and memory are bounded by the world's runtime limits; the limits and their defaults are stated in [World commands: administering and verifying](../reference/cli-worlds-admin.md).

### The capability protocol

Effects are requested, never performed directly. `plaza_wants` — surfaced in Roc as the guest's `wants` entry point — returns a JSON document of the form `{"v":1, "subscribe":[...], "requests":[{"id":"r1","cap":"time.now","args":{}}]}`. The document is at most 64 KiB, with at most 64 subscriptions, at most 64 requests and at most 4 distinct tick intervals. The host answers each request with a participant-`0` `update` event carrying a `cap` result: `ok` with a value, or not `ok` with one of the stable error codes `cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`, `outside_ceiling` and `result_too_large`. Host events are journaled, so a replay feeds the same events back through `update` in the same order.

In Roc, do not build these documents by hand. `RocCap` is the client library: it allocates request ids (`r0`, `r1`, …), tracks which requests are outstanding, and routes result events back to the caller. It provides request builders including `chat_request`, `now_request`, `players_request`, `world_request`, `history_request`, `records_request`, `schedule_at_request` and `schedule_cancel_request`. The provider catalog itself — capability names, argument schemas and bounds — is owned by `id_caps::world_caps`; a request for a capability the world was not granted returns a refusal event, never a silent execution.

## Verify offline: the dev loop

Four commands check a guest without a network, a running serve or a second participant. All four are gated on the `sandbox` feature: in a build without it, the subcommands do not exist. See [Building and testing](building.md) for the feature definitions.

| Command | What it does |
|---|---|
| `id world dev <DIR>` | Runs the fully offline scratch-world loop over a directory: compile, instantiate, feed inputs and fixtures, and print one JSON report. Its flags include `--platform <DIR>`, `--roc <BIN>`, `--seed <N>` (default `1`), repeatable `--input <TEXT>` and `--fixture <FILE>`, `--fuzz <FILE>` with `--fuzz-seed <N>` (default `42`) and `--fuzz-cases <N>` (default `32`), and a `--watch` mode bounded by `--watch-max-cycles <N>` (default `1000`) and `--watch-debounce-ms <N>` (default `150`). |
| `id world doctor` | Checks the authoring environment — the Roc compiler, the platform, a journal, a capability ceiling and an execution profile — and exits non-zero only when a check fails; warnings do not fail it. `--json` prints the report as JSON. |
| `id world replay --module <FILE> --fixture <FILE> --seed <N>` | Replays a recorded fixture against a compiled module. All three flags are required. The verdict line prints first; a record mismatch exits non-zero. `--allow-partial` accepts a partial fixture. |
| `id world verify --module <FILE> --fixture <FILE> --seed <N>` | The double-replay determinism check over the same three required flags: the module is replayed twice and the first divergence exits non-zero. |

`id world measure --module <FILE>` also runs offline, under the same `sandbox` gate: it measures a compiled module's fuel use and prints one JSON object.

A guest is verified when all of the following hold:

1. `just roc-test` passes — the recipe runs the Roc library `expect` tests for `RocJson`, `Screen` and `Key` with the pinned Roc compiler.
2. `id world dev` over the app directory reports no failure, including its fuzz cases.
3. A recorded session fixture replays clean under `id world replay`, and `id world verify` finds no divergence between its two replays.
4. The checked-in `.wasm` build in the app directory is the build of the current source. The host conformance suites load that file; a stale `.wasm` tests the previous version of the app.
5. `id world contract` passes for the shell surface the app presents. Run without a check file, it prints the shell contract; with `--check <FILE>` it fails on any violation.

With toy data, a minimal session looks like this: scaffold with `id world new toy-counter`, write a guest whose `records` returns `{"count":0}` at seed `1`, run `id world dev toy-counter --input inc`, and confirm the report shows the count the toy guest's own rules predict. The example names and inputs here are invented; no real world, token or capability is involved.

## Install, host and the gallery admission path

A compiled guest reaches users by two routes:

- **Direct install on a host.** An operator hosts a world with `id serve --world` and installs the module with `id world install`, or pre-loads it at startup with `serve --world-module <FILE>`. A source-only host (started with `--world-source-only`) accepts Roc source installs only. The operator commands are in [serve and publications](../reference/cli-serve.md) and [World commands: administering and verifying](../reference/cli-worlds-admin.md).
- **The program gallery.** A gallery is a directory an operator serves with `serve --gallery-dir <DIR>` (environment fallback `ID_GALLERY_DIR`). Programs enter it as signed single-file program bundles — the bundle format is owned by `id_worlds::program_bundle`, with export and verification fully offline — and `serve` verifies every bundle file in the gallery directory at startup; a bundle that does not verify is not served. Admission decisions after that are operator actions, recorded on the gallery's public transparency list and performed with the `id gallery` command family (parent flag `--dir <DIR>`, required): `approve`, `pending`, `remove`, `refuse`, `reinstate`, `posture`, `response-hours` and `tag`, each specified in [Gallery, match-audit and walk](../reference/cli-gallery-walk.md). The host-side admission and operator logic lives in `id_worlds::program_gallery`, `id_worlds::program_gallery_admission` and `id_worlds::program_gallery_operator`; the web surface the gallery is read through is in [The web UI: pages and HTTP API](../reference/web-ui.md).

A Roc library that a guest imports goes through a second gate, separate from gallery admission: the library import gate in `id-caps` (`library_manifest`, `library_resolution`, `library_gate`, `library_attach`). A library declares the capabilities it needs in its manifest; resolution and the gate refuse an import whose declared capabilities exceed what the importing world may use. The worked example is `examples/roc-cap-demo/`, whose two negative fixtures must fail compilation, and the fixture matrix in `examples/roc-import-policy/` is executed by the `id-caps` test named in the previous section. A new library-dependent app adds its case to that matrix in the same change.

## The book, in the same change

A new example app is recorded where the book lists the apps: this chapter's table, and [Guest apps and shells as consumers](../library/guests-and-shells.md) where the app consumes a library surface. A change to the platform's exposed modules, the ABI exports or the capability protocol updates this chapter and [Worlds and the Roc guest runtime](../concepts/worlds.md) in the same change. The full contract is [Keeping this book current](keeping-this-book-current.md).
