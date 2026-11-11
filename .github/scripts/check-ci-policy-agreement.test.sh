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

# Codex P2 (PR #8513): block-sequence items indented deeper than
# key+2 are valid YAML and must still be read — a branches-ignore
# key at four spaces with its item at eight excludes main exactly
# like the six-space form above.
build_tree "$TMP/filter-unwaived-deep"
wf_raw "$TMP/filter-unwaived-deep" mcpx-fix-test-guard.yml <<'EOF'
  pull_request:
  workflow_dispatch:
  push:
    branches-ignore:
        - main
EOF
if bash "$SCRIPT" "$TMP/filter-unwaived-deep" >"$TMP/filter-unwaived-deep.out" 2>&1; then
  echo "FAIL: checker certified an unwaived main leg whose deep-indented branches-ignore excludes main" >&2
  exit 1
fi
grep -q "DISAGREEMENT mcpx-fix-test-guard.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-unwaived-deep.out"
echo "pass: deep-indented block-sequence branches-ignore item still excludes main"

# And the mirror image: a deep-indented branches block list that
# includes main satisfies the main leg (the waiver goes stale).
build_tree "$TMP/filter-main-list-deep"
wf_raw "$TMP/filter-main-list-deep" mcpx-nixbuild.yml <<'EOF'
  pull_request:
  workflow_dispatch:
  push:
    branches:
        - main
EOF
if bash "$SCRIPT" "$TMP/filter-main-list-deep" >"$TMP/filter-main-list-deep.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a deep-indented block-sequence branches: [main]" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-main-list-deep.out"
echo "pass: deep-indented block-sequence branches including main satisfies the main leg"

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


# Codex P2 regression on #8513 round 4: character classes inside a
# pattern are pattern syntax, not YAML delimiters — '[a-z]ain'
# matches main, in both the positive and the ignore list.
build_tree "$TMP/filter-class_main"
wf_raw "$TMP/filter-class_main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: ['[a-z]ain']
EOF
if bash "$SCRIPT" "$TMP/filter-class_main" >"$TMP/filter-class_main.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a character-class main push" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-class_main.out"
echo "pass: branches: ['[a-z]ain'] satisfies the main leg (waiver stale)"

build_tree "$TMP/filter-class_ignore"
wf_raw "$TMP/filter-class_ignore" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: ['[a-z]ain']
EOF
bash "$SCRIPT" "$TMP/filter-class_ignore" >"$TMP/filter-class_ignore.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-class_ignore.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-class_ignore.out"
echo "pass: branches-ignore: ['[a-z]ain'] keeps the main leg waived, not stale"


# Codex P2 regressions on #8513 round 5. A **/ component matches
# zero directories: '**/main' excludes main.
build_tree "$TMP/filter-globstar_ignore"
wf_raw "$TMP/filter-globstar_ignore" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore: ['**/main']
EOF
bash "$SCRIPT" "$TMP/filter-globstar_ignore" >"$TMP/filter-globstar_ignore.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-globstar_ignore.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-globstar_ignore.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-globstar_ignore.out"; then
  echo "FAIL: branches-ignore: ['**/main'] was certified as main-tier agreement" >&2
  exit 1
fi
echo "pass: branches-ignore: ['**/main'] keeps the main leg waived, not stale"

# A comma inside a quoted scalar is content, not a separator:
# 'release,main' is one branch pattern that is not main.
build_tree "$TMP/filter-quoted_comma"
wf_raw "$TMP/filter-quoted_comma" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: ['release,main']
EOF
bash "$SCRIPT" "$TMP/filter-quoted_comma" >"$TMP/filter-quoted_comma.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-quoted_comma.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-quoted_comma.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-quoted_comma.out"; then
  echo "FAIL: a quoted comma manufactured a main branch entry" >&2
  exit 1
fi
echo "pass: branches: ['release,main'] keeps the main leg waived, not stale"

