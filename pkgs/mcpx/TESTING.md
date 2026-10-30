# mcpx — which tests to run (agent test tiers)

Decided in developing-today/code#1966. This file is the policy text; the
contributor page (#1976) carries it verbatim. The selector that applies
it mechanically is `scripts/affected-tests.py`, exposed from the repo
root as `just mcpx-test-affected [base] [--run]`.

Run the smallest set that covers your change. The full suite is CI's
job: it runs on every push (`.github/workflows/mcpx.yml`,
`go test ./... -count=1`) and an agent must read the PR's check result
before declaring done or merging.

## The tiers

| Tier | When | Command | Typical cost |
| --- | --- | --- | --- |
| T0 format/vet | always, touched packages | `gofmt -l <files>`; `go vet ./internal/<pkg>/` | < 2 s |
| T1 unit | always, each package with an edited `.go` file | `go test -count=1 ./internal/<pkg>/` | < 2 s each (tui ~9 s) |
| T1b dependents | only when an *exported* identifier changed | T1 for the packages that import it | seconds |
| T2 targeted e2e | when behaviour visible through the binary changed | `go test -count=1 ./internal/e2e -run '^(TestA\|TestB)$'` | budget below |
| T3 guards | when a registry was touched (flag, `/v1` op, setting, default) | the guard tests for that registry (the selector's guard map) | seconds |
| T4 build | only when `package.nix`, `go.mod`/`go.sum` or the vendor hash changed | `nix build .#mcpx` | ~70 s |
| Full suite | **never locally for a routine change**; CI on push. The integrator merging a PR is the exception (rule 7) | — | ~110 s of serial e2e plus compilation, and growing |

**T2 budget (decided #1966):** both numbers, serving different readers —
a **count cap of 15 tests**, which is what an agent can see before
running, and a **printed time estimate** (~0.5 s/test + one binary
build, ~30 s at the cap), which is what actually matters. The selector
truncates at the cap and says so; CI runs the rest.

## The rules

1. **Never run** `go test ./...` or an unfiltered
   `go test ./internal/e2e` locally, unless the user asks for it by name
   or you are the integrator merging (rule 7). The reason is cost:
   ~110 s of serial e2e plus compilation, paid again on every run, and
   the number grows with every e2e test.
2. **Always `-count=1`** for e2e. Confirmed on 2026-10-30 (see
   "Stale-cache check" below): Go's test cache does not track the files
   the binary-under-test build subprocess reads, so a cached e2e result
   can be stale green. For unit packages the cache is correct and
   welcome.
3. **Push, then read CI.** The full run is CI's job; read the PR's
   check result before declaring done or merging.
4. **Harness changes** (`internal/testsupport/**`, `newEnv`, the fake
   servers) affect every e2e test: run the sample
   `^(TestNoOrphanProcessesAfterStop|TestEnvCleanupStopsTheDaemonItStarted|TestEveryDeclaredRouteAnswersOverTheSocket)$`
   locally and let CI run the rest. The selector adds this sample
   automatically.
5. **Reproducing a CI failure:** re-run only the failing test(s), by
   exact anchored name (`-run '^(TestA|TestB)$'` — `-run` is an
   unanchored regexp), with the same run seed and shuffle order the CI
   shard printed. If it does not reproduce once, treat it as a possible
   flake under the flake policy (#1973), not by re-running the whole
   suite.
6. Local e2e runs no longer leak daemons: `newEnv` sets
   `MCPX_AUTOSTART_IDLE_EXIT=60s` itself and cleans up with
   `stop --all`, so there is nothing to export or reap.
7. **The integrator's pre-merge run is the exception, and it is
   deliberate.** Whoever merges a PR runs the full suite locally more
   than once (current practice: 2–3 times). Until repeat/`-race` runs
   move into CI (#1973's territory), that is the only place mcpx looks
   for order- and timing-dependent flakes.

## Finding the tests that touch a change (Approach C, decided #1966)

`scripts/affected-tests.py` (dry-run by default; `--run` executes):

1. Diff the working tree against the base (default `origin/main`,
   committed + uncommitted + untracked), scoped to `pkgs/mcpx`.
2. Each edited `.go` file → its package → T1. If exported identifiers
   changed, add the importing packages (T1b, via a `go list` reverse
   lookup).
3. For e2e: take the *names* the diff touches in registry files —
   setting paths in `internal/settings/wire.go`, op names in
   `internal/api/ops*.go`, flag and command names in `internal/cli` —
   and select e2e test functions whose bodies mention them; add the
   guard tests for any registry touched (T3).
4. Print the commands by tier with the T2 estimate. If nothing is
   selected it prints "no e2e selected; CI runs the full suite" — it
   never falls back to all.

Judgment-and-grep (Approach A) is the documented fallback when the
selector selects nothing for a change you know is visible through the
binary: grep `internal/e2e` for the feature's name, pick those tests,
and run them anchored with `-count=1`.

## Stale-cache check (open question from #1966, confirmed 2026-10-30)

In a scratch worktree: ran
`go test ./internal/e2e -run '^TestEnvCleanupStopsTheDaemonItStarted$'`
(no `-count=1`) twice — second run reported `ok (cached)`. Then edited
a package the e2e test binary does not import (`internal/runner`) and
ran it again: still `ok (cached)`, although the binary under test would
now be built from changed sources. **The hypothesis is confirmed**:
without `-count=1`, a local e2e result can be stale green. Rule 2
stands as written.
