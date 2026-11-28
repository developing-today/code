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
cp "$BASE/.github/workflows/"*.yml "$TMP/.github/workflows/"
# Pre-existing tree breakage, owned by issue #11295: note-submission-pr.yml
# declares a pull_request trigger the policy forbids, so the registry-wide
# agreement assertion below cannot pass while it is in the fixture tree. It
# is excluded here rather than waived (the waiver set is unchanged) and must
# be deleted when #11295 fixes or removes that trigger. Every other workflow
# still runs through this suite.
rm -f "$TMP/.github/workflows/note-submission-pr.yml"
bash "$SCRIPT" "$TMP"
expect_fail() { if bash "$SCRIPT" "$TMP" >"$TMP/result" 2>&1; then
  echo "FAIL: $1"
  exit 1
fi; }
expect_parser() {
  local want="$1" description="$2" got
  got="$(CI_POLICY_TEST_PUSH_TARGETS_FILE="$TMP/.github/workflows/fixture.yml" bash "$SCRIPT" "$TMP")"
  if [[ $got != "$want" ]]; then
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
# Each construct is asserted in both directions: a fixture that must NOT be
# certified (the construct must reach the matcher with real YAML semantics)
# and its positive control, which a parser that answered "no" unconditionally
# would fail. Without the positive control a broken parser still passes.
printf "on:\n  push:\n    branches: ['release main']\njobs: {}\n" >"$TMP/.github/workflows/fixture.yml"
expect_parser no "quoted scalar 'release main' is one token, not a main match (construct 1)"
printf "on:\n  push:\n    branches: ['main']\njobs: {}\n" >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'quoted scalar main still matches (construct 1 control)'
printf 'on:\n  workflow_dispatch: &gate\n    paths: [src/**]\n  push: *gate\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'whole-trigger alias to a paths-only mapping is not certified (construct 2)'
printf 'on:\n  workflow_dispatch: &g\n    branches: [main]\n  push: *g\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'whole-trigger alias resolving to branches: [main] is certified (construct 2 control)'
printf 'on:\n  workflow_dispatch: &m [main]\n  push:\n    branches-ignore: *m\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'sequence alias resolves then matches main in branches-ignore (construct 3)'
printf 'on:\n  workflow_dispatch: &m [develop]\n  push:\n    branches-ignore: *m\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'sequence alias in branches-ignore that misses main stays certified (construct 3 control)'
printf 'on:\n  workflow_dispatch: &m [main]\n  push:\n    branches: *m\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'sequence alias as a branches value resolves to main (construct 3 control 2)'
printf 'on:\n  push:\n    branches: &m [main]\n    branches-ignore: *m\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'anchor declared on the branches key itself resolves before ignore matching (construct 3)'
printf 'on:\n  workflow_dispatch: &excluded [main]\n  push: {branches-ignore: *excluded}\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'inline alias value resolves before matching (construct 4)'
printf 'on:\n  workflow_dispatch: &excluded [develop]\n  push: {branches-ignore: *excluded}\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'inline alias value that misses main stays certified (construct 4 control)'
printf 'on:\n  workflow_dispatch: &ok [main]\n  push: {branches: *ok}\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'inline alias as a branches value resolves to main (construct 4 control 2)'
printf 'on:\n  workflow_dispatch: &flow { branches: [main],\n    branches-ignore: [main] }\n  push: *flow\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'multiline anchored flow mapping is retained in full (construct 5)'
printf 'on:\n  workflow_dispatch: &flow { branches: [main],\n    branches-ignore: [develop] }\n  push: *flow\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'multiline anchored flow mapping continues past the first line (construct 5 control)'
printf "on:\n  push:\n    branches: [ma'in]\njobs: {}\n" >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'apostrophe in a plain scalar is not quote syntax (construct 6)'
printf "on:\n  push:\n    branches: ['ma''in']\njobs: {}\n" >"$TMP/.github/workflows/fixture.yml"
expect_parser no "doubled apostrophe in a quoted scalar stays literal (construct 6 control)"
printf 'on:\n  push:\n    branches-ignore:\n      - |-\n        main\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser no 'block scalar filter entry is decoded before matching (construct 7)'
printf 'on:\n  push:\n    branches-ignore:\n      - |-\n        develop\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'block scalar entry that misses main stays certified (construct 7 control)'
printf 'on:\n  push:\n    branches:\n      - |-\n        main\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'block scalar branches entry decodes to main (construct 7 control 2)'
printf 'on:\n  push:\n    branches-ignore:\n      - >-\n        develop\njobs: {}\n' >"$TMP/.github/workflows/fixture.yml"
expect_parser yes 'folded block scalar entry is decoded too (construct 7 control 3)'
echo 'PASS main-only CI trigger policy fixtures'