# An inline push mapping carries its filters on the same line.
build_tree "$TMP/filter-inline_map"
wf_raw "$TMP/filter-inline_map" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches: [release]}
EOF
bash "$SCRIPT" "$TMP/filter-inline_map" >"$TMP/filter-inline_map.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-inline_map.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-inline_map.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-inline_map.out"; then
  echo "FAIL: an inline push mapping was treated as an unfiltered push" >&2
  exit 1
fi
echo "pass: inline push mapping with branches: [release] keeps the main leg waived"

build_tree "$TMP/filter-inline_map_main"
wf_raw "$TMP/filter-inline_map_main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches: [main]}
EOF
if bash "$SCRIPT" "$TMP/filter-inline_map_main" >"$TMP/filter-inline_map_main.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for an inline main push mapping" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-inline_map_main.out"
echo "pass: inline push mapping with branches: [main] satisfies the main leg"


# Codex P2 regression on #8513 round 5b: a quoted character class
# contains its own ] — that bracket is not the flow-list terminator,
# in multiline sequences or inline mappings.
build_tree "$TMP/filter-flow_class"
wf_raw "$TMP/filter-flow_class" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: [
      '[a-z]ain',
      release
    ]
EOF
if bash "$SCRIPT" "$TMP/filter-flow_class" >"$TMP/filter-flow_class.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a flow list with a quoted class" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-flow_class.out"
echo "pass: multiline flow list with a quoted character class satisfies the main leg"

build_tree "$TMP/filter-inline_class"
wf_raw "$TMP/filter-inline_class" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches: ['[a-z]ain']}
EOF
if bash "$SCRIPT" "$TMP/filter-inline_class" >"$TMP/filter-inline_class.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for an inline mapping with a quoted class" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-inline_class.out"
echo "pass: inline push mapping with a quoted character class satisfies the main leg"



# Codex P2 regression on #8513 round 6: push children may sit at
# any consistent indentation depth, not only four spaces.
build_tree "$TMP/filter-deep_indent"
wf_raw "$TMP/filter-deep_indent" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
      branches: [release]
EOF
bash "$SCRIPT" "$TMP/filter-deep_indent" >"$TMP/filter-deep_indent.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-deep_indent.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-deep_indent.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-deep_indent.out"; then
  echo "FAIL: a six-space branches filter was treated as an unfiltered push" >&2
  exit 1
fi
echo "pass: six-space-deep branches: [release] keeps the main leg waived"

build_tree "$TMP/filter-deep_indent_list"
wf_raw "$TMP/filter-deep_indent_list" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
      branches:
        - main
EOF
if bash "$SCRIPT" "$TMP/filter-deep_indent_list" >"$TMP/filter-deep_indent_list.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a deep-indented branches list" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-deep_indent_list.out"
echo "pass: deep-indented branches list including main satisfies the main leg"


# Codex P2 regressions on #8513 round 7: a quoted block item with a
# trailing comment still unquotes, and quoted filter keys name the
# same filters as bare keys.
build_tree "$TMP/filter-quoted_item_comment"
wf_raw "$TMP/filter-quoted_item_comment" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore:
      - 'main'  # main branch
EOF
bash "$SCRIPT" "$TMP/filter-quoted_item_comment" >"$TMP/filter-quoted_item_comment.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-quoted_item_comment.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-quoted_item_comment.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-quoted_item_comment.out"; then
  echo "FAIL: a quoted block item with a trailing comment was not unquoted" >&2
  exit 1
fi
echo "pass: quoted branches-ignore item with trailing comment keeps the main leg waived"

build_tree "$TMP/filter-quoted_key"
wf_raw "$TMP/filter-quoted_key" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    "branches": [release]
EOF
bash "$SCRIPT" "$TMP/filter-quoted_key" >"$TMP/filter-quoted_key.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-quoted_key.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-quoted_key.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-quoted_key.out"; then
  echo "FAIL: a quoted branches key was treated as an unfiltered push" >&2
  exit 1
