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

# Live only (mode file must read `live`): collect a sink row's
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

1. Verify zero messages via (singular — the plural endpoint 404s; see the Session Contract inventory)`GET /api/session/{id}/messages`
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

## Session Contract (#4912 — closing audit, Phase 4, 2026-11-10)

The contract the claimant judges and drives sessions by, stated once
here so it survives prompt edits (design #4912; phases 1–3 are
#4757/#4870/#4758, this section is the Phase 4 audit, #4916):

1. **Per-id judging.** A session's existence, location and progress
   are read per-id, and cap counts come from the
   `GET /api/session/active` id set. A list read is trusted only
   when its completeness is proven; an unproven or unreadable read
   fails closed (`cap_room: null`) and launches nothing.
2. **Verify before bind.** A created session binds only after its
   create response's `location.directory` equals the requested repo
   primary. A mismatch is a creation failure: the task stays
   unbound, the failure is checkpointed, and the misplaced session
   is deleted per-id.
3. **Every bound state has one disposition** (table below). The
   outcome lands on the task's progress, which is what the fleet
   watchdog reads — agreement via the shared record, no new
   channel.

### Bound-state table

| Bound state                                                                    | Judged by                                                                   | Disposition                                                                                                                                                                                                                                         |
| ------------------------------------------------------------------------------ | --------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Alive, messages/activity present                                               | per-id read + active set                                                    | Supervision leaves it; never relaunch over a live session                                                                                                                                                                                           |
| Gone (absent from the proven-complete list; per-id GET fails)                  | `GET /api/session/{id}`                                                     | Attempts remain: reclaim to unbound (`bind --launched-by opencode-claimant-recovery:gone-session`). Exhausted: terminal fail, session id in the execution record                                                                                    |
| Zero messages, no markers, silent > `ACTIVE_WINDOW_MS` (30 min), lease expired | `GET /api/session/{id}/message` — an unreadable read is UNKNOWN, never zero | Attempts remain: DELETE the verified-empty session first (a failed delete keeps the binding), reclaim to unbound, checkpoint `launch_failed` with the prior session id in an audit field. Exhausted: DELETE, then terminal fail with the session id |
| Created with the wrong `location.directory`                                    | create response, before bind                                                | Never bound; session deleted per-id; creation failure checkpointed                                                                                                                                                                                  |
| Bind failed after create                                                       | bind exit status                                                            | Created session deleted per-id; task stays unbound                                                                                                                                                                                                  |
| Prompt POST non-2xx                                                            | reconcile via the message read before any resend                            | Resend once only on a proven-empty list; an inconclusive reconcile never resends — the binding stands and the launch is flagged. Never double-prompt                                                                                                |

### Server-call inventory (Phase 4, #4916)

Every OpenCode-server call in `poll.py` and every call `prompt.md`
prescribes (steps 1–6; `claimant.py` is their deterministic
implementation), dispositioned against the contract:

| Caller                          | Call                                                              | Disposition                                                                                                                                                                                                                                                                                                                                                |
| ------------------------------- | ----------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `poll.py` `fetch_sessions()`    | `GET /api/session?limit=N`, N escalating 1000 → 10000 → 100000    | List read retained ONLY with a completeness proof: the first read whose count is below its limit is the whole list by construction; if every read lands exactly at its limit the fetch fails closed (`None` → `cap_room: null`). This is the standing form of rule 1 for this reader — stronger than failing closed at page-size equality on a single read |
| `poll.py` `fetch_active_ids()`  | `GET /api/session/active`                                         | Rule 1: cap counts come from the active id set directly. Unreadable → active count UNKNOWN (`None`), `cap_room: null`, fail closed                                                                                                                                                                                                                         |
| `claimant.py`                   | `GET /api/session/{id}`                                           | Rule 1/3: per-id existence for supervision and gone-session recovery                                                                                                                                                                                                                                                                                       |
| `claimant.py` `_get_messages()` | `GET /api/session/{id}/message`                                   | Rule 3: per-id; non-2xx, transport failure, or a non-list body returns UNKNOWN (`None`), never an empty list. The v2 endpoint is the singular `/message` — the plural `/messages` 404s with an empty body, the fail-open trap closed after #7760/#3026                                                                                                     |
| `claimant.py`                   | `DELETE /api/session/{id}`                                        | Rules 2/3: per-id, and only after a verified-empty read, a wrong-location create, or a failed bind; a failed delete keeps the binding                                                                                                                                                                                                                      |
| `claimant.py` step 4b           | `POST /api/session` with body `{"location": {"directory": …}, …}` | Rule 2: the v2 create shape (a top-level `directory` is stripped by v2 — #4870); the response's `location.directory` is verified equal to the primary clone before bind                                                                                                                                                                                    |
| `claimant.py` `_post_prompt()`  | `POST /api/session/{id}/prompt`                                   | Verified delivery: 2xx is the only success; a failure reconciles via the message read before any resend                                                                                                                                                                                                                                                    |
| `prompt.md` steps 1, 4b, 4d, 5  | prescribes the calls above                                        | No prescribed call lacks an implementation, and no implemented call is unprescribed                                                                                                                                                                                                                                                                        |

No call is left undispositioned, so the audit files no follow-up
issues.

### Queue-read disposition (#4916 scope 3)

`poll.py`'s queue-broker `POST /list` reads use `limit=500`
(`LIST_LIMIT`, the broker's own maximum, `LIST_LIMIT_MAX = 500` in
the fleet client). Disposition: **exempt** from the session
fail-closed rule, on this evidence:

- Different service, different failure shape: the broker page is a
  hard server maximum, not a silent default truncation like the
  session list's 50. Requesting the maximum is the completeness
  strategy this API offers; `/list` exposes no offset or count
  probe to prove more.
- The failure direction is safe for launches: a truncated page can
  only under-report candidates and stuck rows, never invent one,
  and every launch additionally requires the fail-closed session
  reads above.
- No truncation evidence: claimed + handed-off rows per queue in
  live queue-health reads number in the tens, an order of
  magnitude under the 500 page.

Named residual risk: at exactly 500 claimed rows in one queue,
supervision could miss a stuck-bound row beyond the page. If
claimed counts ever approach the page size, this disposition must
be revisited with a count probe. No code change is made on this
question — per #4916, no code change without evidence of
truncation, and none exists.

## Seam Contract (#7770 — closing audit, Phase 4, 2026-11-11)

The one contract for all three of the claimant's seams (design
#7770; Phases 1–3 are #7325/#7375/#7706, #7367/#7760,
#7356/#7366/#7732), stated beside #4916's session contract above
so it survives prompt and implementation edits:

> **The claimant acts only on a verified read.** A read is
> verified when it arrived through the seam's one transport, in
> the shape the writer actually produces, with its status
> checked. Anything else is **UNKNOWN** — and unknown never
> authorizes a side effect (launch, delete, unbind) and is never
> recorded as a measurement.

The three seam decisions:

1. **Queue seam — one machine-readable channel, and the yield is
   where the verb writes it.** Queue rows are read in the
   broker's rich `/list` shape (see the divergence note below);
   the yield of record is `progress.last_checkpoint.stage`
   (#7706 option A: `progress.stage` stays accepted for older
   rows, and `normalize_row` only lifts the checkpoint stage
   when no live stage is set, so a live later stage vetoes a
   stale yield). A row the reader cannot classify is not a
   candidate.
2. **Server seam — one authenticated transport; status and shape
   are the verdict.** Every OpenCode-server call in `claimant.py`
   goes through `_server_request` (Basic auth, status captured
   via curl's `-w` trailer); success is HTTP 2xx plus the
   expected body shape. _Zero messages_ is a 2xx whose `data`
   list is empty; _gone_ is a 404 from the per-id GET. A
   subprocess exit code is never a verdict.
3. **Actuation seam — mode is a gate, sequence is verified, the
   log is a measurement.** Shadow mode mutates nothing (checked
   in `launch_step` itself). The landed launch order is create →
   verify location → bind → verified prompt (the #7356
   resolution: a prompt that cannot be landed leaves the binding
   standing for supervision, because unbinding could
   double-launch over a worker that is in fact executing — this
   supersedes the design text's create → prompt → bind order).
   The fire log records only measured values: `cap_room` is what
   `cap_step()` measured or `null`; `candidates_seen` is the
   discovered list, or `null` when discovery itself failed.

### Seam inventory (Phase 4, #7772)

#4916's table above owns the OpenCode-server calls in `poll.py`
and the calls `prompt.md` prescribes; this inventory owns
`claimant.py`'s remaining external reads/calls and both queue
seams. Overlap is cross-referenced, never duplicated
(disposition (c)).

| Caller                                                        | Read/call                                                                                                                                              | Disposition                                                                                                                                                                                                                                                                                                                                                                              |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `poll.py` `_broker_post` / `_fetch_rows`                      | Queue-broker `POST /list` (`rich: true`, `limit=500`) for `claimed` + `handed_off` rows, token from the broker token file in an `X-Agent-Token` header | The queue seam's one read. Transport/protocol failure raises in `_broker_post`; `_fetch_rows` then contributes **no rows for that queue** — fail-closed for launches (no candidates invented), but see the recorded residual below                                                                                                                                                       |
| `poll.py` `normalize_row`                                     | Lifts `last_checkpoint.stage` → `progress.stage` (only when no live stage), payload → `packet`                                                         | Writer-shape bridge for #7706 option A; pinned by `TestFetchClaimedRows`                                                                                                                                                                                                                                                                                                                 |
| `poll.py` `_server_get`, `fetch_sessions`, `fetch_active_ids` | OpenCode server GETs                                                                                                                                   | Owned by #4916's inventory (rules 1–3 there); unchanged                                                                                                                                                                                                                                                                                                                                  |
| `claimant.py` `run_poll`                                      | subprocess `python3 <primary>/pm-stack/opencode-claimant/poll.py`, stdout parsed as JSON                                                               | Verified: non-zero exit or non-JSON stdout raises; the gate treats that as discovery-UNKNOWN (fixed this audit — previously logged as an empty queue). Note: runs the **primary clone's** poll.py while `_load_poll_module` loads the sibling copy for cap counting; the claimant is installed to run from the primary, so the two are the same file in practice — recorded, not changed |
| `claimant.py` `_server_request` + wrappers                    | All OpenCode-server calls (session GET/DELETE/POST, prompt POST, message GET)                                                                          | Decision 2 transport; per-call dispositions in #4916's table. This audit fixes its header comments, which cited the contract as "decision 3" — the server seam is Decision 2                                                                                                                                                                                                             |
| `claimant.py` supervision / launch                            | `repo-queue bind` (recovery + launch), `repo-queue claim --handoff --platform openchamber` (sink collection)                                           | CLI verbs with `--worker opencode-claimant`; verdicts from exit status + parsed stdout (claim prints the claimed row or `{"task": null}`). A claim/bind failure skips or stops — a binding is never improvised. Sink collection never names a task id (#8097 Rule 1)                                                                                                                     |
| `claimant.py` `_github_issue_labels`                          | `gh issue view --json labels` for the held-label gate (#7670)                                                                                          | Verified read: non-zero exit or a non-list body raises and the candidate is skipped (fail closed)                                                                                                                                                                                                                                                                                        |
| `claimant.py` `read_mode`, auth files, `log_fire`             | Local state files (`mode`, `service-prod.json`, broker token file, `fires.jsonl`)                                                                      | Local, not seam calls: a missing/unreadable mode file means `shadow`; unreadable auth means the server rejects the call and the caller fails closed; credential values are never logged. Fire-log measurement rules are Decision 3 above                                                                                                                                                 |

Recorded residuals (routed to their owning follow-ups, not
silently passed):

- **Channel divergence from Decision 1's letter.** The design
  pinned the repo-queue CLI's JSON output as the one channel;
  Phase 1 landed the broker rich `/list` read instead (commit
  `eb1f9987eb`), and the CLI still has no JSON list mode (its
  `list` verb prints the human table; verified live 2026-11-11).
  The pin's intent — a machine shape, fail-closed — is met by
  the rich read; the letter is not. Recorded on #7770 for the
  design thread to amend or re-pin.
- **`_fetch_rows` collapses a failed queue read to an empty
  queue in poll.py's output** (`claimed: 0`, no unknown marker),
  although `_broker_post` itself raises. Launches stay safe (a
  failed queue contributes no candidates), but poll.py's JSON
  cannot say "this queue was unreadable". Owned by open issue
  #8042 (fire log cannot distinguish a failed queue read from
  an empty queue); this audit's gate-exit fix
  (`candidates_seen: null` when discovery itself fails) closes
  the distinction at the claimant's gate, and the per-queue
  marker in poll.py's output remains #8042's to land.
- **Supervision's per-queue failure is a stdout warning only**
  (claimant.py `supervision_step`): an unreadable queue is not
  supervised that fire and the fire log does not record it.
  Actions still fail closed (recovery never acts on an unread
  row). Also #8042's territory; no code change here without
  its fire-log schema decision.

### Stub-shape audit (Phase 4, #7772)

Every test stub names the writer whose shape it mirrors:

| Suite                     | Stub                                                        | Writer mirrored                                                                                                                                      | Verdict                                                                                                                                                    |
| ------------------------- | ----------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `test_poll.py`            | Row fixtures + `_broker_post` stub (`TestFetchClaimedRows`) | The broker's rich `/list` row (progress incl. `last_checkpoint`, payload column) and the checkpoint verb's progress shape                            | Conforming; fixtures cite #7706/#8192/#8234 where the shape is non-obvious                                                                                 |
| `test_poll.py`            | Session fixtures (`TestIsActiveRoot`, `TestFetchSessions`)  | The v2 server's session objects (`time`/`outcome` markers, `parentID`) and its limit-truncation behaviour                                            | Conforming                                                                                                                                                 |
| `test_claimant.py` (root) | `make_fake_run` curl stub                                   | The claimant transport's own writer: curl `-w` status trailer on every reply, rc 0 even for 401 (as `curl -s` behaves), v2 error bodies              | Conforming — the docstring states the shape contract explicitly                                                                                            |
| `test/test_claimant.py`   | `make_fake_run` curl stub                                   | **Was:** the reader's pre-#7770 assumption (bare bodies, no status) — the #7760 defect class; 2 of its 5 tests failed against the verified transport | **Fixed this audit:** replies now carry the status trailer and the session GET answers the v2 server's 404 error body; the stub docstring names the writer |

### Required dispositions (Phase 4, #7772)

- **(a) Verb trio #7349 / #7355 / #7358 — INDEPENDENT of the
  landed channel, per verb.** #7349 (`list`/`health` reject
  `--worker`): the claimant issues no CLI `list`/`health` reads
  (rows come from the broker read; gate counts come from
  poll.py), so the JSON channel does not depend on it.
  #7355 (Rust client does not de-chunk >32 KiB): the claimant's
  broker read is its own urllib POST in `poll.py`, not the Rust
  client, so the trap does not reach it. #7358 (verbs silently
  default to the shadow queue): every claimant CLI call names
  its queue or its task id explicitly (`claim --queue`,
  `bind <id>`), so the default is never consulted. All three
  stay open as CLI-side defects in their own right.
- **(b) #7735 (claim verb targeting) — NOT a phase of #7770**,
  as the design records. The claimant has no use for it: sink
  collection is deliberately broker-picked (`claim --handoff`,
  #8097 Rule 1); the by-id claim remains a special-case verb
  for one-off revivals in `prompt.md`, not a claimant flow.
- **(c) #4916 boundary — resolved by cross-reference.** #4916
  landed first (2026-11-10; the Session Contract section above),
  so its inventory owns `poll.py`'s and `prompt.md`'s server
  calls, including the claimant.py rows it lists as their
  implementation. This audit owns `claimant.py`'s queue/CLI/gh
  reads and both queue seams; no row is duplicated.
- **(d) #6533 — re-scoped with the reason stated.** The
  mechanism chain is fixed in code (rich read `eb1f9987eb`,
  sink collection `1f45283f0f`, sink status `1d1b3c60a7`), but
  #6533's claim is empirical — handed-off tasks actually
  launching on live fires — and no live-fire evidence is in
  this audit's reach. It stays open, re-scoped to that
  verification, with the Phase 1 evidence recorded on it.
- **(e) `prompt.md` generation — YES, follow-up filed as
  issue #8366.** The
  drift class behind #7760 is live in miniature: this audit
  found `prompt.md`'s CLI-path constants stale
  (`~/fleet-brokers/bin/repo-queue`, twice) against
  `claimant.py`'s `~/.local/bin/repo-queue`, and repaired them;
  whether the prose constants are generated from the code or
  pinned by a sync check is #8366's question.

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
