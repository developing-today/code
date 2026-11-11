#!/usr/bin/env bash
# Release gate — refuse to publish from a SHA whose main-gate is not green.
#
# Phase 3 (#1782) step 7: publish/release workflows must not cut a release
# from a red main. This script is the shared check run by
# .github/workflows/goreleaser.yml and .github/workflows/mcpx-release.yml
# before anything publishes.
#
# The main-gate is the set of main-tier workflows watched by
# .github/workflows/main-broken.yml (its `workflow_run.workflows` list).
# Keep WATCHED_WORKFLOWS below in sync with that list: renaming a watched
# workflow requires updating BOTH files.
#
# Rule (fail closed; see .github/ci-policy.md "Release gate"):
#   1. The release SHA must be the current head of main. Tag pushes that
#      point anywhere else are refused — a tag on a stale or detached SHA
#      has no main-tier runs to judge it by.
#   2. Every watched workflow must report a completed `success` verdict:
#      its latest push-to-main run on that exact SHA when one exists, else
#      its latest push-to-main run on main. Path-filtered workflows (id,
#      id bench, mcpx, mcpx conformance) legitimately have no run on SHAs
#      outside their paths — requiring an exact-SHA run from every workflow
#      would refuse every ordinary component release — so the fallback
#      judges main health for exactly those workflows. Missing (no runs at
#      all), in-progress, or non-success verdicts refuse the release.
#      Residual risk: a path-filter gap (a commit that should have triggered
#      a watched workflow but did not) is judged on an older run. Keep the
#      path filters in the watched workflows accurate.
#
# There is deliberately NO bypass flag on the tag path. The override path
# is procedural (fix forward on main, wait for green, re-tag; a genuine
# emergency needs a Drew decision recorded on #1782) — see ci-policy.md.
#
# Usage:
#   release-gate.sh [--audit] [SHA]
#     SHA defaults to $GITHUB_SHA, else `git rev-parse HEAD`.
#     --audit prints the verdict but always exits 0; diagnostic only,
#     never use it to authorize a publishing job.
#   release-gate.sh --self-test
#     No network. Checks argument handling and the verdict logic against
#     fixtures (exit 0 = all pass).
#
# Env: GH_REPO (default $GITHUB_REPOSITORY), GH_TOKEN, GITHUB_EVENT_NAME,
#      MAIN_BRANCH (default main). Requires the `gh` and `jq` CLIs.
set -euo pipefail

MAIN_BRANCH="${MAIN_BRANCH:-main}"

# Must match main-broken.yml `workflow_run.workflows` exactly.
WATCHED_WORKFLOWS=(
  "nix ci"
  "id"
  "mcpx"
  "mcpx conformance"
  "mcpx nix build"
  "go modules build"
  "identity build"
  "map build"
  "id bench"
)

# Watched workflows whose push-to-main trigger is path-filtered (verified
# against the workflow files on main, 2026-11-10: mcpx.yml,
# mcpx-conformance.yml, id-bench.yml). Only these may fall back to their
# latest push-to-main run when the release SHA has no run of their own:
# a commit outside their paths legitimately never triggered them. Every
# other watched workflow must produce a push run on the exact release
# SHA — accepting an older run for an unfiltered workflow would let a
# release through on a verdict that predates the tagged code (CodeRabbit
# finding on PR #4636). Keep this list in sync with the push triggers.
PATH_FILTERED_WORKFLOWS=(
  "mcpx"
  "mcpx conformance"
  "id bench"
)

is_path_filtered() {
  local candidate="$1" wf
  for wf in "${PATH_FILTERED_WORKFLOWS[@]}"; do
    [ "$wf" = "$candidate" ] && return 0
  done
  return 1
}

die() { echo "release-gate: error: $*" >&2; exit 2; }
verdict_fail() { echo "release-gate: REFUSED: $*"; }
verdict_pass() { echo "release-gate: ALLOWED: $*"; }

# verdict_for_event <event> <release_sha> <main_sha>
# Pure logic for the head check; exit 0 = head OK, 1 = refused.
verdict_for_event() {
  [ "$2" = "$3" ] || return 1
  return 0
}

# verdict_for_conclusion <conclusion> <status>
# Pure logic for one workflow's latest run; exit 0 = green.
verdict_for_conclusion() {
  [ "$2" = "completed" ] && [ "$1" = "success" ]
}