fi
echo "pass: quoted branches key with [release] keeps the main leg waived"

build_tree "$TMP/filter-quoted_key_ignore"
wf_raw "$TMP/filter-quoted_key_ignore" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    'branches-ignore': [main]
EOF
bash "$SCRIPT" "$TMP/filter-quoted_key_ignore" >"$TMP/filter-quoted_key_ignore.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-quoted_key_ignore.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-quoted_key_ignore.out"
echo "pass: quoted branches-ignore key with [main] keeps the main leg waived"


# Codex P2 regressions on #8513 round 8: a quoted push key still
# yields its inline mapping, and a tags: substring inside a path
# scalar is not a tag filter.
build_tree "$TMP/filter-quoted_push_inline"
wf_raw "$TMP/filter-quoted_push_inline" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  "push": {branches: [release]}
EOF
bash "$SCRIPT" "$TMP/filter-quoted_push_inline" >"$TMP/filter-quoted_push_inline.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-quoted_push_inline.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-quoted_push_inline.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-quoted_push_inline.out"; then
  echo "FAIL: a quoted push key hid its inline branch filter" >&2
  exit 1
fi
echo "pass: quoted push key with inline branches: [release] keeps the main leg waived"

build_tree "$TMP/filter-paths_tags_substring"
wf_raw "$TMP/filter-paths_tags_substring" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {paths: ['docs/tags:/**']}
EOF
if bash "$SCRIPT" "$TMP/filter-paths_tags_substring" >"$TMP/filter-paths_tags_substring.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a paths-only inline push" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-paths_tags_substring.out"
echo "pass: a tags: substring inside a path scalar is not a tag filter (waiver stale)"


# Codex P2 regression on #8513 round 9: an inline push mapping may
# span lines — the whole mapping is collected before its filters
# are evaluated.
build_tree "$TMP/filter-inline_split"
wf_raw "$TMP/filter-inline_split" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches: [
    release]}
EOF
bash "$SCRIPT" "$TMP/filter-inline_split" >"$TMP/filter-inline_split.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-inline_split.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-inline_split.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-inline_split.out"; then
  echo "FAIL: a split inline push mapping was treated as an unfiltered push" >&2
  exit 1
fi
echo "pass: split inline push mapping with branches: [release] keeps the main leg waived"

build_tree "$TMP/filter-inline_split_main"
wf_raw "$TMP/filter-inline_split_main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches: [
    main]}
EOF
if bash "$SCRIPT" "$TMP/filter-inline_split_main" >"$TMP/filter-inline_split_main.out" 2>&1; then
  echo "FAIL: checker accepted a stale waiver for a split inline main mapping" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-inline_split_main.out"
echo "pass: split inline push mapping with branches: [main] satisfies the main leg"

# Codex P2 regression on #8513 round 11: a double-quoted YAML
# scalar's escapes decode before pattern matching -- "ma\u0069n"
# is the branch main, in flow lists and block items alike. A
# single-quoted scalar keeps its backslashes literally.
build_tree "$TMP/filter-dq_escape_flow"
wf_raw "$TMP/filter-dq_escape_flow" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: ["ma\u0069n"]
EOF
if bash "$SCRIPT" "$TMP/filter-dq_escape_flow" >"$TMP/filter-dq_escape_flow.out" 2>&1; then
  echo "FAIL: checker missed a \u-escaped main in a flow list" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-dq_escape_flow.out"
echo "pass: double-quoted \u escape in a flow list satisfies the main leg"

build_tree "$TMP/filter-dq_escape_block"
wf_raw "$TMP/filter-dq_escape_block" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches:
      - "ma\u0069n"
EOF
if bash "$SCRIPT" "$TMP/filter-dq_escape_block" >"$TMP/filter-dq_escape_block.out" 2>&1; then
  echo "FAIL: checker missed a \u-escaped main in a block item" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-dq_escape_block.out"
echo "pass: double-quoted \u escape in a block item satisfies the main leg"

