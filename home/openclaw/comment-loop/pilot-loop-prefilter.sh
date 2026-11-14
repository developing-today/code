#!/usr/bin/env bash
# pilot-loop-prefilter.sh — zero-token pre-filter for the OpenClaw
# comment loop (developing-today/code#9818, production form of the
# pilot on issue #1190).
#
# Runs as an OpenClaw cron COMMAND payload (shell on the Gateway host,
# no model call) every 2 minutes. It polls ONE designated GitHub issue
# for a new unanswered pilot-loop:v1 round and wakes exactly one agent
# turn (openclaw agent, timeout floor 600s) only when one appears.
# Idle polls cost ~0 agent tokens: the only output is the cron silent
# token NO_REPLY and no agent turn is started.
#
# A round (STATUS-REQUEST or INSTRUCTION with "ROUND: R<n>") counts as
# answered when any comment on the issue carries "REPLY: R<n>" — the
# reply state is derived from GitHub itself, so the filter is
# stateless across restarts except for a wake-attempt stamp that
# rate-limits re-wakes of a still-unanswered round (an agent turn can
# take minutes; without the stamp every poll would wake another).
#
# Gateway resilience leg: the wake needs the gateway. If
# openclaw-gateway.service is not active, the filter tries to start
# it (the documented start step) and only then surfaces the failure
# (non-zero exit, recorded in cron run history). The durable fix is
# declarative: the gateway unit carries [Install] WantedBy=
# default.target in home/openclaw/default.nix and Restart=always, so
# it starts at boot and restarts on crash without any human.
#
# Quota leg: before waking, the filter reads the quota-gatherer's
# current snapshot (pm-aggregator quota-current.json, refreshed by
# quota-gatherer.timer) and injects a one-line quota_left summary —
# tightest measured 5h window across healthy providers, with the
# measurement time — into the wake message. The agent turn copies
# that line verbatim into its SESSION-REPORT, so quota_left is a
# real measured value with provenance, never "unknown" while the
# gatherer is running.
#
# Usage: pilot-loop-prefilter.sh [issue-number]
# Env overrides (defaults shown):
#   COMMENT_LOOP_REPO=developing-today/code
#   COMMENT_LOOP_STATE_DIR=$HOME/.local/state/openclaw/comment-loop
#   COMMENT_LOOP_QUOTA_FILE=$HOME/.local/state/pm-aggregator/quota-current.json
#   COMMENT_LOOP_AGENT_TIMEOUT_SECONDS=900   (clamped to >= 600)
#   COMMENT_LOOP_REWAKE_AFTER_SECONDS=900
#   COMMENT_LOOP_GH=gh COMMENT_LOOP_OPENCLAW=openclaw
#   COMMENT_LOOP_SYSTEMCTL=systemctl
set -uo pipefail

ISSUE="${1:-9818}"
REPO="${COMMENT_LOOP_REPO:-developing-today/code}"
STATE_DIR="${COMMENT_LOOP_STATE_DIR:-$HOME/.local/state/openclaw/comment-loop}"
QUOTA_FILE="${COMMENT_LOOP_QUOTA_FILE:-$HOME/.local/state/pm-aggregator/quota-current.json}"
AGENT_TIMEOUT_SECONDS="${COMMENT_LOOP_AGENT_TIMEOUT_SECONDS:-900}"
REWAKE_AFTER_SECONDS="${COMMENT_LOOP_REWAKE_AFTER_SECONDS:-900}"
GH="${COMMENT_LOOP_GH:-gh}"
OPENCLAW="${COMMENT_LOOP_OPENCLAW:-openclaw}"
SYSTEMCTL="${COMMENT_LOOP_SYSTEMCTL:-systemctl}"

# Timeout floor (#9818 item 4): the pilot's first watcher turn died
# at the 150s cron default in model-call-started on a cold route.
if [ "$AGENT_TIMEOUT_SECONDS" -lt 600 ] 2>/dev/null; then
  AGENT_TIMEOUT_SECONDS=600
fi

mkdir -p "$STATE_DIR"
STAMP_FILE="$STATE_DIR/issue-$ISSUE.wake"
LOG_FILE="$STATE_DIR/issue-$ISSUE.log"

