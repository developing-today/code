# Composition hash spec — canonical CBOR + BLAKE3 identity for ID builds
#
# S1 of developing-today/code#14056. Implements plan §5.2 (the composer's
# §3.2, applied to ourselves) as amended: composition-time parameters are
# hash inputs (plan §1.2.2 rule), aliases never change a hash
# (amendment 2), and the canonical form precedes every other composer
# byte (plan §1.2.1 fix). Two honest implementations hashing the same
# composition differently is a test failure, not a discovery.

## Algorithm

```
identity = BLAKE3-256(canonical_cbor(canonical_manifest))
```

- **Canonical CBOR**: the estate's existing canonical-CBOR discipline,
  `pkgs/id/crates/id-core/src/canonical_cbor.rs` — closed value model
  (uint, text, bytes, arrays, maps, bool, null; no floats, no tags),
  shortest-form integers, definite lengths only, map keys unique and
  sorted by their encoded bytes, strict decoding (non-shortest forms,
  indefinite lengths, out-of-order or duplicate keys, and trailing
  bytes are typed errors). The composition hash uses the uint / text /
  null / array / map subset; anything outside it is a validation
  refusal before hashing is reached.
- **BLAKE3**: 256-bit output, rendered lowercase hex (64 chars).
  Dependency: the `blake3` crate already in the workspace lockfile.
- **No parameters outside the manifest.** Grants, config, and data are
  never hash inputs (plan §5.2).

## Canonical manifest (the hashed structure)

Exactly these fields — all of them (plan §1.2.2: composition-time
parameters are inside the hash, stated as a rule):

| CBOR key (sorted order) | Type | Source |
|---|---|---|
| `catalogue` | text | Catalogue version (`catalogue_version` in catalogue.toml). A catalogue change honestly re-identifies every composition. |
| `features` | array of maps | The **sorted resolved** feature list. Each entry: `{"crates": [sorted "crate@version"...], "name": "feature"}`. Map keys sort `crates` < `name`. Feature versions are the owning seam crates' versions from the workspace at build time. Core capabilities are implicit (present in every profile) and NOT listed — core is versioned via `lockfile` + `catalogue` instead. |
| `lockfile` | text (64 hex) | `BLAKE3-256` of the workspace `Cargo.lock` file bytes at build time. Pins every dependency version. |
| `profile` | text or null | Canonical profile name after alias resolution (`basement` is recorded as `headless`), or null for profile-less explicit compositions. The name is provenance, not identity — the resolved `features` are. |
| `schema` | uint | Manifest schema version (`1` today). A schema bump re-identifies everything, deliberately. |
| `target` | text | Rust target triple. Cross-target builds never collide. |
| `toolchain` | text | Pinned Rust toolchain (`channel` in pkgs/id/rust-toolchain.toml; `1.99.1` today). A toolchain bump honestly re-identifies (composer §3.8.3, applied to ourselves). |

Map key order by encoded bytes (all short text, so lexicographic):
`catalogue` < `features` < `lockfile` < `profile` < `schema` <
`target` < `toolchain`. Feature entries sort by `name`; `crates`
arrays sort byte-wise.

## Version stamp and registry

- The built artefact's long `--version` gains `composition
  <profile-or-custom>:<hash-prefix>` — profile name plus the first 16
  hex chars of the identity (128 bits, collision-irrelevant for a
  display string). Full 64-hex identity lives in the registry record.
- Registry record per composition (plan §5.4, starting as versioned
  JSON/TOML in the repo): canonical manifest, identity, target,
  artefact hash + URL, resolved feature list (human rendering), profile
  name if graduated, served environment + owner, pinning-deployments
  count. Yank semantics (plan §1.2.5): a yanked composition gets no
  new deployments; the artefact stays immutable and the record says
  why. GC criterion: no pinning apps and no grants for a full nightly
  cycle (composer §8.5; nightly = toolchain bump for us).

## Worked example (golden vector)

Resolves `examples/basement-store-node.toml` (`profile = "basement"`
+ `store-turso` + `dht` on `x86_64-unknown-linux-musl`):

- Alias `basement` → canonical `headless`; resolved feature closure:
  `dht, directory, sandbox-exec, serve, store-turso, world-state`
  (closure adds `serve` via `directory`, `world-state` via
  `sandbox-exec`; core stays implicit).
- Owning-crate versions are today's workspace values (`0.1.0`
  throughout) and therefore illustrative — the spec pins the
  *procedure*, and this vector pins one full pass through it.
- `lockfile` is the fixed illustrative input
  `0000…0000` (64 zeros): real manifests carry
  `BLAKE3(Cargo.lock bytes)`. The vector tests encoding + hashing,
  not today's lockfile.

Inputs: `golden/inputs.json`. Procedure: `golden/make_vector.py`
(stdlib-only canonical encoder) → `golden/canonical.hex` (464 bytes)
→ BLAKE3-256 → `golden/hash.hex`:

```
canonical (464 bytes): a766736368656d6101…74312e39392e31 (see canonical.hex)
identity: 960b3f97e0656713540e593d803c752b5ccca6c6cb47f01cb896e121c6557f78
--version stamp: composition headless:960b3f97e0656713
```

Reproduce: `python3 -I golden/make_vector.py` must print exactly
`canonical.hex`; hashing those bytes with BLAKE3 must print exactly
`hash.hex` (see `golden/recheck.sh`; the hash step needs cargo + the
cached `blake3` crate, both present in this estate). The
`headless-alias.toml` example (canonical `profile = "headless"`,
same features) MUST hash identically — alias-invariance is part of
the spec, not an accident.

## Conventions boundary (amendment 4)

This spec covers mechanisms only: composition identity, availability
of a compiled composition, and fuel/cost accounting hooks. The
behaviours layered on those mechanisms — what a world does when a
referenced strategy is unavailable or becomes available (idle, queue,
local-only error, substitute, switch functions, presets) — are world
conventions, not platform semantics, and nothing in later slices may
hard-code one of them as the only outcome (e.g. the runtime must not
force an error state on an uncompiled reference). The runtime exposes
the signals (availability, compile completion); worlds decide.
