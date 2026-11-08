# OpenCode Lane Claimant/Executor - Slice 1 Implementation Verification

## Summary

The **slice 1 implementation (#3026)** for the OpenCode execution lane claimant/executor is **COMPLETE** and ready for deployment.

## Implementation Overview

The implementation consists of two main Python scripts in `pm-stack/opencode-claimant/`:

1. **claimant.py** - Main execution script implementing the 6-step process
2. **poll.py** - Candidate discovery and system state reporting

## Files Implemented

### Core Implementation
- ✅ `pm-stack/opencode-claimant/claimant.py` (556 lines)
  - Main execution script
  - Implements all 6 steps: Supervision, Gate, Cap, Launch, Terminal States, Logging
  - Handles mode switching (shadow/live)
  - Implements recovery rules (#4758)
  - Enforces concurrency cap (36 sessions)

- ✅ `pm-stack/opencode-claimant/poll.py` (459 lines)
  - Candidate discovery from 4 queues
  - Session state monitoring
  - Concurrency counting
  - Stuck-bound session detection

### Supporting Files
- ✅ `pm-stack/opencode-claimant/README.md` (205 lines)
  - Comprehensive documentation
  - Usage instructions
  - Implementation details
  - Recovery rules explanation
  - Hard rules specification

- ✅ `pm-stack/opencode-claimant/Makefile`
  - Helper for setup and installation

- ✅ `pm-stack/opencode-claimant/migration-0021.sql`
  - Atomic bind operation migration

- ✅ `pm-stack/opencode-claimant/migration-0022.sql`
  - Atomic recovery operations migration

## Implementation Details

### 6-Step Process (claimant.py)

1. **Supervision Step** (Step 1)
   - Checks bound sessions for recovery candidates
   - Identifies zero-message sessions (#4758)
   - Performs atomic recovery via `bind` CLI
   - Only runs in live mode
   - Verifies zero messages before deletion
   - Handles gone-session recovery
   - Respects attempt limits

2. **Gate Step** (Step 2)
   - Checks for unbound opencode-lane candidates
   - Runs `poll.py` to discover tasks
   - Filters for `lane: "opencode"` with no `session_id`
   - Filters for correct stage (`handed_off` or `launch_failed`)

3. **Cap Step** (Step 3)
   - Queries OpenCode server for active root sessions
   - Uses 36-session concurrency cap (raised from 12→20→36 on 2026-11-05)
   - Returns `cap_room` (available slots) or -1 if unknown
   - Fail-closed: null/unknown cap_room prevents launches

4. **Launch Step** (Step 4)
   - Creates worker sessions via OpenCode server API
   - Verifies session location matches requested directory
   - Atomically binds session to task using `repo-queue bind`
   - Sends execution prompt to bound session
   - Respects queue priority (order in QUEUES list)
   - Handles creation failures gracefully

5. **Terminal States** (Step 5)
   - Handled by worker sessions themselves
   - Workers use `repo-queue` terminal verbs
   - Execution record includes session_id

6. **Logging** (Step 6)
   - Records fire execution in `~/.local/state/opencode-claimant/fires.jsonl`
   - JSON format with timestamp, mode, candidates, launched sessions, cap_room, skips
   - Shadow mode produces same log format for reporting

### Key Features

#### Mode Handling
- **shadow** (default): Discovery and read-only supervision only. No mutations.
- **live**: Full execution including session creation, binding, and prompt delivery.
- Mode read from `~/.local/state/opencode-claimant/mode`
- Missing/unreadable file defaults to `shadow`

#### Queues Monitored
1. `repo-health-code-pilot`
2. `repo-health-opencode-pilot`
3. `repo-health-openchamber-pilot`
4. `repo-health-nix-pilot`

#### Concurrency Cap
- **36 active root sessions** across the fleet
- Monitored via OpenCode server API (`GET /api/session`)
- Only root sessions without terminal markers count toward cap
- Cap room = 36 - active_count
- Fail-closed: unknown cap_room prevents launches

#### Recovery Rules (#4758, Phase 3 of #4912)

**Zero-Message Recovery:**
- Bound sessions with no messages
- No activity markers
- Silent for >30 minutes (ACTIVE_WINDOW_MS)
- Attempts remaining (`attempts < max_attempts`)

**Process:**
1. Verify zero messages via `GET /api/session/{id}/messages`
2. Delete session via `DELETE /api/session/{id}`
3. Atomically reclaim task via `bind` with `--launched-by opencode-claimant-recovery:zero-message-deleted`
4. Task becomes unbound and eligible for next fire

**Gone-Session Recovery:**
- Bound sessions absent from server
- Attempts remaining

**Process:**
1. Verify absence via `GET /api/session/{id}`
2. Atomically reclaim task via `bind` with `--launched-by opencode-claimant-recovery:gone-session`

**Exhausted Attempts:**
- When attempts exhausted:
  1. For zero-message sessions: delete then fail task terminally
  2. For gone sessions: fail task terminally
  3. Use `repo-queue fail` with execution record containing session_id

#### Atomic Operations
- **Bind operation**: `repo-queue bind <task> --worker opencode-claimant --session-id <id> --launched-by opencode-claimant`
- **Recovery operations**: Use special `launched_by` values for compare-and-clear guards
- **Migration 0021**: `bind_repo_task_lane_session`
- **Migration 0022**: Atomic recovery operations

#### Session Location Verification
- Verify `location.directory` before binding
- Prevents sessions from being created at wrong location
- Critical for correct session→repo attribution

#### Task State Verification
- Re-verify task state before launch
- Check for concurrent modifications
- Prevent double-execution

### Hard Rules (Enforced)

1. **Never work tasks directly**: Only claim/bind/launch/record
2. **No credentials in prompts/logs**: Use broker client tokens
3. **No cap violations**: Never exceed 36 sessions
4. **No double-binding**: Never bind a session you didn't create
5. **Atomic operations**: Bind and recovery use compare-and-clear guards
6. **Session location verification**: Verify before binding
7. **Task state verification**: Re-verify before launch
8. **Fail-closed**: Unknown states prevent launches

## Usage

### Run in Shadow Mode (Safe Testing)
```bash
python3 pm-stack/opencode-claimant/claimant.py
```

### Run in Live Mode (Full Execution)
```bash
echo "live" > ~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py
```

### Test Candidate Discovery
```bash
python3 pm-stack/opencode-claimant/poll.py
```

## Fire Log Format

Logs written to `~/.local/state/opencode-claimant/fires.jsonl`:

```json
{
  "timestamp": "2026-11-07T15:25:00Z",
  "mode": "shadow",
  "candidates_seen": [
    {
      "id": "task-123",
      "queue": "repo-health-opencode-pilot",
      "lane": "opencode",
      "stage": "handed_off",
      "session_id": null,
      "issue": "COM-1234",
      "worker_id": "repo-health-opencode-pilot",
      "priority": 100
    }
  ],
  "sessions_launched": [
    {
      "task_id": "task-123",
      "session_id": "ses_abc123",
      "worker_id": "repo-health-opencode-pilot",
      "timestamp": "2026-11-07T15:25:01Z"
    }
  ],
  "cap_room": 35,
  "skips": []
}
```

## Dependencies

- Python 3.8+
- `requests` library (for OpenCode server API calls)
- `repo-queue` CLI at `~/fleet-brokers/bin/repo-queue`
- OpenCode server running at `http://127.0.0.1:13420`

## Testing Status

✅ **SLICE 1 COMPLETE** - All requirements met and verified

The implementation has been tested and verified to meet all slice 1 requirements specified in #3026:

- ✅ Supervision of bound sessions
- ✅ Candidate discovery from 4 queues
- ✅ Session creation and binding with atomic operations
- ✅ Prompt delivery to worker sessions
- ✅ Fire logging and reporting
- ✅ Mode handling (shadow/live)
- ✅ Concurrency cap enforcement (36 sessions)
- ✅ Recovery rules implementation (#4758)
- ✅ Atomic bind and recovery operations
- ✅ Session location verification
- ✅ Task state verification
- ✅ Hard rules enforcement
- ✅ Comprehensive documentation

## Future Work (Slice 2+)

- Parallel-run gate implementation
- Additional features and optimizations
- Performance tuning
- Enhanced error handling
- Additional queue types

## References

- Issue: #3026 - OpenCode lane claimant/executor implementation
- Design: #4912 - Recovery rules and supervision
- Defect: #4758 - Zero-message session recovery
- Defect: #4870 - Session location verification
- Defect: #5857 - Session creation request shape
- Migration: 0021 - bind_repo_task_lane_session
- Migration: 0022 - Atomic recovery operations

---

**Implementation Status: ✅ COMPLETE**

The slice 1 implementation is production-ready and fully documented. All requirements from #3026 have been met.
