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
- **get_stuck_bound()**: Identifies bound tasks whose sessions meet the fail-closed stale-session preconditions; the claimant must still verify zero messages before recovery
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
These are the intended semantics; `claimant.py:launch_step()` currently does
not enforce the shadow-mode no-mutation rule (#7366), and prompt delivery is
not implemented (#7356). Do not treat a shadow fire as a safety guarantee
until those follow-ups land.

## Queues

The claimant monitors four queues:
- `repo-health-code-pilot`
- `repo-health-opencode-pilot`
- `repo-health-openchamber-pilot`
- `repo-health-nix-pilot`

## Concurrency Cap

- **36 active root sessions** across the fleet
- Monitored via `GET /api/session?limit=N` and `GET /api/session/active`
- Root sessions in the active set count; otherwise recent roots without a
  terminal marker count for the 30-minute active window
- Cap room is `36 - active_count`; unknown/incomplete reads make it unknown
  and block launches

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

## #4912 Session Contract and Call Inventory

This inventory covers the OpenCode-server operations in `poll.py` and the
reads/writes prescribed by `prompt.md`. It distinguishes what the contract
requires from what the current claimant actually verifies; a documented
operation is not evidence that its implementation is complete.

### Contract rules

1. **Judge one session per id; fail closed on incomplete collections.** A
   per-session decision must use that session's own record, not a truncated
   list. When a collection is necessary, accept it as complete only when the
   response proves completeness. `poll.py` escalates the `/api/session` limit
   through 1,000, 10,000, and 100,000; a response equal to every requested
   limit becomes unknown (`None`), never a partial cap or a "gone" verdict.
   An unreadable active-session set likewise makes `cap_room` unknown.
2. **Verify session creation before binding.** Create with
   `location: {directory: ...}`, then verify the response's
   `location.directory` equals the requested repo primary before the atomic
   task bind. Missing/mismatched location is a creation failure: do not bind
   or prompt, and remove the misplaced session before retrying.
3. **Give every bound state one disposition.** A live/progressing binding is
   retained; an unknown or incomplete read is never absence or zero messages;
   an expired-lease gone session is atomically cleared/retried while attempts
   remain; an expired-lease never-prompted session is deleted and verified
   before retry/failure; exhausted attempts fail with the session id in the
   execution record. The table below records the prompt's full state model and
   implementation gaps.

### Bound-state table

| Observed state | Contract disposition | Current implementation / follow-up |
|---|---|---|
| Unbound, `lane=opencode`, yielded at `handed_off` or `launch_failed` | Re-read the task and launch only while it remains claimed, unbound, and under the cap. | `poll.py:is_candidate()` checks the lane, binding, yield stage, and OpenChamber lane id. `claimant.py` re-polls before create. Queue-read reliability remains open: #7325/#7375, #7355, #7349. |
| Bound session is active, recent, progressing, or has a terminal marker | Keep the exact binding; never launch a second session. Queue terminal verbs remain the worker's responsibility. | `poll.py:is_stuck_bound()` rejects active ids, recent activity, terminal markers, and unexpired leases. `claimant.py:supervision_step()` does not use that classifier and can treat a terminal session as a recovery candidate; see #7367 and #7356 for the claimant's related supervision/launch gaps. |
| Bound session is absent, but its lease has not expired | Keep the binding and wait; do not relaunch over the lease. | Required by `prompt.md` step 1. `claimant.py:supervision_step()` does not check lease expiry before recovery. Tracked with the supervision contract gaps in #7367/#7356; add an explicit regression test before relying on this path. |
| Bound session is absent, lease expired, attempts remain | Verify absence per id, then atomically compare-and-clear both task and lane-ledger bindings using the old session id; the next fire may launch it. | `prompt.md` step 1d specifies this. Current `supervision_step()` instead funnels the read through unauthenticated `curl` and treats any response without an `id` as absent; distinguish HTTP 404 from auth/transport/5xx unknown per #7367. |
| Bound session is present, root, nonterminal, inactive, silent for at least 30 minutes, and lease expired | Read messages. Any message/activity/active membership keeps the binding; an unavailable or incomplete read is unknown. Only verified zero messages may proceed. | `poll.py:is_stuck_bound()` applies the list/active-set/lease/activity gates. `prompt.md` steps 1a–1b require the per-id message read. Current message count is based on parsed response length and has no page-size completeness check; see #7356 and the completeness contract in #6622. |
| Verified zero-message session, attempts remain | Delete the exact session and verify the delete effect first; then atomically compare-and-clear the binding, recording the old id and consuming one attempt. | `prompt.md` step 1b–1c. Current delete checks only `curl`'s process return code, not HTTP success or effect; #7367. |
| Verified zero-message session, attempts exhausted | Delete and verify first, then terminally fail as the task worker with the old session id in `--execution`. | Required by `prompt.md` step 1e. Current `supervision_step()` skips exhausted rows rather than issuing the terminal fail; claimant follow-up #7356/#7367. |
| Gone session, attempts exhausted | Verify absence, then terminally fail with the old session id in `--execution`. | Required by `prompt.md` step 1e; current per-id read conflates a non-session response with absence. See #7367. |
| Queue, session-list, active-set, or message read unavailable/incomplete | Preserve bindings; launch nothing on an unknown cap or task state. Do not report unreadable queue as an empty queue. | Session-list and active-set failures block the cap in `poll.py`. The queue CLI's table/chunked output currently parses as empty rows, which hides unreadable from empty; see #7325/#7375 and #7355. `prompt.md` explicitly treats incomplete session/message reads as unknown. |

### Server-call inventory

| Call and source | Contract disposition | Evidence / current gap |
|---|---|---|
| `GET /api/session?limit=N` — `poll.py:257–277`; used for cap roots and session lookup for bound tasks | Collection read: escalate limits and fail closed at equality; never infer absence from a partial list. | Implemented with `(1000, 10000, 100000)` and covered by `test_poll.py` equality/escalation cases. Phase 1 #4757 is closed. Per-entry `time` shape is still dereferenced unchecked; #6868 tracks it and #3069 records the broader overlap. |
| `GET /api/session/active` — `poll.py:280–289` | Active ids are required to classify roots; unreadable or unrecognized response means unknown cap, not zero active sessions. | Implemented as `None` on an unrecognized response, which propagates to `cap_room: null`. The endpoint's own completeness/page-size behavior is not documented in this repo; treat a future bounded/truncated response as unknown, not complete. General completeness work is tracked by #6622. |
| `GET /api/session/{id}` — `prompt.md` steps 1 and 5; used by `claimant.py:217–245` | Judge the bound session per id. A verified 404 may mean gone; auth, transport, server, malformed-body, or incomplete responses mean unknown and leave the binding unchanged. | Prompt names the per-id decision, but current curl call has no auth/status check and maps any body without an `id` to absent. Follow-up #7367. |
| `GET /api/session/{id}/messages` — `prompt.md` steps 1a and 5; `claimant.py:251–268` | Only a complete per-id message read can prove zero. Any message means keep bound; unavailable/incomplete means unknown. | Current implementation counts the parsed body but does not establish page completeness or validate a response envelope. Prompt delivery/reconciliation work is tracked by #7356; read completeness belongs to #6622. |
| `DELETE /api/session/{id}` — `prompt.md` steps 1b, 4b, 4c; `claimant.py:279–307, 445–472` | Delete the exact newly-created or verified-empty session; verify HTTP success/effect before clearing a binding or recording a retry. On unknown delete result, keep the binding. | Current code treats curl exit status 0 as deletion success and does not verify the result. Follow-up #7367. |
| `POST /api/session` — `prompt.md` step 4b; `claimant.py:410–452` | Use the v2 `location` and `model` objects; verify returned `location.directory` before binding. | Body shape and verify-before-bind ordering are present; Phase 2 #4870 is closed. The request is unauthenticated and does not validate the v2 `{data: ...}` envelope before reading `id`/`location`; follow-up #7367. |
| Prompt/message write to the bound session — `prompt.md` steps 4d and 5 | Send only after bind; on an uncertain send, reconcile that same session and never create a competing session. | Prompt requires the write but does not name its endpoint. `claimant.py:479–494` only constructs a string; it does not send it. Follow-up #7356. |

### Queue-read disposition: `limit=500`

The 500-row queue read is **not exempt** from the completeness rule. The broker's
`list` accepts up to 500 rows, but its current response does not provide a
trusted count/truncation signal (`count_exact=false`, no `Content-Range`, no
pagination). Therefore exactly 500 returned rows cannot prove the read was
complete: treat equality as **unknown and fail closed**, not as the whole
claimed-task set. `poll.py` currently omits `--limit` (the CLI default is 20),
so this is a disposition of the proposed 500-row read, not evidence that the
current poll consumes 500 rows. The current CLI read is independently broken:
`list` prints a table/chunked response while `poll.py` parses JSON and converts
that failure to `[]`; it can report unreadable as empty. Track the complete
read and machine-output fixes in #7355 and #7325/#7375; `--worker` read-verb
compatibility is #7349. The 500/equality rule is also within the general
pagination contract in #6622.

The contract's scope is the `poll.py` and `prompt.md` surface. `claimant.py`
implementation gaps are included above only where they execute a prescribed
server call, so the inventory does not misstate prompt requirements as
implemented behavior.

The import/reachability defect originally filed as #6865 is closed by merged
PR #7290: the current caller can execute its `subprocess`/`curl` paths, but
that does not establish successful HTTP behavior. The unauthenticated calls,
response-status handling, and prompt-delivery gaps are tracked separately by
#7367 and #7356. The malformed `session.time` shape dereferences in
`is_active_root()`, `is_stuck_bound()`, and `_stuck_entry()` remain tracked by
#6868 and overlap the broader parsed-shape audit #3069; they are not treated as
evidence that a server read is complete or valid.

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
- Python standard library; `claimant.py` currently shells out to `curl` for
  OpenCode-server calls
- `repo-queue` CLI at `~/.local/bin/repo-queue`
- OpenCode server running at `http://127.0.0.1:13420`

## Testing

The intended deterministic specification is in `prompt.md`; use tests as the
oracle for safety properties. A manual shadow fire is not currently proof of
read-only behavior because of #7366. To inspect a fire:

1. Ensure mode is set to `shadow` for safe testing
2. Run `python3 pm-stack/opencode-claimant/claimant.py`
3. Review fire log output in `~/.local/state/opencode-claimant/fires.jsonl`
4. Verify no mutations occurred (shadow mode)

## Known Limitations

- The prompt's server-call contract is not fully implemented: queue reads,
  auth/status validation, deletion verification, message-read completeness,
  and prompt delivery have open follow-ups listed in the inventory above.
- `/api/session/active` completeness is not evidenced here; keep cap decisions
  unknown if that endpoint is ever shown to return a bounded/truncated set.

## Implementation Status

**Contract audit documented; runtime contract is not fully satisfied.** The
server-call inventory above records the implemented checks and open gaps.
Follow-ups include queue-read completeness/format (#7355, #7325/#7375),
server-call authentication and response validation (#7367), shadow-mode
enforcement (#7366), and prompt delivery/reconciliation (#7356).

Slice 2 (#3026): Parallel-run gate implementation (separate work)
Slice 3+: Additional features and optimizations (future work)

## References

- Issue: #3026 - OpenCode lane claimant/executor implementation
- Design: #4912 - Recovery rules and supervision
- Defect: #4758 - Zero-message session recovery
- Defect: #4870 - Session location verification
- Defect: #5857 - Session creation request shape
- Migration: 0021 - bind_repo_task_lane_session
- Migration: 0022 - Atomic recovery operations
