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
#      - the legacy collector timer must be inactive and the feed must currently
#        be FRESH (age < the staleness threshold). A feed that is
#        already stale is a real incident, not a drill.
#   2. Creates the pm-core pause file; subsequent in-process collector ticks
#      leave the existing feed untouched until the drill restores it.
#      Limitation: the gate is read only when a tick starts (pm-core-wiring.ts
#      runFleetFeed). A collector run already in flight when the file appears
#      finishes and may write one more fresh stamp. That delays the signal by
#      up to one collector run; it cannot produce a false pass, because the
#      drill passes only once the feed age actually crosses FEED_STALE_MS.
#   3. Polls the aggregator's /api/health until the feed's age crosses
#      FEED_STALE_MS — 5 minutes, the #1148 watchdog threshold the code
#      itself uses (src/health.ts, src/sources/live.ts) — and records
#      the health payload at that moment as the fired-signal evidence.
#   4. Restores, ALWAYS once the kill has happened (trap on
#      EXIT/INT/TERM plus an explicit restore step): removes the pause
#      file so the next in-process tick can refresh the feed, then confirms
#      via /api/health that the feed age is back
#      under the threshold. The restore is gated on the kill: a drill
#      that refuses to start (step 1, or the preconditions below)
#      touches nothing — no pause file is created on any refusal path.
#   5. Reports: writes a JSON result to the state dir; with gh available
#      it posts the result as a comment on #1283. If the expected signal
#      does NOT fire, it instead files a new issue under epic #1184 with
#      the drill evidence (that is the drill doing its job: a silent
#      failure of the honesty signals is a finding, not a drill error).
#
# The collector now runs inside the laptop aggregator. The drill is
# laptop-only and pauses that in-process job through its state-file gate.
#
# Environment knobs:
#   PM_DRILL_STATE_DIR     state/lock/result dir (default ~/.local/state/pm-drills)
#   PM_DRILL_INCIDENT_FLAG incident flag path (default $STATE_DIR/incident.flag)
#   PM_DRILL_MAX_WAIT_MIN  max minutes to wait for the stale signal (default 25)
#   PM_DRILL_NO_POST=1     skip the GitHub comment/issue side effects

set -uo pipefail
# Argument contract (#3352): this script takes no command-line
# arguments; anything given is a usage error before any work runs.
if [ "$#" -gt 0 ]; then
  echo "kill-collector-drill.sh: unexpected argument '$1'; usage: bash scripts/kill-collector-drill.sh (knobs are PM_DRILL_* environment variables)" >&2
  exit 2
fi

STATE_DIR="${PM_DRILL_STATE_DIR:-$HOME/.local/state/pm-drills}"
INCIDENT_FLAG="${PM_DRILL_INCIDENT_FLAG:-$STATE_DIR/incident.flag}"
MAX_WAIT_MIN="${PM_DRILL_MAX_WAIT_MIN:-25}"
STALE_MS=300000 # FEED_STALE_MS in aggregator src/feed.ts
COLLECTOR_TIMER="fleet-feed-collector.timer"
AGG_ENV_FILE="$HOME/.local/state/pm-aggregator/.env"
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

# The collector and aggregator now run on the laptop (amd), not on the
# sandbox. Never let a stale checkout or copied script stop a sandbox
# service by mistake.
HOST="$(hostname -s)"
[ "$HOST" = "amd" ] || die "must run on laptop host 'amd'; refusing on '$HOST'"

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

# --- restore: idempotent, and gated on the kill ---------------------------
# The trap is installed before the precondition checks, so without the
# gate a REFUSAL (die -> exit) would fire restore() and remove the
# collector the drill just refused to touch — masking the very
# findings the interlocks exist to surface (#2865). KILLED is armed
# immediately BEFORE pause-file creation — after every
# precondition refusal — so until that point restore() is a no-op and
# every refusal path leaves the system exactly as it found it, while
# a signal interrupting pause-file creation still removes a file that
# may already have been created.
KILLED=0
RESTORED=0
restore() {
  [ "$KILLED" = 1 ] || return 0
  [ "$RESTORED" = 1 ] && return 0
  RESTORED=1
  log "restoring collector (removing pm-core pause file)"
  rm -f "$PAUSE_FILE" || log "WARN: could not remove $PAUSE_FILE"
}
trap restore EXIT INT TERM

# --- pause file: derive exactly as the aggregator does, or refuse ----------
# pm-core-wiring.ts resolves the pause file as PM_CORE_FEED_PAUSE_FILE, else
# <dirname of PM_AGG_FEED_FILE>/fleet-feed.pause. The aggregator reads its
# config from AGG_ENV_FILE (EnvironmentFile of pm-aggregator.service). A
# drill that pauses a different file than the aggregator reads would pass its
# honesty signal without pausing anything, so refuse on any disagreement.
agg_env_value() {
  [ -r "$AGG_ENV_FILE" ] || return 0
  sed -n "s/^$1=//p" "$AGG_ENV_FILE" | tail -n 1
}
[ -r "$AGG_ENV_FILE" ] || die "cannot read aggregator config $AGG_ENV_FILE; refusing to pause a collector path it may not read"
AGG_FEED_FILE="$(agg_env_value PM_AGG_FEED_FILE)"
AGG_FEED_FILE="${AGG_FEED_FILE:-$HOME/.local/state/pm-aggregator/fleet-feed.json}"
AGG_PAUSE_FILE="$(agg_env_value PM_CORE_FEED_PAUSE_FILE)"
AGG_PAUSE_FILE="${AGG_PAUSE_FILE:-$(dirname "$AGG_FEED_FILE")/fleet-feed.pause}"
if [ -n "${PM_CORE_FEED_PAUSE_FILE:-}" ] && [ "$PM_CORE_FEED_PAUSE_FILE" != "$AGG_PAUSE_FILE" ]; then
  die "PM_CORE_FEED_PAUSE_FILE ($PM_CORE_FEED_PAUSE_FILE) disagrees with the aggregator's pause file ($AGG_PAUSE_FILE); refusing"
