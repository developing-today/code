#!/usr/bin/env bash
# Contract tests for check-fleet-smoke.sh (#6400).
# Builds fixture roots and asserts the checker passes the good
# fixture and fails each mutated one, naming the broken leg.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-fleet-smoke.sh"
REAL_ROOT="$(cd "$SCRIPT_DIR/../.." && pwd)"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

failures=0
note_fail() {
  echo "FAIL: $1"
  failures=$((failures + 1))
}

# Fixture: copy exactly the files the checker reads from the real
# tree, so the fixture can only drift when the checker gains a leg.
make_fixture() {
  local dest="$1"
  mkdir -p "$dest/.github/workflows" "$dest/.github/scripts" "$dest/pkgs/fleet"
  cp "$REAL_ROOT/.github/workflows/fleet-smoke.yml" "$dest/.github/workflows/"
  cp "$REAL_ROOT/.github/ci-policy.md" "$dest/.github/"
  cp "$REAL_ROOT/.github/test-suite-registry.json" "$dest/.github/"
  cp "$REAL_ROOT/pkgs/fleet/smoke.sh" "$dest/pkgs/fleet/"
}

expect_pass() { # expect_pass <root> <label>
  if bash "$CHECK" "$1" >/dev/null 2>&1; then
    echo "ok: $2 passes"
  else
    note_fail "$2 should pass"
  fi
}

expect_fail() { # expect_fail <root> <label>
  if bash "$CHECK" "$1" >/dev/null 2>&1; then
    note_fail "$2 should fail"
  else
    echo "ok: $2 fails"
  fi
}

GOOD="$TMP/good"
make_fixture "$GOOD"
expect_pass "$GOOD" "unmutated fixture"

MUT="$TMP/no-summary-assertion"
make_fixture "$MUT"
sed -i '/passed, 0 failed/d' "$MUT/.github/workflows/fleet-smoke.yml"
expect_fail "$MUT" "workflow without the summary assertion"

MUT="$TMP/no-paths"
make_fixture "$MUT"
sed -i '/pkgs\/fleet\/\*\*/d' "$MUT/.github/workflows/fleet-smoke.yml"
expect_fail "$MUT" "workflow without the pkgs/fleet path filter"

MUT="$TMP/no-lock-guard"
make_fixture "$MUT"
sed -i '/bash .github\/scripts\/check-fleet-lock.sh/d' "$MUT/.github/workflows/fleet-smoke.yml"
expect_fail "$MUT" "workflow without the fleet lock guard"

MUT="$TMP/no-atlas-paths"
make_fixture "$MUT"
sed -i '/pkgs\/atlas-search\/crates/d' "$MUT/.github/workflows/fleet-smoke.yml"
expect_fail "$MUT" "workflow without the atlas-search path filter"

MUT="$TMP/pr-trigger"
make_fixture "$MUT"
sed -i 's/^  workflow_dispatch:/  pull_request:\n  workflow_dispatch:/' "$MUT/.github/workflows/fleet-smoke.yml"
expect_fail "$MUT" "workflow with a pull_request trigger (main-only policy)"

MUT="$TMP/no-push"
make_fixture "$MUT"
sed -i 's/^  push:/  pushx:/' "$MUT/.github/workflows/fleet-smoke.yml"
expect_fail "$MUT" "workflow without the push trigger"

MUT="$TMP/registry-uncovered"
make_fixture "$MUT"
python3 - "$MUT/.github/test-suite-registry.json" <<'PYEOF'
import json
import sys

path = sys.argv[1]
with open(path) as fh:
    registry = json.load(fh)
for row in registry["suites"]:
    if row["id"] == "fleet.smoke":
        row["status"] = "uncovered"
        row["venue"] = None
with open(path, "w") as fh:
    json.dump(registry, fh, indent=2)
PYEOF
expect_fail "$MUT" "registry row fleet.smoke not mapped"

if [ "$failures" -ne 0 ]; then
  echo "check-fleet-smoke.test: $failures failure(s)"
  exit 1
fi
echo "check-fleet-smoke.test: OK"
