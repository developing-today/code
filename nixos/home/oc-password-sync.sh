#!/usr/bin/env bash
# oc-password-sync: keep OpenChamber's upstream OpenCode password in sync
# with the prod server's published service file. The prod server rotates
# its password on every start (~/.local/state/opencode/service-prod.json).
#
# Mechanism (2026-11-01, code#3724): OpenChamber no longer reads the
# password from the old hand-applied systemd drop-in
# (openchamber.service.d/external-opencode.conf). That drop-in was
# retired by design when openchamber#766 gave OpenChamber a start
# wrapper (bin/openchamber-start.sh) that reads service-prod.json
# directly at every start and exports OPENCODE_PASSWORD. This sync's
# remaining job is the mid-run rotation case: when the prod password
# changes while OpenChamber is running, restart OpenChamber so its
# wrapper re-reads the file. Drift is judged against the password the
# running OpenChamber process actually holds (its /proc environ),
# falling back to the last-synced hash in the state file.
# Invoked by oc-password-sync.path on service-prod.json changes, and
# manually.
set -uo pipefail
JSON="${OC_SERVICE_JSON:-$HOME/.local/state/opencode/service-prod.json}"
STATE_DIR="$HOME/.local/state/openchamber"
STATE="$STATE_DIR/password-sync.state"
LOG="$STATE_DIR/password-sync.log"
mkdir -p "$STATE_DIR"
ts() { date -Is; }
log() { echo "$(ts) $*" >>"$LOG"; }
if [ ! -f "$JSON" ]; then
  log "no service file at $JSON; skipping"
  exit 0
fi
NEW=$(OCF="$JSON" python3 -c 'import json,os; print(json.load(open(os.environ["OCF"]))["password"])' 2>/dev/null)
if [ -z "${NEW:-}" ]; then
  log "could not parse password from $JSON; skipping"
  exit 0
fi
NEWHASH=$(printf %s "$NEW" | sha256sum | cut -d" " -f1)
# Current password: what the running OpenChamber process holds.
CUR=""
PID=$(systemctl --user show openchamber.service -p MainPID --value 2>/dev/null || echo 0)
if [ "${PID:-0}" -gt 0 ] && [ -r "/proc/$PID/environ" ]; then
  CUR=$(tr "\0" "\n" <"/proc/$PID/environ" | sed -n "s/^OPENCODE_PASSWORD=//p" | head -1)
fi
if [ -n "$CUR" ]; then
  if [ "$NEW" = "$CUR" ]; then
    printf %s "$NEWHASH" >"$STATE"
    log "in sync; no action"
    exit 0
  fi
else
  # No live reading (OpenChamber down): compare against last-synced hash.
  LAST=$(cat "$STATE" 2>/dev/null || true)
  if [ "$NEWHASH" = "$LAST" ]; then
    log "in sync (state); openchamber not running; no action"
    exit 0
  fi
fi
if [ "${OC_DRY_RUN:-0}" = "1" ]; then
  log "DRY RUN: drift detected; would restart openchamber.service"
  echo "drift detected (dry run)"
  exit 0
fi
if ! systemctl --user restart openchamber.service; then
  log "restart FAILED"
  exit 1
fi
# Verify: the restarted process must hold the new password.
for i in $(seq 1 30); do
  PID=$(systemctl --user show openchamber.service -p MainPID --value 2>/dev/null || echo 0)
  if [ "${PID:-0}" -gt 0 ] && [ -r "/proc/$PID/environ" ]; then
    CUR=$(tr "\0" "\n" <"/proc/$PID/environ" | sed -n "s/^OPENCODE_PASSWORD=//p" | head -1)
    if [ "$NEW" = "$CUR" ]; then
      printf %s "$NEWHASH" >"$STATE"
      log "drift corrected; openchamber.service restarted, password verified in new process"
      exit 0
    fi
  fi
  sleep 1
done
log "restart done but password NOT verified in new process within 30s"
exit 1
