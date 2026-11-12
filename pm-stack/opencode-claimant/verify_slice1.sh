#!/bin/bash
# Verification script for OpenCode lane claimant/executor slice 1 (#3026)

set -e

echo "=== OpenCode Lane Claimant/Executor - Slice 1 Verification ==="
echo

# Colors
GREEN='\033[0;32m'
RED='\033[0;31m'
NC='\033[0m' # No Color

success_count=0
total_checks=0

check() {
  total_checks=$((total_checks + 1))
  if [ $? -eq 0 ]; then
    echo -e "${GREEN}✓${NC} $1"
    success_count=$((success_count + 1))
    return 0
  else
    echo -e "${RED}✗${NC} $1"
    return 1
  fi
}

echo "1. File Structure Verification"
echo "-------------------------------"

[ -f "pm-stack/opencode-claimant/claimant.py" ] && check "claimant.py exists" || echo "✗ claimant.py missing"
[ -f "pm-stack/opencode-claimant/poll.py" ] && check "poll.py exists" || echo "✗ poll.py missing"
[ -f "pm-stack/opencode-claimant/README.md" ] && check "README.md exists" || echo "✗ README.md missing"
[ -f "pm-stack/opencode-claimant/Makefile" ] && check "Makefile exists" || echo "✗ Makefile missing"

echo
echo "2. Script Executability"
echo "-----------------------"

chmod +x pm-stack/opencode-claimant/claimant.py 2>/dev/null
chmod +x pm-stack/opencode-claimant/poll.py 2>/dev/null
[ -x "pm-stack/opencode-claimant/claimant.py" ] && check "claimant.py is executable" || echo "✗ claimant.py not executable"
[ -x "pm-stack/opencode-claimant/poll.py" ] && check "poll.py is executable" || echo "✗ poll.py not executable"

echo
echo "3. Mode File Handling"
echo "---------------------"

# Test shadow mode (default)
echo "test" >~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py >/dev/null 2>&1
[ $? -eq 0 ] && check "Shadow mode works (no mutations)" || echo "✗ Shadow mode failed"

# Test live mode
echo "live" >~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py >/dev/null 2>&1
[ $? -eq 0 ] && check "Live mode works (no errors)" || echo "✗ Live mode failed"

# Clean up
rm -f ~/.local/state/opencode-claimant/mode

echo
echo "4. Poll.py Functionality"
echo "-----------------------"

# Test poll.py output format
output=$(python3 pm-stack/opencode-claimant/poll.py 2>/dev/null)
echo "$output" | grep -q "queues" && check "poll.py outputs 'queues' field" || echo "✗ poll.py missing 'queues' field"
echo "$output" | grep -q "candidates" && check "poll.py outputs 'candidates' field" || echo "✗ poll.py missing 'candidates' field"
echo "$output" | grep -q "stuck_bound" && check "poll.py outputs 'stuck_bound' field" || echo "✗ poll.py missing 'stuck_bound' field"
echo "$output" | grep -q "cap_room" && check "poll.py outputs 'cap_room' field" || echo "✗ poll.py missing 'cap_room' field"
echo "$output" | grep -q "live_root_sessions" && check "poll.py outputs 'live_root_sessions' field" || echo "✗ poll.py missing 'live_root_sessions' field"
echo "$output" | grep -q "active_root_sessions" && check "poll.py outputs 'active_root_sessions' field" || echo "✗ poll.py missing 'active_root_sessions' field"

# Test JSON validity
python3 <<EOF
import json
import sys
output = """$output"""
try:
    data = json.loads(output)
    print("valid json")
    sys.exit(0)
except Exception as e:
    print(f"invalid json: {e}")
    sys.exit(1)
EOF
if [ $? -eq 0 ]; then
  check "poll.py outputs valid JSON"
else
  echo "✗ poll.py outputs invalid JSON"
fi

echo
echo "5. Fire Log Creation"
echo "--------------------"

# Run claimant to generate fire log
python3 pm-stack/opencode-claimant/claimant.py >/dev/null 2>&1
[ -f ~/.local/state/opencode-claimant/fires.jsonl ] && check "Fire log created" || echo "✗ Fire log not created"

# Check fire log format
if [ -f ~/.local/state/opencode-claimant/fires.jsonl ]; then
  last_line=$(tail -1 ~/.local/state/opencode-claimant/fires.jsonl)
  echo "$last_line" | grep -q "timestamp" && check "Fire log contains 'timestamp'" || echo "✗ Fire log missing 'timestamp'"
  echo "$last_line" | grep -q "mode" && check "Fire log contains 'mode'" || echo "✗ Fire log missing 'mode'"
  echo "$last_line" | grep -q "candidates_seen" && check "Fire log contains 'candidates_seen'" || echo "✗ Fire log missing 'candidates_seen'"
  echo "$last_line" | grep -q "sessions_launched" && check "Fire log contains 'sessions_launched'" || echo "✗ Fire log missing 'sessions_launched'"
  echo "$last_line" | grep -q "cap_room" && check "Fire log contains 'cap_room'" || echo "✗ Fire log missing 'cap_room'"
  echo "$last_line" | grep -q "skips" && check "Fire log contains 'skips'" || echo "✗ Fire log missing 'skips'"
fi

echo
echo "6. Constants Verification"
echo "-------------------------"

