#!/usr/bin/env bash
# age-feed-drill.sh — chaos drill, second slice of #1283 (epic #1184).
#
# Deliberately age the fleet feed while the collector keeps running, and
# verify the PM stack notices a feed that is alive but OLD — the failure
# mode the kill-collector drill (first slice, #1823) cannot reach: there
# the collector is dead, here it runs on schedule and the data it
# presents is stale, so "everything is fine" is a lie the process tree
# does not contradict.
#
# What it does, in order:
#   1. Interlocks (any failure = refuse to start, nothing is touched):
#      - flock on the drill lock (a second drill, manual or scheduled,
#        cannot overlap this one);
#      - the incident flag file ($PM_DRILL_INCIDENT_FLAG) must not exist —
#        during a real incident the stack is not a drill canvas;
#      - the collector timer must be active (this drill's premise is a
#        running collector) and the feed must currently be FRESH. An
#        already-stale feed is a real incident, not a drill.
#   2. Saves the feed's exact bytes, then arms restore (trap on
#      EXIT/INT/TERM) immediately BEFORE the first write, so every
#      refusal path above leaves the feed untouched.
#   3. Ages the feed by rewriting its top-level generatedAt into the past
#      (PM_DRILL_AGE_MS, default 20 min > the 15-min FEED_STALE_MS in
#      src/health.ts + src/sources/live.ts) while leaving every other
#      field intact. The collector keeps running on its ~5-min timer; if
#      it rewrites the feed with a fresh stamp mid-drill, the drill
#      re-ages it, which is exactly the "collector runs, data is old"
#      scenario.
#   4. Polls /api/health until feed.ageMs crosses FEED_STALE_MS and the
#      payload reports degraded, saving that payload as the fired-signal
#      evidence; it also records (best effort) whether the #1148
#      watchdog's `watchdog:feed` tier item appeared on
#      /api/attention.json.
#   5. Restores ALWAYS once the aging has happened: the saved bytes are
#      written back atomically and the drill waits (up to
#      PM_DRILL_RESTORE_WAIT_MIN) for the collector's next scheduled run
#      to make the feed fresh again, confirming via /api/health. The
#      restore is gated on the aging: a drill that refuses to start
#      touches nothing.
#   6. Reports: writes a JSON result to the state dir; with gh available
#      it posts the result as a comment on #1283. If the expected signal
#      does NOT fire, it instead files a new issue under epic #1184 with
#      the drill evidence (a stale-but-alive feed passing as fresh is a
#      finding, not a drill error).
#
# Where it runs: on the host that owns the feed and the aggregator —
# the laptop, where the collector moved on 2026-10-31 (code#3362/#3363).
# The collector/aggregator are NixOS user units, so the timer check uses
# `systemctl --user`; nothing here needs root.
#
# Scheduling: systemd/age-feed-drill.{service,timer} in this repo's
# pm-stack/aggregator/systemd/ directory (weekly, in a quiet window that
# does not overlap the kill-collector drill's Sunday 04:17 slot). The
# flock makes overlap impossible even if a manual run and the timer
# collide.
#
# Environment knobs:
#   PM_DRILL_STATE_DIR        state/lock/result dir (default ~/.local/state/pm-drills)
#   PM_DRILL_INCIDENT_FLAG    incident flag path (default $STATE_DIR/incident.flag)
#   PM_DRILL_FEED_FILE        feed file (default ~/.local/state/pm-aggregator/fleet-feed.json)
#   PM_DRILL_HEALTH_CMD       command printing the /api/health JSON (default local curl)
#   PM_DRILL_ATTENTION_CMD    command printing /api/attention.json (default local curl)
#   PM_DRILL_AGE_MS           how far into the past to stamp the feed (default 1200000)
#   PM_DRILL_MAX_WAIT_MIN     max minutes to wait for the stale signal (default 25)
#   PM_DRILL_RESTORE_WAIT_MIN max minutes to wait for the collector to refresh (default 12)
#   PM_DRILL_NO_POST=1        skip the GitHub comment/issue side effects

set -uo pipefail

