laptop-agent: note: this call waited 11s for its reply, beyond the time the laptop spent executing — the wait was in the broker/tunnel layer
<!--
  developing-today-forks/opencode fork notice:
  This fork tracks the upstream v2 line (anomalyco/opencode branch dev) exclusively.
  Following upstream's promotion of v2 to dev (with legacy v1 branched to v1), this fork tracks dev.
  Latest upstream commit merged: 00e7472faa2ef15b30db6110440117e4e7e1a72a (Fri Oct 16 15:15:15 2026 +0530).
-->

> [!NOTE]
> **Fork Information (`developing-today-forks/opencode`):**
> This repository is a fork of [OpenCode](https://github.com/anomalyco/opencode) tracking the upstream **`v2`** codebase on `dev` exclusively (upstream promoted v2 to `dev` on Oct 10, 2026; legacy v1 is on `v1`).
> - **Latest upstream commit merged**: `00e7472faa2ef15b30db6110440117e4e7e1a72a` (Fri Oct 16 2026).
>
> **High-Level Additions & Differences in this Fork:**
> - **Model Route Pooling & Scheduled Weights**: Named provider/model route pools (`experimental.model_routes`), recurring time-based weight schedules (`schedules` and `experimental.model_route_schedules` with weekday/month/hour/timezone windows), selection modes (ordered, round-robin, weighted), soft budgets, burst rate-limit continuity, and silent-target hedging.
> - **Indefinite Route Telemetry & Cost Accounting**: Permanent request attempts, failure breakdowns, rate-limit snapshots, and per-request USD cost tracking (`cost_usd`) derived from catalog token pricing.
> - **Autonomous Route Review & Tuning**: Built-in `route_stats` and `route_usage` inspection tools, deterministic tuning suggestions, `route_adjust` runtime steering, and tuner notes support.
> - **Plan & Quota Tracking**: Dedicated plan store (`plans.json`, `plan-notes.jsonl`) with `plan_info` tool for tracking subscriptions, quota reset windows, cost tiers (`cheap`, `standard`, `frontier`), and user experience notes.
> - **Built-in Skills & Commands**: Integrated `/routing-review`, `/plan-update`, `/routing-note`, and `/routing-config` commands, backed by dedicated `routing-review`, `routing-usage`, `plan-tracking`, and `routing-config` skills.
> - **MCPX Integration & Daemon Handover**: Native MCPX tool projection into Code Mode, detached background daemon with JIT secret prompts, and graceful socket handover on `serve` restarts.

<p align="center">
  <a href="https://opencode.ai">
    <picture>
      <source srcset="packages/console/app/src/asset/logo-ornate-dark.svg" media="(prefers-color-scheme: dark)">
      <source srcset="packages/console/app/src/asset/logo-ornate-light.svg" media="(prefers-color-scheme: light)">
===OCHREADME===
<!--
  developing-today-forks/openchamber fork notice:
  This fork tracks upstream openchamber/openchamber (branch main).
  Latest upstream commit merged: 2b94db8470ca77d946917e937e3b04d7b0b678bc (feat(render): let agents ask the user to choose on a page, Fri Oct 16 03:45:18 2026 +0300) via merge commit 8f9956331.
-->

> [!NOTE]
> **Fork Information (`developing-today-forks/openchamber`):**
> This repository is a fork of [OpenChamber](https://github.com/openchamber/openchamber) tracking upstream `main`.
> - **Latest upstream commit merged**: `2b94db8470ca77d946917e937e3b04d7b0b678bc` (Fri Oct 16 2026, merged via `8f9956331`).
>
> **High-Level Additions & Differences in this Fork:**
> - **Model Pools UI & Recurring Schedules**: Settings section for managing OpenCode route pools (`experimental.model_routes`), target memberships, weights, and recurring time-based weight schedules (`ScheduleFields` / `ScheduleListEditor` with timezone, weekdays, and day-of-month windows).
> - **Route Rules Interpreter & Timed Overrides**: Natural-language sentence routing ("Route by sentence") with support for timed windows, recurring schedule extraction, and active override display.
> - **Plans & Usage Section**: Settings page (`Settings → Routing → Plans and usage`) for tracking user subscriptions, quota windows, renewal dates, cost tiers (`cheap`, `standard`, `frontier`), and ad-hoc usage experience notes.
> - **Review & Update Action Buttons**: Direct launcher buttons with configurable trigger mode (prefill in composer vs. auto-send immediately) for `/routing-review` and `/plan-update`.
> - **Control & Workspace Enhancements**: Worktree-by-default sessions, project control actions (`projects.add`/`projects.remove`), and fork-specific release update resolution.
> - **MCPX Secret Management**: Native UI prompts for JIT secrets required by MCP servers across desktop, web, VS Code, and mobile.
> - **Inter-session Messaging**: A Messages surface (inbox of messages agents send you, topics with threads, hide and restore history, soft write limits, meaning-based search, session chains and logs), chat rows for delivered messages, toasts for messages to you, and Settings → Messaging for the local embedding model, remote embeddings, delivery, presets and variables. Needs an OpenCode build with messaging (developing-today-forks/opencode).

# <picture><source media="(prefers-color-scheme: dark)" srcset="docs/references/badges/openchamber-logo-dark.svg"><img src="docs/references/badges/openchamber-logo-light.svg" width="32" height="32" align="absmiddle" /></picture> OpenChamber

[![GitHub stars](https://img.shields.io/github/stars/openchamber/openchamber?style=flat&labelColor=100F0F&color=66800B)](https://github.com/openchamber/openchamber/stargazers)
[![GitHub release](https://img.shields.io/github/v/release/openchamber/openchamber?style=flat&labelColor=100F0F&color=205EA6)](https://github.com/openchamber/openchamber/releases/latest)
[![Discord](https://img.shields.io/badge/Discord-join.svg?style=flat&labelColor=100F0F&color=8B7EC8&logo=discord&logoColor=FFFCF0)](https://discord.gg/ZYRSdnwwKA)
[![Support the project](https://img.shields.io/badge/Support-Project-black?style=flat&labelColor=100F0F&color=EC8B49&logo=patreon&logoColor=FFFCF0)](https://www.patreon.com/openchamber)

## Run agent work. Keep control. Ship from anywhere.

**OpenChamber is an open-source workspace for running and reviewing AI coding work on desktop, web, VS Code, and mobile.**
===CLAIMANT-REST===
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
  "mode_source": {
    "path": "/home/user/.local/state/opencode-claimant/mode",
    "state": "missing",
    "file_mtime": null,
    "dir_mtime": "2026-11-14T14:45:12+00:00",
    "pid": 41234
  },
  "candidates_seen": [...],
  "sessions_launched": [...],
  "cap_room": 36,
  "skips": [...]
}
```

`mode_source` (#10332) says where the fire's mode came from. `state` is
`ok`, `missing`, `unreadable` or `invalid`; every state but `ok` means the
fire fell back to shadow. A deletion changes the state directory's mtime,
so for a `missing` read `dir_mtime` bounds when the file went, and a
live-to-shadow flip can be dated from the log alone.

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

### Broker Unix transport (#11980)

The read-only poll uses the same configured broker and token resolution for
both transports: `REPO_QUEUE_BROKER_URL` or
`~/.config/repo-queue/broker-url`, and `REPO_QUEUE_TOKEN_FILE` or
`~/.config/queue-broker/token`. An explicit
`unix:///absolute/path/to/queue-broker.sock` sends HTTP `POST /list`
over that literal filesystem socket with the usual `X-Agent-Token` header.
It does not append the API path to the socket filename, probe TCP, or
redirect to another broker. HTTP(S) URLs retain the existing urllib path.
Both paths use the existing 30-second socket timeout. Transport, HTTP,
JSON and broker application errors remain failed reads, not successful
empty queues; no configuration changes or claimant launches are implied.

Run the ordinary no-network suite unchanged:
`python3 -I pm-stack/opencode-claimant/run_hermetic_tests.py`.

Run the separate bounded local transport fixtures:
`python3 -I pm-stack/opencode-claimant/test_poll_transport.py`.
They serve only `/list` on a temporary Unix socket, use a dummy token,
guard connections to that fixture path, and mock the TCP regression seam.
They never run the live poll entry point, claim work, or launch sessions.