build_tree "$TMP/filter-dq_escape_hex"
wf_raw "$TMP/filter-dq_escape_hex" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches: ["m\x61in", 'lit\u0065ral']
EOF
if bash "$SCRIPT" "$TMP/filter-dq_escape_hex" >"$TMP/filter-dq_escape_hex.out" 2>&1; then
  echo "FAIL: checker missed a \x-escaped main in a flow list" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-dq_escape_hex.out"
echo "pass: double-quoted \x escape in a flow list satisfies the main leg"

# Codex P2 regressions on #8513 round 12: a brace inside a quoted
# scalar does not end an inline mapping, and an escaped quote does
# not end a double-quoted scalar for the structural scanners. In
# both cases branches-ignore: [main] must be seen — main is
# excluded, so the main leg stays waived.
build_tree "$TMP/filter-inline_quoted_brace"
wf_raw "$TMP/filter-inline_quoted_brace" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {paths: ['docs/}file'],
    branches-ignore: [main]}
EOF
bash "$SCRIPT" "$TMP/filter-inline_quoted_brace" >"$TMP/filter-inline_quoted_brace.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-inline_quoted_brace.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-inline_quoted_brace.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-inline_quoted_brace.out"; then
  echo "FAIL: a quoted brace ended the inline mapping early and hid branches-ignore" >&2
  exit 1
fi
echo "pass: quoted brace inside an inline mapping does not hide the continuation's branches-ignore"

build_tree "$TMP/filter-flow_escaped_quote"
wf_raw "$TMP/filter-flow_escaped_quote" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches-ignore: ["release\"x", "main"]}
EOF
bash "$SCRIPT" "$TMP/filter-flow_escaped_quote" >"$TMP/filter-flow_escaped_quote.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-flow_escaped_quote.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-flow_escaped_quote.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-flow_escaped_quote.out"; then
  echo "FAIL: an escaped quote broke flow scanning and discarded branches-ignore" >&2
  exit 1
fi
echo "pass: escaped quote in a flow list does not discard the branch filter"

# Codex P2 regressions on #8513 round 13: a brace inside a YAML
# comment does not terminate an inline mapping, and YAML anchors /
# aliases on trigger mappings resolve to their filters instead of
# looking like unfiltered pushes.
build_tree "$TMP/filter-inline_comment_brace"
wf_raw "$TMP/filter-inline_comment_brace" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: { # closing brace } in comment
    branches-ignore: [main]}
EOF
bash "$SCRIPT" "$TMP/filter-inline_comment_brace" >"$TMP/filter-inline_comment_brace.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-inline_comment_brace.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-inline_comment_brace.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-inline_comment_brace.out"; then
  echo "FAIL: a brace in a YAML comment ended the inline mapping early and hid branches-ignore" >&2
  exit 1
fi
echo "pass: brace in a YAML comment does not hide the continuation's branches-ignore"

build_tree "$TMP/filter-inline_comment_brace_main"
wf_raw "$TMP/filter-inline_comment_brace_main" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: { # } comment
    branches: [main]}
EOF
if bash "$SCRIPT" "$TMP/filter-inline_comment_brace_main" >"$TMP/filter-inline_comment_brace_main.out" 2>&1; then
  echo "FAIL: checker missed branches: [main] after a comment brace" >&2
  exit 1
fi
grep -q "STALE WAIVER mcpx-nixbuild.yml: main leg now agrees" "$TMP/filter-inline_comment_brace_main.out"
echo "pass: branches: [main] after a comment brace satisfies the main leg"

build_tree "$TMP/filter-anchor_alias_block"
wf_raw "$TMP/filter-anchor_alias_block" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch: &release_only
    branches: [release]
  push: *release_only
EOF
bash "$SCRIPT" "$TMP/filter-anchor_alias_block" >"$TMP/filter-anchor_alias_block.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-anchor_alias_block.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-anchor_alias_block.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-anchor_alias_block.out"; then
  echo "FAIL: an aliased release-only push was certified as targeting main" >&2
  exit 1