fi
PAUSE_FILE="$AGG_PAUSE_FILE"

# --- preconditions ----------------------------------------------------------
if systemctl is-active --quiet "$COLLECTOR_TIMER"; then
  die "$COLLECTOR_TIMER is still active; pm-core drill requires the legacy timer to be retired"
fi
[ ! -e "$PAUSE_FILE" ] || die "collector pause file already exists ($PAUSE_FILE); refusing to inherit another pause"
BASELINE="$(health_json | feed_facts)"
BASE_AGE="${BASELINE%% *}"
[ "$BASE_AGE" -ge 0 ] || die "cannot read aggregator /api/health; refusing to drill blind"
[ "$BASE_AGE" -lt "$STALE_MS" ] || die "feed is already stale at drill start (age ${BASE_AGE}ms >= ${STALE_MS}ms): that is a real incident, not a drill"
log "preconditions ok: legacy timer inactive, feed fresh (age ${BASE_AGE}ms), no incident flag, lock held"

# --- pause -----------------------------------------------------------------
# Arm restoration BEFORE creating the pause file: if TERM/INT lands
# during creation, restore removes it if it exists. Every precondition
# refusal is above this point, so restoration never masks a refusal.
KILLED=1
touch "$PAUSE_FILE" || die "failed to create collector pause file ($PAUSE_FILE)"

# --- wait for the honesty signal ---------------------------------------------
SIGNAL=0
WAITED=0
MAX_WAIT_SEC=$((MAX_WAIT_MIN * 60))
while [ "$WAITED" -lt "$MAX_WAIT_SEC" ]; do
  sleep 60
  WAITED=$((WAITED + 60))
  FACTS="$(health_json | feed_facts)"
  AGE="${FACTS%% *}"
  if [ "$AGE" -ge "$STALE_MS" ]; then
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
for _ in $(seq 1 20); do
  sleep 20
  FACTS="$(health_json | feed_facts)"
  AGE="${FACTS%% *}"
  if [ "$AGE" -ge 0 ] && [ "$AGE" -lt "$STALE_MS" ]; then
    CONFIRMED=1
    log "restoration confirmed: feed fresh again (age ${AGE}ms)"
    break
  fi
done
[ "$CONFIRMED" = 1 ] || log "WARN: restoration not confirmed within ~7min (feed age ${AGE:-unknown}ms)"

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
    "stale_threshold_ms": 300000,
}, open(path, "w"), indent=2)
PY
log "result: $OUTCOME (signal=$SIGNAL restored=$CONFIRMED) -> $RESULT_FILE"

if [ "${PM_DRILL_NO_POST:-0}" != "1" ] && command -v gh >/dev/null 2>&1; then
  if [ "$OUTCOME" = "passed" ]; then
    gh issue comment "$ISSUE" --repo "$ISSUE_REPO" --body "✅ **Chaos drill passed — kill-collector** (queue task 2667, first slice of this issue, epic #$EPIC)

- Paused pm-core's in-process fleet-feed scheduler on laptop host amd at $STARTED_AT; the aggregator's feed-age signal reached the 5-minute \`FEED_STALE_MS\` (#1148) threshold — the honesty signal fired instead of the dashboard presenting a dead feed as fine. Health payload at the signal moment was captured by the drill.
- Collector restored by removing the pause file; restoration was confirmed via \`/api/health\` after an in-process tick: feed fresh again.
- Script: \`pm-stack/aggregator/scripts/kill-collector-drill.sh\` (interlocked: flock, incident-flag refusal, refuses to start on an already-stale feed; always restores via trap). Result JSON is written to \`~/.local/state/pm-drills/kill-collector-result.json\` on laptop host amd per run." >/dev/null 2>&1 &&
      log "posted result comment on #$ISSUE" || log "WARN: gh comment on #$ISSUE failed"
  else
    gh issue create --repo "$ISSUE_REPO" --title "chaos drill FINDING: kill-collector drill did not observe the expected honesty signal" --body "The kill-collector drill (first slice of #$ISSUE, epic #$EPIC) ran at $STARTED_AT and the expected signal did not complete cleanly: signal_fired=$SIGNAL, restoration_confirmed=$CONFIRMED (see \`~/.local/state/pm-drills/kill-collector-result.json\` and \`kill-collector-health-at-signal.json\` on laptop host amd for the captured evidence). A dead collector presenting as 'everything is fine' is exactly what #$ISSUE exists to catch — investigate the aggregator's feed-staleness/health path (FEED_STALE_MS, src/health.ts, src/sources/live.ts) before the next drill." >/dev/null 2>&1 &&
      log "filed finding issue under epic #$EPIC" || log "WARN: gh issue create for the finding failed"
  fi
fi

[ "$OUTCOME" = "passed" ]
