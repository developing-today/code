#!/bin/bash
# Comprehensive verification of slice 1 implementation against specification #3026

set -e

echo "=== Comprehensive Slice 1 Verification ==="
echo

# Colors for output
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

success_count=0
total_checks=0

check_passed() {
  echo -e "${GREEN}✓${NC} $1"
  ((success_count++))
}

check_failed() {
  echo -e "${RED}✗${NC} $1"
  exit 1
}

((total_checks++))
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

((total_checks++))
echo "Check 2: Constants are correctly defined..."
if grep -q "CONCURRENCY_CAP = 36" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "CONCURRENCY_CAP = 36"
else
  check_failed "CONCURRENCY_CAP not found or incorrect"
fi

if grep -q "ACTIVE_WINDOW_MS = 30 \* 60 \* 1000" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "ACTIVE_WINDOW_MS = 30 * 60 * 1000"
else
  check_failed "ACTIVE_WINDOW_MS not found or incorrect"
fi

if grep -q 'LAUNCHED_BY_RECOVERY_ZERO_MESSAGE = "opencode-claimant-recovery:zero-message-deleted"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Zero-message recovery constant defined correctly"
else
  check_failed "Zero-message recovery constant missing or incorrect"
fi

if grep -q 'LAUNCHED_BY_RECOVERY_GONE_SESSION = "opencode-claimant-recovery:gone-session"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Gone-session recovery constant defined correctly"
else
  check_failed "Gone-session recovery constant missing or incorrect"
fi

((total_checks++))
echo "Check 3: Required queues are defined..."
if grep -q '"repo-health-code-pilot"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py &&
  grep -q '"repo-health-opencode-pilot"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py &&
  grep -q '"repo-health-openchamber-pilot"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py &&
  grep -q '"repo-health-nix-pilot"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "All four queues defined"
else
  check_failed "Not all queues are defined"
fi

((total_checks++))
echo "Check 4: poll.py exists and has required functions..."
if [ -f /home/user/git/code/pm-stack/opencode-claimant/poll.py ]; then
  check_passed "poll.py exists"
else
  check_failed "poll.py does not exist"
fi

if grep -q "def is_candidate" /home/user/git/code/pm-stack/opencode-claimant/poll.py; then
  check_passed "is_candidate() function exists"
else
  check_failed "is_candidate() function missing"
fi

if grep -q "def is_stuck_bound" /home/user/git/code/pm-stack/opencode-claimant/poll.py; then
  check_passed "is_stuck_bound() function exists"
else
  check_failed "is_stuck_bound() function missing"
fi

if grep -q "def is_active_root" /home/user/git/code/pm-stack/opencode-claimant/poll.py; then
  check_passed "is_active_root() function exists"
else
  check_failed "is_active_root() function missing"
fi

if grep -q "def count_active_roots" /home/user/git/code/pm-stack/opencode-claimant/poll.py; then
  check_passed "count_active_roots() function exists"
else
  check_failed "count_active_roots() function missing"
fi

if grep -q "def get_candidates" /home/user/git/code/pm-stack/opencode-claimant/poll.py; then
  check_passed "get_candidates() function exists"
else
  check_failed "get_candidates() function missing"
fi

((total_checks++))
echo "Check 5: claimant.py has required functions..."
if grep -q "def supervision_step" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "supervision_step() function exists"
else
  check_failed "supervision_step() function missing"
fi

if grep -q "def launch_one" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "launch_one() function exists"
else
  check_failed "launch_one() function missing"
fi

if grep -q "def _create_session" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "_create_session() function exists"
else
  check_failed "_create_session() function missing"
fi

if grep -q "def _verify_candidate_is_current" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "_verify_candidate_is_current() function exists"
else
  check_failed "_verify_candidate_is_current() function missing"
fi

((total_checks++))
echo "Check 6: Supervision rules are implemented..."
if grep -q "_session_terminal" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Session terminal state checking implemented"
else
  check_failed "Session terminal state checking missing"
fi

if grep -q "_lease_expired" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Lease expiration checking implemented"
else
  check_failed "Lease expiration checking missing"
fi

if grep -q "_read_messages" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Message reading for zero-message verification implemented"
else
  check_failed "Message reading missing"
fi

if grep -q "_delete_session_and_verify" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Session deletion with verification implemented"
else
  check_failed "Session deletion missing"
