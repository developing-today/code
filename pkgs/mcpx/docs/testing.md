# Testing mcpx

What to run when you change mcpx, and what you never run. Each section names the
issue that decides the policy; where this page and an issue disagree, the issue
wins. Agents get the short version from
[`internal/AGENTS.md`](../internal/AGENTS.md), which opencode attaches when a
file under `internal/` is read.

## Never the whole suite locally

`go test ./...` is a CI command. It is minutes of `internal/e2e` for a change
that usually touches one package, and the cost only grows as the suite does.
Run it only to reproduce a CI failure, and then only the packages CI named.
(Decided in #1966; the measurement behind the rule is in #1909.)

## What to run, by change

| You changed | Run | Cost |
| --- | --- | --- |
| anything | nothing — read the diff first | — |
| one package | `go test ./internal/<pkg> -count=1` | seconds |
| a test you just wrote | `go test ./internal/<pkg> -run TestTheName -count=1 -v` | seconds |
| a registry, table or manifest | its guard test (see below), then the package | seconds |
| CLI or daemon behaviour a unit test cannot see | the named e2e tests that cover it: `go test ./internal/e2e -run 'TestThatOne' -count=1` | ~10 s per test |
| a shared helper used by e2e | `go test ./internal/e2e -count=1` | ~2 min |
| anything, before merging | `go test ./... -count=1` in CI, not on your machine | ~3 min |

[`docs/testing/inventory.md`](./testing/inventory.md) is the per-package map:
test counts, which packages hide time, randomness or ports, and which are
flake-prone. Check it before picking e2e tests — it says where the sleeps are.

## A test per bug

Every bug, from any source, gets at least one new test **in the same PR as the
fix**, and the test fails without the fix. Cover the bug's permutations, not
just the input that tripped it: a parser bug gets the neighbouring shapes, a
state bug gets the transitions either side of it. Skip the test only when
writing it is genuinely expensive, and say so in the PR body.
(Decided in #1969.)

## Guards, not instances

When the bug is one member of a class — a declaration and its use that live
apart, a registry whose entries must each satisfy something — the test must
fail for every member of the class, present and future, not the instance you
found. Iterate the registry; do not hand-list entries. A setting must be able
to change behaviour, not merely be read. (Decided in #1971.)

Existing examples to copy: `internal/settings/compendium_test.go` (every
markdown file is placed in the book), `internal/settings/registry_test.go`
(every setting is documented), `internal/conformance` (every protocol cell has
a covering test).

## Flakes

A flake is a bug in the test or the code under it; a retry hides it.

- Look for them with `-race`, `-shuffle=on` and repeat runs:
  `go test ./internal/<pkg> -race -shuffle=on -count=5`.
- Do this on changed tests in the PR, and on everything nightly — not on the
  full suite locally.
- When you find one: fix it. If you cannot yet, quarantine it with a skip that
  names the issue and an expiry date, and file the issue. A quarantine without
  an owner and a date is a silent retry with extra steps.
- CI is Linux only; a macOS-only difference is a flake source CI cannot see
  (#35). Say so in the PR when the change is platform-sensitive.
(Decided in #1973.)

## Reproducing a CI failure

1. Read the failing job, not just the summary — the log names the package and
   the test.
2. Run exactly that, whole-package if the failure could be shared state:
   `go test ./internal/<pkg> -run '<Test>' -count=1 -v`
3. Add `-race` if the failure looks like a data race, `-count=5` if it passed
   once locally.
4. If it only fails in CI, match the Go version in `go.mod` and check whether
   the test is order-dependent (`-shuffle=on`).

## Policy status

The policies above are stated as decided so agents have one rule set, but the
issues are still open and may refine them: #1909 (the policy bundle), #1966
(which tests), #1969 (a test per bug), #1971 (guards), #1973 (flakes). If you
settle one, update this page in the same PR.

## Keeping this page honest

Every command quoted here must exist and run. When a recipe is renamed, the
rename edits this page and `internal/AGENTS.md` in the same PR.
