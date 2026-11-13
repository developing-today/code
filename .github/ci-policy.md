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

## Release gate (#1782)

Tag-based publishing is fail-closed. Before GoReleaser or the mcpx release
workflow can build/publish, `.github/scripts/release-gate.sh` requires the
tag's commit to be the current `main` head and requires a completed
`success` verdict from every main-tier workflow listed in
`.github/workflows/main-broken.yml`'s `workflow_run.workflows` trigger,
judged by trigger class (verified against the workflow files on main,
2026-11-13):

- Unfiltered push workflows (`id`): the latest push-to-main run on the
  exact release SHA. An older run never judges newer code (Codex P1 on
  PR #4636).
- Path-filtered push workflows (`mcpx`, `mcpx conformance`, `mcpx nix
  build`, `go modules build`, `identity build`, `map build`, `id bench`):
  an exact-SHA push run when one exists, else the latest push-to-main
  run — a commit outside their paths legitimately never triggered them.
- Non-push workflows (`nix ci`, schedule/workflow_dispatch only): the
  latest completed run on main, of any event, whose head SHA is the
  release SHA or an ancestor of it (decision on #1782, 2026-11-13,
  option (b) of issue note 6782949534). The judged code is always
  contained in the release; these workflows have no push runs, so a
  push-only rule would refuse every tag release permanently.

The script keeps its watched-workflow lists in sync with
`main-broken.yml` and the workflow triggers explicitly; if a watched
workflow is renamed or its trigger class changes, update both in the
same change. A failed, cancelled, skipped, or in-progress verdict — or
a workflow with no qualifying runs at all — refuses the release. A tag
on a stale commit is refused even if its older runs were green. The
checks use the Actions API and require `actions: read` permission.
Residual risk: a path-filter gap (a commit that should have triggered
a watched workflow but did not) is judged on that workflow's older
run — keep path filters accurate.

`workflow_dispatch` remains a non-publishing snapshot path and does not
weaken the tag gate. There is no routine bypass input or secret switch.
For an exceptional hotfix where waiting for the normal main-tier run is
unacceptable, the override path is: get Drew's explicit approval recorded
on #1782; make a reviewed, temporary change to the gate scoped to the
exact full commit SHA and documented reason; release only that
current-main SHA; then remove the exception in a follow-up PR. Never
add a wildcard or a standing "skip gate" switch. The normal recovery
path for missing or red checks is to fix forward on main, wait for all
watched checks to succeed, and tag that green main head.