fi
echo "pass: push aliased to an anchored block mapping inherits its branches filter"

build_tree "$TMP/filter-anchor_inline_push"
wf_raw "$TMP/filter-anchor_inline_push" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: &release_only {branches: [release]}
EOF
bash "$SCRIPT" "$TMP/filter-anchor_inline_push" >"$TMP/filter-anchor_inline_push.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-anchor_inline_push.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-anchor_inline_push.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-anchor_inline_push.out"; then
  echo "FAIL: an anchored inline release-only push was certified as targeting main" >&2
  exit 1
fi
echo "pass: anchored inline push mapping applies its branches filter"

build_tree "$TMP/filter-anchor_alias_inline"
wf_raw "$TMP/filter-anchor_alias_inline" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch: &release_only {branches: [release]}
  push: *release_only
EOF
bash "$SCRIPT" "$TMP/filter-anchor_alias_inline" >"$TMP/filter-anchor_alias_inline.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-anchor_alias_inline.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-anchor_alias_inline.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-anchor_alias_inline.out"; then
  echo "FAIL: an aliased inline-anchored push was certified as targeting main" >&2
  exit 1
fi
echo "pass: push aliased to an anchored inline mapping inherits its branches filter"

build_tree "$TMP/filter-anchor_unknown_alias"
wf_raw "$TMP/filter-anchor_unknown_alias" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: *never_defined
EOF
bash "$SCRIPT" "$TMP/filter-anchor_unknown_alias" >"$TMP/filter-anchor_unknown_alias.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-anchor_unknown_alias.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-anchor_unknown_alias.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-anchor_unknown_alias.out"; then
  echo "FAIL: an unresolvable alias was certified as an unfiltered push" >&2
  exit 1
fi
echo "pass: an unresolvable push alias is never certified as targeting main"

# Round-14 (Codex P2): a comment that ends the first physical line
# of a split inline mapping must not swallow the closing bracket
# on the continuation line — the comment ends at the newline, so
# the release-only filter survives and the leg stays waived.
build_tree "$TMP/filter-inline_comment_split"
wf_raw "$TMP/filter-inline_comment_split" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches: [ # release only
    release]}
EOF
bash "$SCRIPT" "$TMP/filter-inline_comment_split" >"$TMP/filter-inline_comment_split.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-inline_comment_split.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-inline_comment_split.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-inline_comment_split.out"; then
  echo "FAIL: a comment on a split inline mapping line swallowed its branch filter" >&2
  exit 1
fi
echo "pass: a comment ending a split inline mapping line does not swallow the closing bracket"

# Round-14 (Codex P2): YAML allows whitespace before a mapping
# colon — branches : [release] is the same filter as
# branches: [release], and branches-ignore : [main] still
# excludes main; neither may be read as an unfiltered push.
build_tree "$TMP/filter-spaced_colon"
wf_raw "$TMP/filter-spaced_colon" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches : [release]
EOF
bash "$SCRIPT" "$TMP/filter-spaced_colon" >"$TMP/filter-spaced_colon.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-spaced_colon.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-spaced_colon.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-spaced_colon.out"; then
  echo "FAIL: a spaced-colon branches filter was read as an unfiltered push" >&2
  exit 1
fi
echo "pass: branches with a spaced colon keeps the main leg waived"

build_tree "$TMP/filter-spaced_colon_ignore"
wf_raw "$TMP/filter-spaced_colon_ignore" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push:
    branches-ignore : [main]
EOF
bash "$SCRIPT" "$TMP/filter-spaced_colon_ignore" >"$TMP/filter-spaced_colon_ignore.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-spaced_colon_ignore.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-spaced_colon_ignore.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-spaced_colon_ignore.out"; then
  echo "FAIL: a spaced-colon branches-ignore filter was read as an unfiltered push" >&2
  exit 1
fi
echo "pass: branches-ignore with a spaced colon keeps the main leg waived"

