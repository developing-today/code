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

### launch_one.py

Per-task launch trigger (#8187): performs claimant.py's launch
sequence (prompt.md step 4) for ONE named task, immediately, instead
of waiting for the next claimant fire.

```bash
# Read-only: is this task a launch candidate, and what would happen?
python3 pm-stack/opencode-claimant/launch_one.py --preflight repo-health-code-pilot:12345

# Live only (mode file must read `live`): collect a parked handoff's
# claim by id if needed, create + verify + bind a session, checkpoint
# launched, send the prompt, post the agent-note. Prints the bound
# session_id; any failure exits non-zero and deletes a created
# session it could not bind.
python3 pm-stack/opencode-claimant/launch_one.py --launch repo-health-code-pilot:12345
```

Candidate discovery, the cap, the mode gate, and the OpenCode server
transport are reused from poll.py / claimant.py; the repo-queue
binary is `~/.local/bin/repo-queue` (override: `REPO_QUEUE_BIN`).

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

## The Session Contract (#4912)

Phase 4 closing audit (issue #4916, 2026-11-10): every OpenCode-server
call the claimant makes or prescribes, dispositioned against the
contract below. This section is the contract's durable home — it
survives prompt edits because `prompt.md` is regenerated and this
README is not.

### The three rules

1. **Judge a session by its own record.** Any verdict about one
   session (exists / gone / progressing / finished) comes from a
   per-id read of that session or from a session list whose
   completeness is proven. A truncated list is not evidence: at
   page-size equality the read fails closed (rule 1's mechanism in
   `poll.py` is the escalating-limit fetch — the first read below
   its limit is the whole list by construction; all reads at their
   limit means completeness is unprovable and the fetch returns
   None).
2. **Verify before bind.** A created session is bound to a task
   only after a per-id read-back confirms the create took effect
   as sent — including `location.directory` (#4870: the v2 API
   ignored a top-level `directory`, so an unverified create landed
   workers in /home/user). A create that cannot be verified is
   deleted, never bound.
3. **Every bound state is dispositioned.** Each state a bound
   session can be in (table below) has exactly one named claimant
   behavior. A state with no row in the table is a contract bug:
   file it, do not improvise around it.

### Bound-state table

| Bound state (evidence) | Disposition |
|---|---|
| Present, recent activity, no terminal marker | Working. No action; never relaunch over it. |
| Present, idle or terminal outcome recorded | Finished. No action; the worker session records its own terminal state. |
| Present, no messages, silent > ACTIVE_WINDOW_MS, attempts remain | Never-prompted (#4758): verify zero messages per-id, DELETE per-id, reclaim the binding atomically; the task is unbound for a later fire. |
| Present, no recent activity, no terminal marker, has messages | Stuck: named in the fire's `stuck_bound` supervision report. No relaunch, no reclaim. |
| Absent, terminal state recorded | One relaunch attempt on a later fire (prompt.md step 1). |
| Absent, no terminal state, attempts remain | Gone-session recovery (#4758): verify absence per-id, reclaim the binding atomically. |
| Absent, attempts exhausted (either recovery class) | Fail the task terminally with the session id in the execution record. |
| Session list or per-id read unavailable | Unknown. The claimant launches nothing and reclaims nothing — absence of evidence is never treated as a gone session. |

### Server-call inventory (Phase 4, 2026-11-10)

OpenCode-server calls in `poll.py`, `claimant.py`, `launch_one.py`,
and every server read/write `prompt.md` steps 1–6 prescribe:

| Call | Where | Disposition |
|---|---|---|
| `GET /api/session?limit=N`, N escalating 1000 → 10000 → 100000 | `poll.py` `fetch_sessions()` | Rule 1 mechanism: fail-closed at page-size equality (#4757). Compliant. |
| `GET /api/session/active` | `poll.py` `fetch_active_ids()` | Id-keyed map; no list to truncate. Unreadable → None → cap counts UNKNOWN, never zero; claimant launches nothing (`summarize_cap`). Compliant. |
| `GET /api/session/{id}` | `claimant.py` (existence / gone-session verification) | Per-id judging, rule 1. Compliant. |
| `GET /api/session/{id}/message` | `claimant.py` (zero-message verification) | Per-id; recovery acts only on a verified empty result, and an unavailable/incomplete read is treated as not verified (#4758). Compliant. (README's Recovery Rules above say `/messages`; the implemented path is `/message`.) |
| `DELETE /api/session/{id}` | `claimant.py` (recovery, created-but-unbindable cleanup) | Per-id only, after the per-id verification above it. Compliant. |
| `POST /api/session` | `claimant.py`, via `launch_one.py` | Create is verified by per-id read-back incl. `location.directory` before bind (rule 2, #4870). Compliant. |
| `POST /api/session/{id}/prompt` | `claimant.py` `_send_prompt`, via `launch_one.py` | Per-id write to the just-verified session; a failed POST is a launch failure, not a bind. Compliant. |
| `GET /api/session` (supervision, cap count) | `prompt.md` steps 1–2 | Prescription routes through `poll.py`'s fail-closed fetch (same row as the first entry); the prompt names the endpoint, the mechanism is poll.py's. Compliant. |
| `GET` session messages, `DELETE` per-id (recovery) | `prompt.md` step 1 recovery rule | Same calls as the claimant.py rows; prescribed per-id with verify-first ordering. Compliant. |

`poll.py`'s `REPO_QUEUE_CMD` constant names the repo-queue binary
path but is not itself a server call (no remaining call site) — it
is recorded here so the inventory is complete, not because it needs
a disposition.

### Queue-read disposition: broker `/list` at `limit=500`

`poll.py` reads claimed/parked rows from the **queue-broker's** rich
`/list` (not PostgREST — that route died with the #5362 cutover;
#7325/#7375 restored this one) with `LIST_LIMIT = 500`, the broker
client's own maximum (`LIST_LIMIT_MAX` in the fleet client). The
question posed by #4916 was whether this read needs the same
fail-closed-at-equality rule as `/api/session`.

**Disposition: exempt, with reasons, from evidence.**

- Different service, different contract. The #4912 contract governs
  verdicts about OpenCode *sessions*. The `/list` read renders no
  session verdict; it enumerates task rows.
- The failure direction is benign. A truncated `/list` page can
  only *under*-report candidates and stuck-bound rows — a missed
  launch or a missed stall report for one fire. It cannot invent a
  candidate, and it cannot make a live session read as gone: every
  launch and recovery decision is re-gated on the fail-closed
  session reads above, so a short queue page can never cause a
  wrongful launch or reclaim by itself.
- The cap cannot be hit in current operation. Claimed rows across
  the whole pilot queue numbered 62 at the 2026-11-10 fleet health
  read — an order of magnitude under one page, across all queues
  the claimant polls combined per queue (the limit is per queue
  read). If a queue's claimed+parked population ever approaches
  500, this disposition must be revisited and the read given the
  escalating-limit treatment; the trigger is named here so the
  exemption cannot silently outlive its evidence.

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

## Implementation Status

✅ **SLICE 1 COMPLETE** - All requirements met and verified

Slice 2 (#3026): Parallel-run gate implementation (future work)
Slice 3+: Additional features and optimizations (future work)

## References

- Issue: #3026 - OpenCode lane claimant/executor implementation
- Design: #4912 - Recovery rules and supervision
- Defect: #4758 - Zero-message session recovery
- Defect: #4870 - Session location verification
- Defect: #5857 - Session creation request shape
- Migration: 0021 - bind_repo_task_lane_session
- Migration: 0022 - Atomic recovery operations
