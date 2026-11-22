#!/usr/bin/env bash
set -Eeuo pipefail
umask 077

SCRIPT_DIR="$(cd -- "$(dirname -- "${BASH_SOURCE[0]}")" && pwd)"
JOB_ID="quota-freshness-summary"
STATE_DIR="${OPENCODE_SCHEDULED_JOB_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/opencode-scheduled/$JOB_ID}"
MODEL="${OPENCODE_MODEL:?OPENCODE_MODEL must be set by the unit}"
OPENCODE_BIN="${OPENCODE_BIN:-opencode}"
INPUT="$STATE_DIR/input.json"
LOG="$STATE_DIR/latest.log"
HISTORY="$STATE_DIR/history.jsonl"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
RUN_ID="$(date -u +%Y%m%dT%H%M%SZ)"

mkdir -p "$STATE_DIR"
exec 9>"$STATE_DIR/run.lock"
if ! flock -n 9; then
  echo "$JOB_ID: prior run is still active; skipping overlap"
  exit 75
fi

run_job() {
  printf 'job=%s startedAt=%s route=%s\n' "$JOB_ID" "$STARTED_AT" "$MODEL"
  python3 "$SCRIPT_DIR/summarize.py" >"$INPUT" || return $?
  timeout --signal=TERM --kill-after=10s 180s \
    "$OPENCODE_BIN" run --model "$MODEL" --title "$JOB_ID-$RUN_ID" \
    --file "$INPUT" "$(<"$SCRIPT_DIR/prompt.md")"
}

set +e
run_job >"$LOG" 2>&1
EXIT_CODE=$?
set -e

FINISHED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"
printf 'finishedAt=%s exitCode=%s\n' "$FINISHED_AT" "$EXIT_CODE" >>"$LOG"
python3 - "$HISTORY" "$STARTED_AT" "$FINISHED_AT" "$EXIT_CODE" "$MODEL" "$LOG" "$RUN_ID" <<'PY'
import json
import sys

path, started, finished, code, route, log, run_id = sys.argv[1:]
with open(path, "a", encoding="utf-8") as history:
    history.write(
        json.dumps(
            {
                "job": "quota-freshness-summary",
                "runId": run_id,
                "startedAt": started,
                "finishedAt": finished,
                "exitCode": int(code),
                "route": route,
                "log": log,
            },
            sort_keys=True,
        )
        + "\n"
    )
PY
cat "$LOG"
exit "$EXIT_CODE"
