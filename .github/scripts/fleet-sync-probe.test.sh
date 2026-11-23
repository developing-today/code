#!/usr/bin/env bash
# Contract tests for scripts/fleet/fleet-sync-probe.py (#12235): the probe
# passes a fork whose scheduled sync succeeded recently, and fails — with
# a diagnostic naming the cause — when the latest sync failed, when no
# scheduled success is recent enough, when there are no schedule runs at
# all, and when there are no runs at all. Usage errors exit 2; a missing
# or malformed runs file is a named read failure, never a verdict.
#
# Fixtures A and B are the real 2026-11-22 shapes of the two forks
# (trimmed to the fields the probe reads): the org fork's last scheduled
# success was 2026-11-05, the forks copy's was 2026-11-22 12:40 UTC.
# The probe is judged against a pinned --now so the cases are stable.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROBE="$SCRIPT_DIR/../../scripts/fleet/fleet-sync-probe.py"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

NOW="2026-11-22T14:30:00Z"
RC=0
run_probe() { # run_probe RUNS_FILE [EXTRA ARGS..]
  local runs="$1"; shift
  RC=0
  python3 "$PROBE" --repo developing-today/fleet --runs-json "$runs" \
    --max-age-hours 30 --now "$NOW" "$@" >"$TMP/out" 2>&1 || RC=$?
}

expect_pass() { # expect_pass <name>
  if [ "$RC" -ne 0 ]; then
    echo "FAIL: $1 should have passed the probe" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 passes"
}

expect_fail() { # expect_fail <name>
  if [ "$RC" -eq 0 ]; then
    echo "FAIL: $1 should have failed the probe" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 fails"
}

expect_fail_with() { # expect_fail_with <name> <needle>
  expect_fail "$1"
  if ! grep -qF "$2" "$TMP/out"; then
    echo "FAIL: $1 should name '$2' in its diagnostic" >&2
    cat "$TMP/out" >&2
    exit 1
  fi
  echo "pass: $1 names '$2'"
}

