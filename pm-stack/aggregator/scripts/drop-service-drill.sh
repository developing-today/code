#!/usr/bin/env bash
# drop-service-drill.sh — chaos drill, third slice of #1283 (epic #1184).
#
# Deliberately stop one PM-stack component service (default:
# pm-projector, whose health entry the aggregator HTTP-probes) and
# verify the stack notices: the aggregator's /api/health must report
# the dropped service as down and the stack as degraded, instead of
# presenting "everything is fine" over a dead component.
#
# What it does, in order:
#   1. Interlocks (any failure = refuse to start, nothing is touched):
#      - the shared drill lock ($PM_DRILL_STATE_DIR/pm-drill.lock) —
#        one chaos drill at a time across ALL drill kinds, not just
#        this one; this drill also try-locks the legacy per-drill lock
#        files of the kill-collector and age-feed drills, because
#        those predate the shared lock (see DRILLS.md);
#      - this drill's own lock (drop-service.lock);
#      - an incident flag file ($PM_DRILL_INCIDENT_FLAG) must not
#        exist — during a real incident the stack is not a drill
#        canvas;
#      - the target service must be active and its /api/health entry
#        must currently be up. A target that is already down is a
#        real incident, not a drill.
#   2. Stops the target service (systemctl --user; the PM services
#      are laptop user units since #3362/#3363).
#   3. Polls /api/health until the target's entry reports ok=false
#      (probe cache is 60s, so this can take a couple of minutes) and
#      records the health payload at that moment as evidence. Both
#      named signals are checked: the service entry down AND the
#      top-level degraded flag true.
#   4. Restores, ALWAYS once the stop has happened (trap on
#      EXIT/INT/TERM plus an explicit restore step): starts the
#      service again and confirms via /api/health that its entry is
#      back to ok=true. The restore is gated on the stop: a drill
#      that refuses to start touches nothing (the #2865 lesson).
#   5. Reports: writes a JSON result to the state dir; with gh
#      available it posts the result as a comment on #1283. If an
#      expected signal does NOT fire, it instead files a new issue
#      under epic #1184 with the drill evidence.
#
# Laptop-only, like the other drills: the services and the aggregator
# live on the laptop. The host guard protects manual runs.
#
# Environment knobs:
#   PM_DRILL_STATE_DIR     state/lock/result dir (default ~/.local/state/pm-drills)
#   PM_DRILL_INCIDENT_FLAG incident flag path (default $STATE_DIR/incident.flag)
#   PM_DRILL_TARGET_UNIT   systemd user unit to drop (default pm-projector.service)
#   PM_DRILL_TARGET_ID     /api/health services[] id for the unit (default projector)
#   PM_DRILL_MAX_WAIT_MIN  max minutes to wait for the down signal (default 6)
#   PM_DRILL_NO_POST=1     skip the GitHub comment/issue side effects

set -uo pipefail

STATE_DIR="${PM_DRILL_STATE_DIR:-$HOME/.local/state/pm-drills}"
INCIDENT_FLAG="${PM_DRILL_INCIDENT_FLAG:-$STATE_DIR/incident.flag}"
MAX_WAIT_MIN="${PM_DRILL_MAX_WAIT_MIN:-6}"
TARGET_UNIT="${PM_DRILL_TARGET_UNIT:-pm-projector.service}"
TARGET_ID="${PM_DRILL_TARGET_ID:-projector}"
ISSUE_REPO="developing-today/code"
EPIC=1184
ISSUE=1283

mkdir -p "$STATE_DIR"
RESULT_FILE="$STATE_DIR/drop-service-result.json"
EVIDENCE_FILE="$STATE_DIR/drop-service-health-at-signal.json"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

