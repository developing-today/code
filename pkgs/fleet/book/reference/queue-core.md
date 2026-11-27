# fleet queue: core verbs

`fleet queue` is the repo work queue: enqueue, claim, and complete tasks through the queue broker. Inside the binary the same surface is the standalone `repo-queue` program, and every per-verb usage string says `repo-queue`, not `fleet queue`; the two spellings are identical. The full command list is: `validate, enqueue, list, claim, release, lane, bind, heartbeat, checkpoint, park, wake, block, unblock, dep-recheck, quarantine-recheck, cancel, revive, fail, complete, verify-disposition, timeline, history, show, staleness, needs-user, models, health, signal, fallback, dispatch, session, surfaces, outbox, stack`.

Task states are `queued`, `claimed`, `parked`, `blocked`, `needs_review` (live) and `done`, `failed`, `cancelled` (terminal).

This chapter covers the core verbs — `validate`, `enqueue`, `list`, `claim`, `release`, `show`, `health`. The lifecycle verbs are in [fleet queue: lifecycle verbs](queue-lifecycle.md), the GitHub fallback in [fleet queue: the fallback queue](queue-fallback.md), dispatch and sessions in [fleet queue: dispatch and sessions](queue-dispatch.md), and inspection/admin in [fleet queue: inspection and admin](queue-admin.md).

## How the client reaches the broker

The client never touches the database and never fetches from Bitwarden; it POSTs JSON to the **queue-broker**, which is the only process that talks to the Turso store (database `fleet-queue`). There are two routes, in selection order:

1. **Direct route (on-laptop)**, chosen when a broker URL is configured: environment `REPO_QUEUE_BROKER_URL`, else the file `~/.config/repo-queue/broker-url`. A `unix://<path>` value selects the broker's Unix socket; anything else is read as `http://host:port` TCP (the broker dual-binds TCP + UDS; default TCP bind is the laptop tailnet address, port 4801). The direct-route token comes from env `REPO_QUEUE_TOKEN_FILE`, else `~/.config/queue-broker/token`, sent as header `X-Agent-Token`. Wire: raw HTTP/1.1, read timeout 300 s, write timeout 60 s. With no configured URL, fleet-config's probe-based choice for the `queue-broker` role applies: a live role socket selects UDS; a failed UDS attempt falls back to the session-proxy route within the same call.
2. **Session-proxy route (sandbox default):** the client connects to the laptop-agent session proxy's Unix socket — `$HOME/.cache/laptop-agent/broker.sock` (fleet-config's `broker_socket()` wins when it loads; env beats config file beats that compiled-in default). Framing: 8-byte big-endian length prefix + one JSON envelope `{method:"POST", path, body, port:4801, token_file:"<home>/tools/queue-broker-token"}`. The proxy, on the laptop, reads the named token file there and forwards to the broker on port 4801. Read timeout 660 s. On a failed first attempt the client auto-spawns the proxy/broker program — a sibling `fleet` binary (`fleet proxy serve`) always wins; the Python `~/tools/laptop-agent-broker.py` is the fallback for standalone rollback builds — waits up to ~5 s for the socket, and retries once.

Broker-unavailable on either route is exit 3, or the fallback switch (exit 6) for the work verbs — see [fleet queue: the fallback queue](queue-fallback.md).

Broker envelopes are `{ok:true, ...}` or `{ok:false, exit, error}`; the client prints `error` and exits with `exit`. A broker envelope whose error carries PostgREST code `57014` (statement timeout) or `PGRST002` (schema cache) is retried with exponential backoff + jitter — base 500 ms, ×2, cap 20 s, max 6 attempts. Safe for writes because a 57014 means the statement rolled back.

Broker server facts that shape client behaviour: the pre-body gate refuses from headers alone — wrong/missing token → `401 {"error":"unauthorized"}`; `Content-Length` over the 16 MiB cap → `413 {"error":"request body too large"}`. The only unauthenticated route is `GET /healthz` → `{"service":"queue-broker"}`. Non-POST → 405. Token files are read + trimmed; a missing, unreadable, empty, or whitespace-only file is a hard error naming the file. Before dispatch, the broker re-validates durations and `not_before` with the same ranges as the client; violations answer `{ok:false, exit:2, …}`. `/validate` and `/healthz` are served without the Turso backend; everything else answers exit 3 `queue backend not ready: …` until the backend connects.

