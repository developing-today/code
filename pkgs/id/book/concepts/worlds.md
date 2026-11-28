# Worlds and the Roc guest runtime

## What a world is

A world is a hosted, multiplayer program instance. One guest program, one journal of inputs, one current state, and any number of connected participants, each identified inside the program by a participant ID. Participant 0 is reserved: it is the host itself, and host events enter the program under participant ID 0.

A world has a name. The default world name is `lobby`. World names use the character set `a-z 0-9 - _`. A hosted world's files live in `.id-worlds/<NAME>/` under the serve data directory. A single serve process hosts many worlds under operator limits set at startup: `--world-max-open` defaults to `256`, `--world-max-sessions` defaults to `1024`, and `--world-idle-secs` defaults to `600`, with `0` keeping worlds open.

The shipped guest programs show the range of the model. The packaged app is tic-tac-toe (`pkgs/id/apps/tic-tac-toe/`, `app.toml` declaring `[program] kind = "roc-world"`, guest in `program/main.roc`): two players, digit inputs `0` to `8` as moves, ply parity picking X or O, and a records projection publishing `{"board","plays","winner"}`. The examples add a chat client, a counter, a 4×3 grid demo, a screen demo and a territory game for up to 8 seated players on a 16×8 grid whose seats persist across leave, rejoin and restart. Nine Roc guest apps ship as examples or packages, plus the shared platform package and a Rust twin SDK (library survey §5.1).

## The guest program shape

Every guest provides eight functions. The platform `requires` block in `examples/roc-world/platform.roc` fixes the signatures:

- `init : U64 -> model` — build the initial model from the host seed.
- `update : model, U64, Str -> model` — apply one event. The second argument is the participant ID; `0` means a host event.
- `view : model -> Str` — render: legacy text, or a portable screen document.
- `layout : model, U64, U64 -> Str` — viewport-sized rendering.
- `records : model -> Str` — a JSON object projection of the model, mirrored by the host.
- `wants : model -> Str` — subscriptions and one-shot capability requests, as JSON.
- `snapshot : model -> Str` — the state as text, for journal checkpoints.
- `restore : Str -> model` — the inverse of `snapshot`.

`update` and `view` are pure. The platform requires no `Task`. An app provides all eight; an app that does not use a function provides it as a stub. The platform exposes six pure-Roc modules to apps: `Screen`, `Key`, `RocJson`, `RocGrid`, `RocCap` and `RocText`.

## Plaza ABI v1: the host/guest boundary

Guests compile to WebAssembly, and the contract between host and guest is Plaza ABI v1, documented in `examples/roc-world/ABI.md`. The module exports `memory` plus 14 `plaza_*` exports:

`plaza_abi_version`, `plaza_alloc`, `plaza_free`, `plaza_init`, `plaza_update`, `plaza_view`, `plaza_layout`, `plaza_records`, `plaza_wants`, `plaza_snapshot`, `plaza_restore`, `plaza_out_len`, `plaza_error_ptr`, `plaza_error_len`.

Required exports are `memory`, `plaza_alloc`, `plaza_free`, `plaza_init`, `plaza_update`, `plaza_view` and `plaza_out_len`. `plaza_abi_version` takes no arguments and returns `1`; it is optional for backward compatibility. `plaza_layout`, `plaza_records` and `plaza_wants` are optional. `plaza_snapshot` and `plaza_restore` are both present or both absent. `plaza_error_ptr` and `plaza_error_len` are optional diagnostics, capped at 4096 bytes. Output passes through one shared slot whose length `plaza_out_len` reports. The Rust twin SDK in `examples/rust-world/` emits the same 14 exports from a `plaza_sdk::World` trait and a `plaza_guest!` macro.

There are no Wasm host functions. Guests are import-free Wasm modules: no WASI, and no filesystem, network, clock or randomness imports. Every effect a guest needs goes through the capability protocol below. The host runner is `id_worlds::sandbox::{Sandbox, WorldInstance}` on wasmtime: `Sandbox::compile(wasm, SandboxLimits)` compiles a module, `Sandbox::instantiate(seed)` creates an instance, and the instance exposes `update`, `view`, `layout_at`, `records`, `wants`, `snapshot` and `restore`. Disagreement LIB-7: `ABI.md` and the Rust example READMEs locate this runner at `src/sandbox.rs`; that file does not exist. The runner is `crates/id-worlds/src/sandbox.rs`, surfaced as `id::sandbox`.

Execution limits are literal serve settings, flattened into `serve` as `WorldRuntimeArgs`: `--world-fuel` defaults to `10000000`, `--world-memory-mib` to `16`, `--world-message-kib` to `1024`, and `--world-total-memory-mib` to `64`. The native (ELF) tier has its own limits: `--world-native-memory-mib` defaults to `256`, `--world-native-cpu-secs` to `10` and `--world-native-deadline-ms` to `10000`. Per-world profiles persist through `id world profile`; the profile flags and their aliases are in [World commands: administering and verifying](../reference/cli-worlds-admin.md).

## The wants capability protocol