log() { printf '[drill] %s %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
die() {
  log "REFUSED/FAILED: $*"
  exit 1
}

HOST="$(hostname -s)"
[ "$HOST" = "amd" ] || die "must run on laptop host 'amd'; refusing on '$HOST'"

# --- interlock: one drill at a time, across all drill kinds -----------------
exec 9>"$STATE_DIR/pm-drill.lock"
if ! flock -n 9; then
  die "another chaos drill holds the shared drill lock; not starting"
fi
exec 8>"$STATE_DIR/drop-service.lock"
if ! flock -n 8; then
  die "another drop-service drill holds the lock; not starting"
fi
# Legacy drills predate the shared lock and take only their own lock
# files; refuse if one of those is held right now.
for legacy in kill-collector age-feed; do
  exec 7>"$STATE_DIR/$legacy.lock"
  if ! flock -n 7; then
    die "a $legacy drill holds its lock; not starting"
  fi
  exec 7>&-
done
# --- interlock: no drills during a real incident ---------------------------
if [ -e "$INCIDENT_FLAG" ]; then
  die "incident flag present ($INCIDENT_FLAG); the stack is not a drill canvas during a real incident"
fi

# --- health reads (aggregator loopback, via the laptop ssh channel) --------
health_json() {
  local attempt
  for attempt in 1 2 3; do
    out="$(~/tools/laptop-ssh.sh 'curl -s --max-time 25 http://127.0.0.1:4870/api/health' 2>/dev/null)" && [ -n "$out" ] && {
      printf '%s' "$out"
      return 0
    }
    sleep 5
  done
  return 1
}
# Prints "<ok> <degraded>" for $TARGET_ID in a health payload on stdin;
# ok is true/false/unknown.
target_facts() {
  python3 -c '
import json, sys
target = sys.argv[1]
try:
    d = json.load(sys.stdin)
    svc = next((s for s in d.get("services", []) if s.get("def", {}).get("id") == target or s.get("id") == target), None)
    if svc is None:
        print("unknown", str(d.get("degraded")).lower())
    else:
        ok = svc.get("ok")
        print("true" if ok is True else "false" if ok is False else "unknown", str(d.get("degraded")).lower())
except Exception:
    print("unknown", "unknown")
' "$TARGET_ID"
}

# --- restore: idempotent, and gated on the stop ----------------------------
STOPPED=0
RESTORED=0
restore() {
  [ "$STOPPED" = 1 ] || return 0
  [ "$RESTORED" = 1 ] && return 0
  RESTORED=1
  log "restoring $TARGET_UNIT"
  systemctl --user start "$TARGET_UNIT" 2>/dev/null || log "WARN: could not start $TARGET_UNIT"
}
trap restore EXIT INT TERM

# --- preconditions -----------------------------------------------------------
systemctl --user is-active --quiet "$TARGET_UNIT" || die "$TARGET_UNIT is not active; nothing to drop, and a stopped component outside a drill is itself a finding"
BASELINE="$(health_json | target_facts)"
BASE_OK="${BASELINE%% *}"
[ "$BASE_OK" != "unknown" ] || die "cannot read aggregator /api/health for target '$TARGET_ID'; refusing to drill blind"
[ "$BASE_OK" = "true" ] || die "target '$TARGET_ID' is already down at drill start: that is a real incident, not a drill"
log "preconditions ok: $TARGET_UNIT active, health entry for '$TARGET_ID' up, no incident flag, locks held"

# --- drop --------------------------------------------------------------------
log "stopping $TARGET_UNIT"
# Arm restoration BEFORE the destructive stop (#2865 / Codex P2 on
# PR #3027): a TERM/INT landing while the stop is in flight must
# still restore a service the stop may already have deactivated.
STOPPED=1
systemctl --user stop "$TARGET_UNIT" || die "failed to stop $TARGET_UNIT"

# --- wait for the honesty signals ---------------------------------------------
SIGNAL_DOWN=0
SIGNAL_DEGRADED=0
DEADLINE=$(( $(date +%s) + MAX_WAIT_MIN * 60 ))
while [ "$(date +%s)" -lt "$DEADLINE" ]; do
  PAYLOAD="$(health_json)" || PAYLOAD=""
  if [ -n "$PAYLOAD" ]; then
    FACTS="$(printf '%s' "$PAYLOAD" | target_facts)"
    OK="${FACTS%% *}"
    DEGRADED="${FACTS##* }"
    if [ "$OK" = "false" ]; then
      SIGNAL_DOWN=1
      [ "$DEGRADED" = "true" ] && SIGNAL_DEGRADED=1
      printf '%s' "$PAYLOAD" > "$EVIDENCE_FILE"
      log "signal fired: '$TARGET_ID' reports down (degraded=$DEGRADED); evidence -> $EVIDENCE_FILE"
      break
    fi
  fi
  sleep 20
done
[ "$SIGNAL_DOWN" = 1 ] || log "signal did NOT fire within ${MAX_WAIT_MIN}min: '$TARGET_ID' never reported down"

# --- restore + confirm ----------------------------------------------------------
restore
CONFIRMED=0
for _ in 1 2 3 4 5 6; do
  sleep 20
  FACTS="$(health_json | target_facts)"
  OK="${FACTS%% *}"
  if [ "$OK" = "true" ]; then
    CONFIRMED=1
    log "restoration confirmed: '$TARGET_ID' reports up again"
    break
  fi
done
[ "$CONFIRMED" = 1 ] || log "WARN: restoration not confirmed within ~2min (target ok=${OK:-unknown})"

# --- report ----------------------------------------------------------------------
OUTCOME="failed"
[ "$SIGNAL_DOWN" = 1 ] && [ "$SIGNAL_DEGRADED" = 1 ] && [ "$CONFIRMED" = 1 ] && OUTCOME="passed"
python3 - "$RESULT_FILE" "$STARTED_AT" "$OUTCOME" "$SIGNAL_DOWN" "$SIGNAL_DEGRADED" "$CONFIRMED" "$TARGET_UNIT" "$TARGET_ID" <<'PY'
import json, sys, datetime
path, started, outcome, down, degraded, confirmed, unit, target = sys.argv[1:9]
json.dump({
    "drill": "drop-service", "issue": 1283, "epic": 1184,
    "started_at": started,
    "finished_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "outcome": outcome,
    "target_unit": unit,
    "target_id": target,
    "signal_service_down": down == "1",
    "signal_degraded": degraded == "1",
    "restoration_confirmed": confirmed == "1",
}, open(path, "w"), indent=2)
PY
log "result: $OUTCOME (down=$SIGNAL_DOWN degraded=$SIGNAL_DEGRADED restored=$CONFIRMED) -> $RESULT_FILE"

if [ "${PM_DRILL_NO_POST:-0}" != "1" ] && command -v gh >/dev/null 2>&1; then
  if [ "$OUTCOME" = "passed" ]; then
    gh issue comment "$ISSUE" --repo "$ISSUE_REPO" --body "✅ **Chaos drill passed — drop-service** (slice of this issue, epic #$EPIC)

- Stopped \`$TARGET_UNIT\` on laptop host amd at $STARTED_AT; the aggregator's /api/health reported the \`$TARGET_ID\` entry down (ok=false) and the stack degraded — the honesty signals fired instead of the health page presenting a dead component as fine. Health payload at the signal moment was captured by the drill.
- Service restored and restoration confirmed via /api/health: \`$TARGET_ID\` up again.
- Script: \`pm-stack/aggregator/scripts/drop-service-drill.sh\` (interlocked: shared drill lock + legacy-lock checks, incident-flag refusal, refuses to start on an already-down target; always restores via trap). Result JSON is written to \`~/.local/state/pm-drills/drop-service-result.json\` on laptop host amd per run." >/dev/null 2>&1 &&
      log "posted result comment on #$ISSUE" || log "WARN: gh comment on #$ISSUE failed"
  else
    gh issue create --repo "$ISSUE_REPO" --title "chaos drill FINDING: drop-service drill did not observe the expected honesty signal" --body "The drop-service drill (slice of #$ISSUE, epic #$EPIC) ran at $STARTED_AT against \`$TARGET_UNIT\` and the expected signals did not complete cleanly: signal_service_down=$SIGNAL_DOWN, signal_degraded=$SIGNAL_DEGRADED, restoration_confirmed=$CONFIRMED (see \`~/.local/state/pm-drills/drop-service-result.json\` and \`drop-service-health-at-signal.json\` on laptop host amd for the captured evidence). A dead component presenting as 'everything is fine' is exactly what #$ISSUE exists to catch — investigate the aggregator's health/topology probe path (src/health.ts, src/topology.ts) before the next drill." >/dev/null 2>&1 &&
      log "filed finding issue under epic #$EPIC" || log "WARN: gh issue create for the finding failed"
  fi
fi

[ "$OUTCOME" = "passed" ]
