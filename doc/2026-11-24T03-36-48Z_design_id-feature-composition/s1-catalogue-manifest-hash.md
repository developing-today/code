# S1 — Catalogue + manifest + hash, as a design-of-record (done 2026-11-24)

Slice S1 of developing-today/code#14056 (§10 slice list). Implements
plan §4 (catalogue), §5 (manifest schema + canonical hashing), and the
§0.2 reconciliation — as machine-readable data and specs, validated
against today's tree, honouring all five 2026-11-24 amendments. No
implementation is authorized by S1; S2–S13 build on it.

## What landed

`pkgs/id/composition/` (see its README for the full index):

- `catalogue.toml` — §4 as data: 33 features (6 shipped incl. 2
  unconditional core defaults, 5 seam, 13 designed, 3 placeholder, plus
  core record), 3 exclusivity groups, 7 profiles (`plain`, `full`,
  `headless`+`basement` alias, `embedded`, `composer-host`,
  `alt-store-node`, `hosted-edge`), the unrepresentable list.
- `manifest-schema.md` — §5.1: TOML surface, 8 rules (alias-first,
  closed catalogue, exclusivity refusal, unrepresentable refusal,
  requires closure, grants-never-manifest, descriptive roles), open
  profile sets, the S7 `id-compose` contract.
- `composition-hash-spec.md` — §5.2: BLAKE3-256 over canonical CBOR
  (the `id-core` discipline), full field table, version-stamp +
  registry + yank semantics, conventions boundary (amendment 4),
  worked example with golden vector.
- `examples/` — the plan §5.1 manifest verbatim, its alias pair, and
  a user-defined profile.
- `golden/` — `inputs.json`, stdlib encoder, `canonical.hex` (464
  bytes), `hash.hex` (`960b3f97…7f78`), `recheck.sh` (both reproduce).
- `validate.py` — the done-test, executable (7 check families).

Reconciliation: appended Amendment section to
`doc/2026-11-09T04-43-05Z_design_id5788_build_variants.md` per its own
§8 rule (amendment PR, nothing above rewritten).

## Validation (origin/main 0dd1567e, 2026-11-24)

`python3 -I validate.py` → pass, no warnings. Every root cargo
feature accounted for (5 shipped catalogued, `currency-poc-tb`
unrepresentable); every shipped forwarding exists in its seam crate
today; requires acyclic; groups consistent; all 7 profiles resolve
(≤1 selected member per group); golden feature set equals the
catalogue resolution of `basement-store-node.toml` with crate
versions matching today's workspace. `recheck.sh` reproduces both
golden files. Known findings for later slices (not S1 failures):
`id-web/sandbox` forwarding gap → S2; `world` conflation → S3 split;
core-default group overlap warns until S6.

## Five amendments honoured

1. Compile/host/consume roles on every feature; roles descriptive only.
2. `headless` canonical + `basement` alias (alias-invariance in the
   hash spec); user profiles in the built-in format.
3. Roles, not node classes — one binary may hold any role combination;
   the three app permissions attach per role at the grant layer.
4. Strategy behaviours are world conventions — mechanisms only in the
   spec; worlds decide.
5. `rust` is the compile prerequisite — catalogue entry + in
   `roc-composer`'s requires.

## Consumed by

S2 (hygiene: `known_gap`, unrepresentable rule) · S3 (the `world`
split this catalogue anticipates) · S4/S5 (plain/full composition
crates from these profiles) · S6 (store groups, harden the overlap
warning) · S7 (`id-compose` against both specs; both example
manifests are its first inputs) · S10 (`rust` prerequisite +
conventions boundary as hard constraints).
