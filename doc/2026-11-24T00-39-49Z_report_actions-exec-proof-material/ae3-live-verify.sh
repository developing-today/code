#!/bin/bash
# ACTIONS-EXEC-3 live deploy verification against the production
# queue broker (generation 8). Exercises tier 1 (auto) and tier 2
# (deny window + deny) on a broker-side-only session: open does
# not dispatch a workflow. The two PM pushes this raises are the
# deliverable's own notifications, labeled as verification.
set -u
TOK=$(fleet secrets get fleet.queue-broker-token) || { echo "TOKEN-FETCH-FAILED"; exit 1; }
[ -n "$TOK" ] || { echo "TOKEN-EMPTY"; exit 1; }
B=http://100.99.113.103:4801
H="X-Agent-Token: $TOK"
CT="Content-Type: application/json"

OPEN=$(curl -fsS -X POST "$B/actions-exec-open" -H "$H" -H "$CT" -d '{"host_repo":"dezren39/fleet","workflow":"fleet-exec.yml","seat":"ae3-deploy-verify","lease_s":600,"max_s":900,"context":"https://github.com/developing-today/code/issues/13774"}') || { echo "OPEN-FAILED"; exit 1; }
HANDLE=$(printf %s "$OPEN" | jq -r .handle)
echo "open: handle=$HANDLE context=$(printf %s "$OPEN" | jq -r .session.context)"

EXT1=$(curl -fsS -X POST "$B/actions-exec-extend-request" -H "$H" -H "$CT" -d "{\"handle\":\"$HANDLE\",\"extend_s\":600,\"reason\":\"ACTIONS-EXEC-3 deploy verification, tier 1 - test session, no action needed\"}")
echo "extend1: $(printf %s "$EXT1" | jq -c '{tier, outcome, granted_s, ext_ordinal}')"

ST1=$(curl -fsS -X POST "$B/actions-exec-stream" -H "$H" -H "$CT" -d "{\"handle\":\"$HANDLE\"}")
echo "stream1: $(printf %s "$ST1" | jq -c '.session | {ext_state, ext_count, ext_total_s}')"

EXT2=$(curl -fsS -X POST "$B/actions-exec-extend-request" -H "$H" -H "$CT" -d "{\"handle\":\"$HANDLE\",\"extend_s\":600,\"reason\":\"ACTIONS-EXEC-3 deploy verification, tier 2 deny-window check - test session, will be denied and closed\"}")
echo "extend2: $(printf %s "$EXT2" | jq -c '{tier, outcome, decide_after}')"

DEC=$(curl -fsS -X POST "$B/actions-exec-extend-decide" -H "$H" -H "$CT" -d "{\"handle\":\"$HANDLE\",\"decision\":\"deny\"}")
echo "decide-deny: $(printf %s "$DEC" | jq -c '{decided, decision, note}')"

ST2=$(curl -fsS -X POST "$B/actions-exec-stream" -H "$H" -H "$CT" -d "{\"handle\":\"$HANDLE\"}")
echo "stream2: $(printf %s "$ST2" | jq -c '.session | {ext_state, ext_count, ext_total_s}')"

CLOSE=$(curl -fsS -X POST "$B/actions-exec-close" -H "$H" -H "$CT" -d "{\"handle\":\"$HANDLE\"}")
echo "close: $(printf %s "$CLOSE" | jq -c '{handle, state}')"
echo "VERIFY-DONE handle=$HANDLE"
