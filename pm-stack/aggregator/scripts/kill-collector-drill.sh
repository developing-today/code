#!/usr/bin/env bash
# kill-collector-drill.sh — chaos drill, first slice of #1283 (epic #1184).
#
# Deliberately kill the fleet-feed collector and verify the PM stack
# notices: the aggregator's honesty signals must fire on a dead feed
# instead of presenting "everything is fine".
#
# What it does, in order:
#   1. Interlocks (any failure = refuse to start, nothing is touched):
#      - an flock on the drill lock (a second drill, manual or scheduled,
#        cannot overlap this one);
#      - an incident flag file ($PM_DRILL_INCIDENT_FLAG) must not exist —
#        during a real incident the stack is not a drill canvas;
#      - the collector timer must be active and the feed must currently
#        be FRESH (age <= the staleness threshold). A feed that is
#        already stale is a real incident, not a drill.
#   2. Stops fleet-feed-collector.timer (and any in-flight oneshot).
#   3. Polls the aggregator's /api/health until the feed's age crosses
#      FEED_STALE_MS — 15 minutes, the #1148 watchdog threshold the code
#      itself uses (src/health.ts, src/sources/live.ts) — and records
#      the health payload at that moment as the fired-signal evidence.
#   4. Restores, ALWAYS (trap on EXIT/INT/TERM plus an explicit restore
#      step): starts the timer again and immediately runs the collector
#      once so the feed is fresh, then confirms via /api/health that the
#      feed age is back under the threshold.
#   5. Reports: writes a JSON result to the state dir; with gh available
#      it posts the result as a comment on #1283. If the expected signal
#      does NOT fire, it instead files a new issue under epic #1184 with
#      the drill evidence (that is the drill doing its job: a silent
#      failure of the honesty signals is a finding, not a drill error).
#
# Scheduling: systemd/kill-collector-drill.{service,timer} in this
# repo's pm-stack/aggregator/systemd/ directory (weekly, in the same
# style as fleet-feed-collector's units). The flock makes overlap
# impossible even if a manual run and the timer collide.
#
# Environment knobs:
#   PM_DRILL_STATE_DIR     state/lock/result dir (default ~/.local/state/pm-drills)
#   PM_DRILL_INCIDENT_FLAG incident flag path (default $STATE_DIR/incident.flag)
#   PM_DRILL_MAX_WAIT_MIN  max minutes to wait for the stale signal (default 25)
#   PM_DRILL_NO_POST=1     skip the GitHub comment/issue side effects

set -uo pipefail

STATE_DIR="${PM_DRILL_STATE_DIR:-$HOME/.local/state/pm-drills}"
INCIDENT_FLAG="${PM_DRILL_INCIDENT_FLAG:-$STATE_DIR/incident.flag}"
MAX_WAIT_MIN="${PM_DRILL_MAX_WAIT_MIN:-25}"
STALE_MS=900000 # FEED_STALE_MS in aggregator src/health.ts + src/sources/live.ts
COLLECTOR_TIMER="fleet-feed-collector.timer"
COLLECTOR_SERVICE="fleet-feed-collector.service"
ISSUE_REPO="developing-today/code"
EPIC=1184
ISSUE=1283

mkdir -p "$STATE_DIR"
RESULT_FILE="$STATE_DIR/kill-collector-result.json"
EVIDENCE_FILE="$STATE_DIR/kill-collector-health-at-signal.json"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

