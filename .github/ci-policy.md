# CI tier policy — fast PR tier, full main tier

Design: developing-today/code#1779 (ci-fast-pr-full-main). Phases:
#1780 (cancel-in-progress), #1781 (this policy), #1782 (main-breakage
chain, .github/workflows/main-broken.yml). Tracker: #1599.

Every workflow touched by the tier redesign carries the header
comment . Tuning the tiers
means editing THIS table by PR — never ad-hoc pipeline changes.

## Change classes (§3.3 of the design)

| class | trigger paths (examples) | PR tier |
|---|---|---|
| 0 — prose | , docs/, comments-only changes | nothing, or markdown lint only |
| 1 — config/data |  non-build,  (non-workflow) | syntax parse + lint of changed files |
| 2 — package source (default) | one package's src + tests | lint/format + typecheck/syntax of affected packages + affected unit tests |
| 3 — elevated | lockfiles, / roots, /, build definitions (), , shared/core libraries | class 2 + compile check of affected packages |
| 4 — nix expressions |  | class 1 + parse/eval of the changed files only (never full flake check on a PR) |

Default when a change spans classes: the highest class wins. The
policy ships conservative (more class 3) and loosens as the
main-breakage rate (issues labelled ) proves out.

## Tier assignment in this repo (after #1781)

| workflow | PR tier | main tier | nightly |
|---|---|---|---|
| nix ci | treefmt/format gate only | treefmt + full flake-check | schedule in nix-ci.yml |
| id | guard + affected-crate matrix () | + full  job | schedule in id-check.yml + id-e2e-nightly.yml (full flake check) |
| mcpx conformance | spec-parity leg only (measured 28s exec, run 40801660942, 2026-10-29 — under the 5m fast-leg limit) | all conformance legs + records + spec-parity | — (main covers it) |
| id bench | — (removed) | yes | — |
| go modules build / identity build / map build / mcpx nix build | class-3 paths only (see each workflow) | always (push to main, no path filter) | — |
| mcpx, lab-ops-hub, mod-tidy, research-workflow | unchanged path-gated PR checks (already PR-tier weight) | push to main | — |

## Coverage rule

Nothing removed from PRs may vanish: the union of main-tier +
nightly jobs must cover the old PR matrix. For pkgs/id the full
flake check runs on main (id-check full-flake-check job) and
nightly twice over (id-check schedule, id-e2e-nightly). For nix ci
the flake-check job runs on main and on the nix-ci nightly
schedule. mcpx conformance legs run on every mcpx-touching merge
to main.

## Dedup rule

A commit must not run the same suite twice via push +
pull_request. Standard shape:  for PRs, 
restricted to  (plus tags where already
applicable, e.g. goreleaser).

## Concurrency rule (#1780)

Standard block (design §4): group keyed by  for
push-to-main, by  otherwise, 
— PR/branch runs cancel on supersede; main runs are never
cancelled. Exceptions:  (load-bearing cip=false, #304)
and scheduled singleton/deploy workflows (cip=false) are NOT
touched. Issue-event workflows (auto-assign) use a per-issue group
so cancel-in-progress can only supersede a run for the same issue.
