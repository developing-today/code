#!/usr/bin/env bash
# Regression tests for pilot-loop-prefilter.sh (#9818).
#
# The pre-filter is the production replacement for the pilot's
# every-2-minute LLM poll (39k-81k tokens per idle poll, #1190).
# These tests drive the REAL script with stubbed gh / openclaw /
# systemctl binaries and pin the behaviours the production watcher
# is accepted on:
#
#   * idle issue (no rounds, or every round answered)
#       -> prints NO_REPLY, wakes NO agent turn (~0 idle tokens)
#   * a new unanswered STATUS-REQUEST -> exactly one wake, with a
#     >=600s agent timeout and a wake message whose quota line is a
#     real value computed from the quota-gatherer fixture
#   * an unanswered round wake-attempted moments ago is NOT woken
#     again inside the re-wake window, and IS woken again after it
#   * an unanswered INSTRUCTION round wakes with its ACTION named
#   * gateway down + start step does not recover -> non-zero exit,
#     no wake, failure surfaced (not silent)
#   * gateway down + start step recovers -> the poll proceeds
#   * a configured agent timeout below the 600s floor is clamped
#
# Run: home/openclaw/comment-loop/pilot-loop-prefilter.test.sh
set -uo pipefail

SELF_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
SCRIPT="$SELF_DIR/pilot-loop-prefilter.sh"

TMP_ROOT="$(mktemp -d)"
trap 'rm -rf "$TMP_ROOT"' EXIT

failures=0
pass() { echo "ok   - $*"; }
fail() {
  echo "FAIL - $*"
  failures=$((failures + 1))
}
check() { # check <label> <condition...>
  local label="$1"
  shift
  if "$@"; then pass "$label"; else fail "$label"; fi
}

# --- stub bin dir ---------------------------------------------------
STUB_BIN="$TMP_ROOT/bin"
mkdir -p "$STUB_BIN"

cat >"$STUB_BIN/gh" <<'EOF'
#!/usr/bin/env bash
# Stub gh: serves the fixture comments file for any api call.
cat "$FIXTURE_COMMENTS"
EOF

cat >"$STUB_BIN/openclaw" <<'EOF'
#!/usr/bin/env bash
# Stub openclaw agent: records argv, succeeds.
printf '%s\n' "$*" >>"$STUB_WAKE_LOG"
exit "${STUB_OPENCLAW_RC:-0}"
EOF

cat >"$STUB_BIN/systemctl" <<'EOF'
#!/usr/bin/env bash
# Stub systemctl: gateway state lives in $STUB_GATEWAY_STATE
# ("active" / "inactive" / "recoverable": inactive until start).
for a in "$@"; do
  case "$a" in
  is-active)
    state="$(cat "$STUB_GATEWAY_STATE")"
    if [ "$state" = "active" ]; then
      echo active
      exit 0
    fi
    echo inactive
    exit 3
    ;;
  start)
    echo start >>"$STUB_SYSTEMCTL_LOG"
    if [ "$(cat "$STUB_GATEWAY_STATE")" = "recoverable" ]; then
      echo active >"$STUB_GATEWAY_STATE"
    fi
    exit 0
    ;;
  esac
done
exit 0
EOF
chmod +x "$STUB_BIN/gh" "$STUB_BIN/openclaw" "$STUB_BIN/systemctl"

# --- fixtures ---------------------------------------------------------
FIXTURE_QUOTA="$TMP_ROOT/quota-current.json"
cat >"$FIXTURE_QUOTA" <<'EOF'
{
  "generatedAt": "2026-11-14T07:00:03.412Z",
  "source": "opencode-quota",
  "providers": [
    {
      "providerId": "fixture-pool",
      "state": "ok",
      "windows": [
        {"name": "fixture-model:5h", "pctOfLimit": 25.5, "resetsAt": "2026-11-14T09:00:00.000Z"},
        {"name": "fixture-model:weekly", "pctOfLimit": 40, "resetsAt": "2026-11-21T09:00:00.000Z"}
      ]
    },
    {
      "providerId": "fixture-down",
      "state": "error",
      "windows": [
        {"name": "down-model:5h", "pctOfLimit": 99, "resetsAt": "2026-11-14T09:00:00.000Z"}
      ]
    }
  ]
}
EOF

