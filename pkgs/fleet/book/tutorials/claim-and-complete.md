# Tutorial: claim and complete a queue task

This tutorial works one queue task from claim to a verified completion:
claim it (through `fleet agent next`), keep the lease alive with
heartbeats, record progress with checkpoints, build the completion's
evidence with `verify-disposition`, and complete — including what the
evidence gate's rejection looks like and how to recover from it.

It assumes the `fleet` binary is installed (see
[Installation and bootstrap](../getting-started/installation.md)) and the
seat is enrolled (see [Tutorial: enrol a new seat](enrol-a-seat.md)).

One warning before Step 1, because it decides whether Step 6 works at
all: **a claim taken through `fleet agent next` may be registered under
the worker and lane `agent-next-05`, not under your agent name and not
under the queue client's defaults.** Every holder verb after the claim —
heartbeat, checkpoint and complete — is checked by the broker against
the worker (and the lane, when you supply one). Completing with the
wrong worker or lane does not complete the wrong task; it finds no
matching claimed row at all. Step 2 shows how to read the real worker
and lane out of the claim, and Step 6 uses them exactly:
`--worker agent-next-05 --lane agent-next-05`.

## Step 1: Claim through `fleet agent next`

```bash
fleet agent next --agent <your-agent-name>
```

What the output means: one JSON object, `{agent, rung, task, then,
tried}`. For queue work, `rung` is `"queue"` and `task` is the claimed
task row. The claim behind this rung is an ordinary queue claim with a
four-hour lease (`14400` seconds, overriding the queue client's
45-minute default). The `then` lines are part of the contract: they name
the lease deadline and the exact checkpoint and complete commands for
this claim, with the right task id, worker and lane already filled in.
Prefer those lines over retyping flags from memory.

Exit `5` from `next` means no rung had work — there is nothing to claim
right now, and this tutorial ends there until work exists. Exit `2` is a
usage error; exit `3` is a leg down (run
`fleet agent doctor --agent <your-agent-name>` and fix the leg).

You can also claim directly with the queue client:

```bash
fleet queue claim --worker <id> --lane <l>
```

A direct claim prints the task row verbatim as JSON — `{id, queue,
status, dedupe_key, payload, …}` — and exits `0`. Two direct-claim rules
surprise people:

- A **bare claim is refused before any broker contact**, exit `2`: a
  claim with no task, no filter, no queue scope, no count, no
  handoff/bind and no identity marks (worker, lane, session-id, pool,
  platform, from-session) "is never a meaningful intent". Passing
  `--worker` explicitly counts as intent even when it equals the
  default.
- **No claimable task** prints `{"task": null}` — deliberately carrying
  no id, queue or status a caller could mistake for a task — plus a
  diagnostic on stderr of the form `no claimable task — Q queued (A at
  the attempt cap, D not yet due), P parked due`, and exits **5**.

Keep the claimed row. Everything below needs its `id`, and Steps 2–6
need the worker and lane the claim was stamped with.

## Step 2: Read the claim's worker and lane

```bash
fleet queue show <task-id>
```

What the output means: the task's current details, sourced from the
broker's rich row for the task, plus the tail of its event timeline.
Confirm the row is `claimed`, and note the worker and lane recorded on
it. When the claim came from `fleet agent next`, expect the worker/lane
pair `agent-next-05` / `agent-next-05` — the shared identity the agent
ladder claims under. All scheduled lanes share related worker ids, which
is exactly why the lane matters: it is what tells the broker whose claim
a later verb acts on.

What can go wrong, and the fix:

- `repo-queue show: task <id> not found`, exit `1`: the id is wrong, or
  it never was a broker task id. Go back to the claim output from Step 1
  and copy the `id` from the row itself.
- Exit `3`: the broker path failed (unreachable, token rejected, or a
  broker that does not serve the event routes). The claim itself is
  unaffected; fix the transport and retry the read.

From here on, this tutorial writes the holder flags as
`--worker agent-next-05 --lane agent-next-05`. If your Step 1 claim was
a direct claim under a different worker/lane, substitute the pair your
claim actually used, everywhere, unchanged.

## Step 3: Heartbeat to keep the lease

A claim is a lease. If it lapses, the task returns to the pool and
another worker may take it. A heartbeat sets the lease to *now plus the
extend* on both the task row and the lease record.