STATE_DIR="${PM_DRILL_STATE_DIR:-$HOME/.local/state/pm-drills}"
INCIDENT_FLAG="${PM_DRILL_INCIDENT_FLAG:-$STATE_DIR/incident.flag}"
FEED_FILE="${PM_DRILL_FEED_FILE:-$HOME/.local/state/pm-aggregator/fleet-feed.json}"
HEALTH_CMD="${PM_DRILL_HEALTH_CMD:-curl -s --max-time 25 http://127.0.0.1:4870/api/health}"
ATTENTION_CMD="${PM_DRILL_ATTENTION_CMD:-curl -s --max-time 25 http://127.0.0.1:4870/api/attention.json}"
AGE_MS="${PM_DRILL_AGE_MS:-1200000}"
MAX_WAIT_MIN="${PM_DRILL_MAX_WAIT_MIN:-25}"
RESTORE_WAIT_MIN="${PM_DRILL_RESTORE_WAIT_MIN:-12}"
STALE_MS=900000 # FEED_STALE_MS in aggregator src/health.ts + src/sources/live.ts
COLLECTOR_TIMER="fleet-feed-collector.timer"
ISSUE_REPO="developing-today/code"
EPIC=1184
ISSUE=1283

mkdir -p "$STATE_DIR"
RESULT_FILE="$STATE_DIR/age-feed-result.json"
BACKUP_FILE="$STATE_DIR/age-feed-original.json"
EVIDENCE_FILE="$STATE_DIR/age-feed-health-at-signal.json"
ATTENTION_EVIDENCE="$STATE_DIR/age-feed-attention-at-signal.json"
STARTED_AT="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

log() { printf '[drill] %s %s\n' "$(date -u +%H:%M:%SZ)" "$*"; }
die() {
  log "REFUSED/FAILED: $*"
  exit 1
}

# --- interlock: one drill at a time ---------------------------------------
exec 9>"$STATE_DIR/age-feed.lock"
if ! flock -n 9; then
  die "another age-feed drill holds the lock; not starting"
fi
# --- interlock: no drills during a real incident ---------------------------
if [ -e "$INCIDENT_FLAG" ]; then
  die "incident flag present ($INCIDENT_FLAG); the stack is not a drill canvas during a real incident"
fi

