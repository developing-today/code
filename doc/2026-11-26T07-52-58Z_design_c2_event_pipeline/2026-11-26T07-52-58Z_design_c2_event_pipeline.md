# C2 — Event pipeline: per-kind queues, threshold drains, cron triage

Issue: #9570 (component C2 of the reporting-architecture program,
starter epic #9568). Status: design record. It names the event log,
defines the per-kind queues and their thresholds, sets the path by
which a queue past its threshold wakes Muse to drain it, and
classifies every repeated job the repo defines. It writes no code and
retires nothing; the build slices in §9 carry the work.

Two rules hold throughout:

- **Trigger layer only.** C2 decides when work gets looked at. It
  adds no dispatcher, no ranking and no claim path; drained items
  become ordinary queue tasks that #9196's selection and SEL-02's
  claim clause handle.
- **Nothing retires until its replacement has run for one full
  cycle.** Every retirement in §4 D8 is a disposition, not an action.

## 0. Governing decisions consumed

| Source                                            | What this record takes from it                                                                                                      |
| ------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------- |
| #9570 and Drew's directive, 2026-11-13            | Log and store every event, gather per-kind queues, drain on ≥10 items or oldest ≥15 min, PM stack triggers Muse, triage every cron. |
| Program note (`2026-11-13T15-12-00Z`, C2 entry)   | Drain actions: triage new issues, review open PRs, ensure a task minds each item, report quota current and projected.               |
| C3 (health hooks, same program)                   | Jobs whose failure is itself the event move to C3, not C2.                                                                          |
| C4 forecasting design (same program)              | The projected-quota line in each drain report comes from C4's prediction records.                                                   |
| SEL-02 record (`2026-11-25T01-18-23Z`)            | Hook-spawned tasks are claimed through the ordinary pick; none is `manual_only`.                                                    |
| Agent enrolment (`fleet/book/reference/agent.md`) | Muse's wake hookup: a signal-poll detector under `~/hooks/scripts/` and a hook definition under `~/hooks/definitions/`.             |
| `doc/fleet-schedule-registry.md`                  | The registry of jobs defined outside the repo (Muse cron, OpenChamber, launchd, Codex lanes).                                       |
| Drew's ruling, 2026-11-24 (on #9196)              | Design-only gates without a hard blocker are void. This record does not wait for the #9568 summit.                                  |

## 1. Verified state on main `4f99c90e`

**Receivers.** Five GitHub webhook receivers live in the repo, and
none of them fills a per-kind queue:

| Receiver                        | Where                                              | Kinds handled                                                                                        | Writes                                                                                                                                                                        |
| ------------------------------- | -------------------------------------------------- | ---------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| github-ci Worker (Cloudflare)   | `pkgs/github-ci/worker/src/index.ts`               | `workflow_run`; for fleet repos also `push`, `pull_request`, `issues`, `issue_comment`, `sub_issues` | Turso `ci_events`; fleet-queue `events` via the broker's `/events-append` (`sink.ts:43`, direct insert fallback `:63`); `agent_watermarks` key `dirty:<repo>` (`sink.ts:124`) |
| pm-projector (laptop, Bun)      | `pm-stack/projector/src/server.ts:319`             | `issues`, `pull_request`, `issue_comment`, reviews, review comments, `push`                          | libSQL `pmp_*`, including the ledger `pmp_hot_events` and `pmp_consumer_checkpoints` (`schema.sql:182,202`)                                                                   |
| pm-aggregator metadata listener | `pm-stack/aggregator/src/server.ts:299`            | `issues`, `pull_request` (opened, edited, closed, labeled)                                           | bun:sqlite `pm_metadata_*`; enqueues triage to `repo-health-code-pilot` (`:286`)                                                                                              |
| fleet hosted-worker ingestion   | `pkgs/fleet/hosted-worker/src/routes/ingestion.ts` | `issues`, `pull_request`                                                                             | triage queue `repo-health-code-pilot`; not deployed (local only)                                                                                                              |
| github-backup Worker            | `infra/github-backup/worker/src/index.js`          | any, stored as `{repo, event, action}`                                                               | KV with a 7-day TTL; drained by a laptop timer every 5 minutes                                                                                                                |

