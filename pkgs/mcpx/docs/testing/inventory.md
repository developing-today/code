# mcpx test inventory (deliverable 1 of #1960)

Recounted on current `main` at `32d47f73b` (2026-11-03). This refresh supersedes the 2026-10-30 census at `dd2c88ff` and the older pre-re-home issue table.

## Current census

Scope: Go `*_test.go` files under `pkgs/mcpx/`. The counts are a repeatable source scan, not a judgment that every marker is flaky or unsafe. Test subtests are not counted separately.

- Test files: **468**; `Test*` functions: **2561**; `Fuzz*` functions: **1**.
- `time.Sleep`: **73 files / 178 calls**; `time.Now`: **87 files / 280 calls**; `[]struct` tables: **117 files / 235 matches**.
- `math/rand`: **2 files**; `crypto/rand`: **10 files**; `net.Listen`: **8 files / 10 calls**; `httptest`: **70 files / 317 references**; `t.Parallel`: **0 files / 0 calls**.
- The single native fuzz target is `internal/config/stripjsonc_fuzz_test.go:FuzzStripJSONC`; its corpus runs during ordinary `go test`, while mutation requires `-fuzz`. The package-level table below flags where to inspect; it does not classify every sleep or clock read as a flake.

## Per-package census

Counts are `Test* functions / test files`. Marker columns are matching files/calls; randomness columns are files importing/referencing the package path. `—` means no match. Generator candidates are a shortlist, not implemented generators.

| Package | Tests / files | Fuzz | Sleep files / calls | Now files / calls | Table files / matches | math/rand files | crypto/rand files | net.Listen files / calls | httptest files / refs | Generator candidate |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `internal/e2e` | 356 / 62 | — | 15 / 33 | 14 / 47 | 24 / 49 | 1 | 1 | — | 2 / 2 | exec.flagset; api.v1request |
| `internal/daemon` | 311 / 78 | — | 11 / 25 | 19 / 52 | 22 / 44 | 1 | 1 | 3 / 5 | 35 / 191 | request/operation shapes |
| `internal/config` | 217 / 20 | 1 | — | — | 4 / 37 | — | — | — | — | config.layers; FuzzStripJSONC exists |
| `internal/mcpserver` | 173 / 31 | — | 10 / 23 | 7 / 29 | 7 / 10 | — | 1 | — | 4 / 41 | request shapes |
| `internal/cli` | 170 / 48 | — | 4 / 6 | 2 / 13 | 5 / 5 | — | 1 | 3 / 3 | 5 / 12 | — |
| `internal/mcpclient` | 131 / 36 | — | 10 / 18 | 8 / 28 | 6 / 7 | — | — | 1 / 1 | 8 / 21 | — |
| `internal/runner` | 111 / 7 | — | 1 / 1 | 3 / 5 | 4 / 11 | — | — | — | 1 / 1 | — |
| `internal/pool` | 96 / 23 | — | 6 / 9 | 8 / 28 | 4 / 4 | — | 1 | — | 4 / 16 | — |
| `internal/settings` | 72 / 20 | — | — | — | 5 / 8 | — | — | — | — | settings.precedence |
| `internal/codegen` | 69 / 7 | — | — | — | 1 / 2 | — | — | — | 1 / 5 | codegen.jsonschema |
| `internal/execsvc` | 46 / 11 | — | 2 / 2 | 1 / 1 | 3 / 3 | — | — | — | — | — |
| `internal/logging` | 42 / 5 | — | — | 1 / 5 | 1 / 4 | — | 1 | — | — | redaction inputs |
| `internal/elicit` | 39 / 8 | — | 1 / 2 | 3 / 16 | 2 / 2 | — | 1 | — | — | — |
| `internal/launcher` | 38 / 2 | — | — | — | — | — | — | — | — | launcher.template |
| `internal/artifacts` | 35 / 6 | — | 1 / 1 | 2 / 5 | 1 / 1 | — | 1 | — | — | artifact-name parsing |
| `internal/deps` | 31 / 1 | — | — | — | 1 / 2 | — | — | — | — | — |
| `internal/logstore` | 31 / 5 | — | — | 2 / 4 | — | — | — | — | — | — |
| `internal/tasks` | 31 / 4 | — | 1 / 13 | 2 / 5 | 1 / 1 | — | 1 | — | — | — |
| `internal/adapter` | 30 / 2 | — | — | — | — | — | — | — | — | — |
| `internal/diagnose` | 30 / 3 | — | — | 1 / 8 | 1 / 2 | — | — | — | — | diagnostic parser inputs |
| `internal/preflight` | 30 / 2 | — | — | — | — | — | — | — | — | — |
| `internal/recipes` | 28 / 3 | — | — | — | 1 / 3 | — | — | — | — | — |
| `internal/api` | 26 / 8 | — | — | — | — | — | — | — | — | — |
| `tools/docs-history` | 25 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/registry` | 22 / 2 | — | 1 / 1 | 1 / 1 | 2 / 4 | — | — | — | 2 / 4 | — |
| `internal/conformance` | 21 / 14 | — | 7 / 37 | 4 / 11 | 4 / 4 | — | — | — | 4 / 8 | — |
| `internal/hooks` | 21 / 4 | — | 2 / 5 | 2 / 4 | — | — | — | — | — | — |
| `internal/source` | 21 / 3 | — | — | — | 2 / 2 | — | — | 1 / 1 | — | — |
| `internal/tui` | 21 / 2 | — | — | 1 / 2 | — | — | — | — | — | — |
| `internal/openapi` | 20 / 1 | — | — | — | 1 / 1 | — | — | — | 1 / 5 | schema/parser shapes |
| `internal/secrets` | 20 / 3 | — | — | 1 / 3 | 1 / 2 | — | — | — | — | — |
| `internal/mcpauth` | 17 / 3 | — | — | — | 1 / 1 | — | — | — | — | — |
| `internal/cache` | 16 / 2 | — | — | 1 / 4 | — | — | — | — | — | — |
| `internal/events` | 16 / 3 | — | — | — | — | — | — | — | — | — |
| `internal/model` | 15 / 1 | — | — | — | 1 / 2 | — | — | — | 1 / 8 | — |
| `internal/searchpath` | 15 / 1 | — | — | — | — | — | — | — | — | path parser shapes |
| `internal/testsupport` | 15 / 4 | — | — | 1 / 1 | 2 / 2 | — | — | — | — | — |
| `internal/schemaval` | 13 / 1 | — | — | — | 1 / 2 | — | — | — | 1 / 1 | schema shapes |
| `internal/progspec` | 12 / 1 | — | 1 / 2 | 1 / 2 | 1 / 1 | — | — | — | — | — |
| `internal/testsupport/gen` | 12 / 2 | — | — | — | — | — | — | — | — | — |
| `internal/catalog` | 11 / 4 | — | — | — | — | — | — | — | 1 / 2 | — |
| `internal/mcpspec` | 11 / 1 | — | — | — | 1 / 3 | — | — | — | — | — |
| `internal/plugin` | 11 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/scripting` | 9 / 1 | — | — | — | 1 / 3 | — | — | — | — | — |
| `internal/wasmtools` | 9 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/eventstore` | 8 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/nearmiss` | 8 / 1 | — | — | — | 1 / 4 | — | — | — | — | — |
| `internal/defaults` | 6 / 3 | — | — | — | — | — | — | — | — | — |
| `internal/mcpheaders` | 6 / 1 | — | — | — | 1 / 4 | — | — | — | — | — |
| `internal/opencode` | 5 / 2 | — | — | 1 / 4 | — | — | — | — | — | — |
| `internal/policy` | 5 / 1 | — | — | — | 1 / 2 | — | — | — | — | — |
| `internal/refdoc` | 5 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/spec` | 5 / 1 | — | — | — | 1 / 1 | — | — | — | — | — |
| `internal/randsrc` | 4 / 1 | — | — | — | — | — | 1 | — | — | — |
| `internal/sdnotify` | 4 / 1 | — | — | 1 / 2 | — | — | — | — | — | — |
| `internal/buildcheck` | 2 / 2 | — | — | — | — | — | — | — | — | — |
| `internal/probe` | 2 / 1 | — | — | — | 1 / 1 | — | — | — | — | — |
| `internal/seedstore` | 2 / 1 | — | — | — | — | — | — | — | — | — |
| `examples` | 1 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/dashboard` | 1 / 1 | — | — | — | — | — | — | — | — | — |
| `internal/testsupport/taskmcp` | 1 / 1 | — | — | — | 1 / 1 | — | — | — | — | — |

