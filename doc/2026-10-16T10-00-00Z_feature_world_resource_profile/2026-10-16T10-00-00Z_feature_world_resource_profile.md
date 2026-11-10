# Per-world resource profiles

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) and the [world module provenance record](../2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md).

## Intent

P9.3 needs administrators to tune resource limits for individual worlds while keeping server-configured runtime limits as non-bypassable hard ceilings. This first slice defines the request and resolution contract only; it does not persist profiles or change an already-running guest.

## Decision

- Model per-world resource requests as optional values. An omitted value inherits the current server ceiling.
- Resolve every provided value by taking the stricter of the requested limit and server hard ceiling. Requests above the ceiling are clamped, never allowed to raise it.
- Cover Wasm fuel, memory, and message bytes; and native address-space memory, CPU seconds, and call deadline.
- Keep the module-byte limit, Wasm import-free validation, and native enablement outside this profile. Native programs remain under the existing OS sandbox.
- Keep capability grants separate: execution profiles constrain resources, not authority.
- Defer journal persistence and updates to Wasmtime instances. Profiles are not reported as active until later lifecycle work defines safe re-instantiation/replay behavior.

## Initial implementation boundary

Add `WorldExecutionProfile` to `world_limits.rs` with validation and resolution against `RuntimeLimits`, plus unit tests for inheritance, clamping, and invalid zero values. Follow-up work will add durable per-world profile state and pass the effective limits when restoring or installing programs. A separate policy slice will define the server-wide allowed-function ceiling.

## References

- [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md)
- [Observed world module provenance](../2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md)
- `pkgs/id/src/world_limits.rs`
- `pkgs/id/src/world_store.rs`
- `pkgs/id/src/world_caps.rs`
