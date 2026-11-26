# Worlds and capabilities as a library

Source: primary sources are [pkgs/id/crates/id-worlds/src/lib.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-worlds/src/lib.rs), [pkgs/id/crates/id-worlds/src/sandbox.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-worlds/src/sandbox.rs), [pkgs/id/crates/id-caps/src/lib.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-caps/src/lib.rs), and [pkgs/id/crates/id-caps/src/world_caps.rs](https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-caps/src/world_caps.rs). This chapter is complete without following those links; they record provenance only.

Two crates carry the world system as a library: `id-worlds` owns the runtime — world state, sessions, the guest sandbox, and cross-world authority — and `id-caps` owns the capability data model that the runtime enforces: the provider catalog, execution limits, the world manifest, delegation, and the gates that decide what a guest or an agent may do. A consumer embedding worlds uses both, normally through the facade paths `id::world`, `id::sandbox`, `id::world_caps`, and `id::world_limits` (see [The id facade crate](facade.md)). The guest side of the contract is covered in [Guest apps and shells as consumers](guests-and-shells.md).

## id-worlds

Purpose (from its `Cargo.toml`): "Worlds layer of the id crate: the world runtime, sessions, sandbox and cross-world authority (pkgs/id crate split step 5, #4408)". At 75 `.rs` files it is the largest crate in the workspace. It depends on `id-caps`, `id-core`, `id-directory`, `id-events`, `id-groups`, `id-store`, and `id-sync`.

### Public modules

`id-worlds` declares 50 `pub mod` items: `account_charge`, `attachment_dispatch`, `blob_gate`, `chat_anchor_render`, `creator_announcements`, `creator_index`, `creator_org_views`, `groups` (the worlds bridge), `guest_burn`, `host`, `host_control`, `input_framing`, `kitty_graphics`, `kitty_still`, `program_bundle`, `program_gallery`, `program_gallery_admission`, `program_gallery_operator`, `program_publish_state`, `program_registry`, `program_registry_net`, `program_registry_pkarr`, `program_update`, `safety_appeal`, `safety_report`, `safety_review_queue`, `safety_transparency`, `shell_contract`, `world` (with submodules for actor, core, coordinator, handle, journal, replica, replication, signed, http_fetch, and provider_broker), `world_cache`, `world_compile`, `world_doctor`, `world_export`, `world_fixture`, `world_functions`, `world_hub`, `world_line`, `world_native`, `world_net`, `world_owner_report`, `world_records`, `world_replication_net`, `world_session`, `world_storage`, `world_store`, `world_template`, and `world_terminal`. Two modules are feature-gated: `sandbox` and `world_dev` under the `sandbox` feature, and `world_ssh` under the `ssh` feature.

About 20 further names are alias re-exports, so code moved during the split keeps its historical `crate::` paths inside this crate: `world_caps` and `world_limits` from `id-caps`; `abuse`, `connectivity`, `frames`, `petname`, and `world_presentation` from `id-core`; `directory`, `directory_auth`, `directory_mail`, and `directory_view` from `id-directory`; `event_store` from `id-events`; `chain`, `store`, and `tags` from `id-store`; `artifact` and `envelope_outbox` from `id-sync`; plus `pub use id_core::port`.