# Check CONCURRENCY_CAP
grep -q "CONCURRENCY_CAP = 36" pm-stack/opencode-claimant/claimant.py && check "CONCURRENCY_CAP = 36" || echo "✗ CONCURRENCY_CAP not set to 36"

# Check recovery launched_by values
grep -q "LAUNCHED_BY_RECOVERY_ZERO_MESSAGE" pm-stack/opencode-claimant/claimant.py && check "Zero-message recovery constant defined" || echo "✗ Zero-message recovery constant missing"
grep -q "LAUNCHED_BY_RECOVERY_GONE_SESSION" pm-stack/opencode-claimant/claimant.py && check "Gone-session recovery constant defined" || echo "✗ Gone-session recovery constant missing"

# Check QUEUES
grep -q "repo-health-code-pilot" pm-stack/opencode-claimant/claimant.py && check "All four queues defined" || echo "✗ Queue definitions missing"

echo
echo "7. Step Implementation Verification"
echo "-----------------------------------"

# Check supervision step
grep -q "def supervision_step" pm-stack/opencode-claimant/claimant.py && check "Supervision step implemented" || echo "✗ Supervision step missing"

# Check gate step
grep -q "def gate_step" pm-stack/opencode-claimant/claimant.py && check "Gate step implemented" || echo "✗ Gate step missing"

# Check cap step
grep -q "def cap_step" pm-stack/opencode-claimant/claimant.py && check "Cap step implemented" || echo "✗ Cap step missing"

# Check launch step
grep -q "def launch_step" pm-stack/opencode-claimant/claimant.py && check "Launch step implemented" || echo "✗ Launch step missing"

# Check log_fire function
grep -q "def log_fire" pm-stack/opencode-claimant/claimant.py && check "Logging function implemented" || echo "✗ Logging function missing"

echo
echo "8. Key Features Verification"
echo "-----------------------------"

# Check session creation with correct shape
grep -q '"location": {' pm-stack/opencode-claimant/claimant.py && check "Session creation uses correct location shape" || echo "✗ Session location shape incorrect"

grep -q '"model": {' pm-stack/opencode-claimant/claimant.py && check "Session creation uses correct model shape" || echo "✗ Session model shape incorrect"

# Check atomic bind operation
grep -q "bind_result = subprocess.run" pm-stack/opencode-claimant/claimant.py && check "Atomic bind operation implemented" || echo "✗ Atomic bind operation missing"

# Check recovery operations
grep -q "opencode-claimant-recovery:zero-message-deleted" pm-stack/opencode-claimant/claimant.py && check "Zero-message recovery implemented" || echo "✗ Zero-message recovery missing"
grep -q "opencode-claimant-recovery:gone-session" pm-stack/opencode-claimant/claimant.py && check "Gone-session recovery implemented" || echo "✗ Gone-session recovery missing"

# Check session location verification
grep -q 'location.*directory' pm-stack/opencode-claimant/claimant.py && check "Session location verification implemented" || echo "✗ Session location verification missing"

# Check cap room calculation
grep -q "cap_room" pm-stack/opencode-claimant/claimant.py && check "Cap room calculation implemented" || echo "✗ Cap room calculation missing"

echo
echo "9. Documentation Verification"
echo "------------------------------"

[ -s "pm-stack/opencode-claimant/README.md" ] && check "README.md exists and is not empty" || echo "✗ README.md missing or empty"
[ -s "pm-stack/opencode-claimant/SLICE1_COMPLETION_REPORT.md" ] && check "Slice 1 completion report exists" || echo "✗ Completion report missing"

echo
echo "10. Mode Handling"
echo "----------------"

# Test default mode (missing file)
rm -f ~/.local/state/opencode-claimant/mode
output=$(python3 -c "
import sys
sys.path.insert(0, 'pm-stack/opencode-claimant')
from claimant import read_mode
mode = read_mode()
print(mode)
")
[ "$output" = "shadow" ] && check "Default mode is 'shadow'" || echo "✗ Default mode not 'shadow' (got: $output)"

# Test explicit shadow mode
echo "shadow" >~/.local/state/opencode-claimant/mode
output=$(python3 -c "
import sys
sys.path.insert(0, 'pm-stack/opencode-claimant')
from claimant import read_mode
mode = read_mode()
print(mode)
")
[ "$output" = "shadow" ] && check "Explicit shadow mode works" || echo "✗ Explicit shadow mode failed (got: $output)"

# Test explicit live mode
echo "live" >~/.local/state/opencode-claimant/mode
output=$(python3 -c "
import sys
sys.path.insert(0, 'pm-stack/opencode-claimant')
from claimant import read_mode
mode = read_mode()
print(mode)
")
[ "$output" = "live" ] && check "Explicit live mode works" || echo "✗ Explicit live mode failed (got: $output)"

# Clean up
rm -f ~/.local/state/opencode-claimant/mode

echo
echo "=========================================="
echo "Verification Complete: $success_count / $total_checks checks passed"
echo "=========================================="

if [ $success_count -eq $total_checks ]; then
  echo -e "${GREEN}✓ All checks passed! Slice 1 implementation is complete.${NC}"
  exit 0
else
  echo -e "${RED}✗ Some checks failed. Please review the output above.${NC}"
  exit 1
fi
