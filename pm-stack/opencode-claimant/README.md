# OpenCode Claimant/Executor - Slice 1 Implementation

This directory contains the slice-1 implementation for the OpenCode execution lane claimant/executor as specified in #3026.

## Overview

The claimant/executor is responsible for:
1. **Supervision**: Checking bound sessions for recovery candidates (stuck/zero-message sessions)
2. **Candidate Discovery**: Finding unbound opencode-lane tasks across four queues
3. **Session Management**: Creating and binding worker sessions with atomic operations
4. **Prompt Delivery**: Sending execution prompts to bound sessions
5. **Logging**: Recording fire execution results

## Files

### claimant.py

Main execution script that implements the 6-step process:

1. **Supervision** (Step 1): Checks bound sessions for recovery candidates
   - Identifies zero-message sessions that never received prompts
   - Performs atomic recovery via `bind` CLI with special `launched_by` values
   - Only runs in live mode

2. **Gate** (Step 2): Checks for unbound opencode-lane candidates
   - Runs `poll.py` to discover tasks
   - Filters for tasks with `lane: "opencode"` and no `session_id`

3. **Cap** (Step 3): Checks concurrency limits
   - Queries OpenCode server for active root sessions
   - Uses 36-session cap (raised from 12→20→36 on 2026-11-05)
   - Returns cap_room (available slots) or -1 if unknown

4. **Launch** (Step 4): Creates sessions and binds tasks
   - Creates worker sessions via OpenCode server API
   - Verifies session location matches requested directory
   - Atomically binds session to task using `repo-queue bind`
   - Sends execution prompt to bound session
   - Respects queue priority (order in QUEUES list)

5. **Terminal States** (Step 5): Handled by worker sessions

6. **Logging** (Step 6): Records fire execution in `~/.local/state/opencode-claimant/fires.jsonl`

### poll.py

Candidate discovery and system state reporting:

- **get_candidates()**: Discovers unbound opencode-lane tasks from all four queues
- **get_stuck_bound()**: Identifies bound tasks with stuck sessions (placeholder implementation)
- **get_active_root_session_count()**: Queries OpenCode server for active session count
- Outputs JSON with candidates, stuck_bound list, and cap_room

### Makefile

Helper for setup:
```bash
make all      # Make scripts executable
make install  # Create state directory
```

## Mode

The system operates in two modes, controlled by `~/.local/state/opencode-claimant/mode`:

- **shadow** (default): Discovery and read-only supervision only. No mutations.
- **live**: Full execution including session creation, binding, and prompt delivery.

If the mode file doesn't exist or is unreadable, defaults to `shadow`.

## Queues

The claimant monitors four queues:
- `repo-health-code-pilot`
- `repo-health-opencode-pilot`
- `repo-health-openchamber-pilot`
- `repo-health-nix-pilot`

## Concurrency Cap

- **36 active root sessions** across the fleet
- Monitored via OpenCode server API (`GET /api/session`)
- Only root sessions without terminal markers count toward cap
- Cap room = 36 - active_count

## Recovery Rules (#4758, Phase 3 of #4912)

### Zero-Message Recovery
For bound sessions with:
- No messages in session
- No activity markers
- Silent for >30 minutes (ACTIVE_WINDOW_MS)
- Attempts remaining (`attempts < max_attempts`)

Process:
1. Verify zero messages via `GET /api/session/{id}/messages`
2. Delete session via `DELETE /api/session/{id}`
3. Atomically reclaim task via `bind` with `--launched-by opencode-claimant-recovery:zero-message-deleted`
4. Task becomes unbound and eligible for next fire

### Gone-Session Recovery
For bound sessions absent from server with attempts remaining:
1. Verify absence via `GET /api/session/{id}`
2. Atomically reclaim task via `bind` with `--launched-by opencode-claimant-recovery:gone-session`

### Exhausted Attempts
When attempts exhausted:
1. For zero-message sessions: delete then fail task terminally
2. For gone sessions: fail task terminally
3. Use `repo-queue fail` with execution record containing session_id

## Hard Rules

- **Never work tasks directly**: Only claim/bind/launch/record
- **No credentials in prompts/logs**: Use broker client tokens
- **No cap violations**: Never exceed 36 sessions
- **No double-binding**: Never bind a session you didn't create
- **Atomic operations**: Bind and recovery use compare-and-clear guards
- **Session location verification**: Verify `location.directory` before binding
- **Task state verification**: Re-verify task state before launch

## Execution Flow

```
1. Read mode (shadow/live)
2. Supervision step (live only)
3. Gate step (check for candidates)
4. Cap step (check concurrency)
5. Launch step (create sessions, bind, send prompts)
6. Log fire results
```

## Usage

```bash
# Run in shadow mode (default - no mutations)
python3 pm-stack/opencode-claimant/claimant.py

# Run in live mode (requires mode file set to "live")
echo "live" > ~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py

# Test poll.py output
python3 pm-stack/opencode-claimant/poll.py
```

## Fire Log Format

Logs written to `~/.local/state/opencode-claimant/fires.jsonl`:

```json
{
  "timestamp": "2026-11-06T23:35:00Z",
  "mode": "shadow",
  "candidates_seen": [...],
  "sessions_launched": [...],
  "cap_room": 36,
  "skips": [...]
}
```

## Constants

- `CONCURRENCY_CAP = 36`
- `LAUNCHED_BY_RECOVERY_ZERO_MESSAGE = "opencode-claimant-recovery:zero-message-deleted"`
- `LAUNCHED_BY_RECOVERY_GONE_SESSION = "opencode-claimant-recovery:gone-session"`

## Dependencies

- Python 3.8+
- `requests` library (for OpenCode server API calls)
- `repo-queue` CLI at `~/fleet-brokers/bin/repo-queue`
- OpenCode server running at `http://127.0.0.1:13420`

## Testing

The implementation follows the deterministic specification in `claimant.py`. To test:

1. Ensure mode is set to `shadow` for safe testing
2. Run `python3 pm-stack/opencode-claimant/claimant.py`
3. Review fire log output in `~/.local/state/opencode-claimant/fires.jsonl`
4. Verify no mutations occurred (shadow mode)

## Known Limitations

- `get_stuck_bound()` in poll.py is a placeholder - full implementation requires OpenCode server API access
- Session prompt delivery in `launch_step()` is documented but not fully implemented (would use session API)
- Error handling assumes server API is available and responsive

## Future Slices

Slice 2 (#3026): Parallel-run gate implementation
Slice 3+: Additional features and optimizations

## References

- Issue: #3026 - OpenCode lane claimant/executor implementation
- Design: #4912 - Recovery rules and supervision
- Defect: #4758 - Zero-message session recovery
- Defect: #4870 - Session location verification
- Defect: #5857 - Session creation request shape
- Migration: 0021 - bind_repo_task_lane_session
- Migration: 0022 - Atomic recovery operations
