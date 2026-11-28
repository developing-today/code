# fleet queue: lifecycle verbs

The lifecycle verbs move a claimed task through its life: attribution (`lane`, `bind`), lease care (`heartbeat`, `checkpoint`), waiting (`park`, `wake`, `block`, `unblock`, `dep-recheck`, `quarantine-recheck`, `repair`, `resolution-backfill`), and the terminal/revival verbs (`cancel`, `revive`, `fail`, `complete`, `verify-disposition`).

Shared rules, stated in [fleet queue: core verbs](queue-core.md) and not repeated in full here: the global exit-code vocabulary (0 ok, 1 gate/not-found, 2 usage, 3 transport/401, 4 broker store failure, 5 no-claim/no-row, 6 fallback switch); the `--worker` default `repo-worker-code`; the `--lane` resolution (flag → env `REPO_QUEUE_LANE` → null, broker checks worker alone); and `--progress` / `--progress-file` mutual exclusion.

**Holder-checked verbs.** `lane`, `bind`, `heartbeat`, `checkpoint`, `park`, `release`, `block`, `fail`, `complete` are holder-checked broker-side: the row must be claimed by this worker (and lane, when one is supplied) and non-terminal, else the envelope has no row and the client dies **exit 5** with `<verb>: no matching claimed row (task not claimed by this worker, or already terminal)`. On success they print the broker's returned task row as JSON and exit 0. Several lifecycle verbs answer `{task, lane_ledger}` (and sometimes `follow_on_task_id`) instead of the bare row when the broker includes a ledger.

**The person-wait guard (FDR-0013)** applies to `park` and `block` and is described once under `block` below; `park` references it.

Sources: [commands_lifecycle.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/commands_lifecycle.rs), [commands_query.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/commands_query.rs), [verify_disposition.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/verify_disposition.rs), [disposition.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/disposition.rs), [person_wait.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/person_wait.rs), [turso_queue lifecycle](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/turso_queue/lifecycle.rs).

## fleet queue lane

```text
usage: repo-queue lane <task-id> [--worker <id>] [--exclusion-reason <text>] [--opencode-reason <text>] [--limit-platform <p>] [--limit-reason <text>] [--clear-platform-limit]
```

| Flag                     | Argument        | Default            | Meaning                                                                                     |
| ------------------------ | --------------- | ------------------ | ------------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id         | required           | The claimed task to report lane progress on.                                                |
| `--worker`               | worker id       | `repo-worker-code` | Holder identity.                                                                            |
| `--exclusion-reason`     | text            | unset              | Recorded on the lane ledger as `exclusion_reason`.                                          |
| `--opencode-reason`      | text            | unset              | Recorded on the lane ledger as `opencode_reason`.                                           |
| `--limit-platform`       | platform family | unset              | Record a platform limitation (`platform_limit`). Requires `--limit-reason` (see Behaviour). |
| `--limit-reason`         | text            | unset              | The reason for the platform limitation (`platform_limit_reason`).                           |
| `--clear-platform-limit` | boolean         | off                | Clear a recorded platform limitation.                                                       |