# --- health/feed reads (aggregator loopback on this host) ------------------
health_json() {
  # The aggregator's first health assembly after a quiet period can be
  # slow to prime (health-priming is a known behaviour), so retry: a
  # drill must never act on — or refuse on — one timed-out read.
  local attempt out
  for attempt in 1 2 3; do
    out="$(bash -c "$HEALTH_CMD" 2>/dev/null)" && [ -n "$out" ] && {
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
# Rewrites the feed's top-level generatedAt to PIN_STAMP (atomic replace),
# leaving every other field as the collector wrote it. Prints the written
# path on success.
age_feed_to() {
  python3 - "$FEED_FILE" "$1" <<'PY'
import json, os, sys, tempfile
path, stamp = sys.argv[1], sys.argv[2]
try:
    doc = json.load(open(path))
    mode = os.stat(path).st_mode & 0o777
    doc["generatedAt"] = stamp
    fd, tmp = tempfile.mkstemp(dir=os.path.dirname(path), prefix=".age-feed-drill-", suffix=".json")
    with os.fdopen(fd, "w") as fh:
        json.dump(doc, fh)
    os.chmod(tmp, mode)
    os.replace(tmp, path)
except Exception as exc:
    print(f"age_feed_to failed: {exc}", file=sys.stderr)
    sys.exit(1)
PY
}
# Current generatedAt of the feed file, or empty when unreadable.
feed_stamp() {
  python3 -c '
import json, sys
try:
    print(json.load(open(sys.argv[1])).get("generatedAt") or "")
except Exception:
    print("")
' "$FEED_FILE"
}

# --- restore: idempotent, and gated on the aging --------------------------
# The trap is installed before the precondition checks, so without the
# gate a REFUSAL (die -> exit) would fire restore() and write the feed
# the drill just refused to touch (the #2865 lesson from the
# kill-collector drill). AGED is armed immediately BEFORE the first
# destructive write — after every precondition — so until that point
# restore() is a no-op and every refusal path leaves the system exactly
# as it found it.
AGED=0
RESTORED=0
restore() {
  [ "$AGED" = 1 ] || return 0
  [ "$RESTORED" = 1 ] && return 0
  RESTORED=1
  log "restoring the feed's original bytes (collector stays running)"
  # Never clobber a FRESH feed the collector wrote after our last
  # re-age: the aged pin is ours to undo, a new collection is not.
  if [ "$(feed_stamp)" != "$PIN_STAMP" ]; then
    log "collector already rewrote the feed with a fresh stamp; nothing to restore"
    return 0
  fi
  if ! cp -p "$BACKUP_FILE" "$FEED_FILE.tmp.$$" 2>/dev/null || ! mv -f "$FEED_FILE.tmp.$$" "$FEED_FILE" 2>/dev/null; then
    log "WARN: could not restore $FEED_FILE from $BACKUP_FILE"
  fi
}
trap restore EXIT INT TERM

# --- preconditions ----------------------------------------------------------
systemctl --user is-active --quiet "$COLLECTOR_TIMER" ||
  die "$COLLECTOR_TIMER is not active; this drill's premise is a running collector, and a stopped collector outside a drill is itself a finding"
[ -r "$FEED_FILE" ] || die "cannot read the feed file $FEED_FILE"
[ -n "$(feed_stamp)" ] || die "feed file $FEED_FILE has no readable generatedAt; refusing to drill blind"
BASELINE="$(health_json | feed_facts)" || die "cannot read aggregator /api/health; refusing to drill blind"
BASE_AGE="${BASELINE%% *}"
[ "$BASE_AGE" -ge 0 ] 2>/dev/null || die "cannot read aggregator /api/health; refusing to drill blind"
[ "$BASE_AGE" -le "$STALE_MS" ] || die "feed is already stale at drill start (age ${BASE_AGE}ms > ${STALE_MS}ms): that is a real incident, not a drill"
cp -p "$FEED_FILE" "$BACKUP_FILE" || die "cannot save the feed's original bytes to $BACKUP_FILE"
log "preconditions ok: collector active, feed fresh (age ${BASE_AGE}ms), feed saved, no incident flag, lock held"

# --- age the feed, collector still running ------------------------------------
# Pin the stamp once so every re-age during the drill presents the same
# "alive but old" feed; the collector's own rewrites are re-aged below.
PIN_STAMP="$(python3 -c '
import datetime, sys
ms = int(sys.argv[1])
print((datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(milliseconds=ms)).strftime("%Y-%m-%dT%H:%M:%S.%f")[:-3] + "Z")
' "$AGE_MS")"
AGED=1
age_feed_to "$PIN_STAMP" || die "failed to age $FEED_FILE (the original bytes are saved at $BACKUP_FILE)"
log "aged feed to generatedAt=$PIN_STAMP (${AGE_MS}ms old); collector left running"

# --- wait for the honesty signal ---------------------------------------------
SIGNAL=0
DEGRADED=0
WATCHDOG=0
WAITED=0
MAX_WAIT_SEC=$((MAX_WAIT_MIN * 60))
while [ "$WAITED" -le "$MAX_WAIT_SEC" ]; do
  # Re-age if the collector rewrote the feed with a fresh stamp: this
  # is the drill's scenario, not a failure of the drill.
  CUR_STAMP="$(feed_stamp)"
  [ "$CUR_STAMP" = "$PIN_STAMP" ] || age_feed_to "$PIN_STAMP" || log "WARN: re-age failed (stamp now ${CUR_STAMP:-unreadable})"
  HEALTH="$(health_json)"
  FACTS="$(printf '%s' "$HEALTH" | feed_facts)"
  AGE="${FACTS%% *}"
  DEG="${FACTS##* }"
  if [ "$AGE" -gt "$STALE_MS" ] 2>/dev/null; then
    SIGNAL=1
    [ "$DEG" = "true" ] && DEGRADED=1
    printf '%s' "$HEALTH" >"$EVIDENCE_FILE" 2>/dev/null || true
    log "SIGNAL FIRED: feed age ${AGE}ms crossed the ${STALE_MS}ms staleness threshold after ~${WAITED}s (health payload saved)"
    break
  fi
  log "waiting: feed age ${AGE}ms (threshold ${STALE_MS}ms)"
  sleep 60
  WAITED=$((WAITED + 60))
done
[ "$SIGNAL" = 1 ] || log "SIGNAL DID NOT FIRE within ${MAX_WAIT_MIN}min"

# --- watchdog check (best effort, recorded either way) -----------------------
if [ "$SIGNAL" = 1 ]; then
  if ATT="$(bash -c "$ATTENTION_CMD" 2>/dev/null)" && [ -n "$ATT" ]; then
    printf '%s' "$ATT" >"$ATTENTION_EVIDENCE" 2>/dev/null || true
    if printf '%s' "$ATT" | python3 -c '
import json, sys
d = json.load(sys.stdin)
items = d.get("attention") if isinstance(d, dict) else d
print("yes" if any(isinstance(i, dict) and i.get("id") == "watchdog:feed" for i in (items or [])) else "no")
' 2>/dev/null | grep -qx yes; then
      WATCHDOG=1
      log "watchdog:feed tier item present on /api/attention.json"
    else
      log "watchdog:feed tier item not present on /api/attention.json (health signal alone was recorded)"
    fi
  fi
fi

# --- restore + confirm --------------------------------------------------------
restore
trap - EXIT INT TERM
CONFIRMED=0
WAITED=0
MAX_RESTORE_SEC=$((RESTORE_WAIT_MIN * 60))
while [ "$WAITED" -le "$MAX_RESTORE_SEC" ]; do
  FACTS="$(health_json | feed_facts)"
  AGE="${FACTS%% *}"
  if [ "$AGE" -ge 0 ] 2>/dev/null && [ "$AGE" -le "$STALE_MS" ] 2>/dev/null; then
    CONFIRMED=1
    log "restoration confirmed: feed fresh again (age ${AGE}ms)"
    break
  fi
  log "waiting for the collector's next run: feed age ${AGE}ms"
  sleep 30
  WAITED=$((WAITED + 30))
done
[ "$CONFIRMED" = 1 ] || log "WARN: restoration not confirmed within ${RESTORE_WAIT_MIN}min (feed age ${AGE:-unknown}ms)"

# --- report -------------------------------------------------------------------
OUTCOME="failed"
[ "$SIGNAL" = 1 ] && [ "$CONFIRMED" = 1 ] && OUTCOME="passed"
python3 - "$RESULT_FILE" "$STARTED_AT" "$OUTCOME" "$SIGNAL" "$DEGRADED" "$WATCHDOG" "$CONFIRMED" "$AGE_MS" <<'PY'
import json, sys, datetime
path, started, outcome, signal, degraded, watchdog, confirmed, aged = sys.argv[1:9]
json.dump({
    "drill": "age-feed", "issue": 1283, "epic": 1184,
    "started_at": started,
    "finished_at": datetime.datetime.now(datetime.timezone.utc).strftime("%Y-%m-%dT%H:%M:%SZ"),
    "outcome": outcome,
    "signal_fired": signal == "1",
    "health_degraded": degraded == "1",
    "watchdog_item_seen": watchdog == "1",
    "restoration_confirmed": confirmed == "1",
    "aged_ms": int(aged),
    "stale_threshold_ms": 900000,
}, open(path, "w"), indent=2)
PY
log "result: $OUTCOME (signal=$SIGNAL degraded=$DEGRADED watchdog=$WATCHDOG restored=$CONFIRMED) -> $RESULT_FILE"

if [ "${PM_DRILL_NO_POST:-0}" != "1" ] && command -v gh >/dev/null 2>&1; then
  if [ "$OUTCOME" = "passed" ]; then
    gh issue comment "$ISSUE" --repo "$ISSUE_REPO" --body "✅ **Chaos drill passed — age the feed** (queue task 7919, second slice of this issue, epic #$EPIC)

- Aged \`fleet-feed.json\`'s \`generatedAt\` by ${AGE_MS}ms at $STARTED_AT **while \`$COLLECTOR_TIMER\` stayed active** — the stale-but-alive feed the kill-collector drill cannot produce. Health payload at the signal moment: degraded=$DEGRADED, watchdog \`watchdog:feed\` item seen=$WATCHDOG.
- The aggregator's feed-age signal crossed the 15-minute \`FEED_STALE_MS\` (#1148) threshold instead of presenting old data as fine.
- Restoration confirmed via \`/api/health\`: the original bytes were written back and the collector's next run made the feed fresh again.
- Script: \`pm-stack/aggregator/scripts/age-feed-drill.sh\` (interlocked: flock, incident-flag refusal, refuses to start on an already-stale feed or an inactive collector; always restores via trap). Result JSON is written to \`~/.local/state/pm-drills/age-feed-result.json\` per run." >/dev/null 2>&1 &&
      log "posted result comment on #$ISSUE" || log "WARN: gh comment on #$ISSUE failed"
  else
    gh issue create --repo "$ISSUE_REPO" --title "chaos drill FINDING: age-feed drill did not observe the expected staleness signal" --body "The age-feed drill (second slice of #$ISSUE, epic #$EPIC) ran at $STARTED_AT and the expected signal did not complete cleanly: signal_fired=$SIGNAL, health_degraded=$DEGRADED, restoration_confirmed=$CONFIRMED (see \`~/.local/state/pm-drills/age-feed-result.json\` and \`age-feed-health-at-signal.json\` for the captured evidence). A feed that is alive but old presenting as fresh is exactly what #$ISSUE exists to catch — investigate the aggregator's feed-age/health path (FEED_STALE_MS, src/health.ts, src/sources/live.ts) and the \`$COLLECTOR_TIMER\` cadence before the next drill." >/dev/null 2>&1 &&
      log "filed finding issue under epic #$EPIC" || log "WARN: gh issue create for the finding failed"
  fi
fi

[ "$OUTCOME" = "passed" ]
