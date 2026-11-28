# Roc import policy fixtures

The compile-fixture matrix of the package-import policy design
(`#5994` §4), wired for the Roc library import gate built by `#5956`
§7 (`#10027`). Each case pins its diagnostic — the gate subreason and
the exact excess set — as its oracle; a refusal for an unrelated
reason is a failing fixture, not a passing policy test.

## How it runs

- `fixtures.json` — the cases. Executed by the `id-caps` integration
  test `crates/id-caps/tests/roc_import_policy.rs`, which runs the
  real pipeline per case: manifest parse (step 1) → transitive
  resolution (step 2) → import gate (step 3).
- `roc-import-policy-test.sh` — the example-level runner; invoked by
  `pkgs/id/scripts/roc-test.sh` when cargo is on PATH (roc-test.sh
  says so explicitly when it is not, rather than skipping silently).
- The matrix's two Roc-compile negative fixtures — missing module and
  wrong signature — are the `roc-cap-demo` tests
  (`examples/roc-cap-demo/roc-cap-demo-test.sh`), the baseline the
  design names as this contract's first two fixtures; they stay there
  and are not duplicated here.

## Case forms

A manifest is given either as `kdl` (parsed by the step-1 parser,
fail-closed against the catalog) or as a `record` (fields carried
directly). The `record` form stands for a manifest parsed against a
different catalog vintage: parse-time validation is catalog-relative,
and the gate re-checks requirements against the catalog the world is
compiled against (`#5956` §3) — which is exactly when a requirement
can reach the gate naming no current catalog entry.

## Row status (against `#5994` §4)

- **Pure positive control** — executable (`pure-positive`).
- **Standard-dependent** — executable positive and negative
  (`standard-positive`, `standard-negative-not-on-platform` — the
  `#5956` §5 worked example — and `standard-negative-not-requested`),
  plus the transitive-closure row
  (`transitive-negative-not-on-platform`).
- **Scratch-filesystem / host-read / outbound-network negatives** —
  specified in §4 as "make executable when the provider exists". No
  file or network provider exists in the current catalog, so today
  these requirements name no catalog entry and the gate refuses them
  as `unknown_requirement` (`scratch-fs-negative`,
  `host-read-negative`, `net-negative`). When the providers land, the
  oracles move to `not_on_platform` against the standard-only surface;
  until then the fixtures pin the current, specific refusal — never a
  generic compile failure.
- **URL provenance** (`#5956` §4) — `url-declared-only-refused` /
  `url-declared-only-allowed` pin the declared-not-checked rule and
  the checked-requirements policy.
- **Declaration mismatch** (`#5956` §3 check 3) —
  `declaration-mismatch` pins discovered-used ⊄ declared.
