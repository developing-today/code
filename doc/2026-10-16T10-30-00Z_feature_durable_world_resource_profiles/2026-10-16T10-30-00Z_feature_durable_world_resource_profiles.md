# Durable per-world resource profiles

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) and the [resource profile contract](../2026-10-16T10-00-00Z_feature_world_resource_profile/2026-10-16T10-00-00Z_feature_world_resource_profile.md).

## Intent

Persist an administrator-selected resource profile per durable world and use it when that world's program is next installed or restored. Keep the server's `RuntimeLimits` as hard ceilings. A profile must not change Wasm import policy, native enablement, OS sandboxing, or capability grants.

## Decisions

- Store the requested profile in the world journal and checkpoint; old journals without a profile inherit an empty profile, which means current server limits.
- Resolve the stored request through `WorldExecutionProfile::resolve(server_limits)` each time a new instance is created or restored. This means reducing server limits remains authoritative even if a journal contains higher requests.
- A profile change is durable immediately but does **not** mutate a running guest. The current instance keeps its existing runtime limits. The new request applies on the next module installation or process restart; the admin response must say this explicitly.
- Keep the currently requested profile distinct from an active instance's actual limits. Do not report a profile update as effective for the still-running guest.
- Expose profile changes only to the world admin and preserve the existing authorization boundary.

## Implementation and validation

- `JournalEntry::ExecutionProfile` stores each requested profile change. Checkpoints carry the profile too, so compaction preserves it; older journals/checkpoints deserialize with the empty profile, which inherits current server limits. The journal version remains unchanged because the new fields/entry are additive.
- Restore resolves the stored request against the current `RuntimeLimits`; later installs resolve the current world request the same way. The server ceiling therefore wins after restart, including when it has been lowered since the request was saved.
- The admin-only `set_execution_profile` service operation and `/id-world/1` frame return the requested profile plus a notice that it applies on next install/restart, not to the running instance. `WorldClient::set_execution_profile` exposes the operation. No CLI command was added.
- A service-level operation lock serializes profile updates with installs, so an install cannot race a change and activate a guest under a stale request.
- Tests cover legacy checkpoint decoding/restore, checkpoint and durable restart persistence, clamping after a server ceiling change, admin authorization, the no-live-replacement boundary, protocol response wording, and use of the profile on subsequent Wasm installs. The import-free Wasm validator and native seccomp sandbox tests remain passing.

Focused validation: `cargo fmt --check`; the `world_limits::tests`, `execution_profile`, legacy checkpoint, installed-program fuel, import-free Wasm, and native seccomp tests passed. The `world_limits::tests` also passed without the sandbox feature.

## Implementation boundary

Implemented as specified: journal/checkpoint persistence, current-server resolution on restore and later installs, and an admin-only service/protocol operation with a public `WorldClient` API. No CLI command was added.

## Follow-ups

- Controlled in-place re-instantiation/replay so an administrator can apply a profile without restarting the server or waiting for a module replacement.
- Configuration UI/CLI for profile management.
- Server-wide allowed-function ceilings, which are a separate authorization policy from resource profiles.

## Rollout compatibility note

A binary built before #244 treats an `ExecutionProfile` entry that is the last line as a torn append and truncates it, and refuses the journal when the entry is not last. Restore a pre-change journal backup before rolling back. Binaries built after #244 refuse any complete line they cannot read.

- A profile journal event written by this version is not understood by older binaries; rolling back after changing a profile requires a compatible migration or restoring a pre-change journal backup.

## References

- [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md)
- [Per-world resource profile contract](../2026-10-16T10-00-00Z_feature_world_resource_profile/2026-10-16T10-00-00Z_feature_world_resource_profile.md)
- `pkgs/id/src/world.rs`
- `pkgs/id/src/world_store.rs`
- `pkgs/id/src/world_session.rs`
- `pkgs/id/src/world_net.rs`