# Fixture A: the org fork on 2026-11-22. Three dispatch successes that
# morning, a dispatch failure the day before — but the last SCHEDULED
# success is 2026-11-05, ~17 days old. Dispatches do not satisfy the
# schedule half, so this fails: the exact symptom #12235 step 4 names.
cat >"$TMP/org-2026-11-22.json" <<'EOF'
{"workflow_runs": [
  {"id": 45019411056, "event": "workflow_dispatch", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T11:52:09Z"},
  {"id": 45018819604, "event": "workflow_dispatch", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T11:44:52Z"},
  {"id": 45016159468, "event": "workflow_dispatch", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T11:12:17Z"},
  {"id": 44900082037, "event": "workflow_dispatch", "status": "completed", "conclusion": "failure", "created_at": "2026-11-21T11:38:41Z"},
  {"id": 42028270182, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-05T12:34:20Z"}
]}
EOF
run_probe "$TMP/org-2026-11-22.json"
expect_fail_with "org fork 2026-11-22 (last scheduled success 17 days old)" "no successful scheduled sync within 30h"

# Fixture B: the forks copy on 2026-11-22. A scheduled success at 12:40
# UTC, ~1.8h before the probe clock, with an older failed dispatch
# superseded by later successes. Passes.
cat >"$TMP/forks-2026-11-22.json" <<'EOF'
{"workflow_runs": [
  {"id": 45023460435, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T12:40:01Z"},
  {"id": 45019414056, "event": "workflow_dispatch", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T11:52:11Z"},
  {"id": 44900085581, "event": "workflow_dispatch", "status": "completed", "conclusion": "failure", "created_at": "2026-11-21T11:38:44Z"},
  {"id": 42216261862, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-06T12:15:20Z"}
]}
EOF
run_probe "$TMP/forks-2026-11-22.json"
expect_pass "forks copy 2026-11-22 (scheduled success 1.8h ago)"

# Fixture C: the latest run failed (merge-conflict shape) although a
# scheduled success 10h ago exists. The sync is currently broken, so
# this fails on the latest-run half.
cat >"$TMP/latest-failed.json" <<'EOF'
{"workflow_runs": [
  {"id": 44900082037, "event": "workflow_dispatch", "status": "completed", "conclusion": "failure", "created_at": "2026-11-22T11:38:41Z"},
  {"id": 45023460435, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T04:30:00Z"}
]}
EOF
run_probe "$TMP/latest-failed.json"
expect_fail_with "latest run failed (conflict shape)" "latest sync failed"

# Fixture D: only dispatch successes, all recent, no schedule runs at
# all. The fork is level but its schedule is unproven — fails.
cat >"$TMP/no-schedule.json" <<'EOF'
{"workflow_runs": [
  {"id": 45019411056, "event": "workflow_dispatch", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T11:52:09Z"}
]}
EOF
run_probe "$TMP/no-schedule.json"
expect_fail_with "dispatch-only history (no schedule runs)" "no successful scheduled sync ever"

# Fixture E: no runs at all. Fails, and says so plainly.
echo '{"workflow_runs": []}' >"$TMP/empty.json"
run_probe "$TMP/empty.json"
expect_fail_with "no runs at all" "no sync-upstream runs at all"

# Fixture F: the same healthy shape as B, but as a bare list (what the
# workflow feeds after --jq '.workflow_runs'). Passes.
cat >"$TMP/bare-list.json" <<'EOF'
[
  {"id": 45023460435, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T12:40:01Z"}
]
EOF
run_probe "$TMP/bare-list.json"
expect_pass "bare-list runs input"

# Fixture G: the latest run is still in progress (conclusion null) with
# a scheduled success 2h ago. In progress is neither failure nor
# success, so this passes on the schedule half.
cat >"$TMP/in-progress.json" <<'EOF'
{"workflow_runs": [
  {"id": 45024000000, "event": "workflow_dispatch", "status": "in_progress", "conclusion": null, "created_at": "2026-11-22T14:00:00Z"},
  {"id": 45023460435, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-22T12:40:01Z"}
]}
EOF
run_probe "$TMP/in-progress.json"
expect_pass "in-progress latest run with recent scheduled success"

# Fixture H: a scheduled success 29h ago is inside the 30h window.
cat >"$TMP/inside-window.json" <<'EOF'
{"workflow_runs": [
  {"id": 45020000000, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-21T09:30:00Z"}
]}
EOF
run_probe "$TMP/inside-window.json"
expect_pass "scheduled success 29h ago (inside the 30h window)"

# Fixture I: a scheduled success 31h ago is outside the window.
cat >"$TMP/outside-window.json" <<'EOF'
{"workflow_runs": [
  {"id": 45020000000, "event": "schedule", "status": "completed", "conclusion": "success", "created_at": "2026-11-21T07:30:00Z"}
]}
EOF
run_probe "$TMP/outside-window.json"
expect_fail_with "scheduled success 31h ago (outside the 30h window)" "no successful scheduled sync within 30h"

# Usage errors exit 2 (argparse): unknown flag, stray positional, and a
# missing required option are the same event.
RC=0
python3 "$PROBE" --repo x --runs-json "$TMP/bare-list.json" --nope >"$TMP/out" 2>&1 || RC=$?
if [ "$RC" -ne 2 ]; then echo "FAIL: unknown flag should exit 2 (got $RC)" >&2; cat "$TMP/out" >&2; exit 1; fi
echo "pass: unknown flag exits 2"
# An abbreviated (but unambiguous) flag must also be rejected: the
# probe's parser is strict (allow_abbrev=False) per the operator script
# argument contract (#3352).
RC=0
python3 "$PROBE" --repo x --runs-json "$TMP/bare-list.json" --max-age-hour 30 >"$TMP/out" 2>&1 || RC=$?
if [ "$RC" -ne 2 ]; then echo "FAIL: abbreviated flag should exit 2 (got $RC)" >&2; cat "$TMP/out" >&2; exit 1; fi
echo "pass: abbreviated flag exits 2"
RC=0
python3 "$PROBE" --repo x --runs-json "$TMP/bare-list.json" extra >"$TMP/out" 2>&1 || RC=$?
if [ "$RC" -ne 2 ]; then echo "FAIL: stray positional should exit 2 (got $RC)" >&2; cat "$TMP/out" >&2; exit 1; fi
echo "pass: stray positional exits 2"
RC=0
python3 "$PROBE" --repo x >"$TMP/out" 2>&1 || RC=$?
if [ "$RC" -ne 2 ]; then echo "FAIL: missing --runs-json should exit 2 (got $RC)" >&2; cat "$TMP/out" >&2; exit 1; fi
echo "pass: missing --runs-json exits 2"

# A missing runs file is a named read failure, never a verdict.
run_probe "$TMP/no-such-file.json"
expect_fail_with "missing runs file" "cannot read runs file"
if grep -q "verdict=" "$TMP/out"; then
  echo "FAIL: missing runs file was misdiagnosed as a verdict" >&2
  cat "$TMP/out" >&2
  exit 1
fi
echo "pass: missing runs file reports a cannot-read diagnostic"

# A malformed runs file is a named parse failure, never a verdict.
echo '{not json' >"$TMP/broken.json"
run_probe "$TMP/broken.json"
expect_fail_with "malformed runs file" "invalid JSON"
if grep -q "verdict=" "$TMP/out"; then
  echo "FAIL: malformed runs file was misdiagnosed as a verdict" >&2
  cat "$TMP/out" >&2
  exit 1
fi
echo "pass: malformed runs file reports an invalid-JSON diagnostic"

echo "fleet-sync-probe.test: all contract tests passed"
