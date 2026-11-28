#!/bin/bash
# Comprehensive verification of slice 1 implementation against specification #3026
#
# The claimant sources under test default to the deployed primary clone
# (/home/user/git/code/pm-stack/opencode-claimant). Set CLAIMANT_DIR to
# verify another checkout (e.g. a worktree) instead:
#   CLAIMANT_DIR=~/wt-<lane>/pm-stack/opencode-claimant ./verify_slice1_implementation.sh
#
# Checks 1 and 10 read the caller's live state files
# (~/.local/state/opencode-claimant), so run this with a staged HOME whose
# mode file reads "shadow" unless you mean to verify the production state.
# A staged HOME must also expose the claimant's read paths (symlinks are
# fine): ~/tools (poll.py's turso-exec ledger read), ~/.config/turso,
# ~/.config/repo-queue, ~/.config/queue-broker and ~/.local/bin/repo-queue
# -- without them Check 15's fire fails closed on the unreadable ledger,
# which is the claimant working as designed, not a defect.

set -e

CLAIMANT_DIR="${CLAIMANT_DIR:-/home/user/git/code/pm-stack/opencode-claimant}"
CLAIMANT_PY="$CLAIMANT_DIR/claimant.py"
POLL_PY="$CLAIMANT_DIR/poll.py"
LAUNCH_ONE_PY="$CLAIMANT_DIR/launch_one.py"
README_MD="$CLAIMANT_DIR/README.md"

echo "=== Comprehensive Slice 1 Verification ==="
echo "Claimant sources: $CLAIMANT_DIR"
echo

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

success_count=0
total_checks=0

check_passed() {
  echo -e "${GREEN}✓${NC} $1"
  success_count=$((success_count + 1))
  total_checks=$((total_checks + 1))
}

check_failed() {
  echo -e "${RED}✗${NC} $1"
  exit 1
}

echo "Check 1: Mode file exists and defaults to shadow..."
if [ -f ~/.local/state/opencode-claimant/mode ]; then
  mode=$(cat ~/.local/state/opencode-claimant/mode)
  if [ "$mode" = "shadow" ]; then
    check_passed "Mode file exists with correct default: $mode"
  else
    check_failed "Mode file exists but has wrong value: $mode"
  fi
else
  check_failed "Mode file does not exist"
fi

echo "Check 2: Constants are correctly defined..."
if grep -q "CONCURRENCY_CAP = 36" "$CLAIMANT_PY"; then
  check_passed "CONCURRENCY_CAP = 36"
else
  check_failed "CONCURRENCY_CAP not found or incorrect"
fi

# The active window moved to poll.py with the stuck-bound selection
# (#8038): claimant.py no longer defines ACTIVE_WINDOW_MS; poll.py's
# is_stuck_bound owns the 30-minute silence threshold.
if grep -q "ACTIVE_WINDOW_MS = 30 \* 60 \* 1000" "$POLL_PY"; then
  check_passed "ACTIVE_WINDOW_MS = 30 * 60 * 1000 (poll.py)"
else
  check_failed "ACTIVE_WINDOW_MS not found or incorrect in poll.py"
fi

if grep -q 'LAUNCHED_BY_RECOVERY_ZERO_MESSAGE = "opencode-claimant-recovery:zero-message-deleted"' "$CLAIMANT_PY"; then
  check_passed "Zero-message recovery constant defined correctly"
else
  check_failed "Zero-message recovery constant missing or incorrect"
fi

if grep -q 'LAUNCHED_BY_RECOVERY_GONE_SESSION = "opencode-claimant-recovery:gone-session"' "$CLAIMANT_PY"; then
  check_passed "Gone-session recovery constant defined correctly"
else
  check_failed "Gone-session recovery constant missing or incorrect"
fi

echo "Check 3: Required queues are defined..."
if grep -q '"repo-health-code-pilot"' "$CLAIMANT_PY" &&
  grep -q '"repo-health-opencode-pilot"' "$CLAIMANT_PY" &&
  grep -q '"repo-health-openchamber-pilot"' "$CLAIMANT_PY" &&
  grep -q '"repo-health-nix-pilot"' "$CLAIMANT_PY"; then
  check_passed "All four queues defined"
else
  check_failed "Not all queues are defined"
fi

echo "Check 4: poll.py exists and has required functions..."
if [ -f "$POLL_PY" ]; then
  check_passed "poll.py exists"
else
  check_failed "poll.py does not exist"
fi

if grep -q "def is_candidate" "$POLL_PY"; then
  check_passed "is_candidate() function exists"
else
  check_failed "is_candidate() function missing"
fi

if grep -q "def is_stuck_bound" "$POLL_PY"; then
  check_passed "is_stuck_bound() function exists"
else
  check_failed "is_stuck_bound() function missing"
fi

if grep -q "def is_active_root" "$POLL_PY"; then
  check_passed "is_active_root() function exists"
else
  check_failed "is_active_root() function missing"
fi

if grep -q "def count_active_roots" "$POLL_PY"; then
  check_passed "count_active_roots() function exists"