log() {
  printf '%s %s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)" "$*" >>"$LOG_FILE"
}

# --- gateway leg ---------------------------------------------------
gateway_active() {
  [ "$("$SYSTEMCTL" --user is-active openclaw-gateway.service 2>/dev/null || true)" = "active" ]
}

if ! gateway_active; then
  log "gateway not active; attempting documented start step"
  "$SYSTEMCTL" --user start openclaw-gateway.service 2>/dev/null || true
  for _ in 1 2 3 4 5 6; do
    gateway_active && break
    sleep 5
  done
  if ! gateway_active; then
    echo "comment-loop: openclaw-gateway.service is not active and the start step did not recover it (issue #$ISSUE watcher cannot wake an agent turn)"
    log "gateway still not active after start step; surfacing failure"
    exit 1
  fi
  log "gateway recovered by start step"
fi

# --- poll the designated issue ------------------------------------
comments_file="$(mktemp "$STATE_DIR/comments.XXXXXX")"
trap 'rm -f "$comments_file"' EXIT

if ! "$GH" api "repos/$REPO/issues/$ISSUE/comments?per_page=100" --paginate >"$comments_file" 2>>"$LOG_FILE"; then
  echo "comment-loop: gh comment poll failed for $REPO issue #$ISSUE (see $LOG_FILE)"
  log "gh poll failed"
  exit 1
fi

# Bodies, one base64 blob per comment, in posting order.
bodies="$(jq -s -r '(add // []) | .[] | (.body // "") | @base64' "$comments_file" 2>/dev/null || true)"
if [ -z "$bodies" ]; then
  echo "comment-loop: no readable comments on $REPO issue #$ISSUE"
  log "no comments parsed"
  exit 1
fi

declare -A requested=()
declare -A answered=()
request_type=""
request_action=""
for blob in $bodies; do
  body="$(printf '%s' "$blob" | base64 -d 2>/dev/null || true)"
  case "$body" in
  *"pilot-loop:v1"*) ;;
  *) continue ;;
  esac
  round="$(printf '%s\n' "$body" | sed -n 's/^ROUND: *R\([0-9][0-9]*\).*/\1/p' | head -1)"
  reply="$(printf '%s\n' "$body" | sed -n 's/^REPLY: *R\([0-9][0-9]*\).*/\1/p' | head -1)"
  if [ -n "$reply" ]; then
    answered["$reply"]=1
  fi
  if [ -n "$round" ]; then
    type="$(printf '%s\n' "$body" | sed -n 's/^TYPE: *\([A-Z-]*\).*/\1/p' | head -1)"
    if [ "$type" = "STATUS-REQUEST" ] || [ "$type" = "INSTRUCTION" ]; then
      requested["$round"]=1
    fi
  fi
done

# Oldest unanswered round wins.
target=""
for n in $(printf '%s\n' "${!requested[@]}" | sort -n); do
  if [ -z "${answered[$n]:-}" ]; then
    target="$n"
    break
  fi
done

if [ -z "$target" ]; then
  echo "NO_REPLY"
  exit 0
fi

# Details of the target round (type + instruction action, if any).
for blob in $bodies; do
  body="$(printf '%s' "$blob" | base64 -d 2>/dev/null || true)"
  case "$body" in *"pilot-loop:v1"*) ;; *) continue ;; esac
  if printf '%s\n' "$body" | grep -qx "ROUND: R$target"; then
    request_type="$(printf '%s\n' "$body" | sed -n 's/^TYPE: *\([A-Z-]*\).*/\1/p' | head -1)"
    request_action="$(printf '%s\n' "$body" | sed -n 's/^ACTION: *//p' | head -1)"
    break
  fi
done

# --- re-wake rate limit --------------------------------------------
now_epoch="$(date +%s)"
if [ -f "$STAMP_FILE" ]; then
  read -r stamp_round stamp_epoch <"$STAMP_FILE" || true
  if [ "${stamp_round:-}" = "$target" ] && [ -n "${stamp_epoch:-}" ]; then
    age=$((now_epoch - stamp_epoch))
    if [ "$age" -lt "$REWAKE_AFTER_SECONDS" ]; then
      log "R$target already wake-attempted ${age}s ago (< ${REWAKE_AFTER_SECONDS}s); staying silent"
      echo "NO_REPLY"
      exit 0
    fi
  fi
