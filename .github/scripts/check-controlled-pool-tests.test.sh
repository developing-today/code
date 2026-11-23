#!/usr/bin/env bash
# Contract tests for check-controlled-pool-tests.sh (#1342).
# Builds fixture roots and asserts the checker passes the good
# fixture and fails each mutated one, naming the broken leg.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-controlled-pool-tests.sh"
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
  mkdir -p "$dest/doc/agents" "$dest/.github/scripts" \
    "$dest/pkgs/fleet/crates/fleet-queue/src/turso_queue" \
    "$dest/pkgs/fleet/crates/fleet-queue/src/client"
  cp "$REAL_ROOT/doc/agents/controlled-pool-tests.md" "$dest/doc/agents/"
  cp "$REAL_ROOT/doc/agents/controlled-pool-test-template.md" "$dest/doc/agents/"
  cp "$REAL_ROOT/pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs" "$dest/pkgs/fleet/crates/fleet-queue/src/turso_queue/"
  cp "$REAL_ROOT/pkgs/fleet/crates/fleet-queue/src/turso_queue.rs" "$dest/pkgs/fleet/crates/fleet-queue/src/"
  cp "$REAL_ROOT/pkgs/fleet/crates/fleet-queue/src/client/support.rs" "$dest/pkgs/fleet/crates/fleet-queue/src/client/"
  cp "$REAL_ROOT/pkgs/fleet/crates/fleet-queue/src/client.rs" "$dest/pkgs/fleet/crates/fleet-queue/src/"
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

MUT="$TMP/no-route-pinning-section"
make_fixture "$MUT"
sed -i '/^## Route pinning$/d' "$MUT/doc/agents/controlled-pool-tests.md"
expect_fail "$MUT" "procedure without the Route pinning section"

MUT="$TMP/no-verdict-field"
make_fixture "$MUT"
sed -i '/Verdict/d' "$MUT/doc/agents/controlled-pool-test-template.md"
expect_fail "$MUT" "template without the Verdict field"

MUT="$TMP/no-separate-session-launch"
make_fixture "$MUT"
sed -i '/Launch each arm as a separate session/d' "$MUT/doc/agents/controlled-pool-tests.md"
expect_fail "$MUT" "procedure without the separate-session launch requirement"

MUT="$TMP/no-claim-model-column"
make_fixture "$MUT"
sed -i '/claim_model TEXT/d' "$MUT/pkgs/fleet/crates/fleet-queue/src/turso_queue.rs"
expect_fail "$MUT" "queue code without the claim_model TEXT column"

MUT="$TMP/no-stamp-claim-model"
make_fixture "$MUT"
sed -i '/stamp_claim_model/d' "$MUT/pkgs/fleet/crates/fleet-queue/src/turso_queue/claim.rs"
expect_fail "$MUT" "queue code without stamp_claim_model"

MUT="$TMP/no-derive-telemetry"
make_fixture "$MUT"
sed -i '/derive_completion_telemetry/d' "$MUT/pkgs/fleet/crates/fleet-queue/src/client/support.rs"
expect_fail "$MUT" "queue code without derive_completion_telemetry"

MUT="$TMP/no-last-serving"
make_fixture "$MUT"
sed -i '/lastServing/d' "$MUT/pkgs/fleet/crates/fleet-queue/src/client/support.rs" \
  "$MUT/pkgs/fleet/crates/fleet-queue/src/client.rs"
expect_fail "$MUT" "queue code without the lastServing readout"

# Argument contract (#3352): an extra positional is a usage error,
# exit 2, naming the offender.
set +e
out=$(bash "$CHECK" "$GOOD" extra 2>&1)
code=$?
set -e
if [ "$code" -ne 2 ] || ! printf '%s' "$out" | grep -q "usage error"; then
  note_fail "extra argument should exit 2 with a usage error (got $code)"
else
  echo "ok: extra argument fails as a usage error"
fi

if [ "$failures" -ne 0 ]; then
  echo "check-controlled-pool-tests tests: FAIL — $failures failure(s)"
  exit 1
fi
echo "check-controlled-pool-tests tests: OK"