comments_none="$TMP_ROOT/comments-none.json"
cat >"$comments_none" <<'EOF'
[{"body": "an ordinary comment with no loop traffic"}]
EOF

comments_answered="$TMP_ROOT/comments-answered.json"
cat >"$comments_answered" <<'EOF'
[
  {"body": "pilot-loop:v1\nTYPE: STATUS-REQUEST\nROUND: R1\n"},
  {"body": "pilot-loop:v1\nTYPE: SESSION-REPORT\nREPLY: R1\nquota_left: unknown\n"}
]
EOF

comments_open_r2="$TMP_ROOT/comments-open-r2.json"
cat >"$comments_open_r2" <<'EOF'
[
  {"body": "pilot-loop:v1\nTYPE: STATUS-REQUEST\nROUND: R1\n"},
  {"body": "pilot-loop:v1\nTYPE: SESSION-REPORT\nREPLY: R1\nquota_left: 1%\n"},
  {"body": "pilot-loop:v1\nTYPE: STATUS-REQUEST\nROUND: R2\nASK: reply with a SESSION-REPORT\n"}
]
EOF

comments_instruction="$TMP_ROOT/comments-instruction.json"
cat >"$comments_instruction" <<'EOF'
[
  {"body": "pilot-loop:v1\nTYPE: INSTRUCTION\nROUND: R7\nACTION: checkpoint-then-continue\n"}
]
EOF

# --- case plumbing ------------------------------------------------------
# prep_case <name> <gateway-state>: fresh per-case dirs and logs.
prep_case() {
  CASE_DIR="$TMP_ROOT/$1"
  mkdir -p "$CASE_DIR/state"
  echo "$2" >"$CASE_DIR/gateway-state"
  : >"$CASE_DIR/wakes.log"
  : >"$CASE_DIR/systemctl.log"
}

# run_script <comments-file> <out-file> [extra env KEY=VAL ...]
run_script() {
  local fixture="$1" out="$2"
  shift 2
  env PATH="$STUB_BIN:$PATH" HOME="$TMP_ROOT" \
    FIXTURE_COMMENTS="$fixture" \
    STUB_WAKE_LOG="$CASE_DIR/wakes.log" \
    STUB_SYSTEMCTL_LOG="$CASE_DIR/systemctl.log" \
    STUB_GATEWAY_STATE="$CASE_DIR/gateway-state" \
    COMMENT_LOOP_STATE_DIR="$CASE_DIR/state" \
    COMMENT_LOOP_QUOTA_FILE="$FIXTURE_QUOTA" \
    COMMENT_LOOP_GH="$STUB_BIN/gh" \
    COMMENT_LOOP_OPENCLAW="$STUB_BIN/openclaw" \
    COMMENT_LOOP_SYSTEMCTL="$STUB_BIN/systemctl" \
    "$@" \
    bash "$SCRIPT" 9818 >"$out" 2>&1
  echo $?
}

wake_count() { wc -l <"$CASE_DIR/wakes.log" | tr -d ' '; }

# 1. idle: no rounds at all
prep_case idle active
rc="$(run_script "$comments_none" "$CASE_DIR/out.txt")"
check "idle: exit 0" [ "$rc" -eq 0 ]
check "idle: prints NO_REPLY" grep -qx NO_REPLY "$CASE_DIR/out.txt"
check "idle: no agent wake" [ "$(wake_count)" -eq 0 ]

# 2. idle: the only round is answered
prep_case answered active
rc="$(run_script "$comments_answered" "$CASE_DIR/out.txt")"
check "answered: exit 0" [ "$rc" -eq 0 ]
check "answered: prints NO_REPLY" grep -qx NO_REPLY "$CASE_DIR/out.txt"
check "answered: no agent wake" [ "$(wake_count)" -eq 0 ]