## Generator and flake-review shortlist

- Pure/table-shaped candidates: `codegen.jsonschema`, `launcher.template`, `config.layers`, `settings.precedence`, redaction inputs, parsers (`openapi`, `searchpath`, `schemaval`, `artifacts/name`, `diagnose`), and request/operation shapes in `daemon`/`mcpserver`. Existing tables are candidates to generalize, not a request to fuzz every test.
- Current sleep hotspots by calls/files: `internal/conformance` (37/7), `internal/e2e` (33/15), `internal/daemon` (25/11), `internal/mcpserver` (23/10), `internal/mcpclient` (18/10), `internal/tasks` (13/1), and `internal/pool` (9/6). Inspect each call site for sleep-then-assert before labeling it flaky; intentional delays and deadline polling are not automatically flakes.
- `net.Listen` and `httptest` counts identify tests that bind local sockets. Confirm listeners use ephemeral ports or Unix sockets and are closed; counts alone do not show a port conflict.

## Counting command

Counts above use Go test declarations and simple regex scans of test-file contents. Re-run the named scans above after source changes; review flagged contexts before changing tests.

## Seed and fuzz implementation references

- Seeded generators live in `internal/testsupport/gen`; stream derivation v1 is keyed by run seed, test name, and generator `name@version`. Generators draw only from `gen.Source`; native fuzz adapters use `gen.NewBytesSource(data)`.
- CI run-seed records are stored under `testdata/seeds/v1`; the collector workflow is the writer. The seed-store README defines which run records are retained.
- The fuzz corpus for `FuzzStripJSONC` is under `internal/config/testdata/fuzz/FuzzStripJSONC/`. Ordinary unit tests replay it; `.github/workflows/mcpx-test.yml` runs the bounded mutation smoke. Long fuzz/soak work remains separate.

## History

The prior detailed review was measured at `dd2c88ff` and is superseded by this recount. Its exact file/line references and classifications should be rechecked against current source before reuse.
