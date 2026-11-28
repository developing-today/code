#!/usr/bin/env bash
# C3 Slice D drill driver (issue #9571). Runs REAL failure + recovery
# cycles for both C3 health hooks against disposable local stand-ins:
#   pm-stack-reachable -> aggregator stand-in on 127.0.0.1:14870
#   opencode-working   -> OpenCode stand-in on 127.0.0.1:13420 (the exact
#                         production port the hook probes; verified free
#                         in this sandbox before the drill)
# All hook state lands under /tmp/c3-drill; nothing production is touched.
set -u
WT="$HOME/workspace/issue-9571/wt"
HOOKS="$WT/pm-stack/health-hooks"
D=/tmp/c3-drill
export ALERTS_INTAKE_URL="http://127.0.0.1:14870"
export PM_PRIMARY_CHECKOUT_DIR="$WT"

ts() { date -u +%Y-%m-%dT%H:%M:%SZ; }
log() { echo "[$(ts)] $*" | tee -a "$D/log.txt"; }

mkdir -p "$D"
: >"$D/log.txt"
: >"$D/captures.jsonl"
rm -f "$D/pm-incidents.jsonl" "$D/oc-incidents.jsonl"

fire() { # $1=wrapper $2=incident-file $3=out-name $4=observed-at
  PM_HEALTH_HOOKS_INCIDENT_FILE="$2" "$HOOKS/$1" --caller slice-d-drill \
    --observed-at "$4" --detail "Slice D validation drill (queue task 20918)" \
    >"$D/$3" 2>"$D/$3.err"
  echo "exit=$? -> $3"
}

log "=== PM-STACK-REACHABLE CYCLE ==="
log "PM-1 hard down: nothing listening on 14870"
fire pm-stack-reachable.sh "$D/pm-incidents.jsonl" pm1.json "$(ts)"
sleep 1
echo 503 >"$D/board_status"
python3 "$D/capture_server.py" aggregator 14870 "$D" &
AGG_PID=$!
sleep 1
log "PM-2 surface down (GET /board 503), intake up: POST capture expected"
fire pm-stack-reachable.sh "$D/pm-incidents.jsonl" pm2.json "$(ts)"
sleep 1
echo 200 >"$D/board_status"
log "PM-3 recovered: GET /board 200"
fire pm-stack-reachable.sh "$D/pm-incidents.jsonl" pm3.json "$(ts)"
sleep 1
log "PM-4 steady state: expect transient, no new record"
fire pm-stack-reachable.sh "$D/pm-incidents.jsonl" pm4.json "$(ts)"

log "=== OPENCODE-WORKING CYCLE ==="
log "OC-1 hard down: nothing listening on 13420 (verified free pre-drill)"
fire opencode-working.sh "$D/oc-incidents.jsonl" oc1.json "$(ts)"
sleep 1
python3 "$D/capture_server.py" opencode 13420 "$D" &
OC_PID=$!
sleep 1
log "OC-2 recovered: stand-in serving /global/health 200 on 13420"
fire opencode-working.sh "$D/oc-incidents.jsonl" oc2.json "$(ts)"
sleep 1
log "OC-3 steady state: expect transient, no new record"
fire opencode-working.sh "$D/oc-incidents.jsonl" oc3.json "$(ts)"

kill $OC_PID $AGG_PID 2>/dev/null
wait $OC_PID 2>/dev/null
wait $AGG_PID 2>/dev/null
log "stand-ins stopped; ports released"
log "=== DRILL DONE ==="
