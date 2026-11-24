#!/usr/bin/env bash
# check-fork-schedule-stall.test.sh — contract tests for the fork
# schedule-stall classifier (#14444). Builds fixture runs documents
# and asserts the verdict line plus the measured fields.
#
# Oracle: the stall fixture is an excerpt of live
# developing-today-forks/opencode data from 2026-11-24 (real run
# IDs): last schedule event repo-sweeper 44833661765 at
# 2026-11-20T23:01:45Z, agents 44826053161 failed at 22:06Z on the
# pm-dispatch 502, Dependabot dynamic runs still firing 11-24.
# Verdict must be repo-wide-cron-silence. The healthy and idle
# fixtures are synthetic companions pinning the other branches.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
CHECK="$SCRIPT_DIR/check-fork-schedule-stall.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

fail_count=0

# expect_verdict FIXTURE NOW WANT_VERDICT [WANT_PATTERN...]
expect_verdict() {
  local fixture="$1" now="$2" want="$3"
  shift 3
  local out rc
  out="$(bash "$CHECK" --runs-json "$fixture" --now "$now" 2>&1)" && rc=0 || rc=$?
  if [ "$rc" -ne 0 ]; then
    echo "FAIL $want: exit $rc"
    echo "$out" | head -5
    fail_count=$((fail_count + 1))
    return
  fi
  local got
  got="$(grep -E '^verdict=' <<<"$out" || true)"
  if [ "$got" != "verdict=$want" ]; then
    echo "FAIL $want: got '$got'"
    echo "$out" | head -8
    fail_count=$((fail_count + 1))
    return
  fi
  local pat ok=1
  for pat in "$@"; do
    if ! grep -qF "$pat" <<<"$out"; then
      echo "FAIL $want: output missing '$pat'"
      echo "$out" | head -8
      ok=0
      fail_count=$((fail_count + 1))
    fi
  done
  [ "$ok" -eq 1 ] && echo "pass $want"
}

# Fixture 1: the live 2026-11-24 stall (real run IDs, trimmed fields).
cat >"$TMP/stall.json" <<'JSON'
{"workflow_runs": [
  {"id": 45459750765, "name": "npm_and_yarn", "event": "dynamic", "conclusion": null, "created_at": "2026-11-24T18:27:30Z"},
  {"id": 44833661765, "name": "repo-sweeper", "event": "schedule", "conclusion": "success", "created_at": "2026-11-20T23:01:45Z"},
  {"id": 44832026321, "name": "auto-format", "event": "schedule", "conclusion": "success", "created_at": "2026-11-20T22:49:09Z"},
  {"id": 44826053161, "name": "agents", "event": "schedule", "conclusion": "failure", "created_at": "2026-11-20T22:06:00Z"},
  {"id": 44803563625, "name": "models-snapshot", "event": "schedule", "conclusion": "success", "created_at": "2026-11-20T19:35:29Z"}
]}
JSON

# Fixture 2: healthy — a schedule event 3 hours ago.
cat >"$TMP/healthy.json" <<'JSON'
{"workflow_runs": [
  {"id": 999, "name": "agents", "event": "schedule", "conclusion": "success", "created_at": "2026-11-24T18:00:00Z"},
  {"id": 998, "name": "repo-sweeper", "event": "schedule", "conclusion": "success", "created_at": "2026-11-23T23:00:00Z"}
]}
JSON

# Fixture 3: idle — old schedules, nothing newer of any kind.
cat >"$TMP/idle.json" <<'JSON'
{"workflow_runs": [
  {"id": 997, "name": "agents", "event": "schedule", "conclusion": "failure", "created_at": "2026-11-20T22:06:00Z"},
  {"id": 996, "name": "repo-sweeper", "event": "schedule", "conclusion": "success", "created_at": "2026-11-20T23:01:45Z"}
]}
JSON

NOW="2026-11-24T21:00:00Z"

expect_verdict "$TMP/stall.json" "$NOW" "repo-wide-cron-silence" \
  "workflows_with_schedule_events=4" \
  "newest_schedule_event=2026-11-20T23:01:45Z" \
  "newest_schedule_workflow=repo-sweeper" \
  "days_since_last_schedule=3.9" \
  "nonschedule_runs_since_last_schedule=1"

expect_verdict "$TMP/healthy.json" "$NOW" "healthy" \
  "newest_schedule_workflow=agents"

expect_verdict "$TMP/idle.json" "$NOW" "single-workflow-or-idle" \
  "nonschedule_runs_since_last_schedule=0"

# Bare-array input shape is accepted.
python3 - "$TMP/stall.json" "$TMP/bare.json" <<'PYEOF'
import json, sys
runs = json.load(open(sys.argv[1]))["workflow_runs"]
json.dump(runs, open(sys.argv[2], "w"))
PYEOF
expect_verdict "$TMP/bare.json" "$NOW" "repo-wide-cron-silence"

# Strict contract: unknown flags and missing input are usage errors.
if bash "$CHECK" --bogus-flag 2>/dev/null; then
  echo "FAIL usage: unknown flag accepted"; fail_count=$((fail_count + 1))
else
  echo "pass usage-unknown-flag"
fi
if bash "$CHECK" --runs-json "$TMP/missing.json" 2>/dev/null; then
  echo "FAIL usage: missing file accepted"; fail_count=$((fail_count + 1))
else
  echo "pass usage-missing-file"
fi
if bash "$CHECK" 2>/dev/null; then
  echo "FAIL usage: missing --runs-json accepted"; fail_count=$((fail_count + 1))
else
  echo "pass usage-missing-arg"
fi

if [ "$fail_count" -ne 0 ]; then
  echo "$fail_count failure(s)" >&2
  exit 1
fi
echo "check-fork-schedule-stall: all contract tests pass"
