#!/usr/bin/env bash
# Fleet static release wiring checker (#14999).
#
# The contract: the newest fetchable fleet binary tracks current main
# only while all of these agree —
#   1. .github/workflows/fleet-static-release.yml exists, declares a
#      schedule and workflow_dispatch, and declares no push or
#      PR-derived trigger (main-only CI policy, .github/ci-policy.md,
#      2026-11-13; the cadence is the daily schedule);
#   2. the workflow publishes through scripts/muse/publish-fleet.sh
#      (one publisher, two drivers — the manual path and this one can
#      never drift apart), checks for an existing release before
#      building, and holds contents: write and nothing broader;
#   3. the registry row fleet.static-release in
#      .github/test-suite-registry.json is status "mapped" and names
#      this workflow as its venue;
#   4. doc/fleet-schedule-registry.md carries the fleet-static-release
#      row in its GitHub Actions schedules surface;
#   5. the seat-side keeper scripts/muse/fleet-binary-keeper.sh
#      exists, so a published release has a consumer that restores
#      seats from it.
# A disagreement fails, naming the broken leg.
#
# Usage: check-fleet-static-release.sh [ROOT]   (default: repo root)
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)}"
WF="$ROOT/.github/workflows/fleet-static-release.yml"
REGISTRY="$ROOT/.github/test-suite-registry.json"
SCHEDULES="$ROOT/doc/fleet-schedule-registry.md"
KEEPER="$ROOT/scripts/muse/fleet-binary-keeper.sh"
PUBLISHER="$ROOT/scripts/muse/publish-fleet.sh"

fail=0
err() {
  echo "::error::fleet-static-release contract: $1"
  fail=1
}

for f in "$WF" "$REGISTRY" "$SCHEDULES" "$KEEPER" "$PUBLISHER"; do
  if [ ! -f "$f" ]; then
    err "required file missing: ${f#"$ROOT"/}"
  fi
done
if [ "$fail" -ne 0 ]; then
  exit 1
fi

# 1. Triggers: the daily cadence plus manual dispatch; nothing else.
for trigger in schedule workflow_dispatch; do
  if ! grep -qE "^  ${trigger}:" "$WF"; then
    err "workflow does not declare the ${trigger} trigger"
  fi
done
for trigger in push pull_request pull_request_target merge_group; do
  if grep -qE "^  ${trigger}:" "$WF"; then
    err "workflow declares the ${trigger} trigger — the fleet-static cadence is schedule + dispatch only (main-only CI policy)"
  fi
done
if ! grep -qF 'cron:' "$WF"; then
  err "workflow schedule declares no cron expression"
fi

# 2. One publisher, an existence pre-check, and a narrow token.
if ! grep -qF 'scripts/muse/publish-fleet.sh' "$WF"; then
  err "workflow does not publish through scripts/muse/publish-fleet.sh — a second publishing implementation would drift from the manual path"
fi
if ! grep -qF 'gh release view' "$WF"; then
  err "workflow lost its existing-release pre-check — every run would rebuild even when main has not moved"
fi
if ! grep -qE '^  contents: write$' "$WF"; then
  err "workflow does not declare contents: write, the one permission release creation needs"
fi
for perm in actions packages deployments issues pull-requests; do
  if grep -qE "^  ${perm}:" "$WF"; then
    err "workflow declares ${perm} permission — contents: write is the whole grant"
  fi
done

# 3. The registry row is mapped and names this workflow.
if ! python3 - "$REGISTRY" <<'PYEOF'
import json, sys
rows = {s["id"]: s for s in json.load(open(sys.argv[1]))["suites"]}
row = rows.get("fleet.static-release")
ok = (
    row is not None
    and row.get("status") == "mapped"
    and row.get("venue", {}).get("workflow") == ".github/workflows/fleet-static-release.yml"
)
sys.exit(0 if ok else 1)
PYEOF
then
  err "registry row fleet.static-release is missing, unmapped, or names a different venue workflow"
fi

# 4. The schedule registry carries the row.
if ! grep -qF '`fleet-static-release` (`.github/workflows/fleet-static-release.yml`)' "$SCHEDULES"; then
  err "doc/fleet-schedule-registry.md has no fleet-static-release row in the GitHub Actions schedules surface"
fi

# 5. The keeper exists (existence checked above) and reads the same
# release family the workflow publishes.
if ! grep -qF 'fleet-static-' "$KEEPER"; then
  err "scripts/muse/fleet-binary-keeper.sh does not select fleet-static-* releases — publisher and keeper disagree on the release family"
fi

if [ "$fail" -ne 0 ]; then
  exit 1
fi
echo "fleet-static-release contract: OK — cadence, publisher, registry, schedule record, and keeper agree"
