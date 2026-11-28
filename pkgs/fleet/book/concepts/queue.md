# The queue: tasks, lanes and leases

The queue is the fleet's shared work queue: one queue of _tasks_ (units of
repo work), served by the queue broker over a dedicated Turso store, with a
GitHub-issue-comment fallback queue behind it. The client surface is
`fleet queue` (argv0 alias `repo-queue`) with 40 verbs. The count is one per
dispatch arm of `run()` in the client, which is also the command list
`fleet help queue` prints from the registry: `history` (an alias of
`timeline`) counts as its own verb, and `dispatch`, `session` and
`surfaces` count as three. Inside the binary,
every per-verb usage string spells the program `repo-queue`; the two
spellings are the same code path.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client.rs

## The task model

A task is created from a _packet_: a JSON document validated against
`repo-task-v1.schema.json` plus the fleet queue validator. The envelope
fields live outside the payload: `title`, `body`, `dedupe_key`, `priority`,
`not_before`, `max_attempts`. The payload (`schema` const `"repo-task/v1"`)
carries:

| Field                     | Shape                                                    | Notes                                                                                                                                                                                                                                         |
| ------------------------- | -------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `kind`                    | enum                                                     | The validator accepts six: `pr_shepherd`, `triage`, `small_fix`, `ci_investigation`, plus `second_pass` and `feature_slice`. (The schema file lists only the first four; the validator is the wider, operative list — source behaviour wins.) |
| `repo`                    | enum                                                     | The validator's allowlist has nine repos, including the legacy `dezren39/nix` name kept valid for in-flight dedupe keys. (The schema lists eight.)                                                                                            |
| `implementation_repo`     | `owner/name`, optional                                   | The repo receiving the implementation when it differs from the issue repo. Required when `risk.flags` contains `cross-repo-change`.                                                                                                           |
| `problem`                 | string, non-empty                                        |                                                                                                                                                                                                                                               |
| `evidence`                | string array, at least 1                                 |                                                                                                                                                                                                                                               |
| `acceptance`              | string array, at least 1                                 |                                                                                                                                                                                                                                               |
| `scope`                   | `{include, exclude}`, both required, `exclude` non-empty |                                                                                                                                                                                                                                               |
| `tests`                   | string array, at least 1                                 |                                                                                                                                                                                                                                               |
| `risk`                    | `{level: low\|medium\|high, flags: string[]}`            |                                                                                                                                                                                                                                               |
| `priority_reason`         | string, non-empty                                        |                                                                                                                                                                                                                                               |
| `timebox_minutes`         | integer 1..=240                                          | Design values are 20 (triage) and 45 (standard); other values warn.                                                                                                                                                                           |
| `max_attempts`            | integer 1..=1000                                         | Design default 1000 for every kind.                                                                                                                                                                                                           |
| `context_refs`            | string array                                             |                                                                                                                                                                                                                                               |
| `dedupe`                  | `{checked_at, basis}`, both non-empty                    | The producer's dedupe attestation.                                                                                                                                                                                                            |
| `required_budget_minutes` | integer ≥ 1, optional                                    | The minimum uninterrupted budget a claimer must declare; absent means no requirement.                                                                                                                                                         |
| identity                  | one of `issue`, `prs`, `set`, `source`                   | At least one is required: an issue `{number ≥ 1, url?, title?, state_checked_at?}`, a non-empty PR list, a PR-set, or a pre-filing triage source `{pr, finding}`.                                                                             |

Envelope rules: `priority` must be on the design scale
**[10, 20, 30, 40, 50, 70, 90]** (lower is sooner); payload size has a hard
ceiling of 16,384 bytes (a warning above 4,096); `not_before` must be a
non-empty RFC 3339 timestamp no more than 366 days in the future. Durations
and limits throughout the queue are **rejected, never clamped**, identically
at client and broker.

Two stored-priority exceptions: at enqueue, a payload whose `labels` array
contains `expedite` stores priority **15**, else `next-up` stores **25**.
These stored values are deliberately not on the packet scale.

