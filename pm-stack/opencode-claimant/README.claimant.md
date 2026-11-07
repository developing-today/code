# OpenCode Lane Claimant/Executor - Implementation

This directory contains the implementation for slice 1 of #3026: the OpenCode lane claimant/executor.

## Files

- **claimant.py** - Main implementation of the claimant/executor workflow
  - Runs as an OpenChamber scheduled task on the laptop
  - Implements steps from prompt.md:
    1. Supervision of bound sessions (step 1)
    2. Gate check (step 2)
    3. Capacity check (step 3)
    4. Launch workflow (step 4): create session, bind, checkpoint, send prompt, post agent note
    5. Fire logging (step 6)
  - Mode detection from `~/.local/state/opencode-claimant/mode` (shadow/live)

- **prompt.md** - The scheduled task prompt (source of truth)
  - Contains the complete specification and step-by-step instructions
  - Embedded by install.mjs into the OpenChamber task definition

- **install.mjs** - Idempotent installer for the OpenChamber task
  - Merges the task definition into the laptop's code-project store
  - Preserves all other tasks in the store
  - Supports `--dry-run` for testing
  - Task ID: `f1b4a33b-14d5-4a69-9ad6-d221e42e3ac9`

- **migration-*.sql** - Database migrations for the recovery feature
  - migration-0021.sql: bind_repo_task_lane_session atomic binding
  - migration-0022.sql: guarded recovery mode with compare-and-clear
  - migration-0023.sql, migration-0024.sql: subsequent migrations

## Workflow

### Mode

- **shadow** (default): Discover, verify, and log what would launch — launch nothing
- **live**: Perform actual supervision, candidate discovery, and launches

Mode is read from `~/.local/state/opencode-claimant/mode` (missing/unreadable defaults to shadow).

### Discovery

Uses `python3 pm-stack/opencode-claimant/poll.py` to discover:
- Candidates: claimed tasks with lane="opencode", no session_id, stage in ("handed_off", "launch_failed"), lane_id not a UUID
- Stuck bound: bound tasks whose sessions appear created-but-never-prompted and dead
- Capacity: active root sessions count (max 12)

### Supervision (Step 1)

For every bound opencode-lane task:
1. Verify session still exists and is progressing
2. For stuck_bound tasks (zero messages, lease expired, silent >=30min):
   - Verify zero messages
   - Delete session if zero messages
   - Attempt recovery via atomic bind with `--launched-by opencode-claimant-recovery:<reason>`
   - Increment attempts counter
   - Terminally fail if attempts exhausted

### Gate (Step 2)

Check all four queues show 0 claimed-with-lane-opencode unbound candidates.

### Capacity (Step 3)

Check `cap_room` (12 - active_root_sessions). If 0 or null, launch nothing.

### Launch (Step 4)

For each candidate (highest priority first, while cap_room > 0):
1. Re-verify candidate status
2. Create session: POST /api/session with location.directory=/home/user/git/code, model.id=workhorse (or packet's route)
3. Verify location.directory matches requested (clean up misplaced sessions)
4. Bind atomically: repo-queue bind <id> --worker opencode-claimant --session-id <session_id>
5. Checkpoint launched: repo-queue checkpoint <id> --worker <worker_id> --stage launched
6. Send prompt: POST /api/session/<id>/messages with instructions to work the task using proper worker ID
7. Post agent-note:v1 state=handed off

### Logging (Step 6)

Append JSONL entries to `~/.local/state/opencode-claimant/fires.jsonl` with:
- timestamp
- mode
- candidates discovered
- sessions launched
- cap_room
- skips with reasons

## Constants

- Queues: repo-health-code-pilot, repo-health-opencode-pilot, repo-health-openchamber-pilot, repo-health-nix-pilot
- Concurrency cap: 12 active root sessions
- Route model: "workhorse" (default) or packet's route if present
- Agent: "build"
- OpenCode server: http://127.0.0.1:13420
- Queue broker: http://100.99.113.103:4801

## Testing

```bash
# Dry-run installation
bun pm-stack/opencode-claimant/install.mjs --dry-run

# Install task (creates backup)
bun pm-stack/opencode-claimant/install.mjs

# Run claimant in shadow mode
python3 pm-stack/opencode-claimant/claimant.py

# Run claimant in live mode (after enabling task)
echo "live" > ~/.local/state/opencode-claimant/mode
python3 pm-stack/opencode-claimant/claimant.py

# Check fire log
cat ~/.local/state/opencode-claimant/fires.jsonl
```

## Installation

The task is installed as an OpenChamber scheduled task with:
- Cron: */10 * * * * (America/Chicago)
- Provider: opencode-route
- Model: workhorse
- Agent: build
- Enabled: false (installs disabled; enable via parallel-run gate)

## Hard Rules

- Never work a task yourself; claim/bind/launch/record only
- Never launch for tasks with labels: held, user/owned, do-not-merge, do-not-auto-edit
- Never exceed 12-session cap
- Never double-bind a task
- Never bind a session id you did not just create
- No credentials in prompts, logs, or task records
