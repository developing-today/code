# #5670 phase 1 — Atlas Python reference + eval corpus migration: verification and close-out record

Report record for developing-today/code#5670, the implementation of §8 of
the Scaler-era data architecture revision (design record
`doc/2026-11-04T21-27-34Z_design_atlas_scaler_revision/`, PR #5667).
Drew's order, recorded there: every Atlas/search/embedder script living
outside this repo migrates into developing-today/code + the fleet binary;
nothing canonical lives outside.

This record covers phase 1 — the Python reference implementation and the
eval corpus — which landed on main on 2026-11-07 but was never recorded
on #5670 (the issue's later triage notes still describe phase 1 as
unexecuted; queue task 10606, created for it, stranded at its attempt
cap). It verifies the landing against the outside sources, inventories
every §8 row at 2026-11-24, and slices the remainder.

## 1. What landed, and the parity proof

Commits `c6e5aad02` / `77a3aae8c` ("atlas-search python reference + eval
harness (branch disposition 2026-11-07)") added:

| Outside source (hatch-personal `workspace/turso-atlas/scripts/`) | Destination in this repo |
| --- | --- |
| `atlas_search.py` | `pkgs/atlas-search/atlas_search.py` |
| `eval3.py` | `pkgs/atlas-search/eval/eval3.py` |
| `golden-queries-v3.json` (the golden-311 corpus) | `pkgs/atlas-search/eval/fixtures/golden-311.json` |

Parity, verified fresh on 2026-11-24 against the outside copies as they
stand today:

- The outside sources' SHA-256 hashes match the provenance hashes
  recorded in `pkgs/atlas-search/eval/README.md` exactly:
  `atlas_search.py` `1b805be87b2d08fd5c5f6ed65f45d1c40528c9acc074adeaf0a5665a4cec8d9f`,
  `eval3.py` `d121c442566286b6f1ee3327693756bdf0f031f87f00e95ce486582d09d66360`,
  `golden-queries-v3.json` `b64bc1905384405c9127c744d3874ffd149a9429b6710b0664c2198505bf291c`.
  The outside copies have not drifted since the migration.
- `pkgs/atlas-search/atlas_search.py` is AST-identical to the outside
  source (Python `ast.dump` comparison). The byte difference is
  treefmt formatting only (commit `185853120`).
- `pkgs/atlas-search/eval/eval3.py` is AST-identical to the outside
  source once docstrings are stripped; the only content change is the
  usage text pointing at `fixtures/golden-311.json` instead of
  `golden-queries-v3.json`.
- `golden-311.json` is JSON-identical to `golden-queries-v3.json`
  (canonicalized diff: 0 lines; 311 queries).
- The in-repo fixture generator (`pkgs/atlas-search/fixtures/gen_core_fixtures.py`)
  already imports the in-repo `atlas_search.py`; the 2026-11-05 triage
  note that it read the external home-directory script is stale.

The outside copies are retired as of this record: retirement marks are
applied to the hatch-personal copies on a branch of that repo, and the
seat's `atlas-search-py` wrapper is repointed at the in-repo copy.

## 2. §8 inventory at 2026-11-24

Live rows:

| §8 row | Status | Remainder |
| --- | --- | --- |
| `atlas_search.py` → `pkgs/atlas-search/` | **Landed** 2026-11-07; parity verified §1 | Outside copy retired (this record) |
| `eval3.py` + golden-311 → eval tooling | **Landed** 2026-11-07; parity verified §1 | Outside copy retired (this record); the from-new-home run is #14131 |
| `embed_pass3.py`, `embed_onnx.py` → aggregator embedding stage | Not landed as files — by design the destination is aggregator behaviour (§4.3, `fleet serve` search role) | Coverage verification + retirement: #14128 |
| `atlas-ingest.py` → aggregator promotion path | Same as above | #14128 |
| `~/tools/bin/atlas-search` wrapper → fleet `search` domain | Destination exists: the fleet binary has both the `search` domain and the `serve` search role | Production-path proof + wrapper retirement: #14130 |
| `atlas-backup` → `fleet search` backup verb / serve maintenance role | No backup verb exists in the fleet binary or in `pkgs/atlas-search` nix today | Copy reconciliation (nix vs `~/tmp/atlas-backup.sh` vs hatch-personal `atlas-backup.py`), then the verb: #14129 |
| Snowflake model provisioning → #5355 nix path | #5355 is closed; the seat wrapper already provisions the model from hash-verified pins in `~/.local/state/atlas-search/model` | Artifact deletion only: #14132 |

Historical rows:

| §8 row | Status | Remainder |
| --- | --- | --- |
| `~/atlas/v3/swap-2026-11-03/` scripts + `staging.db` (laptop) | Untouched — gated on the in-flight index rewrite completing and verifying | #14132 |
| `hatch-personal/workspace/turso-atlas/docs/` | Archive in place per §8 (hatch-personal is the record); no action | — |
| Transfer artifacts (`atlas-model-rev.tar` 1.2 GB, `atlas-step6-closure.nar` 300 MB, `~/tmp/atlas-migrate/`, `~/tmp/atlas-parts/`) | Laptop-side; not present on this seat | Reproducibility confirmation + delete: #14132 |

## 3. Acceptance state of #5670

1. "Every §8 row has a landed destination or a recorded archive/delete
   commit" — phase-1 rows landed (this record); every other row now has
   a named slice (#14128–#14132) or an in-place archive disposition.
2. "Grep finds no remaining live Atlas script outside this repo" — not
   yet true; it becomes true as #14128–#14130 retire their copies. The
   final grep is #14132's acceptance step.
3. "The eval harness runs from its new home with golden-311 parity" —
   open; filed as #14131 (needs the laptop corpus and a healthy amd-6).

## 4. Notes

- No secret values were copied in this migration: `atlas_search.py`
  resolves credentials by name at runtime (Bitwarden secret names,
  `ATLAS_TURSO_TOKEN` env), in both the outside and in-repo copies.
- No new dependencies. The verification used Python's standard-library
  `ast`/`hashlib`/`json` only.