```bash
fleet queue heartbeat <task-id> --worker agent-next-05 --lane agent-next-05
```

What the output means: the broker's updated task row as JSON, plus a
`mode_line` describing the route the verb took. The default extend is
**900 seconds (15 minutes)** — note the asymmetry: a claim's default
lease is 2700 seconds, but a heartbeat only adds 900 unless told
otherwise. For a long stretch of work, say so:

```bash
fleet queue heartbeat <task-id> --worker agent-next-05 --lane agent-next-05 --extend-seconds 3600 --progress "halfway through the fix"
```

`--extend-seconds` must be in `1..=86400`; values are rejected, never
clamped, and `0` or a negative value would lapse your own claim, so it
dies with exit `2`. `--progress <p>` (or `--progress-file <f>` — the two
are mutually exclusive, both together is exit `2`) merges a progress
value into the row.

What can go wrong, and the fix:

- Exit `5` with `heartbeat: no matching claimed row (task not claimed by
  this worker, or already terminal)`: the broker does not believe you
  hold this task. The usual causes are a wrong `--worker`/`--lane` pair
  (re-read Step 2), a lease that already lapsed and was re-claimed by
  someone else, or a task that is already terminal. Do not retry blindly
  — check `fleet queue show <task-id>` first.
- Exit `6`: the primary queue store failed and the verb switched this
  work to the GitHub fallback. The output is one JSON object naming the
  switch, with a hint of the form `continue this work with: fleet queue
  fallback scan|claim|heartbeat|release|unblock`. Follow it: from here
  the task is worked through the fallback verbs until the primary store
  returns. Setting `REPO_QUEUE_NO_FALLBACK_SWITCH=1` disables the switch
  and restores plain die-on-error, if you ever need the raw failure.

## Step 4: Checkpoint progress

A checkpoint records where the work has got to, without touching the
lease. The broker merges the progress into the task row, stamps
`progress.last_checkpoint = {stage, at}`, stores a per-worker checkpoint
record, and writes a `checkpoint` event.

```bash
fleet queue checkpoint <task-id> --worker agent-next-05 --lane agent-next-05 --stage implementing --progress "fix written; tests running"
```

`--stage` is **required**. The help line brackets it as optional; the
client does not — omitting it dies with `checkpoint: --stage is
required`, exit `2`. The output is the updated row plus a `mode_line`,
exit `0`.

What can go wrong, and the fix:

- Exit `5`: the same no-matching-claimed-row shape as the heartbeat.
  Same diagnosis: worker/lane pair, lapsed lease, or terminal task.
- Exit `6`: the fallback switch, exactly as in Step 3.

Checkpoint at meaningful boundaries — claim taken, fix written, tests
green, PR opened — not on a timer. A later reader (or a later you, after
a crash) reconstructs the work from these stages.

## Step 5: Build the completion evidence with `verify-disposition`

A completion whose work is verifiable on GitHub must carry a
**disposition**: a fresh, live readback of the issue and its linked PRs,
produced by `verify-disposition`. It reads GitHub live — never a
remembered claim — and prints the record the completion gate will check.

```bash
fleet queue verify-disposition <issue-number> --repo <owner/name> --json
```

`--repo` is required (exit `2` without it). Save the `--json` output to
a file; Step 6 feeds it to `complete` as the result's `disposition`.

What the output means: a record of the form `{"checked_at", "source",
"issue": {number, state, state_reason, closed_at, closed_by}, "pulls":
[{number, state, is_draft, merged_at}], "verdict", "follow_up"}`. The
verb then runs the offline gate on its own record before returning:

- Exit `0`: the record would pass the broker's gate.
- Exit `1`: the gate would reject it, or the issue/PR list could not be
  read from GitHub at all.
- Exit `2`: a usage error.

The verdict is one of exactly five:

| Verdict | Means |
|---|---|
| `merged_verified` | Some pull in the readback is merged |
| `closed_verified` | The issue is closed and no unfinished pulls remain |
| `open_draft_with_followup` | Unfinished pulls exist, nothing merged, and a follow-up is on record |
| `open_draft_superseded` | Operator-asserted variant of the above, also requiring a follow-up |
| `invalid_no_work` | No merged pull, issue open, no unfinished pull |