fi

# --- quota leg -------------------------------------------------------
quota_line="quota source unavailable: $QUOTA_FILE not readable"
if [ -r "$QUOTA_FILE" ]; then
  quota_line="$(jq -r '
    def left: ((100 - .) * 100 | round / 100);
    ([.providers[] | select(.state == "ok") | .windows[]? | select(.name | endswith(":5h"))]
     ) as $wins
    | if ($wins | length) == 0 then
        "quota source present but no healthy 5h windows (\(.generatedAt // "unknown time"))"
      else
        ($wins | max_by(.pctOfLimit)) as $tight
        | "\($tight.pctOfLimit | left)% left on the tightest measured 5h window (\($tight.name), resets \($tight.resetsAt)); measured \(.generatedAt) by the quota gatherer (opencode-quota)"
      end
  ' "$QUOTA_FILE" 2>/dev/null || echo "quota source present but unparsable: $QUOTA_FILE")"
fi

# --- wake exactly one agent turn ------------------------------------
msg_file="$STATE_DIR/wake-$ISSUE-R$target.md"
cat >"$msg_file" <<EOF
You are the OpenClaw comment-loop watcher (production watcher for developing-today/code issue #$ISSUE, issue #9818). The zero-token shell pre-filter detected a new unanswered pilot-loop:v1 round and woke you — you are the only agent turn it wakes for this round. Do exactly the steps for the round, nothing else: never edit, close, label, or assign the issue, and never post any comment other than the round's reply.

Round: R$target
Type: $request_type
Action (instructions only): ${request_action:-none}

1. Read the round comment first: gh issue view $ISSUE --repo $REPO --json comments — find the comment carrying "ROUND: R$target".
2. If the type is STATUS-REQUEST, post ONE comment with gh issue comment $ISSUE --repo $REPO --body-file <a file you write with exactly this body>:
pilot-loop:v1
TYPE: SESSION-REPORT
REPLY: R$target
progress_verdict: <substantive: what the OpenClaw main session is doing right now>
pool_route: opencode-route/workhorse (this gateway's default route)
quota_left: $quota_line
blockers: <none, or the concrete blockers>
3. If the type is INSTRUCTION with ACTION checkpoint-then-continue, post a CHECKPOINT comment (pilot-loop:v1, TYPE: CHECKPOINT, REPLY: R$target, a STATE line) and then a CONTINUED comment (TYPE: CONTINUED, REPLY: R$target, a STATE line), in that order, and no other action.
EOF

# Stamp BEFORE the wake so a long turn is never double-woken.
printf '%s %s\n' "$target" "$now_epoch" >"$STAMP_FILE"
log "waking agent turn for R$target ($request_type) on issue #$ISSUE; quota: $quota_line"

if [ -z "${OPENCLAW_GATEWAY_TOKEN:-}" ] && [ -r /run/secrets/openclaw/gateway-token ]; then
  OPENCLAW_GATEWAY_TOKEN="$(cat /run/secrets/openclaw/gateway-token)"
  export OPENCLAW_GATEWAY_TOKEN
fi

if "$OPENCLAW" agent --agent main --session-key "agent:main:comment-loop-$ISSUE" --timeout "$AGENT_TIMEOUT_SECONDS" --message-file "$msg_file" >>"$LOG_FILE" 2>&1; then
  log "agent turn for R$target completed"
  echo "comment-loop: woke agent turn for R$target ($request_type) on $REPO issue #$ISSUE"
  exit 0
else
  rc=$?
  log "agent turn for R$target failed rc=$rc; will re-wake after ${REWAKE_AFTER_SECONDS}s if still unanswered"
  echo "comment-loop: agent turn for R$target on issue #$ISSUE failed (rc=$rc); re-wake after ${REWAKE_AFTER_SECONDS}s if the round stays unanswered"
  exit "$rc"
fi