self_test() {
  local failures=0
  check() { # check <desc> <expected_exit> -- <args...>
    local desc="$1" expected="$2"; shift 2
    [ "${1:-}" = "--" ] && shift
    local got=0
    "$@" >/dev/null 2>&1 || got=$?
    if [ "$got" = "$expected" ]; then
      echo "ok: $desc"
    else
      echo "NOT OK: $desc (expected exit $expected, got $got)"
      failures=$((failures + 1))
    fi
  }
  check "same SHA passes head check" 0 -- verdict_for_event push abc123 abc123
  check "stale SHA fails head check" 1 -- verdict_for_event push abc123 def456
  check "success/completed is green" 0 -- verdict_for_conclusion success completed
  check "failure/completed is red" 1 -- verdict_for_conclusion failure completed
  check "cancelled/completed is red" 1 -- verdict_for_conclusion cancelled completed
  check "skipped/completed is red" 1 -- verdict_for_conclusion skipped completed
  check "success/in_progress is red" 1 -- verdict_for_conclusion success in_progress
  check "empty conclusion is red" 1 -- verdict_for_conclusion "" completed

  # Exercise the full script against a tiny fake gh API. This proves that
  # failed/missing/pending runs refuse the SHA, that an all-green
  # current-main SHA passes, and that a workflow with no run on the SHA
  # falls back to its latest main run (green passes, red refuses) —
  # all without network access or publishing anything.
  local tmp
  tmp="$(mktemp -d)"
  trap 'rm -rf "$tmp"' RETURN
  mkdir -p "$tmp/bin"
  cat >"$tmp/bin/gh" <<'FAKE_GH'
#!/usr/bin/env bash
set -euo pipefail
endpoint="${2:-}"
case "$endpoint" in
  */branches/main)
    printf '{"commit":{"sha":"success-sha"}}\n'
    ;;
  */actions/workflows\?per_page=100)
    cat <<'JSON'
{"workflows":[{"id":1,"name":"nix ci"},{"id":2,"name":"id"},{"id":3,"name":"mcpx"},{"id":4,"name":"mcpx conformance"},{"id":5,"name":"mcpx nix build"},{"id":6,"name":"go modules build"},{"id":7,"name":"identity build"},{"id":8,"name":"map build"},{"id":9,"name":"id bench"}]}
JSON
    ;;
  */actions/workflows/*/runs\?*)
    id="${endpoint#*/actions/workflows/}"
    id="${id%%/*}"
    case "${GH_FIXTURE:-success}:$id" in
      failed:1)
        printf '{"workflow_runs":[{"head_sha":"success-sha","conclusion":"failure","status":"completed","created_at":"2026-11-02T00:00:00Z","html_url":"https://example.invalid/run/1"}]}\n' ;;
      pending:1)
        printf '{"workflow_runs":[{"head_sha":"success-sha","conclusion":null,"status":"in_progress","created_at":"2026-11-02T00:00:00Z","html_url":"https://example.invalid/run/1"}]}\n' ;;
      missing:1)
        printf '{"workflow_runs":[]}\n' ;;
      fallback:3)
        printf '{"workflow_runs":[{"head_sha":"older-sha","conclusion":"success","status":"completed","created_at":"2026-11-01T00:00:00Z","html_url":"https://example.invalid/run/2"}]}\n' ;;
      fallback-red:3)
        printf '{"workflow_runs":[{"head_sha":"older-sha","conclusion":"success","status":"completed","created_at":"2026-11-01T00:00:00Z","html_url":"https://example.invalid/run/2a"},{"head_sha":"older-sha-2","conclusion":"failure","status":"completed","created_at":"2026-11-01T12:00:00Z","html_url":"https://example.invalid/run/2b"}]}\n' ;;
      *) printf '{"workflow_runs":[{"head_sha":"success-sha","conclusion":"success","status":"completed","created_at":"2026-11-02T00:00:00Z","html_url":"https://example.invalid/run/%s"}]}\n' "$id" ;;
    esac
    exit 0
    ;;
  *) echo "unexpected fake gh endpoint: $endpoint" >&2; exit 9 ;;
esac
FAKE_GH
  chmod +x "$tmp/bin/gh"

  run_gate_case() {
    local fixture="$1" expected="$2" sha="${3:-success-sha}" got=0
    GH_FIXTURE="$fixture" GH_TOKEN=test GH_REPO=owner/repo GITHUB_EVENT_NAME=push \
      PATH="$tmp/bin:$PATH" bash "$0" "$sha" >"$tmp/output" 2>&1 || got=$?
    if [ "$got" = "$expected" ]; then
      echo "ok: full gate fixture $fixture (exit $got)"
    else
      echo "NOT OK: full gate fixture $fixture (expected exit $expected, got $got)"
      cat "$tmp/output"
      failures=$((failures + 1))
    fi
  }
  run_gate_case success 0
  run_gate_case failed 1
  run_gate_case missing 1
  run_gate_case pending 1
  run_gate_case fallback 0
  run_gate_case fallback-red 1
  run_gate_case success 1 stale-sha

  # Argument handling (no network): unknown flags fail fast.
  local arg_status=0
  bash "$0" --bogus >/dev/null 2>&1 || arg_status=$?
  if [ "$arg_status" = 2 ]; then
    echo "ok: --bogus flag rejected"
  else
    echo "NOT OK: --bogus flag exit (expected 2, got $arg_status)"
    failures=$((failures + 1))
  fi
  if [ "$failures" = 0 ]; then
    echo "release-gate self-test: all pass"
    return 0
  fi
  echo "release-gate self-test: $failures failure(s)" >&2
  return 1
}

AUDIT=0
SHA=""

