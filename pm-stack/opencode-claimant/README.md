# OpenCode Lane Claimant/Executor

**Slice 1 of #3026** - Implementation for OpenCode execution lane task management

This directory contains the claimant/executor implementation that discovers, supervises, claims, binds, launches, and records tasks assigned to the OpenCode execution lane.

## Overview

The OpenCode lane claimant/executor is a laptop-side worker that:

1. **Discovers** tasks assigned to the OpenCode lane that are ready for launch
2. **Supervises** bound sessions to recover stuck or failed launches
3. **Claims and binds** tasks to worker sessions atomically
4. **Launches** worker sessions with the appropriate prompt and configuration
5. **Records** all actions in fire logs for audit and debugging

## Architecture

### Components

- **`claimant.py`** - Main executor that runs the claim/bind/launch workflow
- **`poll.py`** - Discovery script that finds candidates and reports system state
- **State directory** - `~/.local/state/opencode-claimant/` - Stores logs, locks, and state
- **Queue CLI** - `~/fleet-brokers/bin/repo-queue` - Interfaces with the queue broker
- **OpenCode server** - `http://127.0.0.1:13420` - Manages worker sessions

### Workflow

```
┌─────────────────────────────────────────────────────────────┐
│                    claimant.py (main)                       │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                      poll.py (discovery)                    │
│  - Lists all lane=opencode tasks                           │
│  - Filters candidates (unbound, handed_off/launch_failed)   │
│  - Counts active root sessions (cap_room)                  │
│  - Identifies stuck_bound tasks for recovery               │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   Supervision Phase (live mode)             │
│  - Checks all bound sessions still exist                    │
│  - Recovers stuck_bound sessions (zero-message or gone)     │
│  - Handles attempt exhaustion                               │
└─────────────────────────────────────────────────────────────┘
                              │
                              ▼
┌─────────────────────────────────────────────────────────────┐
│                   Launch Phase (live mode)                   │
│  - Creates worker session via OpenCode API                  │
│  - Verifies session location matches PRIMARY                │
│  - Atomically binds task to session (migration 0021)        │
│  - Sends launch prompt to worker session                    │
│  - Posts agent-note to issue                                │
└─────────────────────────────────────────────────────────────┘
```

## Modes

### Shadow Mode (Default)

- **File**: `~/.local/state/opencode-claimant/mode` contains `shadow`
- **Behavior**: Discovery and supervision only - no launches or mutations
- **Use case**: Safe testing, validation, dry-runs
- **Output**: Fire logs show what would be launched

### Live Mode

- **File**: Mode file contains `live`
- **Behavior**: Full execution including launches and bindings
- **Use case**: Actual task processing
- **Safety**: Atomic operations prevent double-launches

## Key Features

### 1. Concurrency Control

- **Cap**: 36 active root sessions maximum
- **Mechanism**: `cap_room` from poll.py (counts active sessions via OpenCode API)
- **Enforcement**: Loop exits when cap is reached
- **Safety**: File locking prevents multiple claimants

### 2. Atomic Operations

- **Bind**: `queue bind <id> --session-id <sid>` (migration 0021)
  - Atomically writes to lane ledger AND progress.session_id
  - Prevents race conditions
  
- **Recovery**: `queue bind <id> --launched-by opencode-claimant-recovery:<reason>`
  - Clears both bindings atomically
  - Consumes one attempt

### 3. Session Verification

- **Location check**: Created session's `location.directory` must match PRIMARY
- **Cleanup**: On failure, session is deleted before binding
- **Prevention**: Prevents sessions from running in wrong project

### 4. Recovery Rules (#4758, Phase 3 of #4912)

Two recovery scenarios:

**A. Zero-message session** (never received prompt):
1. Verify session has zero messages
2. Delete session via API
3. Atomic recovery with `launched-by: opencode-claimant-recovery:zero-message-deleted`
4. Task becomes candidate for next fire

**B. Gone session** (absent from server):
1. Verify session absent
2. Atomic recovery with `launched-by: opencode-claimant-recovery:gone-session`
3. Task becomes candidate for next fire

**Attempt exhaustion**:
- After `max_attempts` exhausted, terminally fail task
- Execution record preserves `session_id` for watchdog

### 5. Route Handling

- **Default**: `providerID: "opencode-route"`, `modelID: "workhorse"`, `agent: "build"`
- **Custom**: Task packet can specify custom route
- **Fallback**: Uses default if packet route missing/invalid