**Dedupe.** Enqueue dedupes on `(queue, dedupe_key)` among _live_ rows, and
separately on work identity (repo, issue, kind family): a live task already
holding the work identity means nothing is filed, unless the packet sets
`payload.dedupe_override`. A key held only by terminal rows accepts a new
row. A _cancelled_ row is revived in place by a re-enqueue; a _failed_ row
deliberately is not — re-enqueueing a failed identity files a new row.

**Enqueue has no default queue.** The queue resolves from `--queue`, else
the packet's own `queue` field, else the `REPO_QUEUE_DEFAULT_QUEUE`
environment variable; with none of the three, enqueue exits 2. (The old
shadow default silently misrouted work.) Enqueueing into a queue whose name
ends in `-shadow` additionally requires `--shadow-intent`. Most other verbs
default `--queue` to `repo-health-code-shadow` via the same environment
variable, but `claim` and `list` are _unscoped_ when neither flag nor env is
present: the broker works across the one fleet queue.

## States and transitions

Live states: `queued`, `claimed`, `parked`, `blocked`, `needs_review`.
Terminal states: `done`, `failed`, `cancelled`.

- `enqueue` → `queued` (`attempts = 0`).
- `claim`: `queued` → `claimed`; a due `parked` row is also claimable and
  resumes without consuming an attempt.
- `release`: `claimed` → `queued`, immediately claimable.
- `park` / `wake`: `claimed` → `parked` → (condition met, due, or woken)
  `queued`.
- `block` / `unblock`: `claimed` → `blocked` → `queued`.
- `complete`: `claimed` → `done`. `fail`: `claimed` → `failed`.
  `cancel`: `queued` or `parked` → `cancelled`, for a task that will never be
  worked.
- `revive`: a terminal row, or a queued row dead at the attempt cap, is
  resurrected in place with `attempts = 0`.
- Lease expiry: `claimed` → `queued` with a `lease_expired` event, or — when
  the row is out of attempts — it sits dead at cap with a
  `lease_expired_at_cap` event until `revive`.

Every transition writes an event row; `fleet queue timeline <task-id>`
(aliased `history`) renders the 14 event kinds the timeline reads.

## Lanes and workers

A _worker_ is the claiming identity stamped on a row (default
`repo-worker-code`; `cancel` defaults its worker to `fleet-monitor`). A
_lane_ is a finer identity used when many scheduled lanes share one worker
id: the lane tells the broker whose claim a holder verb acts on. Holder
verbs (heartbeat, checkpoint, park, release, block, fail, complete) resolve
`--lane` from the flag, else the `REPO_QUEUE_LANE` environment variable,
else null — in which case the broker checks the worker alone. A lane is
validated broker-side against the lane registry at claim time: an unknown
lane, a worker mismatch, a disabled lane, or a queue mismatch each refuse
and claim nothing. The canonical lane universe is 6 families (`code`, `any`,
`not-code`, `opencode`, `openchamber`, `nix`) × `a`..=`z` = **156 lanes**,
all enabled, seeded into the store's `lanes` table.

