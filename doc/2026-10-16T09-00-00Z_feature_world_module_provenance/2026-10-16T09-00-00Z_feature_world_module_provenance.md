# Observed world module provenance

## Decision

Record an optional, server-observed origin alongside each installed world module. A raw module upload is tagged `artifact`. A successful source build records a versioned provenance document containing domain-separated BLAKE3 digests of the configured Roc executable, staged platform tree, and submitted source files, together with the target and stable compiler arguments (excluding the ephemeral output path).

The record is metadata, not an attestation. It does not prove that the compiler, platform, or source is trustworthy, and the server admin token is not an individual identity. It does not store source contents or credentials. The executable digest identifies the bytes observed at compile time; no claim is made about the compiler's supply chain or reproducibility.

## Persistence and compatibility

- Provenance is an optional field on the existing `program_installed` event and checkpoint. Old journal entries decode with no provenance, and `None` is omitted when serializing.
- Journal version remains 1 because this is an additive optional JSON field; checkpoints preserve the current module's value across compaction and restore.
- The existing module-info frame gains an optional provenance field. Existing clients can keep using the current tuple-returning API; `WorldClient::info_details` exposes the new field.
- A module installed through an older/direct API may remain unknown (`None`); it is never inferred to be a source build. Network raw-artifact installation is explicitly marked `artifact`.

## Boundaries and follow-ups

- Hashes are integrity identifiers only. There is no signature, remote verification, transparency log, per-user build identity, or compiler version string in this slice.
- The metadata records the source submitted to the configured compiler. The compiler still rewrites the platform reference to the staged platform copy before building.
- Per-world execution profiles and broader allowed-function ceilings remain separate policy work; this change does not relax Wasm imports or native OS sandboxing.

## Validation

Tests cover deterministic source/platform digests, optional-field backward compatibility, checkpoint persistence, source build metadata, and module-info reporting for an artifact install.