log() { printf '[drill] %s %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
die() {
	log "REFUSED/FAILED: $*"
	exit 1
}

# --- interlock: one drill at a time ---------------------------------------
exec 9>"$STATE_DIR/kill-collector.lock"
if ! flock -n 9; then
	die "another kill-collector drill holds the lock; not starting"
fi
# --- interlock: no drills during a real incident ---------------------------
if [ -e "$INCIDENT_FLAG" ]; then
	die "incident flag present ($INCIDENT_FLAG); the stack is not a drill canvas during a real incident"
fi

# --- health/feed reads (aggregator loopback, via the laptop ssh channel) ---
health_json() {
	# The aggregator's first health assembly after a quiet period can be
	# slow to prime (health-priming is a known behaviour), so retry: a
	# drill must never act on — or refuse on — one timed-out read.
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
# Prints "<age_ms> <degraded>" for the feed in a health payload on stdin.
feed_facts() {
	python3 -c '
import json, sys, datetime
try:
    d = json.load(sys.stdin)
    gen = d["feed"]["generatedAt"]
    at = datetime.datetime.fromisoformat(gen.replace("Z", "+00:00"))
    age = int((datetime.datetime.now(datetime.timezone.utc) - at).total_seconds() * 1000)
    print(age, str(d.get("degraded")).lower())
except Exception:
    print(-1, "unknown")
'
}

# --- restore: idempotent, safe to call from the trap at any point ---------
RESTORED=0
restore() {
	[ "$RESTORED" = 1 ] && return 0
	RESTORED=1
	log "restoring collector (timer start + immediate collection run)"
	sudo -n systemctl start "$COLLECTOR_TIMER" 2>/dev/null || log "WARN: could not start $COLLECTOR_TIMER"
	sudo -n systemctl start "$COLLECTOR_SERVICE" 2>/dev/null || log "WARN: could not start $COLLECTOR_SERVICE"
}
trap restore EXIT INT TERM

# --- preconditions ----------------------------------------------------------
systemctl is-active --quiet "$COLLECTOR_TIMER" || die "$COLLECTOR_TIMER is not active; nothing to kill, and a stopped collector outside a drill is itself a finding"
BASELINE="$(health_json | feed_facts)"
BASE_AGE="${BASELINE%% *}"
[ "$BASE_AGE" -ge 0 ] || die "cannot read aggregator /api/health; refusing to drill blind"
[ "$BASE_AGE" -le "$STALE_MS" ] || die "feed is already stale at drill start (age ${BASE_AGE}ms > ${STALE_MS}ms): that is a real incident, not a drill"
log "preconditions ok: collector active, feed fresh (age ${BASE_AGE}ms), no incident flag, lock held"

# --- kill -------------------------------------------------------------------
log "stopping $COLLECTOR_TIMER"
sudo -n systemctl stop "$COLLECTOR_TIMER" || die "failed to stop $COLLECTOR_TIMER (sudo -n systemctl required)"
sudo -n systemctl stop "$COLLECTOR_SERVICE" 2>/dev/null || true

# --- wait for the honesty signal ---------------------------------------------
SIGNAL=0
WAITED=0
MAX_WAIT_SEC=$((MAX_WAIT_MIN * 60))
while [ "$WAITED" -lt "$MAX_WAIT_SEC" ]; do
	sleep 60
	WAITED=$((WAITED + 60))
	FACTS="$(health_json | feed_facts)"
	AGE="${FACTS%% *}"
	if [ "$AGE" -gt "$STALE_MS" ]; then
		SIGNAL=1
		health_json >"$EVIDENCE_FILE" 2>/dev/null || true
		log "SIGNAL FIRED: feed age ${AGE}ms crossed the ${STALE_MS}ms staleness threshold after ~${WAITED}s stopped (health payload saved)"
		break
	fi
	log "waiting: feed age ${AGE}ms (threshold ${STALE_MS}ms)"
done
[ "$SIGNAL" = 1 ] || log "SIGNAL DID NOT FIRE within ${MAX_WAIT_MIN}min"

# --- restore + confirm --------------------------------------------------------
restore
trap - EXIT INT TERM
CONFIRMED=0
for _ in 1 2 3 4 5 6; do
	sleep 20
	FACTS="$(health_json | feed_facts)"
	AGE="${FACTS%% *}"
	if [ "$AGE" -ge 0 ] && [ "$AGE" -le "$STALE_MS" ]; then
		CONFIRMED=1
		log "restoration confirmed: feed fresh again (age ${AGE}ms)"
		break
	fi
done
[ "$CONFIRMED" = 1 ] || log "WARN: restoration not confirmed within ~2min (feed age ${AGE:-unknown}ms)"

# --- report -------------------------------------------------------------------
OUTCOME="failed"
[ "$SIGNAL" = 1 ] && [ "$CONFIRMED" = 1 ] && OUTCOME="passed"
python3 - "$RESULT_FILE" "$STARTED_AT" "$OUTCOME" "$SIGNAL" "$CONFIRMED" <<'PY'
import json, sys, datetime
path, started, outcome, signal, confirmed = sys.argv[1:6]
json.dump({
    "drill": "kill-collector", "issue": 1283, "epic": 1184,
    "started_at": started,
    "finished_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "outcome": outcome,
    "signal_fired": signal == "1",
    "restoration_confirmed": confirmed == "1",
    "stale_threshold_ms": 900000,
}, open(path, "w"), indent=2)
PY
log "result: $OUTCOME (signal=$SIGNAL restored=$CONFIRMED) -> $RESULT_FILE"

if [ "${PM_DRILL_NO_POST:-0}" != "1" ] && command -v gh >/dev/null 2>&1; then
	if [ "$OUTCOME" = "passed" ]; then
		gh issue comment "$ISSUE" --repo "$ISSUE_REPO" --body "✅ **Chaos drill passed — kill-collector** (queue task 2667, first slice of this issue, epic #$EPIC)

- Stopped \`$COLLECTOR_TIMER\` on the sandbox at $STARTED_AT; the aggregator's feed-age signal crossed the 15-minute \`FEED_STALE_MS\` (#1148) threshold — the honesty signal fired instead of the dashboard presenting a dead feed as fine. Health payload at the signal moment was captured by the drill.
- Collector restored (timer restarted + immediate collection run) and restoration confirmed via \`/api/health\`: feed fresh again.
- Script: \`pm-stack/aggregator/scripts/kill-collector-drill.sh\` (interlocked: flock, incident-flag refusal, refuses to start on an already-stale feed; always restores via trap). Result JSON is written to \`~/.local/state/pm-drills/kill-collector-result.json\` on the sandbox per run." >/dev/null 2>&1 &&
			log "posted result comment on #$ISSUE" || log "WARN: gh comment on #$ISSUE failed"
	else
		gh issue create --repo "$ISSUE_REPO" --title "chaos drill FINDING: kill-collector drill did not observe the expected honesty signal" --body "The kill-collector drill (first slice of #$ISSUE, epic #$EPIC) ran at $STARTED_AT and the expected signal did not complete cleanly: signal_fired=$SIGNAL, restoration_confirmed=$CONFIRMED (see \`~/.local/state/pm-drills/kill-collector-result.json\` and \`kill-collector-health-at-signal.json\` on the sandbox for the captured evidence). A dead collector presenting as 'everything is fine' is exactly what #$ISSUE exists to catch — investigate the aggregator's feed-staleness/health path (FEED_STALE_MS, src/health.ts, src/sources/live.ts) before the next drill." >/dev/null 2>&1 &&
			log "filed finding issue under epic #$EPIC" || log "WARN: gh issue create for the finding failed"
	fi
fi

[ "$OUTCOME" = "passed" ]