while [ $# -gt 0 ]; do
  case "$1" in
    --audit) AUDIT=1; shift ;;
    --self-test) self_test; exit $? ;;
    --help|-h) sed -n '2,30p' "$0"; exit 0 ;;
    --*) die "unknown flag $1" ;;
    *) [ -z "$SHA" ] || die "only one SHA argument is accepted"; SHA="$1"; shift ;;
  esac
done

GH_REPO="${GH_REPO:-${GITHUB_REPOSITORY:-}}"
[ -n "$GH_REPO" ] || die "GH_REPO (or GITHUB_REPOSITORY) is required"
command -v gh >/dev/null 2>&1 || die "the gh CLI is required"
command -v jq >/dev/null 2>&1 || die "the jq CLI is required"
[ -n "${GH_TOKEN:-}" ] || die "GH_TOKEN is required (in CI: \${{ github.token }})"

if [ -z "$SHA" ]; then
  if [ -n "${GITHUB_SHA:-}" ]; then
    SHA="$GITHUB_SHA"
  else
    SHA="$(git rev-parse HEAD 2>/dev/null)" || die "no SHA given and HEAD is unreadable"
  fi
fi

refuse() {
  verdict_fail "$*"
  echo "release-gate: override path: see .github/ci-policy.md \"Release gate\" —"
  echo "release-gate: fix forward on main, wait for green, re-tag the new head."
  if [ "$AUDIT" = 1 ]; then
    echo "release-gate: audit mode: not blocking (exit 0)."
    exit 0
  fi
  exit 1
}

echo "release-gate: checking $SHA in $GH_REPO (main-gate: ${#WATCHED_WORKFLOWS[@]} watched workflows)"

main_head="$(gh api "repos/$GH_REPO/branches/$MAIN_BRANCH" | jq -r '.commit.sha')" \
  || die "could not read the current $MAIN_BRANCH head"
echo "release-gate: $MAIN_BRANCH head is $main_head"

if ! verdict_for_event "${GITHUB_EVENT_NAME:-push}" "$SHA" "$main_head"; then
  refuse "release SHA $SHA is not the current $MAIN_BRANCH head ($main_head). Tag the current head after it goes green."
fi

failures=0
workflows_json="$(gh api "repos/$GH_REPO/actions/workflows?per_page=100")" \
  || die "could not list repository workflows"
for wf in "${WATCHED_WORKFLOWS[@]}"; do
  wid="$(jq -r --arg name "$wf" '.workflows[] | select(.name == $name) | .id' <<<"$workflows_json" | head -n 1)"
  if [ -z "${wid:-}" ]; then
    verdict_fail "$wf: no workflow with that name (renamed? update release-gate.sh AND main-broken.yml together)"
    failures=$((failures + 1))
    continue
  fi
  runs_json="$(gh api "repos/$GH_REPO/actions/workflows/$wid/runs?branch=$MAIN_BRANCH&event=push&per_page=100")" \
    || die "could not list runs for $wf"
  run_json="$(jq -c --arg sha "$SHA" '[.workflow_runs[] | select(.head_sha == $sha)] | sort_by(.created_at) | last // empty' <<<"$runs_json")"
  basis="exact-SHA run"
  if [ -z "$run_json" ]; then
    if ! is_path_filtered "$wf"; then
      # Not path-filtered: this workflow runs on every push to main (or is
      # expected to), so a missing exact-SHA run means its verdict on the
      # tagged code does not exist yet — refuse rather than judge an
      # older run.
      verdict_fail "$wf: no push run on $SHA (exact-SHA run required; workflow is not path-filtered)"
      failures=$((failures + 1))
      continue
    fi
    # No push run on this SHA: normal for a path-filtered workflow when the
    # release commit is outside its paths. Fall back to that workflow's
    # latest push-to-main run — its current verdict on main. The SHA is
    # already pinned to the main head above, so this judges main health.
    run_json="$(jq -c '[.workflow_runs[]] | sort_by(.created_at) | last // empty' <<<"$runs_json")"
    basis="latest-main fallback (no push run on $SHA)"
  fi
  if [ -z "$run_json" ]; then
    verdict_fail "$wf: no push-to-main runs at all (missing main-gate signal)"
    failures=$((failures + 1))
    continue
  fi
  conclusion="$(jq -r '.conclusion // empty' <<<"$run_json")"
  status="$(jq -r '.status // empty' <<<"$run_json")"
  url="$(jq -r '.html_url // empty' <<<"$run_json")"
  run_sha="$(jq -r '.head_sha // empty' <<<"$run_json")"
  if verdict_for_conclusion "$conclusion" "$status"; then
    echo "release-gate: green ($basis): $wf ($url)"
  else
    verdict_fail "$wf: conclusion=${conclusion:-none} status=${status:-none} run-sha=${run_sha:-none} [$basis] ($url)"
    failures=$((failures + 1))
  fi
done

if [ "$failures" != 0 ]; then
  refuse "$failures main-gate workflow(s) not green for $SHA."
fi

verdict_pass "$SHA is the current $MAIN_BRANCH head with all ${#WATCHED_WORKFLOWS[@]} main-gate workflows green."