**Behaviour.** Reports lane progress on a task. Broker route `/lane`; writes the `repo_task_lane_ledger` row for the task: `assigned_lane` (the caller's lane context), `exclusion_reason`, `opencode_reason`, `platform_limit`, `platform_limit_reason`. A platform limitation exists only with its recorded reason: the broker refuses a `--limit-platform` with no `--limit-reason`. `--clear-platform-limit` clears a recorded limitation. Prints the returned task row as JSON.

Exit codes: 0 ok · 2 usage / platform limit without a reason · 3 transport · 4 broker store failure · 5 no matching claimed row.

```text
$ fleet queue lane 19479 --worker repo-worker-code --limit-platform macos --limit-reason "needs the macOS runner pool"
{"id": 19479, "status": "claimed", "lane_ledger": {"assigned_lane": "code-a", "platform_limit": "macos", "platform_limit_reason": "needs the macOS runner pool"}}
```

## fleet queue bind

```text
usage: repo-queue bind <task-id> [--worker <id>] --session-id <id> [--launched-by <who>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>]
```

| Flag                     | Argument      | Default             | Meaning                                                                                                                        |
| ------------------------ | ------------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------ |
| `<task-id>` (positional) | task id       | required            | The claimed task to bind a session to.                                                                                         |
| `--worker`               | worker id     | `repo-worker-code`  | Holder identity.                                                                                                               |
| `--session-id`           | session id    | **required**        | The session to bind. Missing → exit 2.                                                                                         |
| `--launched-by`          | who           | `opencode-claimant` | Who launched the bound session; recorded on the lane ledger.                                                                   |
| `--model-provider`       | provider name | unset               | Serving model provider for the model stamp.                                                                                    |
| `--model-id`             | model id      | unset               | Serving model id for the model stamp.                                                                                          |
| `--model-variant`        | variant name  | unset               | Serving model variant for the model stamp. Explicit `--model-*` wins; else the stamp is derived from the session's own record. |

**Behaviour.** Binds a session to a claimed task (`/bind`): records `session_id` + `launched_by` on the lane ledger. The model stamp works exactly as on `claim` (see [core verbs](queue-core.md)). The client also appends `session_bound` (and `model_stamp` when a stamp exists) timeline events, trigger `bind`. Prints the returned task row as JSON.

Exit codes: 0 ok · 2 usage / missing `--session-id` · 3 transport · 4 broker store failure · 5 no matching claimed row.

```text
$ fleet queue bind 19479 --worker repo-worker-code --session-id ses_01J9EXAMPLE0000000000000
{"id": 19479, "status": "claimed", "lane_ledger": {"assigned_lane": "code-a", "session_id": "ses_01J9EXAMPLE0000000000000", "launched_by": "opencode-claimant"}}
```

## fleet queue heartbeat

```text
usage: repo-queue heartbeat <task-id> [--worker <id>] [--lane <id>] [--extend-seconds <n>] [--progress <p>] [--progress-file <f>]
```

| Flag                     | Argument        | Default                          | Meaning                                                                              |
| ------------------------ | --------------- | -------------------------------- | ------------------------------------------------------------------------------------ |
| `<task-id>` (positional) | task id         | required                         | The claimed task whose lease to refresh.                                             |
| `--worker`               | worker id       | `repo-worker-code`               | Holder identity.                                                                     |
| `--lane`                 | lane id         | env `REPO_QUEUE_LANE`, else null | Holder lane check.                                                                   |
| `--extend-seconds`       | integer seconds | 900 (15 min)                     | Minimum renewal window from now. Range 1..=86400, exit 2 outside.                    |
| `--progress`             | text/JSON       | unset                            | Progress value merged into the row. Mutually exclusive with `--progress-file`.       |
| `--progress-file`        | file path       | unset                            | File whose text is read as the progress value. Mutually exclusive with `--progress`. |

**Behaviour.** The Turso broker keeps the later of the current deadline and `now + extend_seconds`. It writes the resulting deadline to the task and its lease in one SQLite statement and refreshes `heartbeat_at`, including on an early heartbeat. A changed claim observed during renewal returns exit 5. Brokers built before [#13553](https://github.com/developing-today/code/issues/13553) replace the deadline with `now + extend_seconds` and can shorten a longer lease. Check the serving broker revision when interpreting older observations. `--progress`/`--progress-file` optionally merge a progress value into the row. Heartbeat goes through the fallback-switch path: a primary-store failure switches the work to the GitHub fallback, exit 6 (see [the fallback queue](queue-fallback.md)). Prints the row + `mode_line`.

Exit codes: 0 ok · 2 usage / out-of-range `--extend-seconds` / both progress flags · 3 transport · 4 broker store failure · 5 no matching claimed row · 6 fallback switch.

```text
$ fleet queue heartbeat 19479 --worker repo-worker-code --lane code-a
{"id": 19479, "status": "claimed", "lease_until": "2026-11-23T04:35:00Z", "mode_line": {"verb": "heartbeat", "mode": "normal", "route": "cloud"}}
```

## fleet queue checkpoint

```text
usage: repo-queue checkpoint <task-id> [--worker <id>] [--lane <id>] [--stage <s>] [--progress <p>] [--progress-file <f>]
```

| Flag                     | Argument   | Default                          | Meaning                                                                              |
| ------------------------ | ---------- | -------------------------------- | ------------------------------------------------------------------------------------ |
| `<task-id>` (positional) | task id    | required                         | The claimed task to checkpoint.                                                      |
| `--worker`               | worker id  | `repo-worker-code`               | Holder identity.                                                                     |
| `--lane`                 | lane id    | env `REPO_QUEUE_LANE`, else null | Holder lane check.                                                                   |
| `--stage`                | stage name | **required**                     | The checkpoint stage. Missing → exit 2.                                              |
| `--progress`             | text/JSON  | unset                            | Progress value merged into the row. Mutually exclusive with `--progress-file`.       |
| `--progress-file`        | file path  | unset                            | File whose text is read as the progress value. Mutually exclusive with `--progress`. |

**Behaviour.** Records progress + stage without touching the lease. The broker (`/checkpoint`) merges the progress (default `{}`) into the task row with `progress.last_checkpoint = {stage, at}` stamped in; upserts the per-worker KV row `checkpoints(worker_id, 'task:<id>') = {stage, progress, at}`; writes a `checkpoint` event. Fallback-switch path, like heartbeat: a primary-store failure switches, exit 6. Prints row + `mode_line`.

Help: the usage writes `--stage` bare — it is **required** — missing → exit 2 `checkpoint: --stage is required`.

Exit codes: 0 ok · 2 usage / missing `--stage` / both progress flags · 3 transport · 4 broker store failure · 5 no matching claimed row · 6 fallback switch.

```text
$ fleet queue checkpoint 19479 --worker repo-worker-code --lane code-a --stage drafting --progress '{"chapters": 2}'
{"id": 19479, "status": "claimed", "progress": {"last_checkpoint": {"stage": "drafting", "at": "2026-11-23T04:20:00Z"}}, "mode_line": {"verb": "checkpoint", "mode": "normal", "route": "cloud"}}
```

## fleet queue park

```text
usage: repo-queue park <task-id> [--worker <id>] [--lane <id>] --reason <text> [--delay-seconds <n>] [--not-before <ts>] [--progress <p>] [--progress-file <f>] [--ready-condition <c>] [--execution <id>] [--required-budget-minutes <n>] [--hands <kind> --action <text>]
```

| Flag                        | Argument                                          | Default                                                        | Meaning                                                                                                                                                                                                        |
| --------------------------- | ------------------------------------------------- | -------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<task-id>` (positional)    | task id                                           | required                                                       | The claimed task to park.                                                                                                                                                                                      |
| `--worker`                  | worker id                                         | `repo-worker-code`                                             | Holder identity.                                                                                                                                                                                               |
| `--lane`                    | lane id                                           | env `REPO_QUEUE_LANE`, else null                               | Holder lane check.                                                                                                                                                                                             |
| `--reason`                  | text                                              | **required**                                                   | Why the task is parked. Missing → exit 2.                                                                                                                                                                      |
| `--delay-seconds`           | integer seconds                                   | broker default 1200 when neither delay nor not-before is given | Relative wait. Range 1..=604800 (7 days); 0 is rejected by design — "a park that waits zero seconds is a requeue, and a requeue already has its own verb (`release`)". Mutually exclusive with `--not-before`. |
| `--not-before`              | RFC 3339 timestamp                                | unset                                                          | Absolute wake time; must be non-empty and ≤366 days in the future. Mutually exclusive with `--delay-seconds`.                                                                                                  |
| `--progress`                | text/JSON                                         | unset                                                          | Progress merged into the row; may carry `ready_condition` (see Behaviour). Mutually exclusive with `--progress-file`.                                                                                          |
| `--progress-file`           | file path                                         | unset                                                          | File whose text is read as the progress value.                                                                                                                                                                 |
| `--ready-condition`         | JSON object                                       | derived (see Behaviour)                                        | What "done waiting" means. Non-JSON → exit 2; non-object JSON → exit 2.                                                                                                                                        |
| `--execution`               | execution id                                      | unset                                                          | Folded into an execution record `{lane, lane_id?, session_id, fallback_reason?}`.                                                                                                                              |
| `--required-budget-minutes` | integer minutes                                   | broker-derived for capacity waits (see Behaviour)              | The minimum remaining shift budget a future claimer must declare. Non-positive values record no requirement.                                                                                                   |
| `--hands`                   | `physical\|interactive-login\|payment\|signature` | unset                                                          | Hands kind for a valid person-wait (FDR-0013); must come with `--action`.                                                                                                                                      |
| `--action`                  | text                                              | unset                                                          | One sentence a person can do in five minutes; required with `--hands` on a person-wait.                                                                                                                        |

**Behaviour.** Parks a claimed task: status becomes `parked`, out of the claimable pool until its `not_before` is due, its ready-condition is released by the revalidation sweeper, or `wake` short-circuits the wait.

- **Ready-condition kinds** the revalidation sweeper can verify from DB state: `task_terminal {task_id}`, `task_status {task_id, status}`, `event_seen {task_id?, event_kind}`, `time {until}`. External kinds (`ci`, `pr_merged`, `issue_closed`, `person_answer`, `review`, …) are recorded faithfully but a DB sweeper cannot verify them. With no explicit condition the broker derives one from `progress.ready_condition`, else `{"kind":"time","until":<not_before>}`.
- **wait_kind** is derived from the condition kind at park time: `time`→`timer`; `triage`→`triage`; `ci|ci_green|pr_merged|issue_closed|upstream_pr`→`ci`; `review|codex_review|pr_review|person_answer`→`review`; `capacity|quota_reset`→`capacity`; anything else→`external`. `park_kind` records `relative` vs `absolute`.
- **Broker wait floors:** whatever delay a lane asks for, a `triage` wait is held at least **6 h (21600 s)** and a `ci` or `capacity` wait at least **1 h (3600 s)** — a requested wake earlier than the floor is bumped to the floor; timer/review/external waits keep the caller's delay. (Lanes re-discovering an unchanged wait every 20–25 min was the park churn this kills.) A park whose recorded stage is `awaiting_triage` is a triage wait whatever condition the caller attached.
- **Handoff parks / the sink:** a non-triage park that is a _handoff_ (the condition defaulted, per the broker's rule) is routed `to_sink` — parked into the handed-off sink where `claim --handoff` / by-id claims collect it, rather than sitting as an ordinary timed park.
- **`--required-budget-minutes`:** for a `capacity` wait the broker defaults it from the packet's `timebox_minutes` when the flag is absent.
- **Person-wait guard (FDR-0013):** a park whose ready-condition kind is `person_answer`, or whose reason text names the owner as the release, is refused client-side (exit 2, before broker contact) unless it is a valid hands item; a valid hands item is stamped into `progress.hands = {kind, action}`. See the full guard under `block` below.
- Park stamps `last_checked_at = now`, resets `check_outcome`, and the **park revalidation sweeper** later records `check_outcome` ∈ `confirmed` (condition unmet; park untouched), `released` (condition met → status `queued`, `not_before` null, `park_released` event), or `stale` (cannot verify).
- **Revalidation mode (#16006):** the scheduled sweep runs LIVE by default — a satisfied broker-condition park (`task_terminal`, `task_status`, `event_seen`) is released to `queued` with a `park_released` event. Pass `observe_only=true` for a non-mutating observe cycle (releases logged as `would_release`). The home-manager sweeper opts out by writing `observe` to `~/.config/queue-broker/revalidator-mode`; an absent file (or `enabled`) keeps the live default. `person_answer` parks never release via the sweeper — they still require the person's `/wake`.
- **Live verification, 2026-11-28 (lane oc-code-08, #16006 criterion 4):** manual live `/sweep {"revalidate":true,"observe_only":false,"min_age_seconds":86400,"limit":40}` (expire/prune/dep/quarantine riders off; those run on their own timers) against the deployed broker. Observe pass first: 163 checked → 21 `would_release`, 0 mutations. Live passes: 63 checked → 5 `released` (tasks 24407, 25348, 25428, 25539, 25461 — every one from the observe `would_release` set). Within minutes 24407/25539 were claimed, 25348/25461 reached done, and 25428 was claimed then re-parked with a new wait: resumed work, not stranded rows. Parked census 319 → 306 across the run; every departure is attributed (5 sweeper releases, 1 external claim, other-lane dispositions in the window; 7 new parks by other lanes). The sweep mutated no unrelated row. Caveats: the laptop's installed `fleet-queue-sweeper` generation still sends explicit `observe_only=true` (mode file absent, pre-#16006 script) — the scheduled 05:17 path goes live on the next home-manager switch, deliberately not run from this lane. Four satisfied parks (23023, 21693, 23789, 21659) remain parked with freshly stamped `last_checked_at` and await the next live cycle.
- **Follow-up census, 2026-11-28 (lane code-g, #16006 criterion 4):** read-only observe `/sweep {"expire":false,"revalidate":true,"observe_only":true,"min_age_seconds":86400,"limit":40,"dep_recheck":false}` → 2 checked, both `stale`, 0 mutations. The four parks named above as awaiting the next live cycle have all moved without a second live sweep: 23789 was released by the oc-code-08 live run and is now done (lane code-h; `progress.park_revalidation.outcome=released at 2026-11-28T03:41:46Z`); 21693 is claimed by lane code-j (lease to 08:11Z); 23023 and 21659 remain parked with active lanes (oc-code-04, `not_before` 06:37Z; oc-code-02, S5a+S5b landed as PR #16433). Installed `fleet-queue-sweeper` generation verified still pre-#16006 (defaults `mode="observe"`, mode file absent) — the scheduled 05:17 path goes live on the next home-manager switch, a hands step deliberately not run from this lane.
- Output: row (or `{task, lane_ledger[, follow_on_task_id]}`); a park closes the session binding window on the timeline.

Exit codes: 0 parked · 2 usage / missing `--reason` / both delay flags / bad ready-condition / person-wait refusal · 3 transport · 4 broker store failure · 5 no matching claimed row.

```text
$ fleet queue park 19479 --worker repo-worker-code --lane code-a --reason "waiting on CI for the linked PR" --ready-condition '{"kind":"ci","repo":"developing-today/code"}' --delay-seconds 600
{"id": 19479, "status": "parked", "wait_kind": "ci", "not_before": "2026-11-23T05:20:00Z"}
```

## fleet queue wake

```text
usage: repo-queue wake <task-id> [--worker <id>] [--reason <text>]
```

| Flag                     | Argument  | Default            | Meaning                                       |
| ------------------------ | --------- | ------------------ | --------------------------------------------- |
| `<task-id>` (positional) | task id   | required           | The parked task to wake.                      |
| `--worker`               | worker id | `repo-worker-code` | Caller identity.                              |
| `--reason`               | text      | empty string       | Why the wait is over; recorded with the wake. |

**Behaviour.** Releases a broker-held parked wait _now_ that its condition is satisfied: the broker flips the row back to `queued`. Canonical caller: the triage step waking an item it just triaged. Prints the row.

Exit codes: 0 woken · 2 usage · 3 transport · 4 broker store failure · 5 no parked row the broker can wake.

```text
$ fleet queue wake 19479 --reason "triaged; ready to work"
{"id": 19479, "status": "queued", "not_before": null}
```

## fleet queue block

```text
usage: repo-queue block <task-id> [--worker <id>] [--lane <id>] --reason-class <c> --blocker <b> [--progress <p>] [--progress-file <f>] [--ledger-id <id>] [--question <text>] [--context <text>] [--options-json <json>] [--links-json <json>] [--blocked-by-pr <n>] [--blocked-by-issue <n>] [--blocked-by-task <id>] [--blocked-by-ci <ref>] [--dep-recheck-seconds <n>] [--dep-max-ttl-seconds <n>] [--hands <kind> --action <text>]
```

| Flag                     | Argument                                          | Default                          | Meaning                                                                                                              |
| ------------------------ | ------------------------------------------------- | -------------------------------- | -------------------------------------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id                                           | required                         | The claimed task to block.                                                                                           |
| `--worker`               | worker id                                         | `repo-worker-code`               | Holder identity.                                                                                                     |
| `--lane`                 | lane id                                           | env `REPO_QUEUE_LANE`, else null | Holder lane check.                                                                                                   |
| `--reason-class`         | class name                                        | **required**                     | Free-form block class (contrast `fail`, whose `--reason-class` is a closed enum). Missing → exit 2.                  |
| `--blocker`              | text                                              | **required**                     | What blocks the task. Missing → exit 2.                                                                              |
| `--progress`             | text/JSON                                         | unset                            | Progress merged into the row. Mutually exclusive with `--progress-file`.                                             |
| `--progress-file`        | file path                                         | unset                            | File whose text is read as the progress value.                                                                       |
| `--ledger-id`            | id                                                | unset                            | Needs-drew payload: the PM ledger id (see Behaviour).                                                                |
| `--question`             | text                                              | unset                            | Needs-drew payload: the question for the owner. A hands item's `--action` serves as the question when none is given. |
| `--context`              | text                                              | unset                            | Needs-drew payload: optional context.                                                                                |
| `--options-json`         | JSON array                                        | unset                            | Needs-drew payload: array of `{"id","label"}` objects; bad shape → exit 2.                                           |
| `--links-json`           | JSON array                                        | unset                            | Needs-drew payload: array of strings; bad shape → exit 2.                                                            |
| `--blocked-by-pr`        | comma-separated `owner/repo#N` refs               | unset                            | Typed dependency: PRs that must land. Bad refs die client-side, exit 2.                                              |
| `--blocked-by-issue`     | comma-separated `owner/repo#N` refs               | unset                            | Typed dependency: issues that must close/resolve. Bad refs die client-side, exit 2.                                  |
| `--blocked-by-task`      | comma-separated bare task ids                     | unset                            | Typed dependency: queue tasks that must reach a terminal state.                                                      |
| `--blocked-by-ci`        | comma-separated `owner/repo#N` refs               | unset                            | Typed dependency: CI runs; here `N` is an Actions **run id**.                                                        |
| `--dep-recheck-seconds`  | integer seconds                                   | 900                              | How often the broker re-checks the deps. Clamped broker-side to 300..=3600.                                          |
| `--dep-max-ttl-seconds`  | integer seconds                                   | 259200 (3 days)                  | Max lifetime of a dep block before TTL expiry releases it. Broker floor 60 s, ceiling 7 days (604800 s).             |
| `--hands`                | `physical\|interactive-login\|payment\|signature` | unset                            | Hands kind for a valid person-wait (FDR-0013); must come with `--action`.                                            |
| `--action`               | text                                              | unset                            | One sentence a person can do in five minutes: a single line of ≤200 chars that must not itself ask for a decision.   |

**Behaviour.** Sets status `blocked`; the row keeps its claim identity in progress/ledger terms but is out of the claimable pool until unblocked, dep-cleared, or TTL-expired. Output: the row, exit 0. (The claim path also uses `/block` internally with reason-class `fallback_held` for the double-serve guard — see [core verbs](queue-core.md).) After a `/block`, the broker fire-and-forget raises the needs-drew alert for an escalated block, mirroring the client's own raise.

**Typed dependency blocks.** The parsed deps array sent to the broker is `[{kind:"pr"|"issue", repo, number} | {kind:"ci", repo, run_id} | {kind:"task", task_id}]` — never a prose block the broker cannot watch. The deps are stored on the task row in the **`blocked_deps`** column (TEXT, JSON) — the state the task calls `dep_blocked` when `blocked_deps` is non-empty — alongside `dep_recheck_seconds`, `dep_next_recheck_at`, and `dep_blocked_until`. The broker watches these deps and releases the task itself when they clear, or when the max TTL expires. Production cadence: the broker's sweep fires the dep recheck every minute; the `dep-recheck` verb is the manual trigger.

**Person-wait guard (FDR-0013).** A block whose reason class is one of `escalated-to-user`, `drew_decision`, `person-answer`, or whose blocker text contains an owner-wait phrase ("awaiting drew", "needs the owner", "owner decision", "waiting for the owner", … — an 18-phrase list), waits on a person and is **refused** (exit 2, before broker contact) unless it is a hands item: `--hands physical|interactive-login|payment|signature` **and** `--action "<one sentence a person can do in five minutes>"`. The refusal text tells the worker a choice is theirs to make. Passing `--hands`/`--action` on a block that does _not_ wait on a person is also an error. A valid hands item is stamped `progress.hands`. The same guard applies to `park`.

**Needs-drew payload.** Only for `--reason-class escalated-to-user` **with** `--ledger-id` and `--question`. Then the client stamps `progress.needs_drew.ledger_id`, sends a `needs_drew {ledger_id, question, context?, options, links}` object in the `/block` body, and _after the block lands_ posts the alert to the PM aggregator's alerts-intake (token resolved by name through fleet-secrets; value never printed). A failed alert post only warns on stderr — the block already succeeded and the verb still exits 0. Without ledger-id + question, an escalated block behaves exactly as before.

Exit codes: 0 blocked · 2 usage / missing `--reason-class` or `--blocker` / bad dep ref / bad options/links shape / person-wait refusal / hands flags on a non-person block · 3 transport · 4 broker store failure · 5 no matching claimed row.

```text
$ fleet queue block 19479 --worker repo-worker-code --lane code-a --reason-class dependency --blocker "waiting on PR developing-today/code#13100" --blocked-by-pr developing-today/code#13100
{"id": 19479, "status": "blocked", "blocked_deps": [{"kind": "pr", "repo": "developing-today/code", "number": 13100}], "dep_recheck_seconds": 900, "dep_blocked_until": "2026-11-26T04:20:00Z"}
```

## fleet queue unblock

```text
usage: repo-queue unblock <task-id> [--worker <id>] [--reason <text>] [--drop-dep <list>]
```

| Flag                     | Argument                        | Default            | Meaning                                                                                                                                                           |
| ------------------------ | ------------------------------- | ------------------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id                         | required           | The blocked task to release.                                                                                                                                      |
| `--worker`               | worker id                       | `repo-worker-code` | Caller identity; the broker audits who overrode.                                                                                                                  |
| `--reason`               | text                            | empty string       | Why the block is overridden; audited.                                                                                                                             |
| `--drop-dep`             | comma-separated `kind:ref` list | unset              | Surgical dep drop: `issue:owner/repo#N`, `pr:owner/repo#N`, `ci:owner/repo#RUN`, `task:ID`. Bad refs die client-side, exit 2; the broker re-validates every spec. |

**Behaviour.** The manual override: releases a blocked task now, dep-blocked or classic. The broker audits who overrode and what dep record was discarded.

- **Without `--drop-dep`:** clears the task's blocked state entirely (deps discarded).
- **With `--drop-dep <list>`:** surgical. Only the named entries are dropped from the task's `blocked_deps` record. **The task stays blocked on the remaining deps; dropping the last one releases it.**

**Quarantined rows:** the full override refuses a row that is still quarantined by the claim path, naming the quarantine and pointing at `fleet queue repair <id>` — unblock cannot clear a packet defect; repair the packet first. The `--drop-dep` path is unchanged (it leaves the row blocked).

Output: the row, exit 0.

Exit codes: 0 unblocked (or last dep dropped) · 2 usage / bad `--drop-dep` ref · 3 transport · 4 broker store failure · 5 no blocked row matches.

```text
$ fleet queue unblock 19479 --reason "PR merged manually; dep record is stale" --drop-dep pr:developing-today/code#13100
{"id": 19479, "status": "queued", "blocked_deps": []}
```

## fleet queue dep-recheck

```text
usage: repo-queue dep-recheck [--task <id>] [--limit <n>]
```

| Flag      | Argument | Default               | Meaning                                                                                                |
| --------- | -------- | --------------------- | ------------------------------------------------------------------------------------------------------ |
| `--task`  | task id  | unset                 | Force one task's recheck instead of a full pass.                                                       |
| `--limit` | integer  | no documented default | Pass size limit; forwarded verbatim to the broker (no default is documented in help or client source). |

**Behaviour.** Runs the broker's dependency recheck pass now, or forces one task's recheck with `--task`. The pass re-reads each dep-blocked task's deps (issue/PR state, CI run state, task terminality) and releases the ones that cleared; TTL-expired dep blocks are released by the same machinery. Output: the pass result row as JSON.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue dep-recheck --task 19479
{"task": 19479, "checked": 1, "released": 1, "still_blocked": 0}
```

## fleet queue quarantine-recheck

```text
usage: repo-queue quarantine-recheck [--task <id>] [--limit <n>]
```

| Flag      | Argument | Default               | Meaning                                            |
| --------- | -------- | --------------------- | -------------------------------------------------- |
| `--task`  | task id  | unset                 | Force one task's recheck instead of a full pass.   |
| `--limit` | integer  | no documented default | Pass size limit; forwarded verbatim to the broker. |

**Behaviour.** Runs the broker's quarantine recheck pass: re-validates **blocked, dep-free** rows whose recorded error is a claim-path quarantine (e.g. a stored packet that fails claim-time validation) and requeues the ones whose error no longer reproduces. Production cadence is the broker sweep's; this is the manual trigger. Output: the pass result row as JSON.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

```text
$ fleet queue quarantine-recheck
{"checked": 3, "requeued": 1, "still_quarantined": 2}
```

## fleet queue repair

```text
usage: repo-queue repair <task-id> [--worker <id>] [--reason <text>] [--dry-run]
```

| Flag                     | Argument  | Default            | Meaning                                                    |
| ------------------------ | --------- | ------------------ | ---------------------------------------------------------- |
| `<task-id>` (positional) | task id   | required           | The quarantined / malformed task to repair in place.       |
| `--worker`               | worker id | `repo-worker-code` | Caller identity; recorded as the repair's operator.        |
| `--reason`               | text      | empty string       | Why the task is repaired; recorded with the repair.        |
| `--dry-run`              | —         | off                | Report the defects and the planned repairs; write nothing. |

**Behaviour.** Repairs a stored packet that is malformed, in place — the gap the neighbouring verbs leave: the claim path quarantines such a row on every claim, `quarantine-recheck` can only release a row whose error no longer reproduces, and `unblock` refuses to clear a packet defect. (Worked example: a `payload.dedupe` missing its `checked_at`/`basis` stamp.) The broker re-runs the packet validators — `validate_packet` plus the claim-guard reason — applies the deterministic repairs whose correct value can be re-derived, re-validates the repaired packet in memory, and only then writes: one status-guarded update clears the quarantine and restores the task claimable (`queued`, or `blocked` with its deps intact when dep-blocked). A `repaired` timeline entry names the verb, the operator, the defects and the repairs. Output: the repaired row.

- **Eligible states:** `blocked`, `queued`, `parked`, `handed_off`. `claimed` is refused — repair never mutates a task a worker holds. `cancelled` / `failed` point at `revive` first; `done` is refused.
- **The one repairable class today:** the dedupe-stamp defect. Repair re-performs the dedupe check live — no other live task may hold the same work identity `(repo, issue, kind)` or the same `dedupe_key` among claim-guard-clean rows — and stamps `checked_at` plus a `basis` stating the re-check. A live duplicate refuses the repair: the stamp cannot be honestly re-derived. A packet carrying neither identity nor key refuses too.
- **Any other defect** is unrepairable today: the broker refuses with the defect list and changes nothing.

Exit codes: 0 repaired (or the dry-run report) · 2 usage · 3 transport · 4 broker store failure · refusals exit non-zero with the reason and mutate nothing.

```text
$ fleet queue repair 19481 --dry-run
{"task_id": 19481, "outcome": "dry_run", "defects": ["..."], "repairs": [{"field": "payload.dedupe", "action": "stamped checked_at/basis by re-performing the dedupe check live"}], "dry_run": true, "status": "blocked", "status_after": "queued"}
```

## fleet queue resolution-backfill

```text
usage: repo-queue resolution-backfill
```

Takes no arguments; any positional or flag is a usage error (exit 2).

**Behaviour.** Runs the terminal-resolution backfill for #14170 Q2 (broker route `/resolution-backfill`) and prints the broker's report as JSON: the terminal census, the rows it proved, and the rows it left unresolved. It writes a verdict only where existing result data, or a unique dedupe twin, evidences it, so a row with no such evidence stays unresolved. The backfill is idempotent: a second run writes nothing new.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure.

## fleet queue cancel

```text
usage: repo-queue cancel <task-id> [--worker <id>] --reason <text> --resolution <name> [--substatus <name>] [--ref <target>]
```

| Flag                     | Argument  | Default             | Meaning                                                                                                                                    |
| ------------------------ | --------- | ------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| `<task-id>` (positional) | task id   | required            | The task to cancel.                                                                                                                        |
| `--worker`               | worker id | **`fleet-monitor`** | Caller identity. Note: cancel is a monitor close-out verb — its worker default is NOT the global `repo-worker-code`.                       |
| `--reason`               | text      | **required**        | Why the task will never be worked. Recorded as `result.cancel_reason`; required for rejection resolutions.                                 |
| `--resolution`           | name      | **required**        | A redirection (`duplicate`, `superseded`, `split_out`) or rejection (`wontfix`, `obsolete`, `invalid`). Success resolutions cannot cancel. |
| `--substatus`            | name      | unset               | Taxonomy refinement for the selected resolution; the broker checks it against the resolution registry.                                     |
| `--ref`                  | target    | unset               | Required for redirections; names the successor task, issue, or PR.                                                                         |

**Behaviour.** For a task that will never be worked (issue closed, self-assigned elsewhere, superseded). Sets status `cancelled`, clears worker/lease, deletes the lease row, records the resolution columns and legacy `result.cancel_reason`, and writes a `cancelled` event. A rejection requires a reason; a redirection requires a reference. Output: the row; cancel closes the timeline binding window. `revive` clears all resolution columns and puts their prior values in the `revived` history detail.

Help: the usage writes `--reason` bare — it is **required** (exit 2 without it) — and its description names cancel's `--worker` default: `fleet-monitor`, not `repo-worker-code`.

Exit codes: 0 cancelled · 2 usage / missing `--reason` · 3 transport · 4 broker store failure · 5 no matching row.

```text
$ fleet queue cancel 19481 --reason "duplicate of surviving queue task" --resolution duplicate --ref task:19482
{"id": 19481, "status": "cancelled", "resolution": "duplicate", "resolution_category": "redirection", "resolution_ref": "task:19482"}
```

## fleet queue revive

```text
usage: repo-queue revive <task-id> [--worker <id>] [--reason <text>] [--not-before <ts>]
```

| Flag                     | Argument           | Default            | Meaning                                                                                                |
| ------------------------ | ------------------ | ------------------ | ------------------------------------------------------------------------------------------------------ |
| `<task-id>` (positional) | task id            | required           | The terminal or dead-at-cap task to revive.                                                            |
| `--worker`               | worker id          | `repo-worker-code` | Caller identity.                                                                                       |
| `--reason`               | text               | empty string       | Why the task is revived; recorded with the revive.                                                     |
| `--not-before`           | RFC 3339 timestamp | unset              | Fresh `not_before` for the revived row; validated as: non-empty, ≤366 days in the future, else exit 2. |

**Behaviour.** Revives a terminal task **in place**: a failed task, or a queued task dead at cap (attempts exhausted), gets `attempts = 0` and a fresh `not_before` through the broker's `/revive` — the supported replacement for direct-SQL resets. Output: the row.

Exit codes: 0 revived · 2 usage / bad `--not-before` · 3 transport · 4 broker store failure · 5 no matching row.

```text
$ fleet queue revive 19483 --reason "transient infra failure fixed; retry the work"
{"id": 19483, "status": "queued", "attempts": 0}
```

## fleet queue fail

```text
usage: repo-queue fail <task-id> [--worker <id>] [--lane <id>] [--error <text>] [--reason-class <c>] [--execution <id>]
```

| Flag                     | Argument                            | Default                          | Meaning                                                                                                                                          |
| ------------------------ | ----------------------------------- | -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `<task-id>` (positional) | task id                             | required                         | The claimed task to fail.                                                                                                                        |
| `--worker`               | worker id                           | `repo-worker-code`               | Holder identity.                                                                                                                                 |
| `--lane`                 | lane id                             | env `REPO_QUEUE_LANE`, else null | Holder lane check.                                                                                                                               |
| `--error`                | text                                | **required**                     | The failure error text. Missing → exit 2. Note: `--error help` records the literal text "help" (value-flag exception to the help rule).          |
| `--reason-class`         | `transient\|deterministic\|blocked` | `transient`                      | Closed enum — anything else dies exit 2 `argument --reason-class: invalid choice: '…'`. (Contrast `block`, whose `--reason-class` is free-form.) |
| `--execution`            | execution id                        | unset                            | Folded into an execution record `{lane, lane_id?, session_id, fallback_reason?}`.                                                                |

**Behaviour.** Terminal `failed`. Broker route `/fail`. Output: row or `{task, lane_ledger}`; fail closes the timeline binding window.

Help: the usage writes `--error` bare — it is **required** (exit 2 without it) — and its description names the `--reason-class` closed enum: `transient` (the default), `deterministic`, `blocked`.

Exit codes: 0 failed · 2 usage / missing `--error` / invalid `--reason-class` · 3 transport · 4 broker store failure · 5 no matching claimed row.

```text
$ fleet queue fail 19479 --worker repo-worker-code --lane code-a --error "packet payload fails validation on retry" --reason-class deterministic
{"id": 19479, "status": "failed", "last_error": "packet payload fails validation on retry", "result": {"reason_class": "deterministic"}}
```

## fleet queue complete

```text
usage: repo-queue complete <task-id> [--worker <id>] [--lane <id>] [--result <json>] [--result-file <f>] [--telemetry <json>] [--execution <id>] [--resolution <name>] [--substatus <name>] [--reason <text>] [--ref <target>]
```

| Flag                     | Argument     | Default                                | Meaning                                                                                                                                                                                                                           |
| ------------------------ | ------------ | -------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id      | required                               | The claimed task to complete.                                                                                                                                                                                                     |
| `--worker`               | worker id    | `repo-worker-code`                     | Holder identity.                                                                                                                                                                                                                  |
| `--lane`                 | lane id      | env `REPO_QUEUE_LANE`, else null       | Holder lane check.                                                                                                                                                                                                                |
| `--result`               | JSON object  | `{}` when neither result flag is given | The completion result object. Must parse as a JSON **object** (non-JSON or non-object → exit 2).                                                                                                                                  |
| `--result-file`          | file path    | unset                                  | File containing the result object. When both are given, `--result-file` is read after and **wins** over `--result`.                                                                                                               |
| `--telemetry`            | JSON object  | unset                                  | Telemetry whose keys must be a subset of the four fields `pool, model, effort, cost_usd` — unknown fields die exit 2 listing the allowed set. Values are merged into the result only for fields the result doesn't already carry. |
| `--execution`            | execution id | unset                                  | Folded into `result.execution = {lane, session_id, fallback: bool, fallback_reason?, assigned?}` when the result doesn't already carry an `execution` key, and also sent as the body's `execution`.                               |
| `--resolution`           | name         | `done`                                 | Success resolution; accepted values are `done`, `already_solved`, and `answered`.                                                                                                                                                 |
| `--substatus`            | name         | `unverified` for `done`                | Taxonomy refinement. A verified disposition readback overrides this with `verified`; callers cannot claim `verified` without that readback.                                                                                       |
| `--reason`               | text         | unset                                  | Required for rejection resolutions; included for resolution-bearing completion records.                                                                                                                                           |
| `--ref`                  | target       | unset                                  | Required for redirection resolutions; names the successor task, issue, or PR.                                                                                                                                                     |

**Behaviour.** Terminal `done`, via POST `/complete` on the fallback-switch path (a primary-store failure switches, exit 6). Output: row or `{task, lane_ledger[, follow_on_task_id]}` — a triage completion can spawn a follow-on task whose id rides the envelope. Completion closes the timeline binding window.

Client-side completion dressing (all fail-soft, explicit caller values win):

- `result.dominant_model`: the bound session's dominant serving model over the task's claim window (route_attempt call count), when derivable.
- Telemetry defaulting: `pool`/`model`/`effort` are filled from the claim stamp, else the completing session's serving leaf / declared route pool; `cost_usd` stays null (no cost readout is sourceable). `effort` is recorded only from resolved session messages, never inferred from route names. A claim stamp written through the route provider (`opencode-route`) names the route id in `claim_model` — that is the pool, not a serving leaf — so it never supplies `model`; the session's serving leaf, or null, does.

**The evidence gate.** Shape validation alone used to let a fix-class row complete `done` on any non-empty evidence string while its issue was open and a draft PR still carried the work. Now, a completion that requires a disposition must carry `result.disposition` — the verified readback record produced by `verify-disposition` (next section):

```json
{ "checked_at": "…", "source": "…", "issue": { "number": 0, "state": "…", "state_reason": "…", "closed_at": "…", "closed_by": "…" }, "pulls": [{ "number": 0, "state": "…", "is_draft": false, "merged_at": "…" }], "verdict": "…", "follow_up": "…" }
```

The gate is pure (no network): it parses the record, checks freshness (≤ **1800 s** old by default; >300 s in the future is unreadable clock skew), checks it describes the task's issue, checks the verdict against the readback, checks every PR the completion result links (`pr`/`pr_url`/`merged_pr` fields) appears in the readback, and checks any follow-up. **Every rejection leaves the task claimed** and names the defect ("…; the task remains claimed"). The five verdicts and their per-verdict rules are specified under `verify-disposition` below.

The task row exposes `resolution`, `resolution_category`, `resolution_substatus`, `resolution_reason`, `resolution_ref`, `resolved_by`, and `resolved_at`. Legacy terminal rows are backfilled only when their results prove a verdict; ambiguous rows remain NULL. The `resolution-backfill` broker verb reports the terminal census and counts inferred by each proof rule.

Exit codes: 0 completed · 1 disposition gate rejection (task remains claimed) · 2 usage / non-object result / unknown telemetry field · 3 transport · 4 broker store failure · 5 no matching claimed row · 6 fallback switch.

```text
$ fleet queue verify-disposition 13165 --repo developing-today/code --json > /tmp/disp.json
$ fleet queue complete 19479 --worker repo-worker-code --lane code-a --result-file /tmp/disp-result.json
{"id": 19479, "status": "done", "completed_at": "2026-11-23T04:25:00Z"}
```

## fleet queue verify-disposition

```text
usage: repo-queue verify-disposition <issue> --repo <owner/name> [--pr <n,n>] [--follow-up-task <id> | --follow-up-issue <n|url> | --follow-up-pr <n>] [--reason <text>] [--staleness-seconds <n>] [--json]
```

| Flag                   | Argument                   | Default                                         | Meaning                                                                                                                                                         |
| ---------------------- | -------------------------- | ----------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<issue>` (positional) | issue number               | required                                        | The issue to read back from GitHub.                                                                                                                             |
| `--repo`               | `owner/name`               | **required**                                    | The issue's repo. Missing → exit 2.                                                                                                                             |
| `--pr`                 | comma-separated PR numbers | unset                                           | Explicitly linked PRs to include in the readback.                                                                                                               |
| `--follow-up-task`     | task id                    | unset                                           | Follow-up target: a queue task (checked by the _broker_ at completion — only the queue knows if it is live). Mutually exclusive with the other follow-up flags. |
| `--follow-up-issue`    | issue number or URL        | unset                                           | Follow-up target: a same-repo issue; its own readback (state + explicit links) is embedded.                                                                     |
| `--follow-up-pr`       | PR number                  | unset                                           | Follow-up target: a successor PR.                                                                                                                               |
| `--reason`             | text                       | **required when any follow-up target is given** | Why the follow-up owns the remainder. Any follow-up target without `--reason` → exit 2.                                                                         |
| `--staleness-seconds`  | integer seconds            | 1800 (30 min)                                   | Freshness window for the record; must be positive (exit 2 otherwise).                                                                                           |
| `--json`               | boolean                    | off                                             | Print the raw record object. Without it, a human summary.                                                                                                       |

**Behaviour.** Reads the issue and its linked PRs **back from GitHub, live** (never from a remembered claim) and prints the disposition a terminal completion must carry. Readback assembly: `gh api repos/<repo>/issues/<n>` (the REST form, because it carries `closed_by`); `gh pr list --state all --limit 300 --json number,state,isDraft,mergedAt,body`; a search union (`"<issue> in:body"`, limit 100) because the 300 newest PRs are not the whole history; then an individual `gh pr view` for any explicitly linked/successor PR the union lacks. A PR counts as _linked_ when it is named by queue provenance / `--pr`, or its body carries an explicit marker — `Refs #n`, `Fixes #n`, `Related program: #n`. A bare prose `#n` is not a link (except that the `Related program:` prose marker _is_ honored — it is the only link the draft PR in the motivating case had to its issue).

Either way (`--json` or human summary) the verb then runs the offline gate on its own record: **exit 0 when the record would pass, exit 1 when the broker's gate would reject it**, and exit 1 when the issue or PR list cannot be read from GitHub at all. Usage errors exit 2.

**The five verdicts** (exactly five; derivation and gate rules):

| Verdict                    | Derived when                                                       | Gate requires                                                                                                                                     |
| -------------------------- | ------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------------------------- |
| `merged_verified`          | some pull in the readback is merged                                | ≥1 merged pull in the readback                                                                                                                    |
| `closed_verified`          | issue closed and no unfinished pulls                               | issue state actually `closed` (a close timestamp on an open issue is a superseded close, not evidence)                                            |
| `open_draft_with_followup` | unfinished pulls exist, nothing merged, issue not cleanly closed   | a recorded `follow_up`; while the issue is OPEN, unfinished pulls are allowed _only_ under this or the next verdict, with the follow-up on record |
| `open_draft_superseded`    | (operator-asserted; same derivation family)                        | a recorded `follow_up`                                                                                                                            |
| `invalid_no_work`          | none of the above (no merged pull, issue open, no unfinished pull) | rejected if any linked pull still carries work                                                                                                    |

A pull is _unfinished_ when it is not merged (`merged_at` null) and not closed.

**Follow-up checks:** reason required; target must be one of `task_id`, `issue_url`, `successor_pr`. A follow-up _issue_ must not be the issue being completed, must be **open**, must link the completed issue with an explicit marker, and must link every unfinished PR. A _successor PR_ must be in the readback and must be **merged**. A _task_ target is checked broker-side: the task must exist, not be the completing task, be in a **live** (non-terminal) status, and name the same issue.

**Intentional-close exception:** a closed issue may leave related work open only when the close was a deliberate act by a person (the readback's `closed_by` is checked against automation) _and_ a verified follow-up owns the remainder.

Exit codes: 0 record would pass the gate · 1 gate would reject it, or GitHub readback unreadable · 2 usage (including missing `--repo`, follow-up without `--reason`, non-positive `--staleness-seconds`).

```text
$ fleet queue verify-disposition 13165 --repo developing-today/code --json
{"checked_at": "2026-11-23T04:24:00Z", "source": "github-readback", "issue": {"number": 13165, "state": "open", "state_reason": null, "closed_at": null, "closed_by": null}, "pulls": [], "verdict": "invalid_no_work", "follow_up": null}
(exit 1 — no merged pull and no follow-up on record; a completion carrying this record would be rejected)
```

## fleet queue reprioritize

```text
usage: repo-queue reprioritize <task-id> <new-priority> [--reason <text>] [--worker <id>]
```

| Flag                          | Argument       | Default                           | Meaning                                                                                                                                             |
| ----------------------------- | -------------- | --------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<task-id>` (positional)      | task id        | required                          | The queued task whose priority changes.                                                                                                             |
| `<new-priority>` (positional) | priority value | required                          | A number, or a category name the validator maps to one (`validate::resolve_priority`). Missing or unresolvable → exit 2.                            |
| `--reason`                    | text           | unset                             | Why the priority changed; recorded with the change.                                                                                                 |
| `--worker`                    | worker id      | the authenticated caller identity | The acting worker. The actor is the authenticated caller identity presented over the broker's token channel; no free-form actor string is accepted. |

**Behaviour.** Calls the broker's `POST /reprioritize`; the broker
applies the new priority in the Turso queue store and syncs the task's
GitHub category to match. Prints the served task row as JSON. Landed in
#14100 (PR #14203) and listed in the queue domain's registry subcommand
inventory, so `fleet queue --help` shows it.

Exit codes: 0 success · 2 usage (including a missing or unresolvable
priority).
