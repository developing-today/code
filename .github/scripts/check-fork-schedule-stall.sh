#!/usr/bin/env bash
# check-fork-schedule-stall.sh — classify a fork's Actions schedule health (#14444).
#
# Distinguishes the two competing diagnoses for a stalled scheduled
# workflow: a single workflow failing (fix the workflow or its
# dispatch path) versus repo-wide cron silence while every other
# event type keeps running (GitHub's scheduled-workflow suppression
# for fork repositories — a platform limit, not a code defect).
#
# Input is a runs JSON document as produced by
# `gh api repos/OWNER/REPO/actions/runs?per_page=N`: either an object
# with a `.workflow_runs` array or a bare array. Each run needs
# `id`, `name`, `event`, `created_at` (conclusion optional).
#
# Output is stable `key=value` lines on stdout: per-workflow last
# schedule event counts, the newest schedule event repo-wide,
# days-since, non-schedule activity after it, and the verdict.
#
# Verdicts:
#   healthy                 newest schedule event within the window
#   repo-wide-cron-silence  schedules older than the window AND at
#                           least one non-schedule run is newer than
#                           the newest schedule event (Actions alive,
#                           crons dead) — escalate as a platform limit
#   single-workflow-or-idle schedules older than the window but no
#                           newer non-schedule activity (repo idle or
#                           one workflow quietly failing — needs a
#                           per-workflow look, not an escalation)
#   no-schedule-data        no schedule events in the input at all
#
# Oracle: the 2026-11-24 developing-today-forks/opencode stall is the
# golden case — last schedule event 2026-11-20T23:01:45Z
# (repo-sweeper success), Dependabot dynamic runs still firing
# 2026-11-24, zero schedule events after 2026-11-20 — verdict
# repo-wide-cron-silence (see the contract test fixtures, which are
# excerpts of that live data with real run IDs).
#
# Usage:
#   check-fork-schedule-stall.sh --runs-json FILE [--now ISO8601] [--min-silent-days N]
#   check-fork-schedule-stall.sh --help
# Exit: 0 = classified (verdict line always present); 1 = usage/read/parse error.
set -euo pipefail

usage() {
  sed -n '2,/^set -euo/p' "$0" | sed 's/^# \?//'
}

RUNS_JSON=""
NOW=""
MIN_SILENT_DAYS="2"

while [ "$#" -gt 0 ]; do
  case "$1" in
  --runs-json)
    [ "$#" -ge 2 ] || { echo "check-fork-schedule-stall: --runs-json needs a value" >&2; exit 1; }
    RUNS_JSON="$2"; shift 2 ;;
  --now)
    [ "$#" -ge 2 ] || { echo "check-fork-schedule-stall: --now needs a value" >&2; exit 1; }
    NOW="$2"; shift 2 ;;
  --min-silent-days)
    [ "$#" -ge 2 ] || { echo "check-fork-schedule-stall: --min-silent-days needs a value" >&2; exit 1; }
    MIN_SILENT_DAYS="$2"; shift 2 ;;
  --help | -h) usage; exit 0 ;;
  *)
    echo "check-fork-schedule-stall: unexpected argument '$1'" >&2
    echo "usage: check-fork-schedule-stall.sh --runs-json FILE [--now ISO8601] [--min-silent-days N]" >&2
    exit 1 ;;
  esac
done

[ -n "$RUNS_JSON" ] || { echo "check-fork-schedule-stall: --runs-json is required" >&2; exit 1; }
[ -r "$RUNS_JSON" ] || { echo "check-fork-schedule-stall: cannot read $RUNS_JSON" >&2; exit 1; }
case "$MIN_SILENT_DAYS" in
'' | *[!0-9]*) echo "check-fork-schedule-stall: --min-silent-days must be a non-negative integer" >&2; exit 1 ;;
esac

exec python3 - "$RUNS_JSON" "$NOW" "$MIN_SILENT_DAYS" <<'PYEOF'
import json, sys
from datetime import datetime, timezone

path, now_arg, min_days_arg = sys.argv[1], sys.argv[2], int(sys.argv[3])

def parse_ts(s):
    return datetime.fromisoformat(str(s).replace("Z", "+00:00"))

try:
    with open(path, encoding="utf-8") as fh:
        doc = json.load(fh)
except Exception as exc:
    print(f"check-fork-schedule-stall: cannot parse {path}: {exc}", file=sys.stderr)
    sys.exit(1)

runs = doc.get("workflow_runs", doc) if isinstance(doc, dict) else doc
if not isinstance(runs, list):
    print("check-fork-schedule-stall: expected an object with .workflow_runs or a bare array",
          file=sys.stderr)
    sys.exit(1)

now = parse_ts(now_arg) if now_arg else datetime.now(timezone.utc)

sched = [r for r in runs if isinstance(r, dict) and r.get("event") == "schedule"
         and r.get("created_at")]
if not sched:
    print("workflows_with_schedule_events=0")
    print("verdict=no-schedule-data")
    sys.exit(0)

by_wf = {}
for r in sched:
    name = str(r.get("name", "?"))
    ts = parse_ts(r["created_at"])
    if name not in by_wf or ts > by_wf[name][0]:
        by_wf[name] = (ts, r.get("id"), r.get("conclusion"))

newest_name = max(by_wf, key=lambda n: by_wf[n][0])
newest_ts, newest_id, newest_conc = by_wf[newest_name]
days_silent = (now - newest_ts).total_seconds() / 86400

fresh_other = 0
for r in runs:
    if not isinstance(r, dict) or r.get("event") == "schedule" or not r.get("created_at"):
        continue
    if parse_ts(r["created_at"]) > newest_ts:
        fresh_other += 1

if days_silent <= min_days_arg:
    verdict = "healthy"
elif fresh_other > 0:
    verdict = "repo-wide-cron-silence"
else:
    verdict = "single-workflow-or-idle"

print(f"workflows_with_schedule_events={len(by_wf)}")
print(f"newest_schedule_event={newest_ts.strftime('%Y-%m-%dT%H:%M:%SZ')}")
print(f"newest_schedule_workflow={newest_name}")
print(f"newest_schedule_run_id={newest_id}")
print(f"newest_schedule_conclusion={newest_conc}")
print(f"days_since_last_schedule={days_silent:.1f}")
print(f"nonschedule_runs_since_last_schedule={fresh_other}")
print(f"verdict={verdict}")
PYEOF
