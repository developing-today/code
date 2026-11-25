# fleet queue: inspection and admin

The inspection and admin verbs read the queue's history and operate its side systems: `timeline` / `history` (one task's event timeline), `staleness` / `needs-user` (the stewardship cycle's canned reads over the status-history table), `signal` (the wake-up signal file), `models` (serving-model rollup), `outbox` (the creation outbox for GitHub objects), and `stack` (merge-intent stacks).

Shared context — the global exit-code vocabulary (0 ok, 1 gate/not-found, 2 usage, 3 transport/401, 4 broker store failure, 5 no-claim/no-row, 6 fallback switch) — is stated in [fleet queue: core verbs](queue-core.md).

Sources: [commands_query.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/commands_query.rs), [commands_lifecycle.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/commands_lifecycle.rs), [fallback_stack.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/fallback_stack.rs), [creation_outbox.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/creation_outbox.rs), [merge_intent.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/merge_intent.rs).

## fleet queue timeline

```text
usage: repo-queue timeline <task-id> [--worker <id>] [--json]
```

| Flag                     | Argument  | Default            | Meaning                                                                                |
| ------------------------ | --------- | ------------------ | -------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id   | required           | The task whose timeline to render.                                                     |
| `--worker`               | worker id | `repo-worker-code` | Attribution; the events queries don't use it beyond the shared envelope.               |
| `--json`                 | boolean   | off                | Print `{"task_id": n, "events": […], "served": […]}` instead of the rendered timeline. |

**Behaviour.** For each of 14 event kinds — `claimed, session_bound, lane_session_bound, model_stamp, session_unbound, session_binding_cleared, completed, lease_expired, lease_expired_at_cap, claim_refused, released, parked, failed, cancelled` — it calls `/events-list {task_id, kind, limit:1000}`, merges, sorts by event id, and collapses duplicates (same kind/session/leaf/trigger within 120 s = one record written by two surfaces). If the broker doesn't serve `/events-list`, it prints the interim direct-DB SQL to stderr and exits **3**. Output: a rendered timeline (binding windows with the exact resolved models per window, joined from the route ledger). With `--json`: `{"task_id": n, "events": […], "served": [{"session_id", "start", "end", "segments": […]}]}`; an empty timeline prints `task <id>: no timeline records` (or the empty JSON shape), exit 0.

Exit codes: 0 ok (including empty timeline) · 2 usage · 3 broker does not serve `/events-list` / transport · 4 broker store failure.

```text
$ fleet queue timeline 19479
task 19479 timeline:
  claimed          2026-11-23T04:05:00Z  worker=repo-worker-code lane=code-a
  session_bound    2026-11-23T04:05:01Z  session=ses_01J9EXAMPLE0000000000000
  checkpoint       2026-11-23T04:20:00Z  stage=drafting
  served: ses_01J9EXAMPLE0000000000000 04:05:00Z–… (1 segment)
```

## fleet queue history

```text
usage: repo-queue history <task-id> [--worker <id>] [--json]
```

| Flag                     | Argument  | Default            | Meaning                            |
| ------------------------ | --------- | ------------------ | ---------------------------------- |
| `<task-id>` (positional) | task id   | required           | The task whose timeline to render. |
| `--worker`               | worker id | `repo-worker-code` | Attribution (as `timeline`).       |
| `--json`                 | boolean   | off                | JSON output (as `timeline`).       |

**Behaviour.** `history` is a pure alias of `timeline`: same function, same output, same exit codes. See `timeline` above for the full behaviour.

Exit codes: 0 ok · 2 usage · 3 broker does not serve `/events-list` / transport · 4 broker store failure.

```text
$ fleet queue history 19479 --json
{"task_id": 19479, "events": [{"kind": "claimed", "created_at": "2026-11-23T04:05:00Z"}], "served": []}
```

## fleet queue staleness

```text
usage: repo-queue staleness [--queue <name>] [--older-than-seconds <n>] [--lease-cap-seconds <n>] [--limit <n>]
```

| Flag                    | Argument     | Default | Meaning                                                                                          |
| ----------------------- | ------------ | ------- | ------------------------------------------------------------------------------------------------ |
| `--queue`               | queue name   | unset   | Restrict both reads to one queue. Unset = estate-wide.                                           |
| `--older-than-seconds`  | seconds      | 86400   | Staleness threshold: a task is stale when its last status-history transition is older than this. |
| `--lease-cap-seconds`   | seconds      | 900     | Lease-cap window: a claim is near its cap when its lease expires within this many seconds.       |
| `--limit`               | row cap      | 200     | Maximum rows per read. Clamped to 1..=50000.                                                     |

**Behaviour.** The stewardship cycle's staleness read (broker route `/staleness`), landed as a Q8 consumer of the queue-schema design (#14170). One call returns two canned reads as a single JSON object: `rows` — every actionable task (queued, claimed, parked, blocked, handed_off) whose last `task_history` transition is older than the threshold, oldest first, each with `last_history_id`, `last_kind`, `last_to_status`, `last_actor`, and `last_transition_at`; and `claims_near_lease_cap` — claimed tasks whose `lease_until` falls inside the lease-cap window (already-expired leases sort first). A task with no history row falls back to its `updated_at` as its last transition and reports a null `last_history_id`, so pre-history tasks stay visible instead of dropping out of the read. The response also carries the thresholds used and the broker's `now`, so a reader can age leases against the same clock the broker used.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue staleness --queue repo-health-code-pilot --older-than-seconds 3600
{"rows": [{"id": 21706, "queue": "repo-health-code-pilot", "status": "parked", "last_kind": "parked", "last_transition_at": "2026-11-25T07:01:55Z"}], "claims_near_lease_cap": [], "older_than_seconds": 3600, "lease_cap_seconds": 900, "queue": "repo-health-code-pilot", "now": "2026-11-25T09:05:00Z"}
```

## fleet queue needs-user

```text
usage: repo-queue needs-user [--queue <name>] [--state <s>] [--limit <n>]
```

| Flag      | Argument   | Default | Meaning                                                                            |
| --------- | ---------- | ------- | ---------------------------------------------------------------------------------- |
| `--queue` | queue name | unset   | Restrict to one queue. Unset = estate-wide.                                        |
| `--state` | text       | unset   | Restrict to questions whose metadata `question.state` equals this value.           |
| `--limit` | row cap    | 200     | Maximum rows. Clamped to 1..=50000.                                                |

**Behaviour.** The stewardship cycle's needs-user read (broker route `/needs-user`): blocked and parked tasks carrying a `question` metadata entry (the registry shape `{ledger_id, state}`), oldest last transition first. Each row carries the question object lifted out of the task's metadata, the task's `last_transition_at` from the status-history table, and `blocked_by_count_cache` — the count of the task's `blocked_deps` cache entries. The count is cache-derived until the Q6 slice normalises dependencies into `task_deps`; the canned query then reads its counts from `task_deps` instead.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue needs-user --state open
{"rows": [{"id": 22735, "queue": "repo-health-code-pilot", "status": "parked", "question": {"ledger_id": "q-0001", "state": "open"}, "last_transition_at": "2026-11-25T01:30:00Z", "blocked_by_count_cache": 1}], "queue": null}
```

## fleet queue signal

```text
usage: repo-queue signal [--queue <name>] [--reason <text>]
```

| Flag       | Argument   | Default                   | Meaning                                                                                                                                                                                |
| ---------- | ---------- | ------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--queue`  | queue name | `repo-health-code-shadow` | The ordinary queue resolution — flag → env `REPO_QUEUE_DEFAULT_QUEUE` → the shadow default. (Unlike `claim`/`list`, signal is NOT unscoped; unlike `enqueue`, it does have a default.) |
| `--reason` | text       | `manual_signal`           | Reason recorded in the signal object.                                                                                                                                                  |

**Behaviour.** Asks the broker `/signal-count {queue}` for `queued_count` + `oldest_queued_at`, then writes a wake-up signal file for the queue's consumers: a JSON object `{"queue", "generation", "queued_count", "oldest_queued_at", "updated_at", "reason"}` where `generation` is one more than the max generation found in the existing signal/state files for that queue. Signal file path: env `REPO_QUEUE_SIGNAL_PATH`, else `$HOME/workspace/goals/keep-the-four-repos-healthy/hidden_files/repo-queue-signal.json`, with a sibling `repo-queue-signal-state.json` carrying `{queue, generation, updated_at}`. Both writes are atomic (tmp + rename); a write failure dies exit 3. The printed output is the signal object itself, exit 0.

Exit codes: 0 ok · 2 usage · 3 transport / unwritable signal file · 4 broker store failure.

```text
$ fleet queue signal --queue repo-health-code --reason "manual wake for the book build"
{"queue": "repo-health-code", "generation": 42, "queued_count": 12, "oldest_queued_at": "2026-11-23T03:58:00Z", "updated_at": "2026-11-23T04:30:00Z", "reason": "manual wake for the book build"}
```

## fleet queue models

```text
usage: repo-queue models [--days <n>] [--json]
```

| Flag     | Argument     | Default | Meaning                                                                                                             |
| -------- | ------------ | ------- | ------------------------------------------------------------------------------------------------------------------- |
| `--days` | integer days | 7       | Rollup window. **Clamped** to 1..=30 (note: clamped, unlike the reject-don't-clamp duration/limit flags elsewhere). |
| `--json` | boolean      | off     | Print an object including `"days"` and the per-model rows instead of the human table.                               |

**Behaviour.** Rolls up serving-model stamps across tasks: which models worked which tasks, joined from claim stamps + timeline binding windows + the route ledger. Output: human table headed `model rollup — last <n>d (since <ts>)`, or with `--json` an object including `"days"` and the per-model rows. If the broker's `/events-list` doesn't serve the cross-task kinds, it prints `repo-queue models: this broker does not serve cross-task …` and a 50000-row scan cap produces a warning line rather than silent truncation.

Exit codes: 0 ok · 2 usage · 3 transport / cross-task events not served · 4 broker store failure.

```text
$ fleet queue models --days 7
model rollup — last 7d (since 2026-11-16T04:30:00Z)
model                     tasks  completed
claude-sonnet (example)   41     38
```

## fleet queue outbox

```text
usage: repo-queue outbox <enqueue|list|metrics|claim|resolve|drain> ...
```

The creation outbox is the queue for **GitHub object creation** (issues, PRs, comments) that must survive crashes and rate limits: producers enqueue, a single serial drainer creates through `gh`, verifies by read-back, and resolves. Pilot producer: `new-issue-creator-code-a`. There is no dual-write and no automatic fallback to direct creation — a broker failure stops the drain visibly.

State machine: kinds `issue | pull_request | issue_comment | pr_comment`; states `pending | leased | uncertain | done | blocked`. Constants: claim lease **120 s**, min gap between creates **2 s**, hourly create budget **300**, retry backoff base **30 s** cap **3600 s**, circuit breaker opens after **3** consecutive rate limits for **300 s**, default max attempts **8**. Entries carry a `fleet-outbox:<id>` marker in the created object's body so the drainer can reconcile by search/read-back.

Each subverb below follows the reference format. All outbox subverbs print the full broker envelope JSON unless noted.

### fleet queue outbox enqueue

```text
usage: repo-queue outbox enqueue [--repo <r>] [--kind <k>] [--dedup-key <d>] [--producer <p>] [--payload <json> | --payload-file <f>]
```

| Flag             | Argument                                         | Default                       | Meaning                                                                     |
| ---------------- | ------------------------------------------------ | ----------------------------- | --------------------------------------------------------------------------- |
| `--repo`         | repo                                             | unset                         | Target repo for the object to create.                                       |
| `--kind`         | `issue\|pull_request\|issue_comment\|pr_comment` | unset                         | Object kind.                                                                |
| `--dedup-key`    | key                                              | unset                         | Dedupe key; the broker table has a unique index on `(repo, dedup_key)`.     |
| `--producer`     | producer name                                    | unset                         | Producing identity (pilot: `new-issue-creator-code-a`).                     |
| `--payload`      | JSON                                             | **required (one of the two)** | Creation payload inline. Neither `--payload` nor `--payload-file` → exit 2. |
| `--payload-file` | file path                                        | **required (one of the two)** | Creation payload from a file.                                               |

**Behaviour.** POST `/outbox-enqueue`; prints the full broker envelope JSON. The entry lands in state `pending`.

Exit codes: 0 ok · 2 usage / missing payload · 3 transport · 4 broker store failure.

```text
$ fleet queue outbox enqueue --repo developing-today/code --kind issue --dedup-key book-13165 --producer new-issue-creator-code-a --payload '{"title":"Fleet book: queue chapters","body":"… fleet-outbox:pending"}'
{"ok": true, "row": {"id": 77, "state": "pending", "kind": "issue", "repo": "developing-today/code"}}
```

### fleet queue outbox list

```text
usage: repo-queue outbox list [--repo <r>] [--state <s>]
```

| Flag      | Argument                                    | Default | Meaning          |
| --------- | ------------------------------------------- | ------- | ---------------- |
| `--repo`  | repo                                        | unset   | Filter by repo.  |
| `--state` | `pending\|leased\|uncertain\|done\|blocked` | unset   | Filter by state. |

**Behaviour.** POST `/outbox-list`; prints the envelope.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue outbox list --state pending
{"ok": true, "rows": [{"id": 77, "state": "pending", "kind": "issue", "repo": "developing-today/code"}]}
```

### fleet queue outbox metrics

```text
usage: repo-queue outbox metrics
```

| Flag     | Argument | Default | Meaning         |
| -------- | -------- | ------- | --------------- |
| _(none)_ | —        | —       | Takes no flags. |

**Behaviour.** POST `/outbox-metrics` with `{}`; prints the envelope (counts by state/kind plus circuit-breaker state).

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue outbox metrics
{"ok": true, "pending": 1, "leased": 0, "uncertain": 0, "done": 128, "blocked": 0, "circuit_open": false}
```

### fleet queue outbox claim

```text
usage: repo-queue outbox claim [--owner <name>]
```

| Flag      | Argument   | Default          | Meaning                                          |
| --------- | ---------- | ---------------- | ------------------------------------------------ |
| `--owner` | owner name | `outbox-drainer` | Claiming drainer identity. Claim lease is 120 s. |

**Behaviour.** Claims the oldest due entry; prints the envelope (its `row` is null when idle).

Exit codes: 0 ok (including idle, `row: null`) · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue outbox claim
{"ok": true, "row": {"id": 77, "state": "leased", "kind": "issue", "repo": "developing-today/code"}}
```

### fleet queue outbox resolve

```text
usage: repo-queue outbox resolve <id> [--outcome done|rate_limited|uncertain|failed] [--github-number <n>] [--github-url <u>] [--retry-after <s>] [--error <text>]
```

| Flag                | Argument                                | Default             | Meaning                                                                   |
| ------------------- | --------------------------------------- | ------------------- | ------------------------------------------------------------------------- |
| `<id>` (positional) | entry id                                | required            | The leased entry to resolve. Missing `<id>` → exit 2 with the usage line. |
| `--outcome`         | `done\|rate_limited\|uncertain\|failed` | **none (see note)** | Resolution outcome.                                                       |
| `--github-number`   | integer                                 | unset               | Created object's number, for `done`.                                      |
| `--github-url`      | URL                                     | unset               | Created object's URL, for `done`.                                         |
| `--retry-after`     | seconds                                 | unset               | Backoff hint for `rate_limited`.                                          |
| `--error`           | text                                    | unset               | Error text for `failed`/`uncertain`.                                      |

**Behaviour.** POST `/outbox-resolve` for the leased entry, recording the outcome and (for `done`) the created object's identity for read-back reconciliation.

Known discrepancy: `--outcome` has **no client-side default or validation** — omitted, it sends the empty string and the broker decides. The usage line presents it as a normal optional flag with a closed value set; the closed set is enforced, if at all, broker-side only. This chapter documents the source behaviour.

Exit codes: 0 ok · 2 usage / missing `<id>` · 3 transport · 4 broker store failure.

```text
$ fleet queue outbox resolve 77 --outcome done --github-number 13170 --github-url https://github.com/developing-today/code/issues/13170
{"ok": true, "row": {"id": 77, "state": "done", "github_number": 13170}}
```

### fleet queue outbox drain

```text
usage: repo-queue outbox drain [--owner <name>] [--max <n>]
```

| Flag      | Argument   | Default          | Meaning                                    |
| --------- | ---------- | ---------------- | ------------------------------------------ |
| `--owner` | owner name | `outbox-drainer` | Drainer identity used for claims.          |
| `--max`   | integer    | 20               | Max items to drain this run, floored at 1. |

**Behaviour.** The pilot drainer itself. Loop: claim → if the entry is `uncertain`, reconcile by marker first (search + strongly-consistent list scan; a lookup that _could not check_ keeps the entry uncertain with the error class recorded and **never re-creates**) → create via `gh` (pacing: ≥2 s between creates; the broker enforces circuit + hourly budget at claim) → verify by read-back → resolve. Per-item output lines are JSON: `{"id","outcome":"done"|"uncertain"|"rate_limited"|"failed"|"reconciled"|"lookup_unchecked", …}`; when the claim comes back empty it prints `{"drained": k, "idle": true, "claim": <envelope>}` and exits 0; after `--max` items, `{"drained": k, "idle": false}`. Create outcomes map: created+verified → `done`; created but read-back failed or ambiguous output → `uncertain`; 403/429/rate-limit → `rate_limited`; other failure → `failed`.

Exit codes: 0 drained / idle · 2 usage · 3 transport (a broker failure stops the drain visibly) · 4 broker store failure.

```text
$ fleet queue outbox drain --max 5
{"id": 77, "outcome": "done", "github_number": 13170}
{"drained": 1, "idle": true, "claim": {"ok": true, "row": null}}
```

## fleet queue stack

```text
usage: repo-queue stack <record|list|transition|restack|replay> ...
```

A stack is a recorded, ordered set of **merge intents** — finished branches with their authoring lane/worker/session — stored in the broker's `merge_intents` table (unique on `(stack_id, position)` and `(repo, branch)`). Intent states: `intended → replaying → landed`, plus `conflicted` and `abandoned` — five states in total; legality of every move is decided by the pure core's transition table, never in SQL. A conflicted intent returns only through its recorded authoring lane.

Each subverb below follows the reference format.

### fleet queue stack record

```text
usage: repo-queue stack record --stack <id> --repo <r> --branch <b> [--base-branch <b>] [--head-sha <sha>] [--issue <n>] [--pr <n>] --lane <l> [--worker <id>] [--session-id <id>]
```

| Flag            | Argument     | Default                              | Meaning                                                                 |
| --------------- | ------------ | ------------------------------------ | ----------------------------------------------------------------------- |
| `--stack`       | stack id     | **required**                         | Stack to append to. Missing → exit 2.                                   |
| `--repo`        | repo         | **required**                         | The intent's repo. Missing → exit 2.                                    |
| `--branch`      | branch name  | **required**                         | The finished branch. Missing → exit 2.                                  |
| `--base-branch` | branch name  | unset                                | Base the branch was authored against.                                   |
| `--head-sha`    | commit SHA   | unset                                | Recorded head SHA.                                                      |
| `--issue`       | issue number | unset                                | Linked issue.                                                           |
| `--pr`          | PR number    | unset                                | Linked PR.                                                              |
| `--lane`        | lane id      | **required**                         | Authoring lane (the owner lane for conflict routing). Missing → exit 2. |
| `--worker`      | worker id    | caller's worker (`repo-worker-code`) | Authoring worker.                                                       |
| `--session-id`  | session id   | unset                                | Authoring session.                                                      |

**Behaviour.** The append is fail-fast validated client-side with the _same_ pure-core rules the broker applies, and the broker additionally validates the (lane, worker) pair against the lane registry — an unregistered owner lane is refused. POST `/stack-record`; prints the row.

Exit codes: 0 ok · 2 usage / missing required flag / validation failure · 3 transport · 4 broker store failure / broker refusal.

```text
$ fleet queue stack record --stack book-13165 --repo developing-today/code --branch writer/w2-queue --lane code-a
{"stack_id": "book-13165", "position": 0, "repo": "developing-today/code", "branch": "writer/w2-queue", "state": "intended"}
```

### fleet queue stack list

```text
usage: repo-queue stack list <stack-id> [--stack <id>] [--worker <id>]
```

| Flag                      | Argument  | Default                                | Meaning                                                                          |
| ------------------------- | --------- | -------------------------------------- | -------------------------------------------------------------------------------- |
| `<stack-id>` (positional) | stack id  | one of positional / `--stack` required | The stack to list.                                                               |
| `--stack`                 | stack id  | unset                                  | Alternative spelling of the positional. If both are given and disagree → exit 2. |
| `--worker`                | worker id | `repo-worker-code`                     | Caller identity.                                                                 |

**Behaviour.** POST `/stack-list`; prints the derived stack state (the full state is answered from the database alone).

Exit codes: 0 ok · 2 usage / disagreeing ids · 3 transport · 4 broker store failure.

```text
$ fleet queue stack list book-13165
{"stack_id": "book-13165", "intents": [{"position": 0, "branch": "writer/w2-queue", "state": "intended"}]}
```

### fleet queue stack transition

```text
usage: repo-queue stack transition --stack <id> --position <n> --to <state> [--pr <n>] [--conflict-detail <text>] [--worker <id>]
```

| Flag                | Argument     | Default                             | Meaning                                                                                                                                   |
| ------------------- | ------------ | ----------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| `--stack`           | stack id     | **required**                        | The stack. Missing → exit 2.                                                                                                              |
| `--position`        | integer      | **required**                        | The intent's position. Missing → exit 2.                                                                                                  |
| `--to`              | intent state | **required**                        | Target state; must parse as an intent state (`intended`, `replaying`, `landed`, `conflicted`, `abandoned`). Missing/unparseable → exit 2. |
| `--pr`              | PR number    | unset                               | PR associated with the move.                                                                                                              |
| `--conflict-detail` | text         | **required when `--to conflicted`** | What the authoring lane must resolve. A move to `conflicted` without it → exit 2.                                                         |
| `--worker`          | worker id    | `repo-worker-code`                  | Caller identity.                                                                                                                          |

**Behaviour.** An illegal move is a broker refusal, not a write. POST `/stack-transition`.

Exit codes: 0 ok · 2 usage / missing required flag / conflicted without detail · 3 transport · 4 broker store failure / illegal move refused.

```text
$ fleet queue stack transition --stack book-13165 --position 0 --to replaying
{"stack_id": "book-13165", "position": 0, "state": "replaying"}
```

### fleet queue stack restack

```text
usage: repo-queue stack restack --stack <id> --position <n> --base <b> [--head-sha <sha>] [--worker <id>]
```

| Flag         | Argument    | Default            | Meaning                                  |
| ------------ | ----------- | ------------------ | ---------------------------------------- |
| `--stack`    | stack id    | **required**       | The stack. Missing → exit 2.             |
| `--position` | integer     | **required**       | The intent's position. Missing → exit 2. |
| `--base`     | branch name | **required**       | The new base branch. Missing → exit 2.   |
| `--head-sha` | commit SHA  | unset              | New head SHA after the restack.          |
| `--worker`   | worker id   | `repo-worker-code` | Caller identity.                         |

**Behaviour.** Acknowledges that an intent's author restacked its branch off a withdrawn base; the broker re-plans with the core's `plan_restack` (taint rules bind there) and refreshes the recorded `base_branch`. POST `/stack-restack`.

Exit codes: 0 ok · 2 usage / missing required flag · 3 transport · 4 broker store failure.

```text
$ fleet queue stack restack --stack book-13165 --position 0 --base main
{"stack_id": "book-13165", "position": 0, "base_branch": "main", "state": "intended"}
```

### fleet queue stack replay

```text
usage: repo-queue stack replay [--stack <id>] [--worker <id>] [--repo-dir <dir>] [--dry-run]
```

| Flag         | Argument  | Default                  | Meaning                                                                         |
| ------------ | --------- | ------------------------ | ------------------------------------------------------------------------------- |
| `--stack`    | stack id  | **required in practice** | The stack to replay. The usage line brackets it optional, but missing → exit 2. |
| `--worker`   | worker id | `repo-worker-code`       | Caller identity.                                                                |
| `--repo-dir` | directory | `.`                      | Checkout to replay in.                                                          |
| `--dry-run`  | boolean   | off                      | Print the plan JSON — exactly what a live pass would do — and change nothing.   |

**Behaviour.** The replay executor: walks the stack's recorded merge order bottom-up and lands it — rebase onto new main, push (force-with-lease pinned to the verified pre-rebase tip), open PR, merge, mark landed, post the outcome note — one intent at a time. `--dry-run` prints `{"stack_id","dry_run":true,"plan":[{"position","branch","action"}]}`. Refuses a checkout with somebody else's rebase already in progress. Replay never resolves a conflict itself.

Exit codes: 0 landed/idle (an empty plan prints a `note` explaining the stack is empty, finished, or blocked at a conflicted head) · 2 usage / missing `--stack` · **3** any other step failure — the intent is left `replaying` and a later `stack replay` resumes it (including the case where the conflict itself could not be recorded) · **4** a rebase/merge conflict was recorded `conflicted` and routed to its authoring lane (the JSON carries `conflict {position, detail, route {owner_lane, owner_worker, owner_session, repo, branch, base_branch, issue_number}}`).

```text
$ fleet queue stack replay --stack book-13165 --dry-run
{"stack_id": "book-13165", "dry_run": true, "plan": [{"position": 0, "branch": "writer/w2-queue", "action": "rebase+merge"}]}
$ fleet queue stack replay --stack book-13165
{"stack_id": "book-13165", "landed": [0], "state": "landed"}
```