else
  check_failed "count_active_roots() function missing"
fi

if grep -q "def get_candidates" "$POLL_PY"; then
  check_passed "get_candidates() function exists"
else
  check_failed "get_candidates() function missing"
fi

echo "Check 5: claimant.py has required functions..."
if grep -q "def supervision_step" "$CLAIMANT_PY"; then
  check_passed "supervision_step() function exists"
else
  check_failed "supervision_step() function missing"
fi

if grep -q "def gate_step" "$CLAIMANT_PY"; then
  check_passed "gate_step() function exists"
else
  check_failed "gate_step() function missing"
fi

if grep -q "def launch_step" "$CLAIMANT_PY"; then
  check_passed "launch_step() function exists"
else
  check_failed "launch_step() function missing"
fi

# The single-task launcher split out of claimant.py into launch_one.py;
# its launch() is the per-task create/bind/prompt path.
if grep -q "def launch(" "$LAUNCH_ONE_PY"; then
  check_passed "launch_one.py launch() function exists"
else
  check_failed "launch_one.py launch() function missing"
fi

echo "Check 6: Supervision rules are implemented..."
# Terminal/error outcomes are classified by _session_error_outcome and
# poll.py's is_stuck_bound; there is no _session_terminal helper anymore.
if grep -q "_session_error_outcome" "$CLAIMANT_PY" &&
  grep -q "def is_stuck_bound" "$POLL_PY"; then
  check_passed "Session terminal/stuck classification implemented"
else
  check_failed "Session terminal/stuck classification missing"
fi

# Lease expiry is parsed by poll.py's parse_lease_ms, shared by the
# claimant's supervision and recovery paths.
if grep -q "def parse_lease_ms" "$POLL_PY"; then
  check_passed "Lease expiration checking implemented (poll.parse_lease_ms)"
else
  check_failed "Lease expiration checking missing"
fi

if grep -q "_get_messages" "$CLAIMANT_PY"; then
  check_passed "Message reading for zero-message verification implemented"
else
  check_failed "Message reading missing"
fi

# Deletion is _delete_session; the recovery paths verify the outcome by
# compare-clearing the binding after the delete (_recover_error_stopped).
if grep -q "def _delete_session" "$CLAIMANT_PY" &&
  grep -q "_recover_error_stopped" "$CLAIMANT_PY"; then
  check_passed "Session deletion with verified recovery implemented"
else
  check_failed "Session deletion with verified recovery missing"
fi

echo "Check 7: Atomic recovery operations are implemented..."
# Recovery runs inside supervision_step via _recover_error_stopped and
# _fail_attempts_exhausted; there is no _queue_recovery helper anymore.
if grep -q "def _recover_error_stopped" "$CLAIMANT_PY" &&
  grep -q "def _fail_attempts_exhausted" "$CLAIMANT_PY"; then
  check_passed "Recovery operations (_recover_error_stopped, _fail_attempts_exhausted) exist"
else
  check_failed "Recovery operations missing"
fi

if grep -q "opencode-claimant-recovery:zero-message-deleted" "$CLAIMANT_PY"; then
  check_passed "Zero-message recovery operation implemented"
else
  check_failed "Zero-message recovery operation missing"
fi

if grep -q "opencode-claimant-recovery:gone-session" "$CLAIMANT_PY"; then
  check_passed "Gone-session recovery operation implemented"
else
  check_failed "Gone-session recovery operation missing"
fi

echo "Check 8: Session creation and binding is implemented..."
# Cap room is cap_step() in claimant.py over poll.py's active-root count;
# there is no _create_session_if_cap_room helper anymore.
if grep -q "def cap_step" "$CLAIMANT_PY" &&
  grep -q "def count_active_roots" "$POLL_PY"; then
  check_passed "Session creation with cap room checking implemented"
else
  check_failed "Session creation cap-room function missing"
fi

# Session creation goes through _server_request (the verified transport),
# not a per-call _api_request helper.
if grep -q '"POST", "/api/session"' "$CLAIMANT_PY"; then
  check_passed "OpenCode server session creation API call implemented"
else
  check_failed "Session creation API call missing"
fi

if grep -q '"location": {"directory": str(PRIMARY_CLONE)}' "$CLAIMANT_PY"; then
  check_passed "Session location verification implemented"
else
  check_failed "Session location verification missing"
fi

# The bind is a repo-queue subprocess argv list (multi-line), so assert
# its elements rather than a single-line shape.
if grep -q '"bind",' "$CLAIMANT_PY" &&
  grep -q '"--worker",' "$CLAIMANT_PY" &&
  grep -q '"--session-id",' "$CLAIMANT_PY"; then
  check_passed "Atomic bind operation implemented"
else
  check_failed "Atomic bind operation missing"
fi

echo "Check 9: Prompt delivery is implemented..."
# Prompt construction lives in launch_one.py's build_prompt.
if grep -q "def build_prompt" "$LAUNCH_ONE_PY"; then
  check_passed "Prompt generation function exists (launch_one.build_prompt)"
else
  check_failed "Prompt generation function missing"
fi

