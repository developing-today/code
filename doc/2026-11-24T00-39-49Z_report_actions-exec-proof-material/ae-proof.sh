#!/usr/bin/env bash
# #13774 live proof battery — runs on the laptop, seat = the
# published fleet-static-5e41070 binary.
set -u
FLEET=/tmp/ae-fleet-bin/fleet
LOG=/tmp/ae-proof.log
: >"$LOG"
say() { echo "== $*" | tee -a "$LOG"; }

# --- Session A: start -> handoff -> exec -> egress -> reconnect -> close
say "A: start (lease 600, max 1200, detach)"
OUT=$($FLEET actions exec start --repo dezren39/fleet --timeout 600 --max-lifespan 1200 --detach 2>&1)
echo "$OUT" | tee -a "$LOG"
HANDLE_A=$(echo "$OUT" | grep -o 'fx-[a-z0-9]*' | head -1)
RUN_A=$(echo "$OUT" | grep -o 'dezren39/fleet#[0-9]*' | grep -o '[0-9]*$')
say "A: handle=$HANDLE_A run=$RUN_A"
sleep 20
say "A: run 1 (echo + hostname)"
$FLEET actions exec run "$HANDLE_A" -- 'echo hello-from-runner; hostname; pwd' 2>&1 | tee -a "$LOG"
say "A: run 1 exit=$?"
say "A: run 2 (egress: write + fleet-exec-put)"
$FLEET actions exec run "$HANDLE_A" -- 'echo actions-exec-proof-content > /tmp/proof.txt && fleet-exec-put /tmp/proof.txt && ls -la $FLEET_EXEC_OUTBOX' 2>&1 | tee -a "$LOG"
say "A: run 3 (reconnect: fresh seat process, portable ref)"
$FLEET actions exec run "dezren39/fleet#$RUN_A" -- 'echo reconnected-ok' 2>&1 | tee -a "$LOG"
say "A: close"
$FLEET actions exec close "$HANDLE_A" 2>&1 | tee -a "$LOG"
echo "HANDLE_A=$HANDLE_A RUN_A=$RUN_A" >/tmp/ae-proof-A.env

# --- Session B: renewal within the cap
say "B: start (lease 60, max 300, detach)"
OUT=$($FLEET actions exec start --repo dezren39/fleet --timeout 60 --max-lifespan 300 --detach 2>&1)
echo "$OUT" | tee -a "$LOG"
HANDLE_B=$(echo "$OUT" | grep -o 'fx-[a-z0-9]*' | head -1)
sleep 15
say "B: renew --timeout 240"
$FLEET actions exec renew "$HANDLE_B" --timeout 240 2>&1 | tee -a "$LOG"
say "B: waiting 70s (past the original 60s lease)"
sleep 70
say "B: run after original lease would have died"
$FLEET actions exec run "$HANDLE_B" -- 'echo still-alive-after-renewal' 2>&1 | tee -a "$LOG"
$FLEET actions exec close "$HANDLE_B" 2>&1 | tee -a "$LOG"
echo "HANDLE_B=$HANDLE_B" >/tmp/ae-proof-B.env

# --- Session C: expiry at the cap, unrenewed
say "C: start (lease 45, max 45, detach)"
OUT=$($FLEET actions exec start --repo dezren39/fleet --timeout 45 --max-lifespan 45 --detach 2>&1)
echo "$OUT" | tee -a "$LOG"
HANDLE_C=$(echo "$OUT" | grep -o 'fx-[a-z0-9]*' | head -1)
say "C: waiting 75s for the cap"
sleep 75
say "C: run after expiry (must be refused)"
$FLEET actions exec run "$HANDLE_C" -- 'echo should-not-run' 2>&1 | tee -a "$LOG"
say "C: run exit=$?"
say "C: watch (must report expired and exit)"
timeout 30 $FLEET actions exec watch "$HANDLE_C" 2>&1 | tee -a "$LOG"
echo "HANDLE_C=$HANDLE_C" >/tmp/ae-proof-C.env

# --- Session D: emergency extension past the max
say "D: start (lease 60, max 90, detach)"
OUT=$($FLEET actions exec start --repo dezren39/fleet --timeout 60 --max-lifespan 90 --detach 2>&1)
echo "$OUT" | tee -a "$LOG"
HANDLE_D=$(echo "$OUT" | grep -o 'fx-[a-z0-9]*' | head -1)
sleep 15
say "D: extend --minutes 30"
$FLEET actions exec extend "$HANDLE_D" --minutes 30 --reason "proof battery" 2>&1 | tee -a "$LOG"
say "D: decide --approve (the v1 write-back of Drew's answer)"
$FLEET actions exec decide "$HANDLE_D" --approve 2>&1 | tee -a "$LOG"
say "D: renew --timeout 600 (past the old max; must not require extension)"
$FLEET actions exec renew "$HANDLE_D" --timeout 600 2>&1 | tee -a "$LOG"
say "D: run"
$FLEET actions exec run "$HANDLE_D" -- 'echo extended-session-works' 2>&1 | tee -a "$LOG"
$FLEET actions exec close "$HANDLE_D" 2>&1 | tee -a "$LOG"
echo "HANDLE_D=$HANDLE_D" >/tmp/ae-proof-D.env
say "battery done"