build_tree "$TMP/filter-spaced_colon_inline"
wf_raw "$TMP/filter-spaced_colon_inline" mcpx-nixbuild.yml <<'EOF'
  workflow_dispatch:
  push: {branches : [release]}
EOF
bash "$SCRIPT" "$TMP/filter-spaced_colon_inline" >"$TMP/filter-spaced_colon_inline.out"
grep -q "ci-policy-agreement: OK" "$TMP/filter-spaced_colon_inline.out"
grep -q "WAIVED mcpx-nixbuild.yml: main tier requires a push trigger on branches: \[main\]" "$TMP/filter-spaced_colon_inline.out"
if grep -q "STALE WAIVER mcpx-nixbuild.yml" "$TMP/filter-spaced_colon_inline.out"; then
  echo "FAIL: a spaced-colon inline branches filter was read as an unfiltered push" >&2
  exit 1
fi
echo "pass: inline branches with a spaced colon keeps the main leg waived"

# Codex P1 regression on #8513: generated coverage for the parser.
# Equivalent YAML representations of the same logical push filter
# must yield the same verdict. Oracle: an independent Python
# reference for GitHub filter-pattern semantics over the single
# branch name "main" (ordered branches evaluation with negation,
# any-match branches-ignore, branch-filter presence beats the
# tags-only rule) — expectations are computed by the oracle from
# the case definitions, never copied from the awk implementation.
# Representations generated per case: flow list, indented block
# sequence, indentless block sequence, multiline flow list.
python3 - "$SCRIPT" "$TMP" <<'PYEOF'
import os, re, subprocess, sys

script, tmp = sys.argv[1], sys.argv[2]

def translate(pat):
    out, i = [], 0
    while i < len(pat):
        c = pat[i]
        if c == '*':
            if pat[i:i+2] == '**':
                if pat[i+2:i+3] == '/':
                    out.append('(?:.*/)?'); i += 3
                else:
                    out.append('.*'); i += 2
            else:
                out.append('[^/]*'); i += 1
        elif c in '?+':
            out.append(c); i += 1
        elif c == '[':
            j = i + 1
            if pat[j:j+1] == '!':
                j += 1
            while j < len(pat) and pat[j] != ']':
                j += 1
            cls = pat[i+1:j]
            if cls.startswith('!'):
                cls = '^' + cls[1:]
            out.append('[' + cls + ']'); i = j + 1
        else:
            out.append(re.escape(c)); i += 1
    return '^' + ''.join(out) + '$'

def matches(pat):
    return re.match(translate(pat), 'main') is not None

def verdict(branches=None, ignore=None, tags=False):
    if ignore and any(matches(p) for p in ignore):
        return 'no'
    if branches is not None:
        allow = False
        for p in branches:
            if p.startswith('!'):
                if matches(p[1:]):
                    allow = False
            elif matches(p):
                allow = True
        return 'yes' if allow else 'no'
    if ignore is not None:
        return 'yes'
    return 'no' if tags else 'yes'

def q(p):
    if any(ch in p for ch in ',#[] ') or p.startswith('!'):
        return "'" + p + "'"
    return p

def render(kind, pats, style):
    if style == 'flow':
        return [f"    {kind}: [{', '.join(q(x) for x in pats)}]"]
    if style == 'block':
        return [f"    {kind}:"] + [f"      - {q(x)}" for x in pats]
    if style == 'indentless':
        return [f"    {kind}:"] + [f"    - {q(x)}" for x in pats]
    if style == 'multiline':
        lines = [f"    {kind}: ["]
        lines += [f"      {q(x)}," for x in pats[:-1]]
        if pats:
            lines.append(f"      {q(pats[-1])}")
        lines.append("    ]")
        return lines
    raise ValueError(style)

