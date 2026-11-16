#!/usr/bin/env bash
# Fleet Rust test wiring checker (#8941).
#
# The contract: the fleet.rust row of .github/test-suite-registry.json
# counts as covered only while all of these agree —
#   1. .github/workflows/fleet-rust-tests.yml exists, is basic-tier,
#      and runs on push to main (pkgs/fleet/** paths) and
#      workflow_dispatch — and declares no pull_request trigger
#      (main-only CI policy, .github/ci-policy.md, 2026-11-13);
#   2. its test step runs the locked workspace test command with the
#      toolchain channel pinned in pkgs/fleet/rust-toolchain.toml;
#   3. the registry row fleet.rust is status "mapped" and names this
#      workflow as its venue;
#   4. .github/ci-policy.md carries the fleet-rust-tests tier row.
# A disagreement fails, naming the broken leg.
#
# Usage: check-fleet-rust-tests.sh [ROOT]   (default: repo root)
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
WF="$ROOT/.github/workflows/fleet-rust-tests.yml"
POLICY="$ROOT/.github/ci-policy.md"
REGISTRY="$ROOT/.github/test-suite-registry.json"
TOOLCHAIN="$ROOT/pkgs/fleet/rust-toolchain.toml"
MANIFEST="$ROOT/pkgs/fleet/Cargo.toml"

fail=0
err() {
  echo "::error::fleet-rust-tests contract: $1"
  fail=1
}

for f in "$WF" "$POLICY" "$REGISTRY" "$TOOLCHAIN" "$MANIFEST"; do
  if [ ! -f "$f" ]; then
    err "required file missing: ${f#"$ROOT"/}"
  fi
done
if [ "$fail" -ne 0 ]; then
  exit 1
fi

# 1. Triggers and tier shape. Main-only CI policy (2026-11-13): the
# workflow runs on push to main and workflow_dispatch only; a
# pull_request trigger is a contract violation, not a requirement.
for trigger in push workflow_dispatch; do
  if ! grep -qE "^  ${trigger}:" "$WF"; then
    err "workflow does not declare the ${trigger} trigger"
  fi
done
for trigger in pull_request pull_request_target merge_group; do
  if grep -qE "^  ${trigger}:" "$WF"; then
    err "workflow declares the ${trigger} trigger - main-only CI policy forbids PR triggers"
  fi
done
if ! grep -qF '"pkgs/fleet/**"' "$WF"; then
  err "workflow path filter does not cover pkgs/fleet/** — a Fleet change could silently skip the suite"
fi
if ! grep -qF 'basic: fleet Rust workspace tests' "$WF"; then
  err "job display name lost its basic-tier prefix ('basic: fleet Rust workspace tests')"
fi

# 2. The locked workspace test command, with the pinned toolchain.
channel="$(sed -n 's/^channel = "\([^"]*\)".*/\1/p' "$TOOLCHAIN" | head -1)"
if [ -z "$channel" ]; then
  err "could not read the toolchain channel from pkgs/fleet/rust-toolchain.toml"
else
  if ! grep -qF "rustup toolchain install ${channel}" "$WF"; then
    err "workflow does not install the pinned toolchain (${channel}) from rust-toolchain.toml"
  fi
  if ! grep -qF "cargo +${channel} test --manifest-path pkgs/fleet/Cargo.toml --workspace --all-targets --locked" "$WF"; then
    err "workflow test step is not the locked workspace command (cargo +${channel} test --manifest-path pkgs/fleet/Cargo.toml --workspace --all-targets --locked)"
  fi
fi

# 3 + 4. Registry row and policy row agree with the workflow.
python3 - "$REGISTRY" <<'PYEOF' || err "registry row fleet.rust is not mapped to .github/workflows/fleet-rust-tests.yml"
import json
import sys

with open(sys.argv[1]) as fh:
    registry = json.load(fh)
rows = [s for s in registry["suites"] if s["id"] == "fleet.rust"]
if len(rows) != 1:
    sys.exit(1)
row = rows[0]
venue = row.get("venue") or {}
if row.get("status") != "mapped" or venue.get("workflow") != ".github/workflows/fleet-rust-tests.yml":
    sys.exit(1)
PYEOF
if ! grep -qiF "fleet-rust-tests" "$POLICY"; then
  err "ci-policy.md names no tier row for fleet-rust-tests"
fi

if [ "$fail" -ne 0 ]; then
  echo "fleet-rust-tests contract: FAILED"
  exit 1
fi
echo "fleet-rust-tests contract: OK"
