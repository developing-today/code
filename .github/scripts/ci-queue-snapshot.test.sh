#!/usr/bin/env bash
# Contract tests for ci-queue-snapshot.sh (#2605): the post-change
# measurement harness for the hosted-runner queue saturation issue.
# Oracle: golden corpus — fixed fixture JSON implies exact summary lines.
# Usage: ci-queue-snapshot.test.sh (no arguments).
set -euo pipefail
[[ $# == 0 ]] || {
  echo 'No arguments accepted' >&2
  exit 2
}
SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/ci-queue-snapshot.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Fixture: 3 queued (2 stale 2026-11-11 pull_request, 1 fresh 2026-11-23 push)
# plus 2 in_progress fresh push runs picked up within minutes.
cat >"$TMP/queued.json" <<'EOF'
{"total_count": 3, "workflow_runs": [
  {"id": 43098034286, "name": "mcpx test", "event": "pull_request", "created_at": "2026-11-11T20:46:35Z", "head_branch": "fix/old"},
  {"id": 43097944766, "name": "scripts-test", "event": "push", "created_at": "2026-11-11T20:45:36Z", "head_branch": "main"},
  {"id": 45110000001, "name": "fleet-smoke", "event": "push", "created_at": "2026-11-23T04:20:00Z", "head_branch": "main"}
]}
EOF
cat >"$TMP/in_progress.json" <<'EOF'
{"total_count": 2, "workflow_runs": [
  {"id": 45112231022, "name": "fleet-smoke", "event": "push", "created_at": "2026-11-23T04:30:12Z", "head_branch": "main"},
  {"id": 45111956255, "name": "fleet-smoke", "event": "push", "created_at": "2026-11-23T04:27:28Z", "head_branch": "main"}
]}
EOF

out="$(bash "$SCRIPT" --queued-json "$TMP/queued.json" --in-progress-json "$TMP/in_progress.json" --now "2026-11-23T04:35:00Z")"
echo "$out" | grep -q "ci-queue-snapshot: queued_total=3 in_progress_total=2"
echo "pass: totals line"
echo "$out" | grep -q "ci-queue-snapshot: queued_by_event pull_request=1 push=2"
echo "pass: queued by event"
echo "$out" | grep -q "ci-queue-snapshot: oldest_queued id=43097944766 created=2026-11-11T20:45:36Z"
echo "pass: oldest queued"
echo "$out" | grep -q "ci-queue-snapshot: fresh_in_progress=2"
echo "pass: fresh in_progress counted"
echo "$out" | grep -q "ci-queue-snapshot: stale_queued_older_than_24h=2"
echo "pass: stale queued counted"

# Strict argument contract: unknown flag is usage exit 2 naming the offender.
if bash "$SCRIPT" --bogus-flag 2>"$TMP/err"; then
  echo "FAIL: unknown flag should exit non-zero" >&2
  exit 1
fi
grep -q "bogus-flag" "$TMP/err"
echo "pass: unknown flag rejected"

# Stray positional is the same event.
if bash "$SCRIPT" stray 2>"$TMP/err2"; then
  echo "FAIL: stray positional should exit non-zero" >&2
  exit 1
fi
grep -q "stray" "$TMP/err2"
echo "pass: stray positional rejected"

echo 'PASS ci-queue-snapshot fixtures'
