#!/usr/bin/env bash
# Scope guard for #14478 slice 1 (design spike: AI tag-suggestions v1).
# Oracle: exit 0 iff the design note exists, states the one-capability
# decision with its safety boundary, cites the closed dependencies, and
# all referenced repo paths resolve — and the working diff touches only
# this doc plus this script (no production implementation).
set -euo pipefail
REPO="$(git rev-parse --show-toplevel)"
DOC="$REPO/doc/2026-11-24_design_id14478_ai-tag-suggestions-v1.md"
fail() {
  echo "FAIL: $1" >&2
  exit 1
}
pass() { echo "PASS: $1"; }

test -f "$DOC" || fail "design note missing: doc/2026-11-24_design_id14478_ai-tag-suggestions-v1.md"

for phrase in \
  "NO-GO for AI inference in v1" \
  "AI-assisted tag suggestions" \
  "Data and safety boundary" \
  "Provider-registry interface" \
  "go-no-go" \
  "#6377" \
  "#1061" \
  "13741" \
  "No production code"; do
  grep -qF "$phrase" "$DOC" || fail "design note missing required phrase: $phrase"
done
pass "design note present with required sections and citations"

for ref in \
  "pkgs/id/book/reference/data-stores.md" \
  "pkgs/id/ARCHITECTURE.md" \
  "pkgs/id/crates/id-native-shell/src/providers.rs" \
  "pkgs/id/crates/id-caps/src/world_caps.rs"; do
  test -e "$REPO/$ref" || fail "referenced path does not resolve: $ref"
done
pass "all referenced repo paths resolve"

# Spike scope: only the doc and this script may differ from origin/main.
changed="$(
  git -C "$REPO" diff --name-only origin/main...HEAD
  git -C "$REPO" status --short | awk '{print $2}'
)"
if echo "$changed" | grep -v '^$' | grep -vE '^(doc/2026-11-24_design_id14478_ai-tag-suggestions-v1\.md|scripts/check-design-14478\.sh)$' | grep -q .; then
  fail "out-of-scope files changed (spike is doc-only): $(echo "$changed" | tr '\n' ' ')"
fi
pass "diff touches only the design note plus this script"

echo "OK: #14478 slice 1 scope guard green"