The claim pick is atomic: highest priority first (`priority ASC,
created_at ASC`) among due `queued` rows and due `parked` rows. A claim with
no task, no filter, no queue scope, no count, no handoff/bind, and no
identity marks is refused before any broker contact, exit 2 — a bare claim
has repeatedly stolen the queue's top task during CLI probing. Batch claims
take 1..=5 rows (`--count`). Claim flags also carry marks, not filters: the
claimer's platform, pool, session id, and model stamp
(`--model-provider`/`--model-id`/`--model-variant`, derived from the
session's own record when not given) are recorded on the row. A
`--remaining-budget-minutes` declaration gates requirement-bearing tasks:
the broker only matches tasks whose `required_budget_minutes` fits, and
omitting it leaves such tasks unclaimed. No claimable task is
`{"task": null}`, exit **5**, with counts (`queued`, `at_cap_queued`,
`queued_not_yet_due`, `parked_due`) explaining why.

A claim may instead name several tasks at once: `--tasks <id,id,...>`
(#9178 §3) claims each named task for the claiming session in one
operation and reports a per-task outcome — `claimed`,
`already-owned-by-this-session`, `skipped-claimed`, `not-found`, or
`failed` with a reason. Tasks claimed by another worker are skipped
with their claims unchanged; `--take-over` transfers a named
live-claimed task directly from its recorded holder session, with
both sides of the transfer on the claimed event. The named form
never picks: it combines with neither `--task`, `--count`,
`--from-session`, `--manual-override`, `--handoff`, nor `--bind`.

An enqueue may set the task's top-level `manual_only` flag. It defaults to
false and is persisted on the task row with the enqueue audit event. Automatic
default and handoff picks always skip flagged tasks, regardless of priority or
queue depth; an explicit by-ID claim can still take one. The task remains in
listings with a `(manual-only)` status marker, distinct from parked or blocked
work. Changes to this flag must be audit logged.

## Leases and heartbeat

A _lease_ is the claim's expiry. Claim's default lease is **2700 s
(45 minutes)**, range 1..=86400, rejected outside it; a zero or negative
lease is born expired. `fleet agent next` claims with a 14,400 s (4-hour)
lease instead, because 45 minutes lapses mid-task.

`heartbeat` keeps the later of the current deadline and the time of the
heartbeat plus `extend_seconds`. The default renewal window is **900 s
(15 minutes)**, range 1..=86400. A heartbeat five minutes into a four-hour
claim preserves the original four-hour deadline. A default heartbeat five
minutes before expiry moves the deadline ten minutes later. It may also
merge a progress value (`--progress` / `--progress-file`, mutually
exclusive). `checkpoint` records progress plus a required `--stage` (the
help line brackets it optional; the source requires it — missing `--stage`
exits 2) without touching the lease. Lease expiry is opportunistic — every
claim first sweeps expired leases — plus a broker sweep, because a scheduler
firing irregularly must not strand work.

Holder verbs are holder-checked: the row must be claimed by this worker
(and lane, when supplied) and non-terminal, else exit 5 with "no matching
claimed row". `release` requires `--reason` (the help line brackets it
optional; the source exits 2 without a non-empty reason). `cancel` likewise
requires `--reason`. `fail` requires `--error`, and its `--reason-class` is
a closed enum — `transient` (the default), `deterministic`, `blocked` —
whereas `block`'s `--reason-class` is free-form.

## Park, wake, and ready-conditions

`park` moves a claimed task out of the way until a _ready-condition_ says the
wait is over. `--reason` is required; `--delay-seconds` (range 1..=604800,
7 days; 0 is rejected — a zero-second park is a requeue, and `release`
exists) and `--not-before` are mutually exclusive; with neither, the
broker's historical default delay is **1200 s**.

The ready-condition is a JSON object. Kinds a database sweeper can verify
from store state: `task_terminal {task_id}`, `task_status {task_id,
status}`, `event_seen {task_id?, event_kind}`, `time {until}`. External
kinds (`ci`, `pr_merged`, `issue_closed`, `person_answer`, `review`, …) are
recorded faithfully but cannot be verified by the sweeper. With no explicit
condition the broker derives one from `progress.ready_condition`, else
`{"kind":"time","until":<not_before>}`. The condition kind determines the
_wait kind_ (`time`→`timer`; `ci`/`ci_green`/`pr_merged`/`issue_closed`/
`upstream_pr`→`ci`; `review`/`codex_review`/`pr_review`/`person_answer`→
`review`; `capacity`/`quota_reset`→`capacity`; `triage`→`triage`; anything
else→`external`), and the broker enforces wait floors whatever delay was
asked: a `triage` wait is held at least **6 h (21,600 s)**, a `ci` or
`capacity` wait at least **1 h (3,600 s)**. The park revalidation sweeper
later records `check_outcome` as `confirmed` (condition unmet), `released`
(condition met → back to `queued`), or `stale` (cannot verify). `wake`
releases a parked wait now that its condition is satisfied.

A park (or block) that waits on a _person_ — a `person_answer` condition, a
person-wait reason class, or blocker text naming the owner as the release —
is refused client-side, exit 2, unless it is a valid _hands item_:
`--hands physical|interactive-login|payment|signature` plus `--action` with
one sentence a person can do in five minutes (a single line, at most 200
characters, not itself asking for a decision). The rule's point: a choice
that is the worker's to make is never parked onto a person.

## Block and dependencies (`dep_blocked`)

`block` sets status `blocked`. `--reason-class` and `--blocker` are both
required. A classic block waits for a manual `unblock`. A _typed dependency
block_ names machine-watchable dependencies instead of prose:
`--blocked-by-pr`, `--blocked-by-issue`, `--blocked-by-ci` take
comma-separated `owner/repo#N` refs (for CI, `N` is an Actions run id), and
`--blocked-by-task` takes bare task ids. Bad refs die client-side, exit 2.

The parsed deps are stored on the task row in the **`blocked_deps`** column
(JSON); a task with a non-empty `blocked_deps` is in the `dep_blocked`
state. Alongside sit `dep_recheck_seconds` (clamped broker-side to
300..=3600, default **900**), `dep_next_recheck_at`, and `dep_blocked_until`
from `--dep-max-ttl-seconds` (default **3 days (259,200 s)**, floor 60 s,
ceiling 7 days). The broker's sweep rechecks deps every minute and releases
the task itself when they clear or the TTL expires; `dep-recheck` is the
manual trigger. `unblock` is the manual override: without `--drop-dep` it
clears the blocked state entirely; with `--drop-dep <list>` (comma-separated
`kind:ref` — `issue:owner/repo#N`, `pr:owner/repo#N`, `ci:owner/repo#RUN`,
`task:ID`) it drops only the named entries — the task stays blocked on the
remaining deps, and dropping the last one releases it. The broker audits who
overrode and what was discarded. A separate sweep, `quarantine-recheck`,
re-validates blocked dep-free rows whose recorded error is a claim-path
quarantine and requeues the ones whose error no longer reproduces.

## The evidence gate and verify-disposition verdicts

Shape validation alone once let a fix-class task complete `done` on any
non-empty evidence string while its issue was still open and a draft PR
still carried the work. Now a completion that requires a disposition must
carry `result.disposition`: a verified readback record produced by
`fleet queue verify-disposition <issue> --repo <owner/name>`.

`verify-disposition` reads the issue and its linked PRs **back from GitHub,
live** — never from a remembered claim. A PR counts as linked when named by
queue provenance or `--pr`, or when its body carries an explicit marker
(`Refs #n`, `Fixes #n`, `Related program: #n`); a bare prose `#n` is not a
link. `--repo` is required (exit 2 without it). The record is
`{checked_at, source, issue: {number, state, state_reason, closed_at,
closed_by}, pulls: [{number, state, is_draft, merged_at}], verdict,
follow_up}`.

The gate is pure (no network). It checks freshness (at most **1800 s** old
by default; more than 300 s in the future is unreadable clock skew), that
the record describes the task's issue, that the verdict matches the
readback, that every PR the completion result links appears in the readback,
and any follow-up. **Every rejection leaves the task claimed** and names the
defect. `verify-disposition` itself exits 0 when its record would pass the
gate, 1 when the gate would reject it or the readback cannot be read.

The five verdicts:

| Verdict                    | Derived when                                                     | Gate requires                                  |
| -------------------------- | ---------------------------------------------------------------- | ---------------------------------------------- |
| `merged_verified`          | some pull in the readback is merged                              | at least 1 merged pull in the readback         |
| `closed_verified`          | issue closed and no unfinished pulls                             | the issue state actually `closed`              |
| `open_draft_with_followup` | unfinished pulls exist, nothing merged, issue not cleanly closed | a recorded `follow_up`                         |
| `open_draft_superseded`    | operator-asserted, same family                                   | a recorded `follow_up`                         |
| `invalid_no_work`          | none of the above                                                | rejected if any linked pull still carries work |

A pull is _unfinished_ when it is not merged and not closed. A follow-up is
exactly one of `--follow-up-task <id>` (checked by the broker at completion —
only the queue knows if it is live), `--follow-up-issue <n|url>` (same-repo,
open, must link the completed issue and every unfinished PR), or
`--follow-up-pr <n>` (must be in the readback and merged); any follow-up
target requires `--reason`. One exception: a closed issue may leave related
work open when the close was a deliberate act by a person (the readback's
`closed_by` is checked against automation) _and_ a verified follow-up owns
the remainder.

`complete`'s other rules: `--result`/`--result-file` must be a JSON object
(the file wins when both are given); `--telemetry` accepts only the keys
`pool, model, effort, cost_usd`; `--queue` is parsed but never used — a dead
flag, stated here because the source says so.

## The fallback queue and the permanent fallback switch

The fallback queue is a queue with no database: all state lives in GitHub
issue comments. Work items are open issues carrying the labels
`queue/queued` (waiting), `queue/claimed` (held), or `queue/blocked`
(blocked) — the same labels the broker-backed system maintains. The sweep
covers five repos in pilot order: `developing-today/code`,
`developing-today-forks/opencode`, `developing-today-forks/openchamber`,
`dezren39/darwin-nix`, `dezren39/fleet` (shortcuts `code`, `opencode`,
`openchamber`, `nix`, `fleet`).

A claim is the first line of a comment, exactly
`<!-- fleet-claim v1 kind=claim worker=<id> ts=<rfc3339-utc> -->` (kinds:
`claim`, `heartbeat`, `release`, `unblock`), with a human-readable line
under it. Because every fleet worker posts to GitHub as the same account,
the marker's worker id is the only identity the lease can be keyed on, so
`--worker` is **required, never defaulted** (or the `FLEET_FALLBACK_WORKER`
environment variable). The lease window is **30 minutes** by default
(`--lease-min` per verb, or `FLEET_QUEUE_FALLBACK_LEASE_MIN`). Folding an
issue's markers in comment order gives its claim state: `free`, `released`,
`live`, `stale` (older than the lease — any worker may claim, superseding),
or `contested` (a claim posted during another's live claim; hands-off for
everyone while the contest is younger than the lease). Claimable = free,
released, or stale. Fallback `complete` runs the same offline disposition
gate as the broker path before posting anything; a rejection posts nothing
and exits 1.

**The fallback switch is permanently armed.** For the work verbs — claim,
heartbeat, checkpoint, complete, health — _any_ primary-store failure
(broker unreachable, or an `{ok:false}` envelope) switches the caller to the
GitHub fallback for that work instead of dying: a message on stderr, one
JSON object on stdout with `"fallback": true, "switch_to":
"github-fallback"` and a hint naming the fallback verbs, and exit code
**6**, so a worker loop can branch on it without parsing prose. The escape
hatch `REPO_QUEUE_NO_FALLBACK_SWITCH=1` restores plain die-on-error. The
primary claim path also guards the other direction: it refuses to
double-serve an issue the fallback currently holds (see the claim path in
[Architecture](architecture.md#data-flows)).

Queue client exit codes, in full: 0 success; 1 gate failure / task not
found; 2 usage; 3 transport or broker-path failure; 4 a broker `{ok:false}`
envelope carrying no exit of its own (and broker-side store failures); 5 no
matching claimed row / no claimable task; 6 the fallback switch.

## The creation outbox

Creating GitHub objects (issues, PRs, comments) must survive crashes and
rate limits, so creation goes through the _creation outbox_: producers
enqueue, a single serial drainer creates through `gh`, verifies by
read-back, and resolves. There is no dual-write and no automatic fallback to
direct creation — a broker failure stops the drain visibly.

Entry kinds: `issue`, `pull_request`, `issue_comment`, `pr_comment`. States:
`pending`, `leased`, `uncertain`, `done`, `blocked`. Constants: claim lease
**120 s**, minimum gap between creates **2 s**, hourly create budget
**300**, retry backoff base **30 s** capped at **3600 s**, a circuit breaker
that opens after **3** consecutive rate limits for **300 s**, default max
attempts **8**. Entries carry a `fleet-outbox:<id>` marker in the created
object's body so the drainer can reconcile by search and read-back. The
verbs are `outbox enqueue|list|metrics|claim|resolve|drain`; `drain`
defaults to at most 20 items (`--max`, floored at 1), and an `uncertain`
entry is reconciled by marker first — a lookup that _could not check_ keeps
the entry uncertain and never re-creates.

## Merge-intent stacks

A _stack_ is a recorded, ordered set of merge intents — finished branches
with their authoring lane, worker, and session — stored in the broker's
`merge_intents` table, unique on `(stack_id, position)` and `(repo,
branch)`. Intent states: `intended → replaying → landed`, plus `conflicted`
and `abandoned`; the legality of every move is decided by a pure transition
table, never in SQL, and a conflicted intent returns only through its
recorded authoring lane.

The verbs are `stack record|list|transition|restack|replay`. `record`
requires `--stack`, `--repo`, `--branch`, and `--lane`. A transition to
`conflicted` requires `--conflict-detail` naming what the authoring lane
must resolve. `replay` is the executor: it walks the stack's recorded merge
order bottom-up — rebase onto new main, push (force-with-lease pinned to the
verified pre-rebase tip), open PR, merge, mark landed, post the outcome note
— one intent at a time. Replay never resolves a conflict itself. Its exit
codes: 0 landed or idle, **4** a conflict was recorded `conflicted` and
routed to its authoring lane, **3** any other step failure (the intent is
left `replaying` and a later `replay` resumes it).

## The Turso store

The broker creates its schema at backend start. The `tasks` table carries:
`id`, `queue`, `title`, `body`, `payload` (default `'{}'`), `dedupe_key`,
`priority`, `status` (default `'queued'`), `not_before`, `worker_id`,
`claimed_at`, `lease_until`, `lane_id`, `platform`, `attempts` (default 0),
`max_attempts` (default 1000), the claim stamps (`claim_provider`,
`claim_model`, `claim_variant`, `claim_session_id`), `progress` and `result`
(default `'{}'`), `last_error`, the park columns (`park_kind`, `wait_kind`,
`ready_condition`, `required_budget_minutes`, `last_checked_at`,
`check_outcome`), the dep-block columns (`blocked_deps`,
`dep_next_recheck_at`, `dep_blocked_until`, `dep_recheck_seconds`),
timestamps, and `completed_at`. Its `(queue, dedupe_key)` index is
deliberately **not unique** — enqueue dedupe is a read-then-insert check.
Alongside: `leases` (one row per task), `lanes` (seeded with the 156
canonical lanes when empty), `workers`, `checkpoints`, `repo_task_lane_ledger`,
`events`, `creation_outbox` (+ its circuit singleton), and `merge_intents`.

Broker protocol facts: every verb is one `POST /<command>` with a JSON args
object; the response envelope is `{ok:true, ...}` or `{ok:false, exit,
error}`. The only unauthenticated route is `GET /healthz`. The broker binds
_first_ and brings the Turso backend up on a startup thread, so `/healthz`
and `/validate` answer immediately while other verbs answer exit 3 with a
`turso_pending` reason until the backend is ready. A transient store error
carrying PostgREST code `57014` or `PGRST002` is retried client-side with
exponential backoff and jitter: base 500 ms, doubling, cap 20 s, at most 6
attempts.
