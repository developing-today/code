# Feature composition for pkgs/id — machine-readable design-of-record (S1 + S2)

S1 of developing-today/code#14056 (§10 slice list): the §4 catalogue,
the §5 manifest schema + canonical hashing spec, and the §0.2
reconciliation — as data and specs, validated against today's tree.
S1 records the catalogue and composition contracts; S2 records immediate
feature hygiene and the single source for shipped variant feature sets.

Plan of record: `doc/2026-11-24T03-36-48Z_design_id-feature-composition/`
(record PR #14055). S1 record note (validation result, amendment
handling, what later slices consume):
`doc/2026-11-24T03-36-48Z_design_id-feature-composition/s1-catalogue-manifest-hash.md`.
Predecessor record: `doc/2026-11-09T04-43-05Z_design_id5788_build_variants.md`
(amended per its §8 rule — see the Amendment section at its end).

## Contents

| File | What |
|---|---|
| `catalogue.toml` | The §4 catalogue as data: features (adds / requires / roles / status / forwards), exclusivity groups, profiles (built-ins + served examples + aliases), the unrepresentable list. |
| `manifest-schema.md` | The §5.1 build-manifest schema (TOML surface, fields, the 8 rules) + open profile sets + the `id-compose` (S7) contract. |
| `composition-hash-spec.md` | The §5.2 composition-hash spec: canonical CBOR profile + BLAKE3-256, the hashed field table, version stamp + registry + yank semantics, conventions boundary, worked example. |
| `examples/` | Three valid manifests: `basement-store-node.toml` (plan §5.1 verbatim), `headless-alias.toml` (alias-invariance pair), `user-profile.toml` (open-set mechanism). |
| `golden/` | `inputs.json` (worked-example inputs), `make_vector.py` (stdlib canonical encoder), `canonical.hex` (464 bytes), `hash.hex` (BLAKE3 identity), `recheck.sh` (reproduce both). |
| `validate.py` | S1 catalogue/tree checks plus the S2 variant-source pin (check 8). Run `python3 -I validate.py` here. |
| `variants.toml` | S2 single source for the `web` and `lib` shipped build feature sets and root defaults. |
| `../scripts/variant-features.sh` | Strict reader used by just/build.sh to obtain a variant's feature set. |

## The five amendments, and where each landed

1. **Three-sided matrix — compile / host / consume** (comment 7121665265).
   Catalogue `roles` on every feature (`compile` = needs it to compile
   a platform, `host` = run-side/server role, `consume` = client half).
   Roles are descriptive, never selective (manifest-schema rule 8).
2. **Headless canonical, basement alias, open profile sets** (comment
   7122875407). `headless` profile with `aliases = ["basement"]`;
   alias resolution precedes hashing (manifest-schema rule 2,
   hash-spec worked example); user profiles share the built-in format
   (`examples/user-profile.toml`).
3. **Server and client are roles, not node classes** (comment 7123140535).
   The `roles` field models per-role feature sets that compose freely
   in one binary; mutual hosting is ordinary. The three ID-side app
   permissions (run a world / precompile a platform / use a compiled
   platform) attach to the acting identity per role — a grant-layer
   concern for later slices, recorded here so the catalogue never
   implies node classes.
4. **Strategy-model behaviours are conventions** (comment 7123206906).
   Recorded as the "Conventions boundary" section of
   `composition-hash-spec.md`: the spec standardizes signals
   (availability, compile completion), never behaviours. Worlds decide.
5. **The compile prerequisite is the `rust` feature** (comment 7123332501).
   Catalogue entry `rust` (`designed`; cargo wiring is later-slice
   work), in `roles = ["compile"]` and in `roc-composer`'s `requires`,
   alongside its per-capability role.

## Validation (2026-11-24, against origin/main 0dd1567e)

`python3 -I validate.py` passes: all 6 root features accounted for
(5 shipped catalogued — `sandbox→sandbox-exec`, `world→world-state`,
`ssh→ssh-serve`, `web`, `dht` — plus `currency-poc-tb` declared
unrepresentable); every shipped forwarding exists in its seam crate today
(`id-web/sandbox` test-only gate removed in S2);
requires acyclic; groups consistent; all 7 profiles resolve with at
most one selected member per group; the golden vector's feature set
equals the catalogue resolution of `basement-store-node.toml` with
owning-crate versions matching today's workspace (`0.1.0`).

## S2 feature hygiene (2026-11-24)

`resolver = "3"` is explicit in the workspace manifest. The `id-web/sandbox`
feature was removed rather than forwarded: all eight cfg sites gated only
test helpers in `world_ws.rs`; production code used no such cfg, and the
web crate's `id-worlds` dev-dependency already enables `sandbox` and
`test-utils` for its tests. Member defaults intentionally remain divergent:
`id-caps` defaults to `sandbox`, `id-cli` to `sandbox,world`, and `id-core`
to `world,sandbox`; the root facade opts out with `default-features = false`
for each member and owns the effective defaults (`sandbox,world`). The
legacy, unreferenced `default.nix` is not a shipped variant consumer.

Nix, just recipes and `scripts/build.sh` now read `variants.toml`; validation
check 8 verifies its root defaults/features and those consumers. The S2
done-check compares the Nix `id-web` and Cargo web feature graphs; it remains
blocked by the upstream `cargo check -p id-store` failure tracked in #14315.

## For later slices

- S2 consumes the `known_gap` note and the unrepresentable rule.
- S3 performs the `world` split this catalogue anticipates
  (`world-state` → core, `sandbox-exec` rename).
- S4/S5 build the `plain`/`full` composition crates from these
  profiles; the `embedded` note constrains S5's library composition.
- S6 wires the store groups and hardens the core-default overlap
  warning into a refusal.
- S7 implements `id-compose` against `manifest-schema.md` and
  `composition-hash-spec.md`; both example manifests and the
  user-profile mechanism are its first acceptance inputs.
- S10 implements `roc-composer` with `rust` as its compile
  prerequisite and the conventions boundary as a hard constraint.
