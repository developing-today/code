#!/usr/bin/env bash
# Fleet smoke wiring checker (#6400).
#
# The contract: the fleet.smoke row of .github/test-suite-registry.json
# counts as covered only while all of these agree —
#   1. .github/workflows/fleet-smoke.yml exists, is basic-tier, and
#      runs on push to main (pkgs/fleet/** paths) and
#      workflow_dispatch — and declares no pull_request trigger
#      (main-only CI policy, .github/ci-policy.md, 2026-11-13);
#   2. its build step builds exactly the four root flake outputs
#      (fleet, fleet-serve, fleet-connect, fleet-connect-static), its
#      run step invokes pkgs/fleet/smoke.sh against them, and the run
#      asserts the suite's "passed, 0 failed" summary line, so a
#      silently skipped suite cannot report green;
#   3. the registry row fleet.smoke is status "mapped" and names this
#      workflow as its venue;
#   4. .github/ci-policy.md carries the fleet-smoke tier row.
# A disagreement fails, naming the broken leg.
#
# Usage: check-fleet-smoke.sh [ROOT]   (default: repo root)
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
WF="$ROOT/.github/workflows/fleet-smoke.yml"
POLICY="$ROOT/.github/ci-policy.md"
REGISTRY="$ROOT/.github/test-suite-registry.json"
SUITE="$ROOT/pkgs/fleet/smoke.sh"

fail=0
err() {
  echo "::error::fleet-smoke contract: $1"
  fail=1
}

for f in "$WF" "$POLICY" "$REGISTRY" "$SUITE"; do
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
if ! grep -qF 'basic: fleet per-variant smoke suite' "$WF"; then
  err "job display name lost its basic-tier prefix ('basic: fleet per-variant smoke suite')"
fi

# 2. The four outputs, the suite invocation, and the summary assertion.
if ! grep -qF '.#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static' "$WF"; then
  err "workflow build step does not build exactly the four flake outputs (.#fleet .#fleet-serve .#fleet-connect .#fleet-connect-static)"
fi
if ! grep -qF 'bash pkgs/fleet/smoke.sh' "$WF"; then
  err "workflow run step does not invoke pkgs/fleet/smoke.sh"
fi
if ! grep -qF 'passed, 0 failed' "$WF"; then
  err "workflow does not assert the suite's 'passed, 0 failed' summary — a skipped suite could report green"
fi

# 3 + 4. Registry row and policy row agree with the workflow.
python3 - "$REGISTRY" <<'PYEOF' || err "registry row fleet.smoke is not mapped to .github/workflows/fleet-smoke.yml"
import json
import sys

with open(sys.argv[1]) as fh:
    registry = json.load(fh)
rows = [s for s in registry["suites"] if s["id"] == "fleet.smoke"]
if len(rows) != 1:
    sys.exit(1)
row = rows[0]
venue = row.get("venue") or {}
if row.get("status") != "mapped" or venue.get("workflow") != ".github/workflows/fleet-smoke.yml":
    sys.exit(1)
PYEOF
if ! grep -qiF "fleet-smoke" "$POLICY"; then
  err "ci-policy.md names no tier row for fleet-smoke"
fi

if [ "$fail" -ne 0 ]; then
  echo "fleet-smoke contract: FAILED"
  exit 1
fi
echo "fleet-smoke contract: OK"
