#!/usr/bin/env bash
# Contract tests for check-fleet-rust-tests.sh (#8941).
# Builds fixture roots and asserts the checker passes the good
# fixture and fails each mutated one, naming the broken leg.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-fleet-rust-tests.sh"
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
  cp "$REAL_ROOT/.github/workflows/fleet-rust-tests.yml" "$dest/.github/workflows/"
  cp "$REAL_ROOT/.github/ci-policy.md" "$dest/.github/"
  cp "$REAL_ROOT/.github/test-suite-registry.json" "$dest/.github/"
  cp "$REAL_ROOT/pkgs/fleet/rust-toolchain.toml" "$dest/pkgs/fleet/"
  cp "$REAL_ROOT/pkgs/fleet/Cargo.toml" "$dest/pkgs/fleet/"
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

MUT="$TMP/no-locked"
make_fixture "$MUT"
sed -i 's/ --locked//' "$MUT/.github/workflows/fleet-rust-tests.yml"
expect_fail "$MUT" "workflow without --locked"

MUT="$TMP/no-paths"
make_fixture "$MUT"
sed -i '/pkgs\/fleet\/\*\*/d' "$MUT/.github/workflows/fleet-rust-tests.yml"
expect_fail "$MUT" "workflow without the pkgs/fleet path filter"

MUT="$TMP/pr-trigger"
make_fixture "$MUT"
sed -i 's/^  workflow_dispatch:/  pull_request:\n  workflow_dispatch:/' "$MUT/.github/workflows/fleet-rust-tests.yml"
expect_fail "$MUT" "workflow with a pull_request trigger"

MUT="$TMP/registry-uncovered"
make_fixture "$MUT"
python3 - "$MUT/.github/test-suite-registry.json" <<'PYEOF'
import json
import sys

path = sys.argv[1]
with open(path) as fh:
    registry = json.load(fh)
for row in registry["suites"]:
    if row["id"] == "fleet.rust":
        row["status"] = "uncovered"
        row["venue"] = None
with open(path, "w") as fh:
    json.dump(registry, fh, indent=2)
PYEOF
expect_fail "$MUT" "registry row fleet.rust not mapped"

MUT="$TMP/toolchain-drift"
make_fixture "$MUT"
sed -i 's/^channel = ".*"/channel = "1.85.0"/' "$MUT/pkgs/fleet/rust-toolchain.toml"
expect_fail "$MUT" "toolchain pin drifted from the workflow"

if [ "$failures" -ne 0 ]; then
  echo "check-fleet-rust-tests.test: $failures failure(s)"
  exit 1
fi
echo "check-fleet-rust-tests.test: OK"