- The github-ci Worker's `wrangler.toml` states that every fleet
  repo's webhook points at it.
- No receiver handles `check_suite`, `check_run` or mentions.
- Nothing in the repo reads the `dirty:<repo>` watermarks.
- The fleet-queue `events` table has `kind` and an index on
  `(kind, created_at)` (`turso_queue/core.rs:1105`). `agent_watermarks`
  is a key/value table in the same database.
- The "≥10 items or ≥15 min" rule appears only in the program note.
  No code implements a threshold drain, and `scripts/muse/` holds no
  drain hook.

**Gap fillers.** `pm-projector-reconcile` re-reads issues, PRs and
comments every 15 minutes from saved cursors
(`nixos/services/pm-stack/default.nix:696`). `queue-coverage-reconcile`
runs hourly at `:41` and imports a task for any open issue without
one (`scripts/fleet/queue-reconcile/`). The hosted-worker sweep has a
`*/15` cron that does nothing, because `INGESTION_REPOS` is empty
(`pkgs/fleet/hosted-worker/wrangler.toml:41-53`).

**Enqueue.** `fleet queue enqueue <packet> --queue Q` and the broker's
`POST /enqueue` (`verb_enqueue`, `turso_queue/stacks_outbox.rs:1016`)
deduplicate on `(queue, dedupe_key)`.

## 2. Intent

New GitHub activity lands in one durable log as it happens. For each
kind that needs a response, the log is also a queue with a depth and
an oldest age. When a queue crosses its threshold, Muse is woken once
to drain it: it triages, makes sure a task minds each item, and
reports. Heavy polling jobs that only existed to notice that activity
are then retired one at a time, each after its replacement has run
for a full cycle.

## 3. Ownership

| Part                                      | Owner                                                         |
| ----------------------------------------- | ------------------------------------------------------------- |
| Intake, kind derivation, threshold signal | github-ci Worker (`pkgs/github-ci/worker`)                    |
| Event log                                 | fleet-queue `events` table (Turso fleet-queue)                |
| Drain cursors and drain-due signals       | `agent_watermarks` keys `drain:<kind>` and `drain-due:<kind>` |
| Thresholds                                | one checked-in file read by the Worker and the drain runner   |
| Drain runner and its actions              | new `scripts/fleet/event-drain/`                              |
| Muse detector and hook definition         | the Muse host's `~/hooks/`, following the enrolment pattern   |
| Projection into PM tables                 | pm-projector, unchanged                                       |
| Cron dispositions                         | this record (§4 D8); execution in slice C2-S4                 |

## 4. Decisions

### D1. One event log: the fleet-queue `events` table

The github-ci Worker is the intake of record and the fleet-queue
`events` table is the log. The Worker is always on, already receives
every fleet repo's webhook, and already writes `events` with a
`kind`. The Muse host may be down; an intake that depends on it would
lose events in that window.

