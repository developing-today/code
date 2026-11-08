# mcpx test inventory (deliverable 1 of #1960)

Recounted on current `main` at `dd2c88ff` (2026-10-30) for
[#1960](https://github.com/developing-today/code/issues/1960).
Supersedes the issue body's table, which was counted at `05c78b2`
(735 tests, 64 files, nix tree, pre-re-home).

## How it was counted

From `pkgs/mcpx/` on the commit above:

- Test files: `find . -name '*_test.go' | wc -l` → **306 files**
- Test functions: `grep -rhoE '^func (Test[A-Za-z0-9_]+)' --include='*_test.go' | wc -l` → **1626 `Test*` funcs**
- Fuzz functions: `grep -rlE '^func Fuzz' --include='*_test.go' | wc -l` → **0**
- Flags per package: files matching `time\.Sleep`, `time\.Now`,
  `\[\]struct`, `math/rand`, `crypto/rand`, `net\.Listen`,
  `httptest\.`, `t\.Parallel` in `*_test.go` (file-level counts).

The old 735 vs new 1626 is growth plus method: the old count predates
the re-home and several large suites (`daemon` 151, `cli` 103,
`conformance` 18, `pool` 76). Subtests (`t.Run`) are not counted
separately here.

Headline deltas since 05c78b2: `time.Sleep` in **59 files** (was 9),
`time.Now` in **60 files** (was 14), `[]struct` tables in **76 files**,
still **zero** `t.Parallel()` and **zero** `Fuzz*` functions.

## Per-package table

`tests` = `Test*` funcs / `files` = `*_test.go` files / `sleep|now|table`
= files matching `time.Sleep` / `time.Now` / `[]struct`.

| Package | Tests / files | Generator candidates | Hidden time / randomness / ports | Flake-prone |
| --- | --- | --- | --- | --- |
| `internal/e2e` | 339 / 55 | `exec.flagset` (`TestEveryExecFlagChangesSomething` table); `api.v1request` (`v1ops_test.go`) | sleeps in 15 files; `time.Now` in 13; `consumer_test.go:328,368` 60 s `time.After` waits, `:389-400` 30 s deadline + 200 ms poll; `v1ops_test.go:421-430` 5 s deadline + 100 ms poll; `crypto/rand` input bytes (`cliops_test.go:111`, round-trip blob — input only, not asserted); `math/rand` jitter (`keepalive_takeover_test.go:253`, `rand.N(40ms)` sleep — unseeded) | yes — highest concentration; daemon-start timeouts under parallel `go build` load (see `testsupport` note) |
| `internal/daemon` | 151 / 40 | request shapes | sleeps in 7 files; `math/rand` jitter (`handoff_conns_test.go:289`); unix-socket listens (`paths_resolve_test.go:56`, `routes_resolve_test.go:149` — no TCP ports) | sleep-jittered handoff timing |
| `internal/mcpserver` | 126 / 23 | request shapes | `server_test.go:656-672,742`: 2 s `time.After` deadlines, 2 s poll loop with 20 ms sleeps (same shape as at 05c78b2, moved lines) | poll-loop deadlines |
| `internal/config` | 133 / 14 | `config.layers` (merge/scope: `merge_test.go:54-177` hand-picked layer stacks, 7 focused tests) | none | no |
| `internal/cli` | 103 / 31 | — | sleeps in 4 files; unix-socket listens (`stopall_test.go:88`, `ssh_endpoint_test.go:82`, `stdio_proxy_test.go:20` — no TCP ports) | daemon-lifecycle timing |
| `internal/mcpclient` | 87 / 24 | — | sleeps in 7 files (`modern_test.go:365` 10 ms); one TCP listen on ephemeral port (`probe_test.go:605`, `127.0.0.1:0` — fine) | short-sleep sync (`modern_test`) |
| `internal/pool` | 76 / 18 | — | `pool_test.go:519-520`: `Sleep(50ms)` then `ReapIdle(time.Now())` — still timing-dependent; `ReapIdle` already takes `now`, so a fixed clock is a one-line change (cheapest fix, unchanged since 05c78b2) | yes — `pool_test.go:519` |
| `internal/settings` | 54 / 13 | `settings.precedence` (`registry_test.go` iterates the registry) | env reads | no |
| `internal/codegen` | 48 / 5 | `codegen.jsonschema` (one schema per test) | none | no |
| `internal/logging` | 38 / 4 | redaction inputs (`redact_test.go` table) | `file_test.go`: timestamps from `time.Now().Truncate(time.Hour)` — hour/day-boundary sensitivity stands | boundary-sensitive, not sleep-flaky |
| `internal/launcher` | 31 / 2 | `launcher.template` (`define_test.go`, `template_test.go`) — smallest proposed first generator | none | no |
| `internal/elicit` | 29 / 4 | — | `elicit_test.go:53` 50 ms, `:264` 600 ms, `:300` 450 ms sleeps | yes — three sleep-then-assert sites |
| `internal/logstore` | 28 / 4 | — | `time.Now` in 2 files, no sleeps | no |
| `internal/diagnose` | 21 / 2 | `diagnose` parsers | `time.Now` in 1 file | no |
| `internal/recipes` | 20 / 3 | — | none | no |
| `internal/registry` | 19 / 2 | — | `paging_test.go:57` `Sleep(f.delay)` (fault-injection delay — intentional, not flake) | no |
| `internal/adapter` | 19 / 1 | — | none | no |
| `internal/api` | 18 / 5 | — | none | no |
| `internal/conformance` | 18 / 11 | — | sleeps in 7 files (client-features/lifecycle/messages suites) | sleep-then-assert across suites |
| `internal/artifacts` | 16 / 2 | `artifacts/name` parsers | `time.Now` in 1 file | no |
| `internal/openapi` | 16 / 1 | `openapi` parsers | `httptest` only (localhost, ephemeral — fine) | no |
| `internal/secrets` | 15 / 3 | — | none | no |
| `internal/tui` | 15 / 1 | — | `time.Now` in 1 file | no |
| `internal/searchpath` | 14 / 1 | `searchpath` parsers | none | no |
| `internal/schemaval` | 13 / 1 | schema shapes | none | no |
| `internal/mcpauth` | 13 / 2 | — | none | no |
| `internal/execsvc` | 13 / 2 | — | 1 sleep + `time.Now` file | watch timing |
| `internal/source` | 12 / 1 | — | none | no |
| `internal/hooks` | 11 / 2 | — | 1 sleep + `time.Now` file | mild |
| `internal/mcpspec` | 11 / 1 | — | none | no |
| `internal/plugin` | 11 / 2 | — | none | no |
| `internal/runner` | 11 / 3 | — | `time.Now` in 1 file | no |
| `internal/cache` | 9 / 1 | — | `time.Now` in 1 file (TTL — candidate for fixed clock) | no |
| `internal/preflight` | 9 / 1 | — | none | no |
| `internal/eventstore` | 8 / 1 | — | none | no |
| `internal/tasks` | 7 / 1 | — | `tasks_test.go:16` 20 ms sleep | mild |
| `internal/scripting` | 7 / 1 | — | none | no |
| `internal/defaults` | 6 / 3 | — | none | no |
| `internal/nearmiss` | 5 / 1 | — | none | no |
| `internal/refdoc` | 5 / 1 | — | none | no |
| `internal/spec` | 5 / 1 | — | none | no |
| `internal/sdnotify` | 4 / 1 | — | unixgram listen (`sdnotify_test.go:16` — no ports) | no |
| `internal/testsupport` (+`taskmcp`) | 3+3 / 1+1 | — | `build.go`: `FakeMCPBinary`/`MCPXBinary` compile once per test binary into `os.MkdirTemp("mcpx-fake-" …)` / `("mcpx-bin-" …)` dirs that are **never removed** (no `RemoveAll`, no `t.TempDir`) — the #129 temp-dir candidate stands; the file's own comment notes parallel `go build` load pushing start timeouts over the edge | load-induced timeouts (environmental) |
| `internal/opencode` | 2 / 1 | — | none | no |
| `internal/buildcheck` | 2 / 2 | — | none | no |
| `internal/wasmtools` | 3 / 1 | — | none | no |
| `internal/catalog`, `internal/events`, `internal/mcpheaders`, `internal/dashboard`, `examples` | 7/0/0/0/0 | — | none | no |

Totals: **1626 tests, 306 files**; 59 files `time.Sleep` (143 hits);
60 files `time.Now` (203 hits); 76 files `[]struct` (120 hits);
40 files `httptest` (all localhost/ephemeral — fine); 8 files
`net.Listen` (7 unix/unixgram, 1 ephemeral TCP — fine); 2 files
`math/rand` (sleep jitter); 1 file `crypto/rand` (test input, not
asserted); 0 `t.Parallel`; 0 `Fuzz*`.

## Generator shortlist (unchanged direction, refreshed refs)

Pure-function, table-shaped, highest yield first: `codegen.jsonschema`,
`launcher.template`, `config.layers` (`merge_test.go:54-177`),
`settings.precedence`, redaction inputs (`redact_test.go`), parsers
(`openapi`, `searchpath`, `schemaval`, `artifacts/name`, `diagnose`),
`exec.flagset`, `api.v1request`. No `Fuzz*` twins exist yet anywhere.

## Cheapest flake fixes (proof-the-inventory-pays candidates)

1. `internal/pool/pool_test.go:519-520` — pass a fixed `now` to
   `ReapIdle` instead of sleeping 50 ms (one-line change; the seam
   already exists).
2. `internal/tasks/tasks_test.go:16`, `internal/mcpclient/modern_test.go:365`
   — 10–20 ms sleep-then-assert; replace with channel/poll-with-deadline.
3. `internal/testsupport/build.go` — the `mcpx-bin-*`/`mcpx-fake-*` dirs
   are never removed (#129 candidate), but the fix is NOT
   `t.Cleanup(RemoveAll)` inside the helpers: the binaries are cached
   behind package-global `sync.Once`, so test-scoped cleanup would delete
   the shared binary while later tests still need it. Per-test temp dirs
   should use `t.TempDir`; the shared binaries need package-lifetime
   cleanup (e.g. a `TestMain` hook) or a cache redesign first.

## Not in this slice (per packet scope)

Seed helper / wrapper / first generator (deliverable 2 — waits on
#1949 seed format and #1956 engine choice); policy text (deliverable 3 —
belongs with the #99-successor policy issue). No test or production code
was changed for this inventory.

## Open question for #99

This file lives at `pkgs/mcpx/docs/testing/inventory.md` as the packet's
fallback home. If #99 picks a different docs home, move it there.