### 6. Hard Rules (Enforced)

- Never work a task yourself (only claim/bind/launch/record)
- Never exceed 36-session cap
- Never double-bind a task
- Never bind a session without verification
- Never launch on issues with `held`, `user/owned`, `do-not-merge`, or `do-not-auto-edit` labels

## Files

### claimant.py (464 lines)

Main executor with these key functions:

- `main()` - Entry point with lock acquisition
- `read_mode()` - Reads mode from file (shadow/live)
- `discover(task_id)` - Calls poll.py to get candidate list
- `run_supervision()` - Supervises bound sessions and recovers stuck ones
- `launch()` - Creates session, binds, sends prompt, posts note
- `create_session()` - Creates worker session via OpenCode API
- `send_prompt()` - Sends launch prompt to worker
- `add_issue_note()` - Posts agent-note to issue
- `atomic_recover()` - Recovers stuck sessions atomically
- `terminal_fail()` - Terminally fails task with attempt exhaustion
- `append_log()` - Writes fire log entry

### poll.py (168 lines)

Discovery script that outputs JSON with:

- `candidates` - Tasks ready for launch (unbound, handed_off/launch_failed)
- `tasks` - All lane=opencode tasks (for supervision)
- `stuck_bound` - Bound tasks needing recovery (lease expired, no progress)
- `cap_room` - Active root session count (None if server unreadable)
- `active_roots` - Alias for cap_room

### State Files

- `~/.local/state/opencode-claimant/mode` - Current mode (shadow/live)
- `~/.local/state/opencode-claimant/fires.jsonl` - Fire log (append-only)
- `~/.local/state/opencode-claimant/claimant.lock` - Single-instance lock
- `~/.local/state/opencode-claimant/*.json` - Previous fire outputs

## Execution

### Running Manually

```bash
# From primary clone
cd ~/git/code

# Run in shadow mode (default)
python3 pm-stack/opencode-claimant/claimant.py

# Run in live mode (temporarily)
echo "live" > ~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py

# Restore shadow mode
echo "shadow" > ~/.local/state/opencode-claimant/mode
```

### Running via cron/OpenChamber

The OpenChamber scheduler will call:
```bash
python3 /home/user/git/code/pm-stack/opencode-claimant/claimant.py
```

### Validation

```bash
# Run validation script
python3 pm-stack/opencode-claimant/validate_simple.py
```

## Fire Log Format

Each fire appends one JSON line to `fires.jsonl`:

```json
{
  "time": "2026-11-06T17:00:01.000Z",
  "mode": "shadow",
  "candidates_seen": ["task-123", "task-456"],
  "sessions_launched": [
    {
      "task_id": "task-123",
      "session_id": "ses_abc123",
      "status": "launched",
      "route": {"providerID": "opencode-route", "modelID": "workhorse", "agent": "build"}
    }
  ],
  "recoveries": [
    {"task_id": "task-789", "session_id": "ses_def456", "action": "deleted_and_reclaimed"}
  ],
  "cap_count": 0,
  "cap_room": 0,
  "skips": [
    {"task_id": "task-999", "reason": "issue hold marker: held"}
  ]
}
```

## Constants

```python
PRIMARY = Path.home() / "git" / "code"  # Primary clone directory
QUEUES = (  # All four queues to check health
    "repo-health-code-pilot",
    "repo-health-opencode-pilot",
    "repo-health-openchamber-pilot",
    "repo-health-nix-pilot",
)
DEFAULT_ROUTE = {
    "providerID": "opencode-route",
    "modelID": "workhorse",
    "agent": "build"
}
SERVER = "http://127.0.0.1:13420"  # OpenCode server
QUEUE_CLI = Path.home() / "fleet-brokers" / "bin" / "repo-queue"
```

## Queue Commands Used

The claimant uses these queue CLI commands:

- `queue health --queue <name>` - Check queue health
- `queue list --lane opencode` - List lane tasks (via poll.py)
- `queue bind <id> --session-id <sid>` - Atomic bind (migration 0021)
- `queue bind <id> --session-id <sid> --launched-by <reason>` - Atomic recovery
- `queue checkpoint <id> --worker <worker> --stage launched` - Mark as launched
- `queue note <id> --issue <issue> --note "agent-note:v1 state=handed off task=<tid> session=<sid>"` - Post agent note
- `queue fail <id> --worker <worker> --error "..." --reason-class deterministic --execution "{...}"` - Terminal failure