The other receivers keep their jobs and are not C2 queues:
pm-projector keeps projecting into `pmp_*` (the "inserted into the
appropriate databases" part); the aggregator listener keeps its
metadata store; github-backup keeps backing up. The hosted-worker
ingestion route stays undeployed and is not adopted. No new receiver
is built.

### D2. A per-kind queue is a cursor over the log

A per-kind queue is not a table. Queue `K` is the `events` rows with
`kind = K` and `id` greater than the watermark `drain:K`. Its depth
is a count over that range; its oldest age is now minus the
smallest `created_at` in it. Both reads use the existing
`(kind, created_at)` index. Draining advances `drain:K` forward
only, and never past the highest id the drain read at its start, so
a failed drain resumes from where it stopped and replays safely (the
rule `pmp_consumer_checkpoints` already follows).

This keeps one copy of every event and makes "gathered into a
queue" a read, not a second write.

### D3. Event kinds and thresholds

| Queue kind   | Comes from                                                 | Received today | Threshold (count or oldest) | Drain action                            |
| ------------ | ---------------------------------------------------------- | -------------- | --------------------------- | --------------------------------------- |
| `issue_new`  | `issues` opened or reopened                                | yes            | 10 items or 15 min          | triage; ensure a task minds the issue   |
| `pr_open`    | `pull_request` opened, reopened or ready for review        | yes            | 10 items or 15 min          | ensure a review task exists for the PR  |
| `ci_failure` | `workflow_run` completed with failure on a default branch  | yes            | 1 item or 5 min             | ensure a task minds the failing run     |
| `mention`    | `issue_comment` or review body naming a fleet identity     | no (derived)   | 1 item or 5 min             | ensure a task answers the mention       |
| `pr_review`  | `pull_request_review` submitted with changes requested     | no             | 10 items or 15 min          | ensure the PR's task sees the review    |
| log only     | `push`, `sub_issues`, other `issue_comment`, other actions | yes            | none                        | none; logged for projection and history |

The default from the directive is 10 items or 15 minutes. Kinds where
someone is waiting on a reply (a failed default-branch run, a
mention) drain on the first item, with a 5-minute age backstop.
Thresholds live in one checked-in file, so tuning a kind is a reviewed
change, not a hand edit on a host.

Two kinds are derived by the Worker at intake. `ci_failure` is a
`workflow_run` filtered by conclusion and branch. `mention` is a
comment or review whose body names a configured fleet identity. Each
derived row is written to `events` beside the raw event, under the
same delivery id, so the raw history stays intact.

### D4. How a threshold wakes Muse

1. **On append.** After the Worker writes a row of a queued kind, it
   reads that kind's depth. At or above the count threshold, it sets
   `drain-due:K` to the current time, unless that key is already
   set.
2. **On age.** A Worker cron every 5 minutes reads each queued kind's
   oldest age and sets `drain-due:K` the same way. The age rule
   cannot fire on an append, so it needs this clock. It runs on the
   Worker, not the laptop, so the signal is raised even while the
   Muse host is down.
3. **Wake.** A detector script on the Muse host, following the
   enrolment wake pattern (`~/hooks/scripts/` and
   `~/hooks/definitions/`), reads the `drain-due:*` keys and wakes
   Muse with the `event-drain` hook definition, naming the due kinds.
   This is the PM-stack-to-Muse trigger: the signal names the work;
   Muse does not scan GitHub to find it.
4. **Clear.** The drain clears `drain-due:K` only after it advanced
   `drain:K`. A drain that fails leaves the signal set, so the next
   detector pass wakes Muse again.

While the Muse host is down, signals and events accumulate in Turso
and nothing is lost; the first wake drains everything past each
cursor. The hourly coverage reconcile stays as the backstop that
every open issue has a task (D8).

### D5. What a drain does

The drain runner reads each due kind from its cursor to the high
water it read at start, and for each event:

- **Ensures a task minds the item** with `fleet queue enqueue`,
  deduplicated on a key per item (`gh:<repo>#<n>` for issues and PRs,
  `ci:<repo>:<run_id>` for runs, `mention:<comment_id>`). The issue
  key is the same key the coverage reconcile importer uses, so the
  two never create twin tasks.
- **Triages** new issues with the existing triage rules (labels and
  priority), through the aggregator's existing route, not new
  rules.
- **Writes one report** per drain: counts by kind, the tasks it
  created or found, and quota current and projected, read from the
  quota gatherer and C4's prediction records. Where C4 has no
  prediction yet, the line says so rather than guessing.

The drain makes no claim, merges nothing, and closes nothing.
Judgement about the items happens in the tasks it ensures, which the
ordinary pick hands out.

### D6. Integration topology

| Part                     | Role in C2                                                  | Why                                                        |
| ------------------------ | ----------------------------------------------------------- | ---------------------------------------------------------- |
| Cloudflare Workers       | intake, kind derivation, threshold signals, age cron        | always on; already the webhook target                      |
| Turso (fleet-queue)      | event log, cursors, signals                                 | reachable from the Worker, the laptop and sandboxes        |
| PM stack                 | projection and reports; raises nothing new                  | already projects every event; C2 only reads it for reports |
| Muse (OpenClaw)          | woken by the detector to run the drain and write the report | the directive's owner of judgement-bearing fleet jobs      |
| OpenChamber and OpenCode | run the tasks the drain ensures, through the normal pick    | no new dispatch path                                       |

This record is the proposal for the FDR that #9570 asks for. Its entry
in the #9180 decision index is filed with slice C2-S1.

### D7. Out-of-repo jobs

Muse's cron (about 131 jobs), OpenChamber scheduled tasks, launchd
jobs and Codex lanes are defined outside this repo. Their triage uses
the same dispositions as D8 and works from `doc/fleet-schedule-registry.md`,
whose rows are the source of truth for those definitions. That is
slice C2-S4, not this record.

### D8. Cron triage of repo-defined jobs

Dispositions: **keep** (time-based by nature, not noticing events);
**keep as backstop** (a gap filler that stays after hooks land);
**to C3** (failure is the event); **convert** (replace with a hook,
then retire after one verified cycle); **retire** (does nothing
useful today).

| Job                                                   | Schedule                   | Disposition      | Reason                                                              |
| ----------------------------------------------------- | -------------------------- | ---------------- | ------------------------------------------------------------------- |
| pm-projector-reconcile                                | every 15 min               | keep as backstop | catch-up scan for missed webhooks; feeds the PM projection          |
| queue-coverage-reconcile                              | hourly at :41              | keep as backstop | proves every open issue has a task; same dedupe key as D5           |
| hosted-worker ingestion sweep                         | every 15 min (Worker)      | retire           | inert: `INGESTION_REPOS` is empty and the route is not adopted (D1) |
| github-backup drain, daily, weekly, 01:30             | 5 min and calendar         | keep             | backup, not event noticing                                          |
| pm-git-sync                                           | every 30 s                 | convert          | polls for pushes; the Worker already logs `push`                    |
| fleet-feed-collector                                  | every 5 min                | convert          | collects the fleet feed by polling; the event log replaces the poll |
| auto-format workflow                                  | every 15 min               | convert          | formatting follows pushes; trigger on `push` instead                |
| pm-dispatch workflow                                  | issue labeled, weekly cron | keep             | already event-driven; the weekly run is its backstop                |
| pm-kaneo, pm-kaneo-sync-guard                         | 30 min, hourly             | keep             | sync with an external tool that sends no events here                |
| quota-gatherer and the quota exporters                | every 15 min               | keep             | C4 needs evenly spaced samples                                      |
| pm-quota-watchdog                                     | every 15 min               | to C3            | its failure is the event                                            |
| pm-tunnel-watchdog, pm-host-health                    | 2 min, 5 min               | to C3            | reachability checks                                                 |
| opencode, sandbox-link, lab-tunnel, pm-apps watchdogs | various                    | to C3            | reachability checks                                                 |
| queue-transport-alert (timer and Worker cron)         | every 10 min               | to C3            | its failure is the event                                            |
| conductor-silence                                     | every 30 min               | to C3            | detects silence; the absence is the event                           |
| dispatch-health-probe workflow                        | every 30 min               | to C3            | reachability check                                                  |
| pm-digest daily and weekly, quota-weekly-review       | calendar                   | keep             | calendar outputs                                                    |
| pm-invariant-sweeper, stale-run-sweeper               | daily, every 2 h           | keep             | invariant checks over state, not event noticing                     |
| agents, program-tending, daily ideation workflows     | calendar                   | keep             | scheduled work, not event noticing                                  |
| nightly test reruns (about 40 workflows)              | 07:17 daily                | keep             | scheduled tests are time-based by design                            |
| backups, reapers, janitors, scratch sweep, autocommit | various                    | keep             | maintenance                                                         |
| message-bridge, lab-ops-hub Worker crons              | daily                      | keep             | scheduled maintenance                                               |

Each **convert** row retires only after its hook replacement has run
for one full cycle of the old schedule's longest period with no
missed item, checked against the event log. Each **to C3** row moves
under C3's design and is not changed by C2.

## 5. Negative cases

- An event is never dropped because the Muse host is down: the log
  and the signals are in Turso, written by the Worker.
- A drain never advances a cursor past the high water it read at
  start, and never clears a signal before advancing the cursor.
- A replayed drain never creates a second task for an item: every
  enqueue carries a dedupe key, shared with the coverage reconcile.
- A drain never claims, merges, closes or reviews by itself.
- A log-only kind never raises a signal.
- No job is retired by this record.

## 6. Oracles and tests

- **Worker (vitest, run in CI):** kind derivation for `ci_failure`
  and `mention` with fixture payloads; a raw event and its derived
  row share a delivery id; depth at the count threshold sets
  `drain-due:K` once; the age cron sets it for an old item and not
  for a fresh one; a log-only kind sets nothing.
- **Drain runner:** with a fake log, the cursor stops at the start
  high water when new rows arrive mid-drain; a failure mid-drain
  leaves the signal set and the cursor at the last fully handled
  row; a replay creates no second task (fake enqueue sees the same
  dedupe key and returns the existing task).
- **End to end, the acceptance kind:** open a test issue in a
  scratch fleet repo; observe the `events` row, the `drain-due:issue_new`
  signal after the threshold, the wake, a task minding the issue, and
  the report. This is #9570's second acceptance box and belongs to
  slice C2-S2.
- **Cycle check for a convert:** for one full old period, every item
  the old job would have noticed appears in the event log and was
  drained; the check is a query, recorded in the retirement PR.

## 7. Compatibility, migration and reversal

- No schema change: `events` and `agent_watermarks` exist. New keys
  are namespaced `drain:` and `drain-due:`.
- New derived rows are additive; existing readers of `events` filter
  by kind and are unaffected.
- Reversal: stop the Worker's signal writes and the detector. The log
  keeps filling, cursors stay where they are, and the backstop jobs
  (D8) were never retired, so coverage continues unchanged.
- The Worker is deployed from main under the standing rule for
  Cloudflare Workers; the detector is installed on the Muse host.

## 8. Requirement trace

| #9570 acceptance or directive statement               | Where           |
| ----------------------------------------------------- | --------------- |
| Events logged and stored in databases                 | D1              |
| Gathered into per-kind queues                         | D2, D3          |
| Event-kind inventory, per-kind queue, threshold table | D3              |
| Muse hook drains on thresholds                        | D4, D5          |
| PM stack triggers Muse directly                       | D4 step 3, D6   |
| Triage, review, task minds each item, quota report    | D5              |
| Integration options evaluated, topology recorded      | D6              |
| Cron triage with a named disposition per job          | D7, D8          |
| Retire only after one verified cycle                  | D8, §6          |
| GitHub issues drain end to end                        | §6, slice C2-S2 |

## 9. Gates and build slices

| Slice | Content                                                                                                              | Hard edge                          |
| ----- | -------------------------------------------------------------------------------------------------------------------- | ---------------------------------- |
| C2-S1 | Worker: `ci_failure` and `mention` derivation, `pull_request_review` intake, count signal, age cron, thresholds file | none                               |
| C2-S2 | Drain runner, cursor rules, `issue_new` actions, Muse detector and hook definition, end-to-end check                 | C2-S1                              |
| C2-S3 | Actions for the other queued kinds; the report's quota lines                                                         | C2-S2; C4's records for projection |
| C2-S4 | Cron conversions and retirements per D8, one at a time; triage of out-of-repo jobs per D7                            | C2-S2 verified for one cycle       |

## 10. Non-goals

- A second dispatcher, ranking or claim path.
- Moving PM projection off pm-projector.
- Deploying the hosted-worker ingestion route.
- C3's hook scripts for reachability and failure checks.
- C4's forecasting model.
- Retiring any job in this record.
