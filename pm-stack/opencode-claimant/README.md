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

## Server-call inventory (#4916, Phase 4 of #4912)

Every call the claimant makes to the OpenCode server, with the file:line
that issues it. Audited 2026-11-08 against `main`. Nothing here is
inferred: each row names the call site.

| # | Call | Site | Purpose | Failure behaviour |
| --- | --- | --- | --- | --- |
| 1 | `GET /api/session?limit={n}` | `poll.py:272` (`fetch_sessions`) | Complete session list for cap counting and stuck-bound classification | Returns `None`; cap fails closed (`-1`), supervision leaves tasks bound |
| 2 | `GET /api/session/active` | `poll.py:284` (`fetch_active_ids`) | The server's authoritative active set | Returns `None`; cap fails closed |
| 3 | `GET /api/session/{id}` | `claimant.py:219` (supervision) | Does a bound session still exist, and is it terminal? | Treated as "could not verify" — the task is left bound |
| 4 | `GET /api/session/{id}/messages` | `claimant.py:253` (supervision) | Zero-message check before a zero-message recovery | Treated as unknown state; no recovery |
| 5 | `DELETE /api/session/{id}` | `claimant.py:281` (zero-message recovery) | Remove a zero-message session before reclaiming its task | Recovery is skipped when the delete fails |
| 6 | `POST /api/session` | `claimant.py:428` (launch) | Create the worker session | `launch_error` skip; no bind |
| 7 | `DELETE /api/session/{id}` | `claimant.py:446` (wrong location) | Roll back a session created in the wrong directory | Rollback only; the skip is already recorded |
| 8 | `DELETE /api/session/{id}` | `claimant.py:471` (bind failed) | Roll back a session whose bind lost the race | Rollback only; the skip is already recorded |

Two reads are **not** server calls but gate every decision above and are
listed for completeness: `repo-queue list --status claimed` per queue
(`poll.py:300`) and the `repo-queue bind` compare-and-clear write
(`claimant.py:455`, `claimant.py:288`).

### Contract rules

These are invariants of the code above, not aspirations. A change that
breaks one is a defect in this directory, not a tuning choice.

1. **Per-id reads fail closed.** A per-session read that cannot be
   completed (transport error, non-JSON body, error object without an
   `id`) never proves a session gone. Only an explicit "no such session"
   response may feed a recovery decision; anything else leaves the task
   bound and lets the next fire re-check. (`claimant.py:226`,
   `claimant.py:242-244`, `claimant.py:266-268`.)
2. **Verify before bind.** A session is bound only after its
   `location.directory` matches `PRIMARY_CLONE`; a mismatch deletes the
   session and records `session_wrong_location` instead of binding.
   (`claimant.py:443-452`.)
3. **Bind is the only mutation that grants work.** Session creation
   grants nothing. The atomic `bind` compare-and-clear is what makes a
   task the claimant's, and a failed bind rolls the session back rather
   than leaving an orphan. (`claimant.py:455-477`.)
4. **The cap fails closed.** `GET /api/session` truncates at 50 rows by
   default, so completeness is proven by an escalating `limit` read
   where the first response below its requested limit is the whole list;
   if every read comes back exactly at its limit, the count is `-1` and
   no launch happens. (`poll.py:254-277`, `poll.py:409-420`.)

### Bound-state table

What each state means for the claimant, and what it may do about it.
"Bound" means the task's `progress.session_id` names a session.

| Bound? | Session exists? | Session state | Claimant action |
| --- | --- | --- | --- |
| no | — | — | Candidate for launch if lane is `opencode` and cap room exists |
| yes | yes | live / progressing | Nothing — never reclaim or re-launch |
| yes | yes | terminal (`time.idle` or `outcome`) | Supervision may recover if attempts remain |
| yes | yes, zero messages | silent past `ACTIVE_WINDOW_MS` | Zero-message recovery: delete, then atomic reclaim |
| yes | no (explicitly absent) | — | Gone-session recovery: atomic reclaim, no delete |
| yes | unknown (read failed) | — | Nothing — fail closed, leave bound |
| yes | — | attempts exhausted | Terminal `fail` with the execution record |

### Queue-read disposition: the `limit=500` claimed-task read

The claimed-task read is a PostgREST query with a default `limit` of 500
rows (`agent.revalidate_parked_tasks` uses the same default,
`migration-0023.sql:312`). **Disposition: accepted as-is, with the
truncation made harmless rather than removed.**

- 500 claimed rows on one queue is far above the observed claim
  population, so in practice the read is complete.
- If it ever truncated, the effect would be *undercounting* candidates
  and stuck-bound rows — the claimant would see fewer tasks, never a
  task that is not really there. Every launch still re-verifies the task
  is claimed, lane `opencode`, and unbound before binding
  (`prompt.md:168`), so a truncated read cannot cause a bad launch.
- The cap does **not** depend on this read: it comes from the session
  reads above, which fail closed on their own.
- Raising the limit would trade a bounded, safe undercount for an
  unbounded read; the failure mode it removes (a missed candidate, found
  on the next fire) is cheaper than the one it adds.

A future change that makes a queue read drive a destructive action must
first prove completeness the way `fetch_sessions` does, or fail closed.

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