## Error Handling

- **ClaimantError**: Custom exception for claimant-specific errors
- **Exception handling**: All exceptions caught and logged in skips
- **Atomic guarantees**: Operations either fully succeed or leave system unchanged
- **Validation**: Session location verified before binding

## Supervision Details

Supervision runs on EVERY fire, even when no launches occur:

1. **Bound task check**: For every task with a bound session_id
2. **Session existence**: Verify session still exists via OpenCode API
3. **Stuck bound identification**: Check if task is in stuck_bound list
4. **Lease expiration**: Only recover if lease has expired
5. **Zero-message verification**: For stuck_bound tasks, check they have no messages
6. **Recovery execution**: Delete session, then atomic recovery
7. **Attempt tracking**: Recovery consumes one attempt

## Prompt Format

The launch prompt sent to worker sessions includes:

- Task identification (task ID, queue name)
- Issue reference
- Packet contents (JSON)
- Queue runbook
- Instructions to use `--worker <worker_id>` on all queue commands
- Instructions to heartbeat while working
- Instructions to use `--execution '{"lane": "opencode", "session_id": "<id>"}'` on terminal verbs

## Migration History

- **Migration 0021**: Atomic bind operation (`bind_repo_task_lane_session`)
- **Migration 0022**: Atomic recovery with `launched_by` field
- **Binding rate fix**: Increased from 12 → 20 → 36 concurrent sessions

## Testing

```bash
# Test discovery
python3 pm-stack/opencode-claimant/poll.py

# Test claimant in shadow mode
python3 pm-stack/opencode-claimant/claimant.py

# Validate implementation
python3 pm-stack/opencode-claimant/validate_simple.py

# Check fire logs
tail -20 ~/.local/state/opencode-claimant/fires.jsonl
```

## Troubleshooting

### Common Issues

**1. Mode file missing or unreadable**
- Defaults to `shadow` mode
- Create with: `echo "shadow" > ~/.local/state/opencode-claimant/mode`

**2. State directory permissions**
- Claimant creates directory if missing
- Ensure user has write access to `~/.local/state/`

**3. OpenCode server unavailable**
- `cap_room` will be `null`
- No launches will occur
- Check server at `http://127.0.0.1:13420/api/session`

**4. Queue CLI timeout**
- Server may be slow to respond
- Retry or check queue broker status

**5. File lock contention**
- Only one claimant can run at a time
- Previous instance may have crashed
- Remove lock file: `rm ~/.local/state/opencode-claimant/claimant.lock`

### Debugging

```bash
# Enable verbose logging
# Add print statements to claimant.py or poll.py

# Check previous fire output
jq . ~/.local/state/opencode-claimant/fires.jsonl | less

# Test API endpoints manually
curl -s http://127.0.0.1:13420/api/session | jq .

# Test queue CLI
~/fleet-brokers/bin/repo-queue list --lane opencode
```

## Security

- **No credentials in prompts**: Queue CLI uses broker's client token
- **Atomic operations**: Prevent race conditions
- **File locking**: Single-instance guarantee
- **Location verification**: Prevents session/project misattribution

## Performance

- **Discovery time**: ~1-2 seconds (poll.py)
- **Launch time**: ~2-5 seconds per task (session creation + bind + prompt)
- **Supervision time**: ~1 second per bound task
- **Concurrency**: Up to 36 tasks simultaneously
- **Typical fire duration**: < 10 seconds (shadow) or < 30 seconds (live with launches)

## Future Enhancements

Potential improvements for future slices:

1. **Parallel candidate processing**: Process multiple candidates concurrently within a fire
2. **Dynamic cap adjustment**: Adjust cap based on system load
3. **Priority queues**: Support priority-based candidate ordering
4. **Metrics export**: Export fire metrics to monitoring system
5. **Health checks**: More sophisticated server health monitoring
6. **Retry logic**: Exponential backoff for transient failures
7. **Cancellation handling**: Better handling of cancelled tasks

## References

- **Issue**: #3026 - OpenCode lane claimant/executor implementation
- **Recovery rule**: #4758 - Zero-message session recovery
- **Design**: #4912 - Session lifecycle and recovery design
- **Migration**: 0021 - Atomic bind operation
- **Migration**: 0022 - Atomic recovery with launched_by field
- **Constants**: See constants defined in claimant.py and poll.py

## License

This implementation is part of the OpenCode project. See project LICENSE for details.
