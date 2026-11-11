#!/usr/bin/env bash
# Contract tests for check-ci-policy-agreement.sh (#7089 Phase 1).
# Oracle: the agreement contract — for every ci-policy-named
# workflow in the registry, tier, declared triggers and the recorded
# waiver set must agree; a disagreement without a waiver fails
# naming the workflow and the leg, and a waiver whose leg now
# agrees is stale and fails.
set -euo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/check-ci-policy-agreement.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

wf() { # wf <root> <file> <trigger-lines...>
  local root="$1" file="$2"
  shift 2
  mkdir -p "$root/.github/workflows"
  {
    echo "name: $file"
    echo "on:"
    for t in "$@"; do echo "  $t:"; done
    echo "jobs: {}"
  } >"$root/.github/workflows/$file"
}

policy() { # policy <root>: a ci-policy.md naming every registry row
  local root="$1"
  mkdir -p "$root/.github"
  cat >"$root/.github/ci-policy.md" <<'EOF'
| workflow-lint | basic | — | — |
| proxy-port-fallbacks | basic | — | — |
| fleet-search-parity-guard | basic | — | — |
| opencode-config-guard | basic | main | — |
| mcpx fix-test guard (#1969) | basic | main | — |
| production-line guard (#1411) | basic | — | — |
| go modules build / identity build / map build / mcpx nix build | class-3 | main | — |
EOF
}

build_tree() { # build_tree <root>: the current-main shape
  local root="$1"
  policy "$root"
  wf "$root" workflow-lint.yml pull_request workflow_dispatch
  wf "$root" proxy-port-fallbacks.yml pull_request
  wf "$root" fleet-search-parity-guard.yml pull_request workflow_dispatch
  wf "$root" opencode-config-guard.yml pull_request workflow_dispatch
  wf "$root" mcpx-fix-test-guard.yml pull_request push workflow_dispatch
  wf "$root" production-line-guard.yml workflow_dispatch
  wf "$root" go-modules-build.yml pull_request push workflow_dispatch
  wf "$root" identity-build.yml pull_request push workflow_dispatch
  wf "$root" map-build.yml pull_request push workflow_dispatch
  wf "$root" mcpx-nixbuild.yml pull_request push workflow_dispatch
}

# Agreeing-with-waivers tree: exactly the recorded Phase 2-3
# disagreement set, everything else in agreement — passes, and every
# waived leg is reported.
build_tree "$TMP/ok"
bash "$SCRIPT" "$TMP/ok" >"$TMP/ok.out"
grep -q "ci-policy-agreement: OK" "$TMP/ok.out"
grep -q "WAIVED opencode-config-guard.yml: main tier" "$TMP/ok.out"
grep -q "issue #6953" "$TMP/ok.out"
echo "pass: tree with exactly the waived disagreement set passes"

# Unwaived disagreement: workflow-lint loses its pull_request leg.
build_tree "$TMP/bad"
wf "$TMP/bad" workflow-lint.yml workflow_dispatch
if bash "$SCRIPT" "$TMP/bad" >"$TMP/bad.out" 2>&1; then
  echo "FAIL: checker accepted an unwaived disagreement" >&2
  exit 1
fi
grep -q "DISAGREEMENT workflow-lint.yml: pr tier requires trigger 'pull_request'" "$TMP/bad.out"
echo "pass: unwaived disagreement fails naming the workflow and leg"

# Stale waiver: opencode-config-guard gains its main-tier push leg, so the
# #6953 main waiver for it must be deleted, not left behind.
build_tree "$TMP/stale"
wf "$TMP/stale" opencode-config-guard.yml pull_request workflow_dispatch push
if bash "$SCRIPT" "$TMP/stale" >"$TMP/stale.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver" >&2
  exit 1
fi
grep -q "STALE WAIVER opencode-config-guard.yml: main leg now agrees" "$TMP/stale.out"
echo "pass: a waiver whose leg now agrees fails as stale"

# Missing policy row: production-line guard's row vanishes.
build_tree "$TMP/norow"
grep -v "production-line guard" "$TMP/norow/.github/ci-policy.md" >"$TMP/norow/policy.tmp"
mv "$TMP/norow/policy.tmp" "$TMP/norow/.github/ci-policy.md"
if bash "$SCRIPT" "$TMP/norow" >"$TMP/norow.out" 2>&1; then
  echo "FAIL: checker accepted a missing ci-policy row" >&2
  exit 1
fi
grep -q "DISAGREEMENT production-line-guard.yml: ci-policy.md names no row" "$TMP/norow.out"
echo "pass: a vanished ci-policy row fails naming the workflow"

# Missing workflow file.
build_tree "$TMP/nofile"
rm "$TMP/nofile/.github/workflows/map-build.yml"
if bash "$SCRIPT" "$TMP/nofile" >"$TMP/nofile.out" 2>&1; then
  echo "FAIL: checker accepted a missing workflow file" >&2
  exit 1
fi
grep -q "DISAGREEMENT map-build.yml: workflow file missing" "$TMP/nofile.out"
echo "pass: a missing workflow file fails naming it"

echo "check-ci-policy-agreement.test: all contract tests passed"