When the honest verdict needs a follow-up — typically
`open_draft_with_followup`, because a draft PR still carries work —
name exactly one follow-up target and give a reason (any follow-up
target **requires** `--reason`, exit `2` without it):

```bash
fleet queue verify-disposition <issue-number> --repo <owner/name> --follow-up-pr <n> --reason "the remaining work moved to the successor PR" --json
```

The alternatives are `--follow-up-task <id>` (checked by the broker at
completion time — only the queue knows whether that task is live) and
`--follow-up-issue <n|url>` (same repo only; its own readback is embedded
and checked: it must be open, must not be the issue being completed, and
must link the completed issue and every unfinished PR with an explicit
marker such as `Refs #n`). A successor PR must be in the readback and
must be merged.

Two freshness facts to plan around: the record goes stale after **1800
seconds (30 minutes)** by default (`--staleness-seconds` changes it for
this verb), and a record more than 300 seconds in the future is
unreadable clock skew. Run this step immediately before completing, not
at the start of the work.

## Step 6: Complete — with the evidence gate

```bash
fleet queue complete <task-id> --worker agent-next-05 --lane agent-next-05 --result-file result.json
```

where `result.json` is a JSON **object** carrying, for a verifiable
completion, the disposition record from Step 5 under the key
`disposition`, plus any `pr` / `pr_url` / `merged_pr` fields naming the
PRs the completion claims. (`--result <json>` passes the object inline
instead; when both are given, `--result-file` wins. A result that is not
a JSON object dies with exit `2`.)

What the output means on success: the task row — or
`{task, lane_ledger}`, and for a triage completion possibly a
`follow_on_task_id` — with the task now in the terminal status `done`.
Exit `0`.

The evidence gate is pure (no network): before accepting the completion,
the broker parses the disposition record and checks its freshness, that
it describes this task's issue, that the verdict matches the readback,
that **every PR the result links appears in the readback**, and any
follow-up. A completion that requires a disposition and does not carry
one is rejected the same way.

### What a gate rejection looks like, and how to recover

A rejection names the defect and ends with the sentence that matters:
**"…; the task remains claimed"**. The completion did not land, nothing
terminal was written, and your claim — worker, lane and lease — is
exactly where it was. Recovery is therefore always the same shape:

1. Read the named defect literally. The common ones: the record is
   stale (older than 1800 s); the verdict does not match the readback
   (for example claiming `merged_verified` with no merged pull in it);
   a linked PR is missing from the readback; unfinished pulls remain
   with no follow-up on record; a follow-up issue does not link the
   completed issue or is itself closed.
2. Fix the underlying fact, not the wording. Merge the PR, close the
   issue properly, file and link the follow-up, or correct the result's
   PR fields so they name only PRs the readback contains.
3. Re-run Step 5 to mint a **fresh** disposition against the corrected
   reality — never edit a stale record by hand; its `checked_at` is part
   of what the gate checks.
4. Re-run this step's `complete` with the same worker and lane. Keep
   heartbeating (Step 3) while you work through a rejection, so the
   lease does not lapse mid-recovery.

What else can go wrong here:

- Exit `5`, no matching claimed row: wrong worker/lane (see Step 2) or
  the lease lapsed during a long recovery. If the task was re-claimed
  by another worker, it is theirs now — do not complete over them.
- Exit `6`: the fallback switch. In the fallback, completion is
  `fleet queue fallback complete <repo> <issue-number> --worker <id>
  --summary <text> --result <path>`, where `--result` is the same
  `verify-disposition --json` record; it must pass the same offline gate
  **before anything is posted**, and a rejection there also posts
  nothing, leaves the claim live, and exits `1`.
- `--telemetry <json>`, if you pass it, may only carry the keys `pool`,
  `model`, `effort`, `cost_usd`; an unknown field dies with exit `2`
  listing the allowed set.

## You are done when

- `fleet queue show <task-id>` reports the task with status `done`.
- The completion you submitted carried a disposition record that
  `fleet queue verify-disposition <issue-number> --repo <owner/name>`
  itself exited `0` on, minted within the staleness window before the
  completion.
- No gate rejection is outstanding: if `complete` ever answered "…; the
  task remains claimed", you fixed the named defect, re-minted the
  disposition, and completed again — under the same `--worker` and
  `--lane` the claim was taken with.