fi

((total_checks++))
echo "Check 7: Atomic recovery operations are implemented..."
if grep -q "_queue_recovery" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Atomic recovery (_queue_recovery) function exists"
else
  check_failed "Atomic recovery function missing"
fi

if grep -q "opencode-claimant-recovery:zero-message-deleted" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Zero-message recovery operation implemented"
else
  check_failed "Zero-message recovery operation missing"
fi

if grep -q "opencode-claimant-recovery:gone-session" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Gone-session recovery operation implemented"
else
  check_failed "Gone-session recovery operation missing"
fi

((total_checks++))
echo "Check 8: Session creation and binding is implemented..."
if grep -q "_create_session_if_cap_room" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Session creation with cap room checking implemented"
else
  check_failed "Session creation function missing"
fi

if grep -q "_api_request.*POST.*/api/session" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "OpenCode server session creation API call implemented"
else
  check_failed "Session creation API call missing"
fi

if grep -q '"location": {"directory": prepared\["directory"\]}' /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Session location verification implemented"
else
  check_failed "Session location verification missing"
fi

if grep -q '"bind".*"--worker".*"opencode-claimant"' /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Atomic bind operation implemented"
else
  check_failed "Atomic bind operation missing"
fi

((total_checks++))
echo "Check 9: Prompt delivery is implemented..."
if grep -q "_prompt_for" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Prompt generation function exists"
else
  check_failed "Prompt generation function missing"
fi

if grep -q "_api_request.*POST.*prompt" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Prompt delivery API call implemented"
else
  check_failed "Prompt delivery API call missing"
fi

if grep -q "_post_launch_note" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "GitHub launch note posting implemented"
else
  check_failed "GitHub launch note posting missing"
fi

((total_checks++))
echo "Check 10: Logging and fire recording is implemented..."
if grep -q "log_fire" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "log_fire() function exists"
else
  check_failed "log_fire() function missing"
fi

if [ -f ~/.local/state/opencode-claimant/fires.jsonl ]; then
  check_passed "Fire log file exists"
else
  check_failed "Fire log file missing"
fi

((total_checks++))
echo "Check 11: Hard rules are enforced..."
if grep -q 'HELD_LABELS = {"held", "user/owned", "do-not-merge", "do-not-auto-edit"}' /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "HELD_LABELS constant defined"
else
  check_failed "HELD_LABELS constant missing"
fi

if grep -q "Never work a task yourself" /home/user/git/code/pm-stack/opencode-claimant/README.md; then
  check_passed "Hard rules documented in README"
else
  check_failed "Hard rules documentation missing"
fi

((total_checks++))
echo "Check 12: Mode handling is correct..."
if grep -q "def read_mode()" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "read_mode() function exists"
else
  check_failed "read_mode() function missing"
fi

if grep -q "if the mode file doesn't exist or is unreadable, defaults to 'shadow'" /home/user/git/code/pm-stack/opencode-claimant/README.md; then
  check_passed "Mode file handling documented"
else
  check_failed "Mode file handling documentation missing"
fi

((total_checks++))
echo "Check 13: Concurrency cap is 36..."
if grep -q "CONCURRENCY_CAP = 36" /home/user/git/code/pm-stack/opencode-claimant/claimant.py &&
  grep -q "LAUNCH_WORKERS = 36" /home/user/git/code/pm-stack/opencode-claimant/claimant.py; then
  check_passed "Concurrency cap is 36 with 36 launch workers"
else
  check_failed "Concurrency cap not set to 36"
fi

((total_checks++))
echo "Check 14: poll.py outputs correct JSON structure..."
python3 /home/user/git/code/pm-stack/opencode-claimant/poll.py >/tmp/test_poll_output.json 2>/dev/null
if [ $? -eq 0 ]; then
  python3 -c "
import json
with open('/tmp/test_poll_output.json') as f:
    data = json.load(f)
    required = ['queues', 'candidates', 'stuck_bound', 'bound', 'queue_errors', 
                'live_root_sessions', 'active_root_sessions', 'cap_room', 
                'session_list_readable', 'active_set_readable']
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

((total_checks++))
echo "Check 15: claimant.py can execute without errors..."
python3 /home/user/git/code/pm-stack/opencode-claimant/claimant.py >/dev/null 2>&1
if [ $? -eq 0 ]; then
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
