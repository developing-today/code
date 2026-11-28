#!/usr/bin/env bash
# ownership-map.test.sh — doc-rot guard for the #3989 ownership-map family
# (#3989 criterion 1: each service has one named canonical data home and
# one active writer, documented with evidence).
#
# Oracle: doc/pm-stack/current-state-reconciliation-2026-11-24.md and the
# repo declarations it cites. The doc exists, it names the current homes
# and writers per service, it records the muse-dev2 deletion hazard
# explicitly (so no later worker inherits the dead Supabase target
# silently), and every machine-checkable citation in it still holds
# against the tree. No docker, no network, no live host: pure repo reads.
#
# Run from anywhere: bash pm-stack/scripts/ownership-map.test.sh
set -uo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
DOC="$REPO_ROOT/doc/pm-stack/current-state-reconciliation-2026-11-24.md"

FAILURES=0
pass() { printf 'ok - %s\n' "$1"; }
fail() {
  printf 'FAIL - %s\n' "$1"
  FAILURES=$((FAILURES + 1))
}
# $1 = file, $2 = fixed string, $3 = label
assert_contains() {
  if grep -qF "$2" "$1" 2>/dev/null; then
    pass "$3"
  else
    fail "$3 (missing $(printf '%s' "$2" | head -c 80) in $1)"
  fi
}
assert_exists() {
  if [ -f "$1" ]; then
    pass "$2"
  else
    fail "$2 (absent: $1)"
  fi
}

# 1. The reconciliation doc itself exists — without it criterion 1 has no
# current evidence (the 2026-11-05 map names the deleted muse-dev2 target).
assert_exists "$DOC" "reconciliation doc exists"

# 2. The doc records the muse-dev2 deletion hazard explicitly.
assert_contains "$DOC" "muse-dev2" "doc names the muse-dev2 target"
assert_contains "$DOC" "deleted" "doc records the muse-dev2 deletion"

# 3. Aggregator: backend switch offers exactly file|turso and retires supabase.
assert_contains "$REPO_ROOT/pm-stack/aggregator/src/store-backend.ts" \
  'export type StoreBackendKind = "file" | "turso"' \
  "store-backend offers exactly file|turso"
assert_contains "$REPO_ROOT/pm-stack/aggregator/src/store-backend.ts" \
  '"supabase"' \
  "store-backend names the retired supabase value"
assert_contains "$DOC" "PM_AGG_STORE_BACKEND=turso" \
  "doc records the deployed aggregator backend"

# 4. install.sh renders the turso backend (the deployed writer's config).
assert_contains "$REPO_ROOT/pm-stack/aggregator/install.sh" \
  "PM_AGG_STORE_BACKEND=turso" \
  "install.sh renders the turso backend"

# 5. Kaneo/Postgres: still the laptop compose service until the re-plan lands.
assert_contains "$REPO_ROOT/pm-stack/compose.yml" "kaneo:" \
  "compose declares the kaneo service"
assert_contains "$REPO_ROOT/pm-stack/compose.yml" "postgres_data:/var/lib/postgresql/data" \
  "compose declares the laptop postgres volume"
assert_contains "$DOC" "pm-stack/compose.yml" "doc cites the kaneo compose declaration"

# 6. Grist/Pad: still laptop compose volumes until the re-plan lands.
assert_contains "$REPO_ROOT/pm-stack/grist/compose.yml" "grist_data:/persist" \
  "grist compose declares the laptop grist volume"
assert_contains "$REPO_ROOT/pm-stack/pad/compose.yml" "pad_data:/data" \
  "pad compose declares the laptop pad volume"

# 7. Fencing gate and runbook/drill evidence the doc relies on are present.
assert_exists "$REPO_ROOT/pm-stack/scripts/fence-check.sh" "fence-check.sh present"
assert_exists "$REPO_ROOT/pm-stack/scripts/kaneo-cutover.sh" "kaneo-cutover.sh present"
assert_exists "$REPO_ROOT/doc/pm-stack/COLD-RESTORE-RUNBOOK.md" "runbook present"
assert_exists "$REPO_ROOT/doc/pm-stack/restore-drills/2026-11.md" "drill record present"

# 8. The doc points at the open re-plan owner (#6816), not a silent close.
assert_contains "$DOC" "#6816" "doc names the #6816 re-plan owner"

if [ "$FAILURES" -ne 0 ]; then
  printf '%s assertion(s) failed\n' "$FAILURES"
  exit 1
fi
printf 'ownership-map suite passed.\n'