## Global parsing rules and defaults

- **Help:** `repo-queue` with no args prints the top usage and exits 2; a single `--help`/`-h`/`help` exits 0. `<verb> --help` anywhere in the verb's args prints that verb's exact usage and exits 0 — help wins over every other argument, _except_ when the help-looking token is the value of a value-flag (`fail 5 --error help` records the error text "help"; `park 5 --reason -h` parks with reason "-h").
- **Parsing errors are exit 2:** unknown flag, duplicate flag, a boolean given an inline value (`--dry-run=false`), a value flag with no value, a value flag followed by another `--flag` token, extra positionals past a verb's arity (rejected before any broker contact), and non-numeric values for numeric flags (the message names the flag). Message + the verb's usage go to stderr.
- **`--worker` default:** `repo-worker-code`. Exceptions: `cancel` defaults its worker to `fleet-monitor` (see [lifecycle verbs](queue-lifecycle.md)); `stack record`'s owner worker and the outbox owner defaults differ (see [inspection and admin](queue-admin.md)).
- **`--queue` default — the divergence.** The nominal default is `repo-health-code-shadow`, overridable by env `REPO_QUEUE_DEFAULT_QUEUE` — **but** `claim` and `list` are _unscoped_ when neither flag nor env is present (one-queue model: the broker works across the one fleet queue), and `enqueue` has **no** default at all. Every other verb keeps the shadow default. The help surface presents all three behaviours identically (`[--queue <name>]` everywhere), so the divergence is invisible from help alone. Known discrepancy: this divergence is deliberate in source (#7016, #8097) but undocumented in help.
- **`--lane`** on the holder-checked verbs (heartbeat, checkpoint, park, release, block, fail, complete): flag, else env `REPO_QUEUE_LANE`, else null — the broker then checks the worker alone. All scheduled lanes share one worker id, so the lane is what tells the broker whose claim a verb acts on.
- **`--progress` / `--progress-file`** are mutually exclusive (both → exit 2). The file form reads the file's text as the progress value.
- **`--execution <id>`** (park, fail, complete) is parsed into an execution record `{lane, lane_id?, session_id, fallback_reason?}`; a lane-looking value (e.g. `opencode-a`, `any-n`) is split into mode (`opencode`/`direct`) + `lane_id`; the mode-only values `direct`/`opencode` pass through unchanged.
- **Durations are rejected, never clamped**, identically at client and broker: lease 1..=86400 (default 2700), heartbeat extend 1..=86400 (default 900), park delay 1..=604800 (no default), list limit 1..=500 (default 20). `not_before` must be a non-empty RFC 3339 timestamp no more than 366 days in the future.
- **Mode line:** claim, heartbeat, checkpoint, complete (and the fallback-switch envelope) carry a `mode_line` object `{verb, mode, route}` selected by the routing contract from observed legs (database / replica / github-write / github-read). Modes: `normal` (route `cloud`), `gh-read-degraded`, `gh-write-degraded`, `db-degraded` (route `github-markers`).

## Exit codes (the whole queue client)

Stated once here and referenced by every other queue chapter:

- **0** — success.
- **1** — gate / not-found: `verify-disposition` gate failure or unreadable GitHub readback; `show` task not found; fallback `complete` disposition rejection.
- **2** — usage error (message + usage on stderr).
- **3** — transport / broker-path failure: broker unreachable, HTTP 401 (token rejected), HTTP ≥400 broker error, malformed broker reply, unwritable signal file, `/events-list` not served by the broker (timeline/show), home unresolvable.
- **4** — broker store failure: the default when a broker envelope `{ok:false}` carries no `exit` of its own; broker-side store/transport failures are emitted as exit 4. Also `stack replay`'s routed-conflict exit.
- **5** — no-claim / no-row: the verb ran but there is no matching claimed row / no claimable task (`require_row` failure: "no matching claimed row (task not claimed by this worker, or already terminal)").
- **6** — fallback switch: a work verb's primary-store failure switched the work to the GitHub fallback (see [the fallback queue](queue-fallback.md)).

Sources: [fleet-queue client](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client.rs), [client support](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/support.rs), [client transport](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/transport.rs), [queue broker](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/queue_broker.rs), [Turso queue store](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/turso_queue.rs).

## fleet queue validate

```text
usage: repo-queue validate <packet-path>
```

| Flag                         | Argument  | Default  | Meaning                                                                                     |
| ---------------------------- | --------- | -------- | ------------------------------------------------------------------------------------------- |
| `<packet-path>` (positional) | file path | required | The packet JSON file to validate.                                                           |
| _(no flags)_                 | —         | —        | `validate` has no flags at all — not even `--json`; output is always the JSON object below. |

**Behaviour.** Loads the packet and validates it _through the broker_ (`/validate`). An unreadable file dies exit 2 `cannot read packet file: …`; invalid JSON dies exit 2 `packet file is not valid JSON: …`. Validation failure dies with the broker's exit code and the joined error list. On success it prints, on stdout, `{"valid": true, "dedupe_key": <packet.dedupe_key|null>, "kind": <payload.kind|null>, "title": <packet.title|null>}`.

The validator checks, among other things: `priority` on the design scale [10, 20, 30, 40, 50, 70, 90]; `max_attempts` 1..=1000 (design default 1000); payload size hard ceiling 16384 bytes (warn above 4096); the payload `schema` const `"repo-task/v1"`; `kind` one of six values the validator allows (`pr_shepherd, triage, small_fix, ci_investigation, second_pass, feature_slice`); `repo` on the validator's 9-repo allowlist; and the identity rule — a packet must carry at least one of `issue`, `prs` (non-empty), `set`, or `source`.

Known discrepancy: the published packet schema (`repo-task-v1.schema.json`) allows only 4 kinds and 8 repos; the validator allows 6 kinds (adding `second_pass`, `feature_slice`) and 9 repos (adding legacy `dezren39/nix`, kept for in-flight dedupe keys after the rename to `dezren39/darwin-nix`). A packet valid to the broker can be invalid against the published schema, and vice versa. This chapter documents the validator (source) behaviour.

Exit codes: 0 valid · 2 unreadable file / invalid JSON / usage · broker's exit code on validation failure.

```text
$ fleet queue validate ./task.json
{"valid": true, "dedupe_key": "fix: repo-health:v1:fix:developing-today/code:issue:13165", "kind": "small_fix", "title": "Write the fleet book queue chapters"}
```

## fleet queue enqueue

```text
usage: repo-queue enqueue <packet-path> [--queue <name>] [--platform <p>] [--dry-run] [--shadow-intent]
```

| Flag                         | Argument        | Default                       | Meaning                                                                                                                                                              |
| ---------------------------- | --------------- | ----------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<packet-path>` (positional) | file path       | required                      | The packet JSON file to enqueue.                                                                                                                                     |
| `--queue`                    | queue name      | **none — required in effect** | Target queue stream. Resolution: `--queue` flag → the packet's own `"queue"` field → env `REPO_QUEUE_DEFAULT_QUEUE`. With none of the three: exit 2 (see Behaviour). |
| `--platform`                 | platform family | unset                         | Restricts the item to a platform family; validated client-side. Absent = unset, claimable anywhere.                                                                  |
| `--dry-run`                  | boolean         | off                           | Validate + ask the broker to build the row, print it, file nothing.                                                                                                  |
| `--shadow-intent`            | boolean         | off                           | Required to enqueue into a queue whose name ends in `-shadow`; without it, exit 2.                                                                                   |

**Behaviour.** Flow: read packet (exit 2 shapes as in `validate`) → broker `/validate` (a dry run still validates, with `dry_run` noted) → shadow guard → POST `/enqueue` `{packet, queue, dry_run, shadow_intent, platform}`.

Queue resolution deliberately differs from every other verb: with no flag, no packet `"queue"` field, and no env, enqueue dies exit 2 with `enqueue: no queue resolved — … (enqueue has no default queue; the old shadow default silently misrouted work)`.

Outputs:

- **Dry run:** the would-be task row as JSON, with two extra keys: `"_dry_run": true` and `"_rpc": "agent.enqueue_repo_task"`. Exit 0.
- **Real run:** one human line, any `warning: …` lines, then the task row as JSON. Exit 0. The line is one of:
  - `enqueued new task <id> (queue <q>, status <s>)` — row created within 10 s skew of the call;
  - `dedupe hit: existing task <id> (queue <q>, status <s>, created <ts>)` — the dedupe key already has a live row; nothing was filed. Warnings are added when the holding row is in a different queue than requested, or in a terminal state;
  - `revived: existing task <id> (…)` — dedupe hit on a `queued` row whose `updated_at` is fresh (broker revived it in place);
  - `revived: new task <id> (…), predecessor task <pid> (<pstatus>)` — the envelope carried `revived_from`: the key's prior task was terminal, a fresh task was created linked to it;
  - `work already queued: existing task <id> (queue <q>, status <s>) holds <identity>; nothing filed (set payload.dedupe_override to file a deliberate parallel)` — the envelope carried `work_identity_collision`: a _live_ task already holds this work identity (repo, issue, kind family), regardless of dedupe key.
  - `not filed: dedupe key already belongs to completed task <id> (queue <q>, status <s>); suffix the dedupe key to file this work again, or set payload.dedupe_override for a deliberate parallel` — the envelope carried `dedupe_match` with rule `dedupe_key_terminal`: the `(queue, dedupe_key)` already belongs to a completed task, which absorbs the filing (#13091). The report is the caller's cue to suffix the key and re-file in the same pass; before it existed, the completed row alone read as a successful filing and left `queue/queued` issues with no live task (#10627). A live key holder instead reports rule `dedupe_key_live` under the plain `dedupe hit:` line. The CLI merges `dedupe_match` (and `work_identity_collision`) into the printed row JSON, so machine consumers of the enqueue output see the absorption too.

Dedupe semantics: dedupe is by `(queue, dedupe_key)` among **live** rows (the Turso backend does a read-then-insert check — its `(queue, dedupe_key)` index is deliberately NOT unique). A key held only by terminal rows accepts a new row — except a **completed** row, which absorbs the filing and returns itself, reported as `dedupe_match` rule `dedupe_key_terminal` (#13091, #10627); failed and cancelled rows revive as described below. A _cancelled_ row is revived in place by enqueue (fields reset, attempts 0, lease deleted, `enqueued` event flagged `revived_from_cancelled`); a _failed_ row deliberately does not revive — re-enqueue of a failed identity files a new row or is an explicit operator action (the `revive` verb).

At enqueue the broker also: enriches a packet that names an issue but carries no `labels` array from the GitHub mirror in the same store; selects the stored priority from label bands (`expedite` → 15, `next-up` → 25 — stored values deliberately not on the packet priority scale); and normalizes `max_attempts` / `required_budget_minutes` from the packet copy into the payload mirror.

Exit codes: 0 enqueued / dedupe hit / revived / dry run · 2 unreadable packet, invalid JSON, no queue resolved, shadow queue without `--shadow-intent`, usage · 3 transport · 4 broker store failure.

```text
$ fleet queue enqueue ./task.json --queue repo-health-code
enqueued new task 19479 (queue repo-health-code, status queued)
{"id": 19479, "queue": "repo-health-code", "status": "queued", "dedupe_key": "fix: repo-health:v1:fix:developing-today/code:issue:13165", "payload": {"schema": "repo-task/v1", "kind": "small_fix", "…": "…"}}
```

## fleet queue list

```text
usage: repo-queue list [--queue <name>] [--status <s>] [--limit <n>] [--not-before <ts>] [--lane <l>] [--worker <w>] [--task <id>] [--json]
```

| Flag           | Argument           | Default  | Meaning                                                                                                                               |
| -------------- | ------------------ | -------- | ------------------------------------------------------------------------------------------------------------------------------------- |
| `--queue`      | queue name         | unscoped | Flag → env `REPO_QUEUE_DEFAULT_QUEUE` → **no scope** (all queues). Never the shadow default.                                          |
| `--status`     | status string      | unset    | Filter by status. Help names `queued\|parked\|claimed\|blocked` as the actionable set; any status string is forwarded.                |
| `--limit`      | integer            | 20       | Range 1..=500, rejected (never clamped) outside it, exit 2 naming `--limit`.                                                          |
| `--not-before` | RFC 3339 timestamp | unset    | Filter; validated client-side with the park/revive rule, exit 2 on garbage.                                                           |
| `--lane`       | lane id            | unset    | Filter forwarded to the broker.                                                                                                       |
| `--worker`     | worker id          | unset    | Filter forwarded to the broker.                                                                                                       |
| `--task`       | task id            | unset    | Narrow to one task regardless of status ordering — the recovery path for verifying a just-completed task.                             |
| `--json`       | —                  | off      | Exact-task status form: requires `--queue` and `--task` together; see **Exact form** below. The bulk flags do not apply in that form. |

**Behaviour.** A bare `list` prints a scope note to **stderr**: it shows one queue (or every queue when unscoped) in the broker's default ordering — _newest terminal rows first_ — "not the actionable set", with pointers to `--status`, `--queue`, `--limit`. Fetching is paged at 100 rows/page with broker offsets, because a single oversized `/list` reply used to render as a silent "(no tasks)". A reply with no `rows` array is an error (exit 3), never an empty list.

Output is a table with columns `task, priority, status, age, dedupe_key, title` (title truncated to 72 chars); `(no tasks)` when empty. When the broker reports a total: `(N matching tasks)`, or `(truncated: showing X of N matching tasks — raise --limit to see more)`.

**Exact form (`--json`, #8581 pilot).** With `--json`, `--queue` and `--task` are required together (exit 2 without both) and no other list flag applies (exit 2 naming it). The verb makes one scoped rich `/list` read and prints a single JSON record instead of the table: `verb`, `mode` and `route` actually used (the #8581 contract vocabulary), `queue`, `task_id`, `as_of` (when this seat observed the answer), and a `result` of `found` (the broker row rides under `task`, attributed to `source: cloud`, with the row's own `task_as_of` when it carries a timestamp), `not_found`, `partial`, or `unknown`. Only a complete authoritative zero — ok reply, empty rows, total reported as 0, no truncation — is `not_found`; a truncated window, an uncertified zero, or a reply naming other tasks is `partial` with a `detail` naming why, and a transport failure, a denial, a broker error envelope, a malformed reply, or a matching row that names a different queue is `unknown` with its own `detail`. This slice attempts no replica or marker fallback: when nothing served, the record says `mode: db-degraded`, `route: unavailable` rather than naming a route it never took. Exit codes in this form: 0 found · 1 not_found · 3 partial / unknown / transport · the broker envelope's own exit (default 4) on a broker error.

Exit codes: 0 ok · 2 usage (including out-of-range `--limit`, bad `--not-before`) · 3 transport / malformed reply · 4 broker store failure.

```text
$ fleet queue list --status queued --limit 3
task     priority  status  age   dedupe_key                                                        title
19479    20        queued  4m    fix: repo-health:v1:fix:developing-today/code:issue:13165         Write the fleet book queue chapters
19480    30        queued  2m    triage: repo-health:v1:triage:developing-today/code:issue:13170   Triage the failing nightly
(2 matching tasks)
```

## fleet queue claim

```text
usage: repo-queue claim [--queue <name>] [--worker <id>] [--lease-seconds <n>] [--lane <l>] [--task <id>] [--manual-override <reason>] [--count <n>] [--from-session <id>] [--platform <p>] [--session-id <id>] [--model-provider <p>] [--model-id <id>] [--model-variant <v>] [--filter <f>] [--pool <p>] [--launched-by <who>] [--remaining-budget-minutes <n>] [--handoff] [--bind]
```

| Flag                         | Argument          | Default                                       | Meaning                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| ---------------------------- | ----------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--queue`                    | queue name        | unscoped                                      | Flag → env → unscoped across the one fleet queue, priority order.                                                                                                                                                                                                                                                                                                                                                                                       |
| `--worker`                   | worker id         | `repo-worker-code`                            | Claiming worker identity, stamped on the row + `leases` table. Passing it explicitly counts as claim intent even though it equals the default (see bare-claim refusal).                                                                                                                                                                                                                                                                                 |
| `--lease-seconds`            | integer seconds   | 2700 (45 min)                                 | Lease length. Range 1..=86400, rejected outside, exit 2. A 0/negative lease is "born expired".                                                                                                                                                                                                                                                                                                                                                          |
| `--lane`                     | lane id           | unset                                         | Lane identity for this claim; validated broker-side against the lane registry (unknown lane / worker mismatch / disabled lane / queue mismatch each refuse and claim nothing). Canonical lane universe: 6 families (`code, any, not-code, opencode, openchamber, nix`) × `a..=z` = 156 lanes, all enabled.                                                                                                                                              |
| `--task`                     | task id           | unset                                         | By-id claim: take exactly that row when the by-id predicate allows (a queued row under the ordinary predicate, or a sink/handed-off row).                                                                                                                                                                                                                                                                                                               |
| `--count`                    | integer           | 1                                             | Batch claim: the broker runs the same atomic pick n times. Range 1..=5, enforced client- and broker-side; outside → exit 2.                                                                                                                                                                                                                                                                                                                             |
| `--from-session`             | session id        | unset                                         | With `--task`: session-to-session **transfer**. The named session must be the task's current holder (its session mark is the capability); a mismatch refuses and leaves the claim untouched. The task never passes through queued/parked.                                                                                                                                                                                                               |
| `--platform`                 | platform family   | unset                                         | A _mark_, not a filter: stamped on the row and the claimed event; a mismatched or undeclared claim claims items all the same.                                                                                                                                                                                                                                                                                                                           |
| `--session-id`               | session id        | unset                                         | The claiming session's own id, recorded as a mark on the claimed event; also enables model-stamp derivation and `--bind`. Session ids have the shape `ses_` + alphanumerics.                                                                                                                                                                                                                                                                            |
| `--model-provider`           | provider name     | unset                                         | The claiming session's serving model provider, stamped by the broker as the row's `claim_provider`.                                                                                                                                                                                                                                                                                                                                                     |
| `--model-id`                 | model id          | unset                                         | Stamped as the row's `claim_model`.                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `--model-variant`            | variant name      | unset                                         | Stamped as the row's `claim_variant`. If all model flags are absent and `--session-id` is given, the stamp is _derived_ from the session's own record; derivation failure never fails the claim.                                                                                                                                                                                                                                                        |
| `--filter`                   | filter expression | unset                                         | Claimer-side boolean filter expression over task metadata (repo, queue, pool, platform, label, kind, priority, lane, model), evaluated by the broker inside the claim.                                                                                                                                                                                                                                                                                  |
| `--pool`                     | pool identity     | unset                                         | The claimer's declared pool identity; recorded like the other marks and used only to satisfy a task's own `requires.pool`.                                                                                                                                                                                                                                                                                                                              |
| `--launched-by`              | who               | `opencode-claimant` (only sent with `--bind`) | Who launched the bound session.                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `--remaining-budget-minutes` | integer minutes   | unset                                         | The claimer's remaining shift budget. The broker only matches requirement-bearing tasks whose `required_budget_minutes` fits; omitting it leaves requirement-bearing tasks unclaimed. Skips are reported (see no-claim output).                                                                                                                                                                                                                         |
| `--handoff`                  | boolean           | off                                           | Pick = the next recorded opencode handoff (status `handed_off`) from the sink; the broker picks the row, the caller names no task.                                                                                                                                                                                                                                                                                                                      |
| `--bind`                     | boolean           | off                                           | Claim + lane-assign + session-bind in ONE broker round trip via `/claim-flow`. Requires `--session-id`, a single (non-batch) claim, and no `--from-session`; violations die exit 2 before broker contact. A post-claim lane/bind failure inside the flow is answered as a partial SUCCESS envelope carrying `claim_flow_partial` (never a retryable error) — the client prints the note on stderr and stamps `claim_flow_partial` onto the printed row. |

**Behaviour — bare-claim refusal.** A claim with no task, no filter, no queue scope, no count, no handoff/bind, and no identity marks (worker, lane, session-id, pool, platform, from-session) is refused _before any broker contact_, exit 2: "a bare claim is never a meaningful intent and has repeatedly stolen the queue's top task during CLI probing".

**Behaviour — the pick.** One atomic conditional update (libSQL is single-writer, so the claim is one conditional UPDATE whose WHERE re-checks claimability against the picked row; a competing claim after the first lands matches no row and returns no claim, exit 5). Eligible: `queued` rows with `attempts < max_attempts` and `not_before` null or due, plus `parked` rows whose `not_before` is due. Order: `priority ASC, created_at ASC` (lowest number = highest priority). Claiming sets status `claimed`, `worker_id`, `claimed_at`, `lease_until = now + lease_seconds`, upserts the `leases` row, writes a `claimed` event — and increments `attempts` by 1 for a fresh (queued) claim but **0 for a parked resume** (park/resume must not consume attempts; the event carries `resumed_from_park`). Every claim first runs lease expiry opportunistically: abandoned claims whose lease lapsed become claimable at the moment anyone looks.

**Behaviour — no-double-serve guard.** After a successful primary claim, the client checks the task's issue against the GitHub fallback (see [the fallback queue](queue-fallback.md)). If the issue is `live` or `contested` in the fallback under a _different_ worker, the client immediately blocks the task back with reason-class `fallback_held` and prints `{"task": null, "guard": "fallback_held", "held_by": <w>, "repo": …, "issue": …}`, exit 5. If the GitHub check itself fails, the claim is served with a `guard_warning` key added (the guard fails open; a GitHub outage must not stall every claim). Batch claims run the same guard per row; held rows land in `guard_held`.

**Behaviour — timeline mirroring.** On a pre-#9046 broker the client appends `session_bound` / `model_stamp` events after a claim (and a `session_unbound` for the from-session on a transfer); a post-#9046 broker stamps in-op and its rows carry `claim_session_id`, making the appends a no-op. Timeline appends are fail-soft — they warn, never fail the claim.

**Output.** Success: the task row **verbatim** as JSON (the row is the contract: `{id, queue, status, dedupe_key, payload, …}`), exit 0, plus a `mode_line`. A row that arrives with a null or empty-object `payload` is treated as payload-less and reported as _no claim_. Batch output: `{"tasks": […rows…], "claimed": k, "requested": n, "guard_held": […]}`, exit 0 when k ≥ 1. No-claim output: `{"task": null}` — deliberately carrying no id/queue/status a caller could mistake for a task — plus any of `budget_skipped`, `max_required_budget_minutes`, `queued`, `at_cap_queued`, `queued_not_yet_due`, `parked_due` the broker supplied; stderr gets the diagnostic `no claimable task — Q queued (A at the attempt cap, D not yet due), P parked due` (with `; every queued task is at the attempt cap` when that is the whole story). Exit 5.

Exit codes: 0 claimed · 2 usage / bare-claim refusal / out-of-range lease or count / `--bind` precondition failure · 3 transport · 4 broker store failure · 5 no claimable task, or claim held by the no-double-serve guard · 6 fallback switch on primary-store failure.

```text
$ fleet queue claim --worker repo-worker-code --lane code-a --session-id ses_01J9EXAMPLE0000000000000
{"id": 19479, "queue": "repo-health-code", "status": "claimed", "worker_id": "repo-worker-code", "lane_id": "code-a", "lease_until": "2026-11-23T04:50:00Z", "dedupe_key": "fix: repo-health:v1:fix:developing-today/code:issue:13165", "payload": {"schema": "repo-task/v1", "kind": "small_fix"}, "mode_line": {"verb": "claim", "mode": "normal", "route": "cloud"}}
$ fleet queue claim --worker repo-worker-code --lane code-a
{"task": null, "queued": 0, "parked_due": 0}
no claimable task — 0 queued (0 at the attempt cap, 0 not yet due), 0 parked due
(exit 5)
```

**Manual parked claim.** `--task ID --manual-override REASON` deliberately claims an
under-cap parked task without incrementing attempts. The reason must be non-empty;
transfer (`--from-session`), handoff picks, and batches cannot be combined with it.
The task's `result.manual_override` and the `claimed` event's `manual_override`
record the worker/lane, supplied session, time, reason, and prior status, attempts,
max-attempts, `not_before`, park reason/kind, wait kind, and ready condition. The live
wait fields are cleared and unrelated result data is preserved. Without the flag a
parked by-ID claim still refuses; queued and handed-off by-ID claims keep their
ordinary behavior. Attempt-capped overrides and supervision handling are separate
slices.

## fleet queue release

```text
usage: repo-queue release <task-id> [--worker <id>] [--lane <id>] [--reason <text>]
```

| Flag                     | Argument  | Default                          | Meaning                                                                                 |
| ------------------------ | --------- | -------------------------------- | --------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id   | required                         | The claimed task to release.                                                            |
| `--worker`               | worker id | `repo-worker-code`               | Holder identity; the row must be claimed by this worker (and lane, when supplied).      |
| `--lane`                 | lane id   | env `REPO_QUEUE_LANE`, else null | Holder lane check; null means the broker checks the worker alone.                       |
| `--reason`               | text      | **required**                     | Why this task goes back to the pool; recorded on a `released` event. Must be non-empty. |

**Behaviour.** Returns the task to `queued`, immediately claimable; the reason is recorded on a `released` event. Used by dispatch for an item whose sub-task failed to launch/bind — back to the pool in the same touch instead of sitting claimed until lease expiry. Release also closes the session binding window on the timeline (`session_unbound`, trigger `release`). On success it prints the broker's returned task row as JSON and exits 0. When no matching claimed row exists, it dies exit 5 with `release: no matching claimed row (task not claimed by this worker, or already terminal)`.

Help: the usage writes `--reason` bare — it is **required and non-empty** — missing → exit 2 `release requires --reason "<why this task goes back to the pool>" (#8217)`. The top-level usage line lists the same flags as the per-verb usage, `[--lane <id>]` included; a parity test cross-checks the two so they cannot drift apart again.

Exit codes: 0 released · 2 usage / missing or empty `--reason` · 3 transport · 4 broker store failure · 5 no matching claimed row.

```text
$ fleet queue release 19479 --worker repo-worker-code --lane code-a --reason "sub-task failed to launch; returning to pool"
{"id": 19479, "queue": "repo-health-code", "status": "queued", "worker_id": null, "lease_until": null}
```

## fleet queue show

```text
usage: repo-queue show <task-id> [--limit <n>] [--json]
```

| Flag                     | Argument | Default  | Meaning                                                                                      |
| ------------------------ | -------- | -------- | -------------------------------------------------------------------------------------------- |
| `<task-id>` (positional) | task id  | required | The task to show.                                                                            |
| `--limit`                | integer  | 10       | History tail length; validated with the list-limit rule (1..=500), rejected outside, exit 2. |
| `--json`                 | boolean  | off      | Print the details + history as JSON instead of the rendered form.                            |

**Behaviour.** Shows one task's current details + recent history: the rich `/list` row for the task is the source of truth for mutable state, plus the tail of the same 14-kind event timeline `timeline` reads (see [inspection and admin](queue-admin.md)). Task not found: stderr `repo-queue show: task <id> not found`, exit 1. A broker without `/events-list`: exit 3.

Exit codes: 0 ok · 1 task not found · 2 usage · 3 transport / broker does not serve `/events-list` · 4 broker store failure.

```text
$ fleet queue show 19479
task 19479 — repo-health-code — claimed by repo-worker-code (lane code-a)
title: Write the fleet book queue chapters
lease_until: 2026-11-23T04:50:00Z
recent events: claimed, session_bound, checkpoint
```

## fleet queue health

```text
usage: repo-queue health [--queue <name>] [--worker <id>]
```

| Flag       | Argument   | Default            | Meaning                                                                                                                                                   |
| ---------- | ---------- | ------------------ | --------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--queue`  | queue name | unset              | Scopes the request body when given. Health uses the plain flag only — no env/shadow-default resolution is applied to the body.                            |
| `--worker` | worker id  | `repo-worker-code` | **Attribution only** — accepted because the claimant packet mandates `--worker` on every verb; it changes neither the request body nor the printed table. |

**Behaviour.** POSTs `/health` through the fallback-switch path: a dead primary switches the work to the GitHub fallback, exit 6 (see [the fallback queue](queue-fallback.md)). Output: first line `project: <netloc> (resolved via <source>)` (source defaults to `bitwarden`), then a table whose columns are whatever keys the broker's health rows carry (per-queue counts by status); `(no queues have tasks)` when empty. This verb is distinct from the broker's unauthenticated `GET /healthz`.

Exit codes: 0 ok · 2 usage · 3 transport · 4 broker store failure · 6 fallback switch on primary-store failure.

```text
$ fleet queue health
project: db.dezren39.turso.io (resolved via bitwarden)
queue                 queued  claimed  parked  blocked  done
repo-health-code      12      3        1       0        481
```