`plaza_wants` returns a JSON document: `{"v":1, "subscribe":[…], "requests":[{"id":"r1","cap":"time.now","args":{…}}]}`. The document is capped at 64 KiB, with at most 64 subscriptions, at most 64 requests and at most 4 distinct tick intervals.

The host answers each request with a participant-0 `update` event: `{"cap":"result","id":…, "ok":true, "value":…}` on success. On failure the event carries `"ok":false` and one stable error code: `cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`, `outside_ceiling` or `result_too_large`. A request for a known capability the world does not hold returns a refusal event. The host never executes an ungranted request silently. Grants and ceilings are enforced host-side; the model is in [Capabilities, ceilings and providers](capabilities.md).

`RocCap` is the guest-side client library for this protocol. It keeps a session with request IDs `r0, r1, …` and provides `issue`, `attach`, `requests` and `route`, plus request builders: `chat_request` (`chat.say`), `now_request` (`time.now`), `players_request` (`players.list`), `world_request` (`world.info`), `history_request` (`chat.history`), `records_request` (`records.read`), `schedule_at_request` and `schedule_cancel_request`.

Because host answers enter as journaled events, replay is deterministic: the journal records the host event, and replay feeds that recorded event back through `update` instead of asking the provider again. A guest that asked for the time receives, on replay, the time the host recorded.

## Hosting and clients: the F13.8 boundary

`id serve` is the only command that hosts a world. Its long help states the boundary directly: this is the only command that hosts or serves a world. Hosting requires `--world` and `--world-admin-token`. A module is supplied by `--world-module <FILE>` (a precompiled Wasm module in the Roc ABI), by Roc source installs (restricted to source-only under `--world-source-only`, from env `ID_WORLD_SOURCE_ONLY`), or by later client installs. Serving a world over SSH takes `--world-ssh-port`, bound to `--bind`, and exists only in builds with the `ssh` feature.

Every `id world` subcommand is a client. `join`, `records`, `caps`, `install`, `compile`, `mirror`, `download`, `replica` and the rest connect to a host; none of them hosts. Two commands state the distinction in their own docs. `id world mirror` runs a local replica endpoint that accepts sync connections; it is not a world host. `id world dev` runs a fully offline scratch-world loop on the local machine, with no host and no network. The client commands are in [World commands: joining and playing](../reference/cli-worlds.md) and [World commands: administering and verifying](../reference/cli-worlds-admin.md).

## The shell contract

Shells do not speak the guest ABI. A shell speaks the shell contract to a host, and no shell frame depends on the guest ABI. The contract of record is the machine-readable file `pkgs/id/contract/shell-contract.v1.json`: contract `id-shell`, version 1, revision 15. It defines the `/id-world/1` ALPN protocol over three transports: an iroh QUIC stream, WebSocket at `/ws/world`, and SSH exec `id-world/1`. The maximum frame is 1 MiB. Session phases are opening, query, install and joined. Frame classes are six: event, ephemeral, query, control, admin and delegation. The WebSocket binding of the same protocol carries 43 client frame kinds and 35 server frame kinds (web-config survey §1.3). The conformance suite `shell-suite.v1.json` names four shells — terminal, browser, native, hardware — with each shell's presented and sent frame sets. Only the native shell declares a client provider: `client.file.read`.

Three admin toggles travel as contract frames and as `id world` commands: isolation (`id world isolation <NODE> <isolated|unisolated>`), cross-world writes (`id world cross-world-write <NODE> <on|off>`, plus `force-cross-world-write` for force-on only) and file scope (`id world file-scope <NODE> <confined|unrestricted>`, with `--write-override <on|off>`). All three require the admin token.

## Journal, checkpointing and replay

The host keeps a journal per world (`world_store::{FileJournal, ModuleDir}` in `id-worlds`). Inputs and host events append to the journal in order. A checkpoint writes the guest `snapshot` text into the journal, and later replay can start from the checkpoint instead of from the seed. `id serve --world-checkpoint-every` defaults to `1000`; `0` never trims. After startup, a boot window refuses replay: `--world-boot-window-secs` defaults to `600`, and `0` disables the window.

Four offline commands work on journals and modules without a host, all gated on the `sandbox` feature. `id world journal check <DIR>` validates a journal directory and exits non-zero when the journal is invalid or written in a newer format than the binary reads. `id world journal backup <DIR> <OUT>` writes a backup; the output must not already exist, and the journal must pass check first. `id world replay --module <FILE> --fixture <FILE> --seed <N>` replays a recorded fixture and prints its verdict line first, then exits non-zero on a record mismatch. `id world verify` has the same shape and runs a double-replay determinism check, exiting non-zero at the first divergence. Session recording to replayable fixtures is the input side of the same mechanism.

Designs that go past this shipped core — hot-world sharding, native-spill write leases, journal replication to signed read replicas as a network service, and recurring cross-world schedules — are design records, not shipped behaviour. They are carried in [Design: worlds and the guest runtime](../design/worlds-runtime.md) and [Design: sync, replication and FileOps](../design/sync-fileops.md).