# 3. new unanswered STATUS-REQUEST R2 -> one wake, real quota line
prep_case open-r2 active
rc="$(run_script "$comments_open_r2" "$CASE_DIR/out.txt")"
check "open R2: exit 0" [ "$rc" -eq 0 ]
check "open R2: exactly one wake" [ "$(wake_count)" -eq 1 ]
check "open R2: wake uses the 900s default timeout" grep -q -- "--timeout 900" "$CASE_DIR/wakes.log"
msg="$CASE_DIR/state/wake-9818-R2.md"
check "open R2: wake message written for R2" [ -f "$msg" ]
check "open R2: quota line is the fixture's real value (74.5% left)" grep -q "74.5% left on the tightest measured 5h window (fixture-model:5h" "$msg"
check "open R2: quota line names its measurement time" grep -q "measured 2026-11-14T07:00:03.412Z by the quota gatherer" "$msg"
check "open R2: unhealthy provider windows are excluded" bash -c "! grep -q 'down-model' '$msg'"

# 4. immediate re-run: same round still unanswered but just woken
rc="$(run_script "$comments_open_r2" "$CASE_DIR/out2.txt")"
check "re-wake window: exit 0" [ "$rc" -eq 0 ]
check "re-wake window: still exactly one wake" [ "$(wake_count)" -eq 1 ]
check "re-wake window: prints NO_REPLY" grep -qx NO_REPLY "$CASE_DIR/out2.txt"

# 5. after the re-wake window the still-unanswered round wakes again
printf '2 %s\n' "$(( $(date +%s) - 7200 ))" >"$CASE_DIR/state/issue-9818.wake"
rc="$(run_script "$comments_open_r2" "$CASE_DIR/out3.txt")"
check "after window: exit 0" [ "$rc" -eq 0 ]
check "after window: second wake happens" [ "$(wake_count)" -eq 2 ]

# 6. INSTRUCTION round wakes with its action named
prep_case instruction active
rc="$(run_script "$comments_instruction" "$CASE_DIR/out.txt")"
check "instruction: exit 0" [ "$rc" -eq 0 ]
check "instruction: exactly one wake" [ "$(wake_count)" -eq 1 ]
check "instruction: wake names checkpoint-then-continue" grep -q "checkpoint-then-continue" "$CASE_DIR/state/wake-9818-R7.md"

# 7. gateway down, start step cannot recover -> surface, no wake
prep_case gw-down inactive
rc="$(run_script "$comments_open_r2" "$CASE_DIR/out.txt")"
check "gateway down: non-zero exit" [ "$rc" -ne 0 ]
check "gateway down: start step attempted" grep -qx start "$CASE_DIR/systemctl.log"
check "gateway down: failure surfaced in output" grep -q "not active" "$CASE_DIR/out.txt"
check "gateway down: no agent wake" [ "$(wake_count)" -eq 0 ]

# 8. gateway down but the start step recovers it -> poll proceeds
prep_case gw-recover recoverable
rc="$(run_script "$comments_none" "$CASE_DIR/out.txt")"
check "gateway recover: exit 0" [ "$rc" -eq 0 ]
check "gateway recover: start step attempted" grep -qx start "$CASE_DIR/systemctl.log"
check "gateway recover: poll proceeds (NO_REPLY on idle issue)" grep -qx NO_REPLY "$CASE_DIR/out.txt"

# 9. timeout floor: a configured 60s is clamped to 600s
prep_case clamp active
rc="$(run_script "$comments_open_r2" "$CASE_DIR/out.txt" COMMENT_LOOP_AGENT_TIMEOUT_SECONDS=60)"
check "clamp: exit 0" [ "$rc" -eq 0 ]
check "clamp: wake uses the 600s floor, not 60s" grep -q -- "--timeout 600" "$CASE_DIR/wakes.log"

echo
if [ "$failures" -gt 0 ]; then
  echo "$failures test(s) FAILED"
  exit 1
fi
echo "all prefilter tests passed"