POLICY = """| workflow-lint | basic | — | — |
| proxy-port-fallbacks | basic | — | — |
| fleet-search-parity-guard | basic | — | — |
| opencode-config-guard | basic | main | — |
| mcpx fix-test guard (#1969) | basic | main | — |
| production-line guard (#1411) | basic | — | — |
| go modules build / identity build / map build / mcpx nix build | class-3 | main | — |
"""
BASE = {
    'workflow-lint.yml': ['pull_request', 'workflow_dispatch'],
    'proxy-port-fallbacks.yml': ['pull_request'],
    'fleet-search-parity-guard.yml': ['pull_request', 'workflow_dispatch'],
    'opencode-config-guard.yml': ['pull_request', 'workflow_dispatch'],
    'mcpx-fix-test-guard.yml': ['pull_request', 'push', 'workflow_dispatch'],
    'production-line-guard.yml': ['workflow_dispatch'],
    'go-modules-build.yml': ['workflow_dispatch'],
    'identity-build.yml': ['workflow_dispatch'],
    'map-build.yml': ['workflow_dispatch'],
}

def build(case_root, push_lines):
    os.makedirs(case_root + '/.github/workflows', exist_ok=True)
    with open(case_root + '/.github/ci-policy.md', 'w') as f:
        f.write(POLICY)
    for name, triggers in BASE.items():
        with open(f'{case_root}/.github/workflows/{name}', 'w') as f:
            f.write(f'name: {name}\non:\n')
            for tr in triggers:
                f.write(f'  {tr}:\n')
            f.write('jobs: {}\n')
    with open(case_root + '/.github/workflows/mcpx-nixbuild.yml', 'w') as f:
        f.write('name: mcpx-nixbuild.yml\non:\n  workflow_dispatch:\n  push:\n')
        f.write('\n'.join(push_lines) + '\njobs: {}\n')

BRANCH_CASES = [
    ['main'], ['release'], ['main', '!main'], ['!main', 'main'],
    ['m*'], ['*'], ['**/main'], ['ma+in'], ['main?'], ['[a-z]ain'],
    ['[!a]ain'], ['release,main'], ['mai'], [],
]
IGNORE_CASES = [
    ['main'], ['release'], ['*'], ['**/main'], ['main?'], ['[a-z]ain'],
]
STYLES = ['flow', 'block', 'indentless', 'multiline']

cases = []
for pats in BRANCH_CASES:
    cases.append((f'branches {pats}', render('branches', pats, 'flow'), verdict(branches=pats)))
    for style in STYLES[1:]:
        cases.append((f'branches {pats} [{style}]', render('branches', pats, style), verdict(branches=pats)))
for pats in IGNORE_CASES:
    for style in STYLES:
        cases.append((f'branches-ignore {pats} [{style}]', render('branches-ignore', pats, style), verdict(ignore=pats)))
cases.append(('tags only', render('tags', ['v*'], 'flow'), verdict(tags=True)))
cases.append(('ignore + tags', render('branches-ignore', ['release'], 'flow') + render('tags', ['v*'], 'flow'), verdict(ignore=['release'], tags=True)))

failures = 0
for idx, (name, push_lines, want) in enumerate(cases):
    root = f'{tmp}/gen-{idx}'
    build(root, push_lines)
    proc = subprocess.run(['bash', script, root], capture_output=True, text=True)
    out = proc.stdout + proc.stderr
    if want == 'yes':
        ok = proc.returncode != 0 and 'STALE WAIVER mcpx-nixbuild.yml: main leg now agrees' in out
    else:
        ok = proc.returncode == 0 and 'WAIVED mcpx-nixbuild.yml: main tier' in out and 'STALE WAIVER mcpx-nixbuild.yml' not in out
    if not ok:
        failures += 1
        print(f'GEN FAIL {name}: want verdict {want}, rc={proc.returncode}')
        print(out[:600])
print(f'generated coverage: {len(cases)} cases, {failures} failures')
sys.exit(1 if failures else 0)
PYEOF
echo "pass: generated representation/pattern coverage agrees with the oracle"

echo "check-ci-policy-agreement.test: all contract tests passed"
