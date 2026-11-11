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

wf_raw() { # wf_raw <root> <file>: workflow whose on: block is stdin
  local root="$1" file="$2"
  mkdir -p "$root/.github/workflows"
  {
    echo "name: $file"
    echo "on:"
    cat
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
  wf "$root" go-modules-build.yml workflow_dispatch
  wf "$root" identity-build.yml workflow_dispatch
  wf "$root" map-build.yml workflow_dispatch
  wf "$root" mcpx-nixbuild.yml workflow_dispatch
}

# Agreeing-with-waivers tree: exactly the recorded Phase 2-3
# disagreement set, everything else in agreement — passes, and every
# waived leg is reported.
build_tree "$TMP/ok"
bash "$SCRIPT" "$TMP/ok" >"$TMP/ok.out"
grep -q "ci-policy-agreement: OK" "$TMP/ok.out"
grep -q "WAIVED go-modules-build.yml: main tier" "$TMP/ok.out"
grep -q "issue #6981" "$TMP/ok.out"
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

# Stale waiver: mcpx-nixbuild gains its main-tier push leg, so the
# #6981 main waiver for it must be deleted, not left behind.
build_tree "$TMP/stale"
wf "$TMP/stale" mcpx-nixbuild.yml workflow_dispatch push
if bash "$SCRIPT" "$TMP/stale" >"$TMP/stale.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/stale.out"
echo "pass: a waiver whose leg now agrees fails as stale"

# Main-tier branch filters (#7950): a push that cannot run on main
# does NOT satisfy the main leg. mcpx-nixbuild's main leg is waived
# (#6981), so a mis-filtered push keeps the waiver recorded — the
# tree still passes and the verdict is WAIVED, never STALE.
for case in release ignore_main tags_only; do
  build_tree "$TMP/filter-$case"
done
wf_raw "$TMP/filter-release" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: [release]
EOF
bash "$SCRIPT" "$TMP/filter-release" >"$TMP/filter-release.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-release.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-release.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-release.out"; then
  echo "FAIL: branches: [release] was certified as main-tier agreement" >&2
  exit 1
fi
echo "pass: push branches: [release] keeps the main leg waived, not stale"

wf_raw "$TMP/filter-ignore_main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: [main]
EOF
bash "$SCRIPT" "$TMP/filter-ignore_main" >"$TMP/filter-ignore_main.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-ignore_main.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-ignore_main.out"
echo "pass: push branches-ignore: [main] keeps the main leg waived, not stale"

wf_raw "$TMP/filter-tags_only" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    tags: ["v*"]
EOF
bash "$SCRIPT" "$TMP/filter-tags_only" >"$TMP/filter-tags_only.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-tags_only.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-tags_only.out"
echo "pass: tags-only push keeps the main leg waived, not stale"

# A push that does target main satisfies the leg: the #6981 waiver
# for mcpx-nixbuild's main leg is stale and must be deleted. Both
# the flow-list and the block-sequence branches: forms agree.
build_tree "$TMP/filter-main"
wf_raw "$TMP/filter-main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: [main]
EOF
if bash "$SCRIPT" "$TMP/filter-main" >"$TMP/filter-main.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a main-targeting push" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-main.out"
echo "pass: push branches: [main] satisfies the main leg (waiver stale)"

build_tree "$TMP/filter-main-list"
wf_raw "$TMP/filter-main-list" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches:
      - release
      - main
EOF
if bash "$SCRIPT" "$TMP/filter-main-list" >"$TMP/filter-main-list.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a block-sequence branches: [main]" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-main-list.out"
echo "pass: block-sequence branches including main satisfies the main leg"

# An unwaived main leg with a mis-filtered push is a disagreement,
# never an agreement: mcpx-fix-test-guard's main leg has no waiver.
build_tree "$TMP/filter-unwaived"
wf_raw "$TMP/filter-unwaived" mcpx-fix-test-guard.yml <<'EOF'
  pull_request:
  workflow_dispatch:
  push:
    branches-ignore:
      - main
EOF
if bash "$SCRIPT" "$TMP/filter-unwaived" >"$TMP/filter-unwaived.out" 2>&1; then
  echo "FAIL: checker certified an unwaived main leg whose push excludes main" >&2
  exit 1
fi
grep -q "DISAGREEMENT mcpx-fix-test-guard.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-unwaived.out"
echo "pass: unwaived main leg with branches-ignore: [main] fails as disagreement"

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


# Codex P2 regressions on #8513: pattern, comment and tags-ignore
# semantics of the same main-tier question. A tags-ignore-only push
# never runs for branch pushes, including main.
build_tree "$TMP/filter-tags_ignore"
wf_raw "$TMP/filter-tags_ignore" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    tags-ignore: ["v*"]
EOF
bash "$SCRIPT" "$TMP/filter-tags_ignore" >"$TMP/filter-tags_ignore.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-tags_ignore.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-tags_ignore.out"
echo "pass: tags-ignore-only push keeps the main leg waived, not stale"

# A trailing YAML comment is not a filter entry: the word main in a
# comment must not certify a branches: [release] push.
build_tree "$TMP/filter-comment_branches"
wf_raw "$TMP/filter-comment_branches" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: [release] # main is intentionally excluded
EOF
bash "$SCRIPT" "$TMP/filter-comment_branches" >"$TMP/filter-comment_branches.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-comment_branches.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-comment_branches.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-comment_branches.out"; then
  echo "FAIL: a YAML comment was treated as a branches entry" >&2
  exit 1
fi
echo "pass: trailing comment on branches: is not a filter entry"

# ...and the mirror image: a comment must not turn an agreeing
# branches-ignore into a disagreement either.
build_tree "$TMP/filter-comment_ignore"
wf_raw "$TMP/filter-comment_ignore" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: [release] # main remains enabled
EOF
if bash "$SCRIPT" "$TMP/filter-comment_ignore" >"$TMP/filter-comment_ignore.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a comment-bearing branches-ignore" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-comment_ignore.out"
echo "pass: trailing comment on branches-ignore: is not a filter entry"

# Branch filters are patterns evaluated in order: a later '!main'
# unmatches an earlier literal main.
build_tree "$TMP/filter-negated"
wf_raw "$TMP/filter-negated" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: [main, '!main']
EOF
bash "$SCRIPT" "$TMP/filter-negated" >"$TMP/filter-negated.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-negated.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-negated.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-negated.out"; then
  echo "FAIL: branches: [main, '!main'] was certified as main-tier agreement" >&2
  exit 1
fi
echo "pass: branches: [main, '!main'] keeps the main leg waived, not stale"

# branches-ignore patterns match as globs: '*' excludes main too.
build_tree "$TMP/filter-ignore_star"
wf_raw "$TMP/filter-ignore_star" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: ['*']
EOF
bash "$SCRIPT" "$TMP/filter-ignore_star" >"$TMP/filter-ignore_star.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-ignore_star.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-ignore_star.out"
echo "pass: branches-ignore: ['*'] keeps the main leg waived, not stale"

# A glob that matches main satisfies the leg like a literal listing.
build_tree "$TMP/filter-pattern_main"
wf_raw "$TMP/filter-pattern_main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: ['m*']
EOF
if bash "$SCRIPT" "$TMP/filter-pattern_main" >"$TMP/filter-pattern_main.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a pattern-matched main push" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-pattern_main.out"
echo "pass: branches: ['m*'] satisfies the main leg (waiver stale)"


# Codex P2 regressions on #8513 round 2: indentless block sequences
# are valid YAML — items aligned with the filter key still count.
build_tree "$TMP/filter-indentless"
wf_raw "$TMP/filter-indentless" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches:
    - main
EOF
if bash "$SCRIPT" "$TMP/filter-indentless" >"$TMP/filter-indentless.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for an indentless branches sequence" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-indentless.out"
echo "pass: indentless branches sequence including main satisfies the main leg"

# A branch filter that does not exclude main means branch pushes
# run, even when a tag filter is also declared (the tag filter
# governs tag pushes separately) — not a tags-only push.
build_tree "$TMP/filter-ignore_plus_tags"
wf_raw "$TMP/filter-ignore_plus_tags" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: [release]
    tags: ['v*']
EOF
if bash "$SCRIPT" "$TMP/filter-ignore_plus_tags" >"$TMP/filter-ignore_plus_tags.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a branches-ignore + tags push" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-ignore_plus_tags.out"
echo "pass: branches-ignore (not main) plus tags still satisfies the main leg"


# Codex P2 regressions on #8513 round 3: GitHub filter patterns use
# quantifiers, not shell glob — 'main?' excludes main (the ? makes
# the preceding n optional), and 'ma+in' matches it.
build_tree "$TMP/filter-ignore_quantifier"
wf_raw "$TMP/filter-ignore_quantifier" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: ['main?']
EOF
bash "$SCRIPT" "$TMP/filter-ignore_quantifier" >"$TMP/filter-ignore_quantifier.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-ignore_quantifier.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-ignore_quantifier.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-ignore_quantifier.out"; then
  echo "FAIL: branches-ignore: ['main?'] was certified as main-tier agreement" >&2
  exit 1
fi
echo "pass: branches-ignore: ['main?'] keeps the main leg waived, not stale"

build_tree "$TMP/filter-plus_quantifier"
wf_raw "$TMP/filter-plus_quantifier" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: ['ma+in']
EOF
if bash "$SCRIPT" "$TMP/filter-plus_quantifier" >"$TMP/filter-plus_quantifier.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a ma+in pattern push" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-plus_quantifier.out"
echo "pass: branches: ['ma+in'] satisfies the main leg (waiver stale)"

# Multiline flow sequences: the entries live on their own lines
# between the [ and ] lines and must all be collected.
build_tree "$TMP/filter-flow_multiline"
wf_raw "$TMP/filter-flow_multiline" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: [
      release,
      main
    ]
EOF
if bash "$SCRIPT" "$TMP/filter-flow_multiline" >"$TMP/filter-flow_multiline.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a multiline flow branches list" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-flow_multiline.out"
echo "pass: multiline flow branches list including main satisfies the main leg"

echo "check-ci-policy-agreement.test: all contract tests passed"