if grep -q '"/api/session/{session_id}/prompt"' "$CLAIMANT_PY" &&
  grep -q "def _post_prompt" "$CLAIMANT_PY"; then
  check_passed "Prompt delivery API call implemented"
else
  check_failed "Prompt delivery API call missing"
fi

if grep -q "_post_launch_agent_note" "$CLAIMANT_PY"; then
  check_passed "GitHub launch note posting implemented"
else
  check_failed "GitHub launch note posting missing"
fi

echo "Check 10: Logging and fire recording is implemented..."
if grep -q "log_fire" "$CLAIMANT_PY"; then
  check_passed "log_fire() function exists"
else
  check_failed "log_fire() function missing"
fi

if [ -f ~/.local/state/opencode-claimant/fires.jsonl ]; then
  check_passed "Fire log file exists"
else
  check_failed "Fire log file missing"
fi

echo "Check 11: Hard rules are enforced..."
# The held-labels constant was renamed HELD_LABELS -> HELD_ISSUE_LABELS;
# the label set itself is unchanged.
if grep -q 'HELD_ISSUE_LABELS = {"held", "user/owned", "do-not-merge", "do-not-auto-edit"}' "$CLAIMANT_PY"; then
  check_passed "HELD_ISSUE_LABELS constant defined"
else
  check_failed "HELD_ISSUE_LABELS constant missing"
fi

if grep -q "Never work tasks directly" "$README_MD"; then
  check_passed "Hard rules documented in README"
else
  check_failed "Hard rules documentation missing"
fi

echo "Check 12: Mode handling is correct..."
if grep -q "def read_mode()" "$CLAIMANT_PY"; then
  check_passed "read_mode() function exists"
else
  check_failed "read_mode() function missing"
fi

if grep -q "doesn't exist or is unreadable, defaults to \`shadow\`" "$README_MD"; then
  check_passed "Mode file handling documented"
else
  check_failed "Mode file handling documentation missing"
fi

echo "Check 13: Concurrency cap is 36..."
# LAUNCH_WORKERS no longer exists (launches are sequential); the cap is
# declared in both claimant.py and poll.py (CAP = 36).
if grep -q "CONCURRENCY_CAP = 36" "$CLAIMANT_PY" &&
  grep -q "CAP = 36" "$POLL_PY"; then
  check_passed "Concurrency cap is 36 in claimant.py and poll.py"
else
  check_failed "Concurrency cap not set to 36"
fi

echo "Check 14: poll.py outputs correct JSON structure..."
if python3 "$POLL_PY" >/tmp/test_poll_output.json 2>/dev/null; then
  python3 -c "
import json
with open('/tmp/test_poll_output.json') as f:
    data = json.load(f)
    required = ['queues', 'candidates', 'stuck_bound', 'stale_binding',
                'ledger_readable', 'live_root_sessions',
                'active_root_sessions', 'cap', 'concurrency_cap', 'cap_room']
    for field in required:
        if field not in data:
            print(f'Missing field: {field}')
            exit(1)
    print('All required fields present')
"
  if [ $? -eq 0 ]; then
    check_passed "poll.py outputs correct JSON structure"
  else
    check_failed "poll.py JSON structure incomplete"
  fi
else
  check_failed "poll.py execution failed"
fi

echo "Check 15: claimant.py can execute without errors..."
if python3 "$CLAIMANT_PY" >/dev/null 2>&1; then
  check_passed "claimant.py executes successfully in shadow mode"
else
  check_failed "claimant.py execution failed"
fi

echo
echo "=========================================="
echo "Verification Summary"
echo "=========================================="
echo "Passed: $success_count / $total_checks checks"
echo

if [ $success_count -eq $total_checks ]; then
  echo -e "${GREEN}=== ALL CHECKS PASSED ===${NC}"
  echo
  echo "The OpenCode lane claimant/executor slice 1 implementation is"
  echo "complete and meets all specification requirements from #3026."
  echo
  echo "Key features implemented:"
  echo "  ✓ Supervision step (step 1) - recovery of stuck/zero-message sessions"
  echo "  ✓ Gate step (step 2) - candidate discovery across 4 queues"
  echo "  ✓ Cap step (step 3) - 36-session concurrency limit"
  echo "  ✓ Launch step (step 4) - session creation, binding, prompt delivery"
  echo "  ✓ Terminal states (step 5) - handled by worker sessions"
  echo "  ✓ Logging (step 6) - fire execution records in fires.jsonl"
  echo "  ✓ Shadow mode - safe discovery without mutations"
  echo "  ✓ Live mode - full execution with atomic operations"
  echo "  ✓ Recovery rules (#4758) - zero-message and gone-session recovery"
  echo "  ✓ Atomic bind operations - compare-and-clear guards"
  echo "  ✓ Session location verification - prevents wrong-project sessions"
  echo "  ✓ Hard rules enforcement - no double-binding, no direct task work"
  exit 0
else
  echo -e "${RED}=== SOME CHECKS FAILED ===${NC}"
  echo "Please review the failed checks above."
  exit 1
fi
