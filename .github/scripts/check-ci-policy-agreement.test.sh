#!/usr/bin/env bash
# Oracle: the November 13 main-only automatic trigger policy.
# Usage: check-ci-policy-agreement.test.sh (no arguments).
set -euo pipefail
[[ $# == 0 ]] || {
  echo 'No arguments accepted' >&2
  exit 2
}
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$BASE/.github/scripts/check-ci-policy-agreement.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/.github/workflows"
cp "$BASE/.github/ci-policy.md" "$TMP/.github/"
# The agreement registry is the intended input for this contract suite. The
# repository-wide guard is checked separately against the real tree.
for workflow in workflow-lint.yml proxy-port-fallbacks.yml fleet-search-parity-guard.yml \
  opencode-config-guard.yml mcpx-fix-test-guard.yml production-line-guard.yml \
  go-modules-build.yml identity-build.yml map-build.yml mcpx-nixbuild.yml; do
  cp "$BASE/.github/workflows/$workflow" "$TMP/.github/workflows/"
done
bash "$SCRIPT" "$TMP"
expect_fail() { if bash "$SCRIPT" "$TMP" >"$TMP/result" 2>&1; then
  echo "FAIL: $1"
  exit 1
fi; }
expect_parser() {
  local want="$1" description="$2" got
  got="$(CI_POLICY_TEST_PUSH_TARGETS_FILE="$TMP/.github/workflows/fixture.yml" bash "$SCRIPT" "$TMP")"
  if [[ "$got" != "$want" ]]; then
    echo "FAIL: $description (expected $want, got $got)"
    exit 1
  fi
}
for event in pull_request pull_request_target pull_request_review pull_request_review_comment merge_group; do
  printf 'on:\n  %s:\njobs: {}\n' "$event" >"$TMP/.github/workflows/fixture.yml"
  expect_fail "$event should fail"
done
for branch in develop '*' 'main, develop'; do
  printf 'on:\n  push:\n    branches: [%s]\njobs: {}\n' "$branch" >"$TMP/.github/workflows/fixture.yml"
  expect_fail 'non-main push should fail'
done
printf 'on: [push, pull_request]\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_fail 'inline PR/push should fail'
printf 'on:\n  push:\n    tags: [v*]\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_fail 'tag-only push should fail'
printf 'on:\n  push:\n    branches: [main]\n    tags: [v*]\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_fail 'main plus tags should fail'
printf 'on:\n  push:\n    branches: [main]\n  workflow_dispatch:\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
bash "$SCRIPT" "$TMP"
# Regression fixtures for the seven YAML constructs dispositioned from #8513.
printf "on:\n  push:\n    branches: ['release main']\njobs: {}\n" >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'quoted scalar with spaces remains one branch token'
printf 'on:\n  workflow_dispatch: &gate\n    paths: [src/**]\n  push: *gate\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'whole-trigger alias to paths-only mapping is not certified'
printf 'on:\n  workflow_dispatch: &m [main]\n  push:\n    branches-ignore: *m\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'sequence alias resolves as branches-ignore value'
printf 'on:\n  workflow_dispatch: &excluded [main]\n  push: {branches-ignore: *excluded}\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'inline alias value resolves before matching'
printf 'on:\n  workflow_dispatch: &flow { branches: [main],\n    branches-ignore: [main] }\n  push: *flow\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'multiline anchored flow mapping is retained in full'
printf "on:\n  push:\n    branches: [ma'in]\njobs: {}\n" >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'apostrophe in plain scalar is not treated as quote syntax'
printf 'on:\n  push:\n    branches-ignore:\n      - |-\n        main\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'block scalar filter entry is decoded before matching'
echo 'PASS main-only CI trigger policy fixtures'
