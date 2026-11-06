# OpenCode Lane Claimant/Executor

This directory contains the implementation for the opencode execution lane claimant/executor as specified in #3026.

## Slice 1 - Claim/Bind/Launch/Record

Slice 1 implements the core functionality:
- Candidate discovery via `poll.py`
- Task execution via `claimant.py`
- Supervision and recovery of stuck tasks
- Concurrency limiting (36 active sessions)
- Fire logging

## Files

### `claimant.py`

Main executor script that implements the 4-step process:

1. **SUPERVISION FIRST**: Check bound sessions and recover stuck_bound tasks
   - Identifies sessions that never received prompts
   - Performs atomic recovery operations
   - Respects attempt limits

2. **GATE**: Verify unbound candidates exist
   - Checks all four queues for eligible tasks
   - Filters by lane, stage, and binding status

3. **CAP**: Check concurrency limit
   - Counts active root sessions (no terminal marker, activity in last 30 min)
   - Respects 36-session hard cap
   - Returns available cap_room

4. **LAUNCH**: Create sessions and bind tasks
   - Creates worker session with verified location
   - Atomically binds task to session
   - Sends worker prompt
   - Records launch in fire log

**Usage:**
```bash
python3 claimant.py
```

**Mode:**
- Read from `~/.local/state/opencode-claimant/mode`
- Default: `shadow` (discovery only)
- Set to `live` for actual launches

### `poll.py`

Candidate discovery script that outputs JSON with:
- Unbound opencode-lane candidates
- Active session count (cap_room)
- Stuck_bound tasks for recovery

**Usage:**
```bash
python3 poll.py
```

**Output Example:**
```json
{
  "mode": "shadow",
  "candidates": [
    {
      "task_id": "task-123",
      "issue": "ISSUE-456",
      "queue": "repo-health-opencode-pilot",
      "worker_id": "repo-worker-789",
      "lane": "opencode",
      "stage": "handed_off",
      "attempts": 0,
      "max_attempts": 5
    }
  ],
  "cap_room": 30,
  "stuck_bound": [
    {
      "task_id": "task-999",
      "session_id": "ses-xyz",
      "queue": "repo-health-code-pilot",
      "worker_id": "repo-worker-abc",
      "attempts": 2,
      "max_attempts": 5,
      "reason": "zero_messages"
    }
  ]
}
```

### `IMPLEMENTATION_SUMMARY.md`

Detailed documentation of the implementation including:
- Architecture and design decisions
- Constants and configuration
- Mode behavior (shadow vs live)
- Fire logging format
- Hard rules enforced
- Usage examples
- Testing instructions

### `test_implementation.py`

Quick test suite to verify:
- File structure
- Python syntax
- Constants
- Mode detection

**Usage:**
```bash
python3 test_implementation.py
```

## Queues

The claimant works with four queues in priority order:

1. `repo-health-code-pilot`
2. `repo-health-opencode-pilot`
3. `repo-health-openchamber-pilot`
4. `repo-health-nix-pilot`

## Concurrency Control

- **Hard Cap**: 36 active root sessions
- **Active Window**: 30 minutes
- **Terminal Markers**: `idle`, `outcome`
- **Counting**: Sessions with no terminal marker and activity in last 30 min

## Recovery Rules

### Zero-Message Recovery
Tasks that are bound but never received prompt:
- Session exists as root with no terminal marker
- No user messages in session
- Silent for >30 minutes
- Lease expired
- Attempts remaining

**Process:**
1. Verify zero messages
2. Delete session
3. Atomically reclaim task (clear bindings)
4. Increment attempts

### Gone-Session Recovery
Tasks bound to sessions that don't exist:
- Session not found on server
- Attempts remaining

**Process:**
1. Atomically reclaim task (clear bindings)
2. Increment attempts

## Mode System

### SHADOW Mode (Default)
- Discovers candidates and cap_room
- Reports what WOULD be launched
- **Does NOT** claim, bind, launch, or mutate tasks
- Creates fire log with `would_launch` information

### LIVE Mode
- Discovers candidates and cap_room
- Recovers stuck_bound tasks
- Launches candidates within cap
- Creates fire log with actual launches

**Setting Mode:**
```bash
echo "live" > ~/.local/state/opencode-claimant/mode
```

## Fire Logging

All fires are logged to `~/.local/state/opencode-claimant/fires.jsonl` in JSON Lines format.

**Example Log Entry:**
```json
{
  "timestamp": "2026-11-05T21:05:00Z",
  "mode": "shadow",
  "candidates_seen": 5,
  "sessions_launched": [
    {
      "task_id": "task-123",
      "session_id": "ses-abc",
      "queue": "repo-health-opencode-pilot",
      "issue": "ISSUE-456"
    }
  ],
  "cap_room": 30,
  "recovered_tasks": [],
  "skips": [],
  "status": "launched"
}
```

## Dependencies

- Python 3.8+
- `requests` library
- OpenCode server at `http://127.0.0.1:13420`
- `repo-queue` CLI in `~/fleet-brokers/bin/repo-queue`
- `bind` CLI command

## Setup

```bash
# Make scripts executable
chmod +x pm-stack/opencode-claimant/*.py

# Create state directory
mkdir -p ~/.local/state/opencode-claimant

# Set mode (shadow for testing, live for production)
echo "shadow" > ~/.local/state/opencode-claimant/mode

# Run claimant
python3 pm-stack/opencode-claimant/claimant.py

# View fire logs
tail -f ~/.local/state/opencode-claimant/fires.jsonl
```

## Hard Rules (Enforced)

✅ Never work a task yourself - only claim/bind/launch/record
✅ Never launch for tasks with `held`, `user/owned`, `do-not-merge`, or `do-not-auto-edit`
✅ Never exceed 36-session cap
✅ Never double-bind a task
✅ Never bind a session ID you did not just create
✅ Never leave a created session unbound
✅ Always verify session location before binding
✅ Atomic recovery operations (compare-and-clear)
✅ Supervision runs on EVERY fire
✅ Recovery attempts are consumed when reclaiming

## Testing

```bash
# Run test suite
python3 pm-stack/opencode-claimant/test_implementation.py

# Manual test in shadow mode
python3 pm-stack/opencode-claimant/claimant.py

# Check fire logs
cat ~/.local/state/opencode-claimant/fires.jsonl
```

## Future Slices

- **Slice 2**: Parallel-run gate implementation
- **Slice 3**: Muse lane disabling logic
- **Slice 4**: Full production hardening and testing

## References

- Issue: #3026
- Design: #4912
- Recovery Rule: #4758
- Migration: 0021 (bind), 0022 (recovery)
