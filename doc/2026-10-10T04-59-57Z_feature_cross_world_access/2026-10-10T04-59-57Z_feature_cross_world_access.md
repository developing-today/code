# Cross-world access: writes, force, and file scope

## Request

Extend per-world isolation (see [world isolation](../2026-10-10T03-56-25Z_feature_world_isolation/2026-10-10T03-56-25Z_feature_world_isolation.md)) with:

- **A. Cross-world write.** A per-world `CrossWorldWrite` setting, off by default. A write from another world is allowed only when the writer is unisolated, the target is unisolated, and the target's setting is on. Isolated worlds never accept or grant cross-world writes.
- **B. Force.** The server admin can force the setting on for an unisolated world. Moderators cannot force it. A forced setting is locked against moderator changes.
- **C. Unrestricted files.** A per-world file scope, `confined` (default) or `unrestricted`, plus an admin write override.
- **D. CLI.** `id world` subcommands for each setting.
- **E/F.** Tests and a results record.

## Findings: what "files" means here

No world file capability exists. The capability catalog (`world_caps::CATALOG`) lists only time, random, players, chat, world info, `Key` and `Screen`. Roc platform modules expose only `Screen` and `Key`. Native workers are hardened with a Landlock ruleset that grants nothing except executing the worker binary (`world_native::apply_native_sandbox`). The file operations that do exist (`fileops.rs`, `store.rs`) belong to the node's blob store, not to worlds.

So the file scope is implemented as a stored, journaled, checked policy function (`WorldPolicy::file_allowed`). No existing code path consumes it yet, and the native sandbox is unchanged.

## Design

- **Gate.** One function, `world_hub::cross_world_allowed(access, reader, owner, owner_write)`. Reads need both worlds unisolated. Writes additionally need `owner_write`. `WorldLease::authorize_read` and `authorize_write` both call it.
- **Journal entries** (absent means default):
  - `CrossWorldWrite { enabled }`: moderator or admin toggle. `enabled: false` also clears force.
  - `CrossWorldWriteForced`: admin force. Replay sets on and forced.
  - `FileScope { scope }`: moderator or admin.
  - `FileWriteOverride { enabled }`: admin only.
- **Checkpoint** writes the current state with the same entries.
- **Wire.** New `ClientFrame`s: `set_cross_world_write`, `force_cross_world_write`, `set_file_scope`. Reply: `policy`.

## Choices where the request was open

1. **Force permission: the server admin token only.** Moderators are refused in the actor (`Authority::Admin` check) and in the transport (`is_admin_token`). This matches who holds the admin token, the server operator.
2. **Forced lock.** A moderator can turn a forced setting on but not off. An admin off clears the force. Replay of an off entry clears it too.
3. **Isolated worlds cannot be forced** (refused at force time). If a forced world is later isolated, the gate still denies writes. Replay does not re-check isolation, since the journal was validated when written.
4. **Read-only default for Isolated worlds.** An Isolated world can write files only with the admin override, under any scope, including its own root. Unisolated worlds write under their scope. The override is admin-only.
5. **Scope is set by a moderator or admin.** The override is set only by the admin token.

## Results

Implemented as specified in the request, with the choices above.

- **Write gate.** `cross_world_allowed(access, reader, owner, owner_write)`: reads need both worlds unisolated; writes also need the target's cross-world write setting on. Truth table tested.
- **Force.** Admin token only. Refused for moderators (actor and transport) and for Isolated worlds. Moderators cannot turn a forced setting off.
- **File scope.** `Confined` (default) or `Unrestricted`. Isolated worlds are read-only unless the admin override is on. Scope set by moderator or admin; override admin only. Journaled, replayed on restart, kept by checkpoint.
- **CLI.** `id world cross-world-write`, `force-cross-world-write`, `file-scope`.
- **Tests.** `cargo test --lib --features world`: 693 passed, 3 failed. The 3 failures are the known `world_compile` tests (`examples/roc-world` lacks `targets/wasm32/host.wasm`), unchanged by this work. Added tests cover the write gate truth table, write-setting replay, force semantics, Isolated refusal, file-access truth table, a restart round trip, and the bad admin token refusal for force and override.
- **Format.** `cargo fmt --check` clean.
- **Clippy.** `-D warnings` already fails on the base tree (crate-wide pedantic lints). On added lines the only remaining lints are four `map_err(|_| ...)` sites that copy the existing pattern (origin/main has 17).

Gaps:
- **File scope has no consumer.** `WorldPolicy::file_allowed` is a checked function that no caller invokes yet. The native Landlock sandbox (`world_native.rs`) is unchanged; it already grants no file access beyond the worker binary.
- **Transport coverage.** Only the admin-token refusal is tested over the session frames; there is no signed-moderator transport test for the new frames.
- **CLI integration.** No additions to `tests/cli_integration.rs`.
