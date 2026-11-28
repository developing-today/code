# Guest apps and shells as consumers

Source: primary sources are [pkgs/id/examples/roc-world/ABI.md](https://github.com/developing-today/code/blob/main/pkgs/id/examples/roc-world/ABI.md), [pkgs/id/examples/roc-world/platform.roc](https://github.com/developing-today/code/blob/main/pkgs/id/examples/roc-world/platform.roc), [pkgs/id/crates/id-worlds/src/sandbox.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-worlds/src/sandbox.rs), [pkgs/id/crates/id-native-shell/src/lib.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-native-shell/src/lib.rs), and [pkgs/id/crates/id-hw-shell/src/lib.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-hw-shell/src/lib.rs). This chapter is complete without following those links; they record provenance only.

This chapter covers the two kinds of consumer that sit at the edge of the library: guest apps, which run inside a world under the sandbox, and shells, which connect to a world from outside and render it. Both consume the public surface only. Guests consume the Plaza platform package and the capability protocol; shells consume the facade crate (see [The id facade crate](facade.md)). The host-side crates are covered in [Worlds and capabilities as a library](worlds-and-caps.md).

## The shared platform package

All Roc guests build against one platform package, `pkgs/id/examples/roc-world/`: `platform.roc` plus a freestanding Zig host adapter (`host/src/host.zig`, `host/src/native_worker.zig`, and the generated `host/src/roc_platform_abi.zig`), pinned to Roc nightly-2026-10-04-130536d per the app READMEs, with Zig 0.16. The platform exposes six pure-Roc modules to apps, declared in `platform.roc` as `exposes [Screen, Key, RocJson, RocGrid, RocCap, RocText]`.

Every app provides the same eight functions (the `requires` block in `platform.roc`); an app with no use for one provides a stub:

- `init : U64 -> model` — the model, seeded from the host.
- `update : model, U64, Str -> model` — the second argument is the participant id; participant 0 is host events.
- `view : model -> Str` — legacy text or a portable screen document.
- `layout : model, U64, U64 -> Str` — viewport-sized rendering.
- `records : model -> Str` — a JSON object projection, mirrored by the host.
- `wants : model -> Str` — subscriptions and one-shot capability requests, as JSON.
- `snapshot : model -> Str` — state as text, for journal checkpoints.
- `restore : Str -> model` — the inverse of `snapshot`.

`update` and `view` are pure. The platform requires no `Task`.

## The Plaza ABI v1

The host/guest contract is the Plaza ABI v1, specified in `pkgs/id/examples/roc-world/ABI.md`. A guest module exports `memory` plus 14 `plaza_*` exports: `plaza_abi_version` (`() -> i32`, returns 1; optional for back-compatibility), `plaza_alloc`, `plaza_free`, `plaza_init` (`(i64 seed) -> i32 model`), `plaza_update` (`(model, participant, event_ptr, event_len) -> model`), `plaza_view`, `plaza_layout` (optional), `plaza_records` (optional), `plaza_wants` (optional), `plaza_snapshot` and `plaza_restore` (both present or neither), `plaza_out_len` (one shared output slot), and `plaza_error_ptr` / `plaza_error_len` (optional diagnostics, at most 4096 bytes). The required set is `memory`, `plaza_alloc`, `plaza_free`, `plaza_init`, `plaza_update`, `plaza_view`, and `plaza_out_len`.

There are no Wasm host functions. Guests compile to import-free Wasm modules: no WASI, and no filesystem, network, clock, or randomness imports. Every effect a guest needs goes through the `wants` capability protocol below. This is the central fact of the guest model, and it is what makes journal replay deterministic: host events are journaled, so replay feeds the same events back through `update`.

One documentation disagreement names the runner's location. `ABI.md` and the Rust example READMEs locate the import-free guest runner at `pkgs/id/src/sandbox.rs`. That file does not exist. The runner is `crates/id-worlds/src/sandbox.rs` — `id_worlds::sandbox::{Sandbox, WorldInstance}`, built on wasmtime — surfaced to consumers as `id::sandbox` (disagreement LIB-7). The code location is the one to use.

## The wants capability protocol

`plaza_wants` returns a JSON document of the form `{"v":1, "subscribe":[…], "requests":[{"id":"r1","cap":"time.now","args":{…}}]}`. The bounds are literal: at most 64 KiB per document, at most 64 subscriptions, at most 64 requests, and at most 4 distinct tick intervals. The host answers each request with a participant-0 `update` event of the form `{"cap":"result","id":…,"ok":true,"value":…}`, or with `"ok":false` and one of the stable error codes `cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`, `outside_ceiling`, `result_too_large`. Grants and ceilings are enforced host-side; a known but ungranted request returns a refusal event, never silent execution.

The runtime provider set named in `ABI.md` is `time.now`, `time.tick` (subscribed as `time.tick:<ms>`), `random.u64`, `players` (subscription), `players.list`, `chat.say`, `chat.history`, `records.read`, `schedule.at`, `schedule.cancel`, and `world.info`. The wider catalog registered in `id_caps::world_caps` adds `kv.get`, `kv.set`, `kv.delete`, `blob.put`, `blob.get`, `blob.delete`, `http.fetch`, `notify`, `device.info`, `form.present`, `form.result`, `form.cancel`, `loc.position`, and `loc.watch` (see [Worlds and capabilities as a library](worlds-and-caps.md)).

The Roc client library is `RocCap`: a session issuing request ids `r0`, `r1`, and so on, with `issue`, `attach`, `requests`, and `route`, and builders `chat_request`, `now_request`, `players_request`, `world_request`, `history_request`, `records_request`, `schedule_at_request`, and `schedule_cancel_request`.

## The ten Roc guest apps

Checked-in `.wasm` artefacts sit beside most apps, so the host conformance suites run without a Roc toolchain. `pkgs/id/apps/tic-tac-toe/` is the sole packaged app; the other nine live in `pkgs/id/examples/`.

| App              | Location                     | What it does                                                                                                                                                                                                                                                                                                                                        |
| ---------------- | ---------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| tic-tac-toe      | `apps/tic-tac-toe/`          | Packaged two-player tic-tac-toe. Its `app.toml` declares `[program] kind = "roc-world"` and the guest is `program/main.roc`; the directory contains no Rust. Digit `0`–`8` moves, ply parity picks X or O, and `records` publishes `{"board","plays","winner"}`. The install path is serve-time: `id serve --world-module …` or `id world install`. |
| lounge           | `examples/lounge/`           | Capability-system demo: subscribes to `players` and `time.tick:5000`, greets arrivals through `chat.say` requests with refusals counted as `denied=N`, and polls `time.now`.                                                                                                                                                                        |
| roc-counter      | `examples/roc-counter/`      | Tiny pure counter world: guests send `inc`, all participants see `count=N`; demonstrates `records` mirroring and the `Screen` presentation.                                                                                                                                                                                                         |
| roc-text-counter | `examples/roc-text-counter/` | The no-`Screen` legacy-text twin of roc-counter: imports no platform modules and keeps the plain-text presentation path covered (#3181).                                                                                                                                                                                                            |
| roc-chat         | `examples/roc-chat/`         | Minimal chat client on `RocCap` with `chat.say` and `chat.history`; typing `hist` shows the newest 8 lines.                                                                                                                                                                                                                                         |
| roc-grid         | `examples/roc-grid/`         | `RocGrid` demo: a 4×3 byte grid where `set` claims the centre cell; bounds-checked reads with a fallback.                                                                                                                                                                                                                                           |
| roc-mind-map     | `examples/roc-mind-map/`     | Mind map for `mind-map/same-label-distinct-nodes` (#13609): nodes distinguished by stable ID with duplicate `Archive` labels, a typed `related` cross-link through reparent and rename, refused single-history cycles, preserved positions; `records` and `snapshot` publish the same domain document.                                              |
| roc-screen       | `examples/roc-screen/`       | `Screen` and `Key` demo: a bordered-box counter screen document, `+` increments; presenters (SSH ANSI, browser DOM) render the structured grid.                                                                                                                                                                                                     |
| territory        | `examples/territory/`        | Multiplayer territory game: at most 8 seated players on a 16×8 grid, adjacency-only claiming, seats persist across leave, rejoin, and restart.                                                                                                                                                                                                      |
| roc-cap-demo     | `examples/roc-cap-demo/`     | Library-import worked example (#5957): a capability-dependent Roc library (`lib/Greeter.roc` over `records.read`) plus its app, with two compile-must-fail negative tests — a missing capability on `platform-no-caps.roc`, and a wrong `RocCap` signature.                                                                                         |

Two neighbours in `examples/` are not apps. `examples/roc-world/` is the shared platform package itself, and `examples/roc-import-policy/` is a compile-fixture matrix (`fixtures.json`) for the import gate (#5994, #5956), executed by the `id-caps` integration test.

## The Rust twins

The Plaza ABI has a Rust guest SDK beside the Roc platform: `examples/rust-world/` provides the `plaza_sdk::World` trait and the `plaza_guest!` macro, which emit the same 14 exports, with modules `screen`, `json`, `grid`, `cap`, and `text` mirroring the Roc libraries. `examples/rust-counter/` is its counter example; the checked-in `counter.wasm` is the host conformance fixture. A Rust guest and a Roc guest are interchangeable to the host: the runner checks exports and the `wants` protocol, not the source language. Writing a first guest is covered in [Add a Roc guest app](../development/add-a-guest.md).

One Rust file in `examples/` is not a guest at all: `examples/session_checkpoint_poc.rs` is a self-contained throwaway proof of concept (#2241) that deliberately uses no `id::` API. The first-app proof of concept (`examples/first-app-poc/`) is a README only; its executable artefact is the integration test `pkgs/id/tests/first_app_poc.rs`, by design.

## Shells as consumers of the facade

Both shells are workspace members but not default members, and both production-depend on the facade with `default-features = false, features = ["world"]`. ARCHITECTURE.md lists them as workspace members with no dependents, which is true and incomplete: they are consumers of the public surface — in fact the only two production consumers of the facade outside the workspace's internal graph (disagreement LIB-12). Each also dev-depends on `id-worlds` with the `sandbox` feature for tests.

### id-native-shell

Purpose (from its `Cargo.toml`): "Native reference shell for id (F13.6, design #788): a Bevy client speaking the F13.1 shell contract". The contract itself is `pkgs/id/contract/shell-contract.v1.json`, owned host-side by `id_worlds::shell_contract`.

The shell is a client only. It connects to a world host over the existing session protocol, renders the host's `view` frames, and emits only the client frames the contract already names: `input`, `paste`, `resize`, `chat`. It adds no frame kinds, needs no host change, and no shell frame depends on the guest ABI.

Public modules (7): `audio`, `events`, `form`, `providers`, `render`, `scene`, `widget`; a private `session` module's surface is re-exported at the crate root, as is `providers`. The library surface includes the frame builders `input_frame`, `input_frame_with_revision`, `paste_frame`, `resize_frame`, `chat_frame`, `agent_join_frame`, `screen_subscribe_frame`; the view fallback `present(data, surface)`; `ShellConfig::new(world)`; `FrameError`; and `session::run_session`, the live `WorldClient` session loop feeding `render::PresentedView`. The binary is `src/main.rs`: it parses `--node` and `--capability`, builds a Bevy `App` with `DefaultPlugins` plus the shell's render, scene, audio, and widget plugins, and — when a node is given — runs a `SessionBridge` resource.

The session connects through the facade, not through `id-worlds` directly: the `connect` helper above `main` calls `id::world_net::WorldClient::connect(&endpoint, addr)`. That call is the shell-as-consumer pattern in one line. The shell's command-line surface — a hand-rolled parser with no clap, no `--help`, and exit code 2 on argument errors — is documented in [id-native-shell](../reference/native-shell.md).

Renderer coverage follows design #788 steps 1 and 2, both in the code: step 1 delivered the isolated package, identity and connection bootstrap, contract frame builders, the view fallback, and a transcript fixture checked against the machine-readable contract; step 2 added the window and render plugin set and the live session loop. `render` renders validated surfaces as styled Bevy UI; `scene` renders validated `scene/1` surfaces as 3D entities via bevy_pbr; `audio` turns validated `audio/1` surfaces into scheduled cue entities — presentation state only, since this feature set carries no audio output backend; `widget` places validated `widget/1` surfaces as control entities. A shell without a renderer for a surface keeps that surface's text projection; the fallback rule is uniform across scene, audio, and widget. Bevy is pinned at 0.19 with a hand-picked feature set (`bevy_core_pipeline`, `bevy_pbr`, `bevy_render`, `bevy_text`, `bevy_ui`, `bevy_window`, `bevy_winit`, `default_font`, with x11 and wayland gated to Linux targets). `tests/live_session.rs` proves the durable-outcome half of the parity criterion against a live host.

### id-hw-shell

Purpose (from its `Cargo.toml`): "ESP32-class hardware shell spike for id (more-shells v1 slice 3, #10929): the presence + journal-head + confirm/act subset of the F13.1 shell contract". The design source is `doc/2026-11-10T20-40-00Z_design_more_shells_v1` §3.

The whole crate is one `src/lib.rs` of 996 lines with 0 public modules. Its API: `CONTRACT_NAME = "shell"` and `CONTRACT_VERSION = 1`; `capability_statement() -> serde_json::Value`; `HwShell` (`new()`, `head() -> u64`, `presence() -> &BTreeSet<String>`, `diverged()`, `last_status()`, `choice() -> Option<PresentedChoice>`, and the fold/update entry points returning `HwUpdate`); `PresentedChoice` and `SelectableField` (id, label, required, options, confirm); and `HwError`.

The subset is the deliverable. The shell implements exactly three parts of the contract: presence (a wake-time read of the snapshot's participant list, with journal joined and left events folded on top), journal-head state in compressed form (the newest durable sequence folded, nothing else), and confirm/act (answering a presented form choice with a `form_result` frame: submit or cancel, choice selections, yes/no). Rich rendering, free text, and world hosting are excluded by name in `capability_statement()`, and the code enforces the exclusions: the only client frames the crate can build are `join` and `form_result`, and a form requiring free text is refused with `HwError::FreeTextExcluded`. Every built frame is checked against the embedded F13.1 contract before return, and every folded frame is checked on the way in.

The crate is transport-free by design: the frame builders and the fold are pure, so the same code runs over a real iroh session on a device and over recorded frames in tests. Its power posture is that no live session is held open — the device wakes, joins, folds to the journal head, acts if a choice is presented, and disconnects. If a resume replay does not continue contiguously from the recorded head, continuity is unprovable and the shell discards its folded presence and waits for a fresh snapshot. It is a spike, complete at its stated subset, not a port in progress.
