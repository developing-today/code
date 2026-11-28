# hosted-primitives

Shared TypeScript primitives for the hosted Workers — G2 of
[#9085](../../doc/2026-11-26T03-00-00Z_design_hosted-second-version-typescript-9085/2026-11-26T03-00-00Z_design_hosted-second-version-typescript-9085.md)
(component G §9, slice G2).

Both `pkgs/id/hosted-worker` and `pkgs/fleet/hosted-worker` import
these modules by relative path (the same convention the fleet suite
already used for its BLAKE3 test oracle). There is no workspace
linking: each worker keeps its own `package.json` with no runtime
dependencies.

Contents:

- `src/blake3.ts` — BLAKE3-256, one-shot `blake3()` plus the
  incremental `Blake3Hasher`. Merges the two copies this replaces
  (the id worker's one-shot hash and the fleet worker's streaming
  hasher, which used the one-shot form as its test oracle).
- `src/canonical-cbor.ts` — the deterministic CBOR profile, the
  TypeScript mirror of `crates/id-core/src/canonical_cbor.rs`.
- `src/encoding.ts` — hex, base64 and base64url helpers. Replaces the
  per-module copies in `id` `src/ed25519.ts` /
  `src/migration-snapshot.ts` and fleet `src/jwt.ts` /
  `src/stamp-sig.ts` / `src/config.ts`.
- `src/index.ts` — re-exports the whole surface.

Oracle: the G1 shared fixtures
(`pkgs/id/tests/fixtures/hosted-second-version-g1.json`) plus the
official BLAKE3 vectors; both workers' existing suites must pass
unchanged against this package.
