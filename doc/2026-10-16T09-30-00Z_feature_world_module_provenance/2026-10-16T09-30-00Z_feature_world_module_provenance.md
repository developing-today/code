# World module build provenance

See the [Phase 9 plan](../../../../.opencode/plans/phase-9-portable-platform.md) and the [platform-level decision record](../../../../doc/2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md).

## Intent

P9.3 needs an auditable distinction between a module compiled from source by the server and a raw artifact uploaded by an administrator. The record must survive restart and compaction without claiming that a source build is trustworthy. It must not store source text, administrator credentials, or change the sandbox policy.

## Initial design

- Use an optional, versioned `ModuleProvenance` value: `artifact` or `source_build`.
- For source builds, capture domain-separated BLAKE3 digests for the configured compiler executable, staged platform tree, and submitted source files, plus target and the stable compiler argument vector. Exclude the temporary output path.
- Persist provenance on the existing program-install journal event and checkpoint. Optional fields with Serde defaults preserve old journal decoding; missing provenance remains unknown.
- Expose the value as an optional field in module info while retaining the existing tuple-based `WorldClient::info` API.
- Keep this as server-observed metadata, not a signature, human identity, source attestation, or reproducibility claim.

## Rollout update

The initial slice is implemented in `world_compile.rs`, `world.rs`, `world_session.rs`, and `world_net.rs`. The module event and checkpoint carry provenance through replay and compaction. Raw network artifact installs are explicitly labeled; older/direct installs without metadata remain `None`. `WorldClient::info_details` exposes the optional record without breaking `info` callers.

Focused tests cover deterministic digesting, legacy event/checkpoint decoding, checkpoint restore, and module-info reporting. Full per-world runtime profiles, expanded capability ceilings, and a human-readable compiler version are intentionally separate follow-ups.

## References

- [Phase 9 plan](../../../../.opencode/plans/phase-9-portable-platform.md)
- [Platform-level provenance record](../../../../doc/2026-10-16T09-00-00Z_feature_world_module_provenance/2026-10-16T09-00-00Z_feature_world_module_provenance.md)
- `pkgs/id/src/world_compile.rs`
- `pkgs/id/src/world.rs`
- `pkgs/id/src/world_session.rs`
- `pkgs/id/src/world_net.rs`
