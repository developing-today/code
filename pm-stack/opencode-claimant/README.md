# OpenCode Lane Claimant/Executor - Slice 1

## Overview

This directory contains the implementation for slice 1 of OpenCode lane claimant/executor (#3026).

## Scope (Slice 1)

Slice 1 implements the **read-only discovery and reporting** phase for the opencode execution lane:

1. ✅ **Supervision** - Reports stuck_bound tasks (read-only in shadow mode)
2. ✅ **Candidate Discovery** - Discovers unbound opencode-lane tasks from the four queues
3. ❌ **Session Creation/Binding** - NOT implemented in slice 1 (would be live mode only)
4. ❌ **Prompt Delivery** - NOT implemented in slice 1 (would be live mode only)
5. ✅ **Fire Logging** - Always logs execution results to `~/.local/state/opencode-claimant/fires.jsonl`

## Execution Modes

### Shadow Mode (Default)

When `~/.local/state/opencode-claimant/mode` contains `shadow` (or is missing/readable):
- Discovers candidates via `poll.py`
- Reports what **would** be launched without mutating any state
- Does NOT create sessions, bind tasks, or send prompts
- Always logs fire results

### Live Mode

When mode file contains `live`:
- Performs supervision checks on bound sessions
- Discovers candidates via `poll.py`
- Creates sessions and binds tasks atomically
- Sends prompts to worker sessions
- Logs fire results

**Note:** Full live mode implementation is planned for future slices.

## Files

### claimant.py

Main entry point for the claimant/executor. Implements the 6-step process:

1. **Supervision** - Checks bound sessions for recovery candidates
2. **Gate** - Verifies unbound opencode-lane candidates exist
3. **Cap** - Checks concurrency limit (36 active root sessions)
4. **Launch** - Simulates or executes session creation and binding
5. **Terminal States** - Handled by worker sessions
6. **Fire Logging** - Records execution results

### poll.py

Read-only candidate discovery (provided, not modified in slice 1):
- Discovers unbound opencode-lane tasks from 4 queues
- Identifies stuck_bound tasks for recovery
- Calculates concurrency cap room
- Returns structured JSON output

### test_shadow_simple.py

Test script that verifies shadow mode execution with simulated poll.py output.

## Constants

- **Queues**: `repo-health-code-pilot`, `repo-health-opencode-pilot`, `repo-health-openchamber-pilot`, `repo-health-nix-pilot`
- **Concurrency Cap**: 36 active root sessions
- **Recovery launched_by values**:
  - `opencode-claimant-recovery:zero-message-deleted`
  - `opencode-claimant-recovery:gone-session`

## Usage

### Run in shadow mode (default)

```bash
python3 pm-stack/opencode-claimant/claimant.py
```

### Run in live mode

```bash
echo "live" > ~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py
```

### Run tests

```bash
python3 pm-stack/opencode-claimant/test_shadow_simple.py
```

## Fire Log Format

Each execution creates a JSON line in `~/.local/state/opencode-claimant/fires.jsonl`:

```json
{
  "timestamp": "2026-11-07T22:35:33.713683+00:00",
  "mode": "shadow",
  "candidates_seen": [...],
  "sessions_launched": [...],
  "cap_room": 35,
  "skips": [...]
}
```

## Implementation Notes

### Shadow Mode Behavior

In shadow mode, the claimant:
1. Reads mode from file (defaults to "shadow")
2. Runs supervision step (reports stuck_bound tasks)
3. Runs gate step (checks for candidates)
4. Runs cap step (checks concurrency limit)
5. Runs launch step (simulates launches with fake session IDs)
6. Logs fire results

### Live Mode Behavior (Future Slices)

In live mode, the claimant will:
1. Run supervision step (verifies and recovers stuck sessions)
2. Run gate step (checks for candidates)
3. Run cap step (checks concurrency limit)
4. Run launch step (creates real sessions and binds tasks atomically)
5. Send prompts to bound sessions
6. Log fire results

### Atomic Operations

The implementation ensures:
- Never double-bind a task
- Never bind a session ID that wasn't just created
- Never exceed concurrency cap
- Session creation is verified before binding
- Bind operations are atomic (via repo-queue CLI)

### Error Handling

- Unknown cap room → fail closed (launch nothing)
- Session creation failure → leave task unbound for next fire
- Bind failure → delete session and leave task unbound
- Prompt delivery failure → reconcile bound session before retrying

## Future Work (Beyond Slice 1)

1. **Live mode supervision recovery**: Delete zero-message sessions and reclaim tasks
2. **Session creation**: POST /api/session with verified location
3. **Atomic binding**: repo-queue bind with session ID
4. **Prompt delivery**: Send task-specific instructions to worker
5. **Agent notes**: Post handoff notes to issues
6. **Conductor integration**: Full live mode execution

## Testing

The test script `test_shadow_simple.py` demonstrates:
- Mode detection (shadow mode)
- Gate step (finds candidates)
- Cap step (calculates available slots)
- Launch step (simulates session creation)
- Fire logging (creates log entry)

## References

- Issue: #3026 - OpenCode lane claimant/executor
- Design: doc/fleet-schedule-registry.md
- Migration: 0021 (bind_repo_task_lane_session), 0022 (atomic recovery)
- Hard rules: Never work tasks yourself, never double-launch, never exceed cap
