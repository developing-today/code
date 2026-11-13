# CI policy — automatic push CI only on main

Owner ruling, 2026-11-13: turn off all push and PR triggers and required
checks that run CI except on main. This supersedes the fast-PR-tier rules
from #1779/#7089 and the November 8 PR-only guard dispositions.

## Merge and verification

No automated PR check is a merge requirement. Reviewers verify the diff and
run relevant focused checks manually before merging. Passing main CI is
post-merge evidence, not a pre-merge gate. Do not wait for a PR check that no
longer runs. Do not auto-enable PR CI to satisfy historical policy text.

Main has no merge queue. Use direct merge commits under the repository's
worktree and review rules. Do not infer authorization to bypass protection.
The November 13 API audit found no rulesets and no required_status_checks
on main protection. Other protection flags were left unchanged.

## Host evaluation (nix-ci `host-eval`)

Decision recorded for #3039 (Drew's answer `host-eval-gate-3039`, applied
2026-11-13): the `host-eval` job in `nix-ci.yml` is **advisory only**. It is
not a required check, it gates no merge, and no merge-queue or
required-status-check route is to be built around it — the November 13 API
audit found no rulesets and no `required_status_checks` on main, and this
section supersedes the merge-queue-era text that once implied otherwise.

`nix-ci` itself runs on schedule (nightly) and `workflow_dispatch` only —
the full flake check, `host-eval` included, is a nightly/manual signal,
not a push or PR gate. The main-tier controls that carry the host-eval
signal instead are:

- the automatic `ci-fix` dispatch when `host-eval` fails
  (commit 43807e56f4, #705), which turns a red host evaluation into filed,
  routed fix work without blocking merges;
- `main-broken.yml`, the main broken watcher, which files or appends main
  breakages from failing main runs;
- the push-to-main suites in the agreement registry below, whose
  post-merge conclusions are the merge evidence for main.

A red `host-eval` therefore means "fix work is filed and routed", never
"main is gated".

## Trigger contract

- Existing push CI is restricted to exactly `branches: [main]`. Preserve
  existing path filters, so this does not activate every suite on every merge.
- No `pull_request`, `pull_request_target`, PR review/comment, or `merge_group`
  triggers. PR-only venues become manual-only, not new deployments on main.
- Tag-push triggers are disabled, including `goreleaser` and `mcpx-release`.
  Their manual snapshot runs remain; automatic tag publication no longer runs.
- Existing schedules, workflow_dispatch inputs, release-published collectors,
  issue-label handoffs and reusable workflow_call contracts remain unchanged.
  A manual dispatch may explicitly select a non-main ref; that is deliberate,
  not automatic push/PR CI. Schedules use GitHub's default-branch behavior.
- `main-broken` keeps its `workflow_run` filter `branches: [main]`; no
  completion from an unrelated branch triggers that chain. Its main-failure
  monitoring and schedule/manual entrypoints remain.
- Do not change Actions enablement, runner/security settings, credentials or
  in-flight runs as part of this trigger policy.

## Agreement registry

The existing guard/build registry remains checked by
`.github/scripts/check-ci-policy-agreement.sh`; its main-only trigger check
also scans the full workflow directory. Main and manual venues are explicit:

| Workflow | Automatic venue | Manual |
| --- | --- | --- |
| workflow-lint | none | yes |
| proxy-port-fallbacks | none | yes |
| fleet-search-parity-guard | none | yes |
| opencode-config-guard | main push | yes |
| mcpx fix-test guard | main push contract tests | yes |
| production-line guard | main push, path gated | yes |
| go modules build | main push, path gated | yes |
| identity build | main push, path gated | yes |
| map build | main push, path gated | yes |
| mcpx nix build | main push, path gated | yes |

Other former PR venues retain their existing main/manual/nightly venues;
`deploy-flake-checks` remains manually runnable. Historical `basic:` job
names are retained for compatibility but no longer mean required PR checks.
The legacy PR-only job conditions remain unreachable and may be cleaned up
separately; this change does not silently enable their publication behavior.

## Coverage and operations

`.github/test-suite-registry.json` remains the suite inventory. A venue being
configured does not prove a test ran: report passed, failed and not-run
separately. Preserve focused local tests, property/model-based invariants,
manual workflow dispatch and existing main/nightly coverage. Historical
PR-tier coverage promises are superseded, not represented as passing.

Concurrency, schedules and main failure reporting are unchanged. Existing
cache fallbacks remain performance optimizations, not correctness verdicts.
The empty `.gitlab-ci.yml` defines no GitLab pipeline here; no cross-provider
settings or other repositories were changed.