Two disagreements attach to this crate's documentation. First, the `id-worlds` `lib.rs` Contents section lists the directory cluster — `directory`, `directory_auth`, `directory_view`, `directory_mail` — as this crate's own contents and states that step 6's extraction must still invert or re-home `WorldCore`'s hold on the directory. Step 6b (#4409) has landed: those four names are re-exports from `id-directory` (`pub use id_directory::directory;` and its siblings at `crates/id-worlds/src/lib.rs:93-96`), and `crates/id-directory/src/lib.rs` records the move. The code behaviour is that the directory lives in `id-directory` and `id-worlds` re-exports it (disagreement LIB-8). Second, the `id-groups` `lib.rs` states that the `FoldedWorlds` implementation of the blob-gate trait stays in the facade until step 5 extracts `id-worlds`. Step 5 has landed: `FoldedWorlds` is defined at `crates/id-worlds/src/blob_gate.rs:36`, and no `FoldedWorlds` definition exists in the facade's `src/` (disagreement LIB-9). The `id-cli` bridge module re-exports both halves of the gate for command code: `id_groups::blob_gate::{BlobPolicy, Request, decide}` and `id_worlds::blob_gate::{FoldedWorlds, RefreshingWorlds}`.

### Key types and functions

- `sandbox::{Sandbox, WorldInstance, GuestCall, CallStats, CallMetrics}` — the wasmtime runner for the Plaza ABI v1. `Sandbox::compile(wasm, SandboxLimits)` compiles a guest module; `Sandbox::instantiate(seed)` returns a `WorldInstance`; `WorldInstance` provides `update`, `view`, `layout_at`, `records`, `wants`, `snapshot`, `restore`, `set_charge_key`, and `set_burn_budget`. The runner accepts import-free guests only. Fuel budgets are the determinism canary named in the `id-worlds` `lib.rs`.
- `world::{WorldCore, WorldHandle}` — the world state machine and its handle.
- `world_hub` — the multi-world host; `world_hub::testing` is available under the `test-utils` feature.
- `world_store::{FileJournal, ModuleDir}` — the on-disk journal and module storage.
- `shell_contract` — the F13.1 shell contract that both shells speak; the contract file itself is `pkgs/id/contract/shell-contract.v1.json`.
- `program_bundle` — the signed single-file program bundle (#8280): export, verify, and fork, fully offline.
- `guest_burn` — guest fuel and burn budgets.
- `account_charge::{WorldChargeConfig, WorldAccountRegistry, ChargeMode}` — per-account charging for world use.

### Features and dependents

Features: `default = []`; `sandbox = ["dep:wasmtime", "id-caps/sandbox", "id-core/sandbox"]`; `ssh = ["dep:russh"]`; `test-utils = []`, never in production builds. Production dependents: the facade, `id-cli`, and `id-web`. Both shells dev-depend on `id-worlds` with the `sandbox` feature for their in-process host harnesses.

### Embedding example

The facade's game-post path is a real call site. `pkgs/id/src/boards.rs:1592-1594` compiles a guest package through the facade's `id::sandbox` re-export of this crate's `sandbox` module:

```rust
let sandbox = crate::sandbox::Sandbox::compile(package_bytes,
    crate::sandbox::SandboxLimits::default())?;
let mut instance = sandbox.instantiate(0)?;
let genesis_bytes = instance.snapshot()?;
instance.restore(&genesis_bytes)?;
```

## id-caps

Purpose (from its `Cargo.toml`): "Capability catalog, execution limits and world manifest of the id crate (pkgs/id crate split step 1, #4403)". Its `lib.rs` describes the crate as the capability data model of `id`: the capability catalog, execution limits, and the declarative world manifest. It depends on `id-core` for `id_core::json_unique` only.

The `lib.rs` doc header is stale in the same pattern as the facade's: it describes only the original three moved modules — `world_caps` and `world_limits` from `id-core`, `world_manifest` from the facade package. The crate publishes 29 public modules, not three (disagreement LIB-14).

### Public modules

`agent_delegation`, `cap_baseline`, `cap_class`, `cap_diff`, `declaration`, `http_fetch_url`, `library_attach`, `library_gate`, `library_manifest`, `library_resolution`, `mcp_bridge`, `mcp_policy`, `portal`, `used_caps`, `world_caps`, `world_limits`, `world_manifest`, `world_scopes`.

The modules fall into families:

- **The catalog and its arithmetic:** `world_caps` (the provider catalog), `world_limits` (execution limits), `world_manifest` (the declarative world manifest), `world_scopes`, `cap_baseline`, `cap_class`, `cap_diff`, and `used_caps` (which capabilities a program actually used).
- **Delegation:** `agent_delegation` — `AgentId`, `Role`, `RoleBinding`, `Delegation`, `JoinGrant`, `DelegationRefusal`, and `verify_join`. This is the library form of the join-grant flow that [Tutorial: join a world and run a guest app](../tutorials/join-a-world.md) exercises from the command line.
- **The Roc library-import gate:** `library_manifest`, `library_resolution`, `library_gate`, and `library_attach` — the pipeline that decides whether a Roc library import is admitted, exercised end to end by the fixtures in `pkgs/id/examples/roc-import-policy/`.
- **The MCP bridge:** `mcp_bridge` and `mcp_policy` (#7914) — the translator and policy point that map MCP tool calls onto world-session frames. The bridge holds no credentials; the full tool list and policy rules are in [The MCP bridge](../reference/mcp.md).
- **Singles:** `declaration` (the provider declaration lifecycle: `Filed`, `Admitted`, `Live`, `Suspended`, `Superseded`, `Stale`, `Tombstoned`), `http_fetch_url` (URL handling for the `http.fetch` provider), and `portal`.

### The provider catalog

`world_caps` registers the providers a guest can request through the `wants` protocol: `time.now`, `time.tick`, `random.u64`, `players`, `players.list`, `chat.say`, `chat.history`, `records.read`, `kv.get`, `kv.set`, `kv.delete`, `blob.put`, `blob.get`, `blob.delete`, `http.fetch`, `notify`, `schedule.at`, `schedule.cancel`, `world.info`, `device.info`, `form.present`, `form.result`, `form.cancel`, `loc.position`, and `loc.watch`, each with argument schemas and bounds. The client-provider namespace constant is `CLIENT_PROVIDER_NAMESPACE = "client."` at `crates/id-caps/src/world_caps.rs:312`. Grants and ceilings are enforced host-side: a known but ungranted request returns a refusal event with a stable error code, never silent execution. The error codes are `cap_denied`, `unknown_cap`, `bad_args`, `failed`, `actor_denied`, `outside_ceiling`, and `result_too_large`.

`world_limits` holds `RuntimeLimits` and the `SandboxLimits` / `NativeLimits` vocabulary. Serve-side defaults that reach these types are literal: `--world-fuel` defaults to 10000000 units, `--world-memory-mib` to 16, and `--world-message-kib` to 1024 (see [serve and publications](../reference/cli-serve.md)).

On the guest side, the Roc client library for the same catalog is `RocCap`, one of the six modules the shared platform package exposes (`Screen`, `Key`, `RocJson`, `RocGrid`, `RocCap`, `RocText`). `RocCap` keeps a session with request ids `r0`, `r1`, and so on, and provides `issue`, `attach`, `requests`, and `route`, plus request builders: `chat_request` (`chat.say`), `now_request` (`time.now`), `players_request` (`players.list`), `world_request` (`world.info`), `history_request` (`chat.history`), `records_request` (`records.read`), `schedule_at_request`, and `schedule_cancel_request`. The wire form is defined in [Guest apps and shells as consumers](guests-and-shells.md).

### Features and dependents

Features: `default = ["sandbox"]`; `sandbox = []`, a marker forwarded through the graph; `test-provider = []`, a test-only provider for the #1061 registry exit proof, never in production builds. Production dependents: the facade, `id-cli`, `id-directory`, `id-web`, and `id-worlds`.

### Embedding example

The import gate's real call site is a test that runs production code: `crates/id-caps/tests/roc_import_policy.rs` runs the full manifest, resolution, and gate pipeline from this crate against `pkgs/id/examples/roc-import-policy/fixtures.json`; the fixture directory's README names that test as its executor. Inside the facade's own source, `id_caps::` is named 5 times, and the facade's dev-dependency enables `id-caps/test-provider` for `pkgs/id/tests/provider_registry_exit_proof.rs`.
