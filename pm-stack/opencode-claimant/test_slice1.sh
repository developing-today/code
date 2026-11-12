#!/bin/bash
# Simple test to verify slice 1 implementation works correctly

set -e

echo "=== Testing OpenCode Lane Claimant/Executor - Slice 1 ==="
echo

# Test 1: Run in shadow mode (should work without errors)
echo "Test 1: Running in shadow mode..."
python3 pm-stack/opencode-claimant/claimant.py
if [ $? -eq 0 ]; then
  echo "✓ Shadow mode execution successful"
else
  echo "✗ Shadow mode execution failed"
  exit 1
fi
echo

# Test 2: Verify fire log was created
echo "Test 2: Verifying fire log creation..."
if [ -f ~/.local/state/opencode-claimant/fires.jsonl ]; then
  echo "✓ Fire log created at ~/.local/state/opencode-claimant/fires.jsonl"

  # Show last fire entry
  echo "Last fire entry:"
  tail -1 ~/.local/state/opencode-claimant/fires.jsonl | python3 -m json.tool || echo "(JSON formatting failed, but file exists)"
else
  echo "✗ Fire log not created"
  exit 1
fi
echo

# Test 3: Test poll.py output
echo "Test 3: Testing poll.py output..."
python3 pm-stack/opencode-claimant/poll.py >/tmp/poll_output.json 2>/dev/null
if [ $? -eq 0 ]; then
  echo "✓ poll.py executed successfully"

  # Check for required fields (poll.py outputs to stdout, warnings to stderr)
  python3 <<'EOF'
import json
with open('/tmp/poll_output.json') as f:
    data = json.load(f)
    assert 'queues' in data, 'Missing queues field'
    assert 'candidates' in data, 'Missing candidates field'
    assert 'stuck_bound' in data, 'Missing stuck_bound field'
    assert 'cap_room' in data, 'Missing cap_room field'
    assert 'live_root_sessions' in data, 'Missing live_root_sessions field'
    assert 'active_root_sessions' in data, 'Missing active_root_sessions field'
    print('✓ poll.py output contains all required fields')
EOF
else
  echo "✗ poll.py execution failed"
  exit 1
fi
echo

# Test 4: Test mode switching
echo "Test 4: Testing mode switching..."

# Test shadow mode
echo "shadow" >~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py >/dev/null 2>&1
if [ $? -eq 0 ]; then
  echo "✓ Explicit shadow mode works"
else
  echo "✗ Explicit shadow mode failed"
  exit 1
fi

# Test live mode
echo "live" >~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py >/dev/null 2>&1
if [ $? -eq 0 ]; then
  echo "✓ Explicit live mode works"
else
  echo "✗ Explicit live mode failed"
  exit 1
fi

# Clean up
rm -f ~/.local/state/opencode-claimant/mode

echo

# Test 5: Verify constants
echo "Test 5: Verifying constants..."
if grep -q "CONCURRENCY_CAP = 36" pm-stack/opencode-claimant/claimant.py; then
  echo "✓ CONCURRENCY_CAP = 36"
else
  echo "✗ CONCURRENCY_CAP not set correctly"
  exit 1
fi

if grep -q "LAUNCHED_BY_RECOVERY_ZERO_MESSAGE" pm-stack/opencode-claimant/claimant.py; then
  echo "✓ Zero-message recovery constant defined"
else
  echo "✗ Zero-message recovery constant missing"
  exit 1
fi

if grep -q "LAUNCHED_BY_RECOVERY_GONE_SESSION" pm-stack/opencode-claimant/claimant.py; then
  echo "✓ Gone-session recovery constant defined"
else
  echo "✗ Gone-session recovery constant missing"
  exit 1
fi

echo
echo "=== All Tests Passed! ==="
echo "OpenCode lane claimant/executor slice 1 is working correctly."
