#!/usr/bin/env bash
# Oracle: the November 13 main-only automatic trigger policy.
# Usage: check-ci-policy-agreement.test.sh (no arguments).
set -euo pipefail
[[ $# == 0 ]] || { echo 'No arguments accepted' >&2; exit 2; }
BASE="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
SCRIPT="$BASE/.github/scripts/check-ci-policy-agreement.sh"
TMP="$(mktemp -d)"; trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/.github/workflows"
cp "$BASE/.github/ci-policy.md" "$TMP/.github/"
cp "$BASE/.github/workflows/"*.yml "$TMP/.github/workflows/"
bash "$SCRIPT" "$TMP"
expect_fail() { if bash "$SCRIPT" "$TMP" >"$TMP/result" 2>&1; then echo "FAIL: $1"; exit 1; fi; }
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
echo 'PASS main-only CI trigger policy fixtures'
