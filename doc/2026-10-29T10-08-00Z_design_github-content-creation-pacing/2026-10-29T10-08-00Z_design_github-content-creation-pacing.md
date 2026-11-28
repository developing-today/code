# Fleet content-creation pacing: measured rates, the 2026-10-28 block, and a shared throttle

Status: **investigation and design (queue task 3334, issue
[#2013](https://github.com/developing-today/code/issues/2013)).**
No implementation yet; the follow-on issue owns it.

## Summary

The fleet ran into GitHub's secondary rate limit for **content
creation** on 2026-10-28 and the filing scripts did not notice: `gh
issue create` exited 0 with no output while creating nothing. Both
halves of that sentence are measured below.

- Every lane authenticates as the same account (`dezren39`), so
  GitHub's content-creation ceiling is a **single account-wide budget**
  that all jobs and both worker lanes share. No job can see the
  others' usage.
- In `developing-today/code` alone the fleet produced **2 916
  content-creating events on 2026-10-28** (issues, PRs and comments),
  peaking at **511 in one hour** (22:00Z) — the documented ceiling is
  500 per hour.
- Creation stopped dead for **31 minutes 48 seconds** after issue #1999
  at 23:08:35Z, resumed at 23:41:24Z, and the prepared 42-issue program
  filing did not complete that evening.
- Comments, not issues, dominate: **359 of the 511** events in the peak
  hour were comments, most of them agent notes.

The design that follows is three parts: a **creation ledger** every
filing path writes through, a **shared throttle** with per-lane budgets
and a breakage reserve, and **verify-by-read** after every create call
so a silent no-op can never be recorded as success.

## What GitHub documents

Checked 2026-10-29 against
[Rate limits for the REST API](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api).

| Limit                          | Value                                           |
| ------------------------------ | ----------------------------------------------- |
| Content creation per minute    | ~80 content-generating requests                 |
| Content creation per hour      | ~500 content-generating requests                |
| Concurrent requests            | 100 (shared across REST and GraphQL)            |
| Points per endpoint per minute | 900 REST; 2 000 GraphQL                         |
| CPU time                       | 90 s per 60 s wall clock (≤ 60 s of it GraphQL) |

Facts that matter for the design, all from that page:

- Content creation limits cover **web UI actions as well as REST and
  GraphQL**, and some endpoints have **lower** limits.
- There is **no way to query secondary-limit status**. `GET /rate_limit`
  and the `x-ratelimit-*` headers describe the primary limit only; they
  can be near-empty while creation is refused.
- A breach returns **403 or 429**, optionally with `retry-after`. With
  no `retry-after`, wait at least 60 s. The page adds: "Continuing to
  make requests while you are rate limited may result in the banning of
  your integration."
- The 5 000/hour primary budget is per user across tokens and apps, so
  adding a token does not create a second creation budget.

## The incident, as measured

Source: GitHub API. Issues and PRs from
`repos/developing-today/code/issues?state=all`, comments from
`repos/developing-today/code/issues/comments`, bucketed by `created_at`
on the laptop on 2026-10-29.

| Day (UTC)   | Issues + PRs | Comments    | Total events |
| ----------- | ------------ | ----------- | ------------ |
| 2026-10-26  | 107          | not pulled¹ | —            |
| 2026-10-27  | 494          | 1 363       | 1 857        |
| 2026-10-28  | 479          | 2 437       | 2 916        |
| 2026-10-29² | 217          | 1 325       | 1 542        |

¹ The comment pull used `since=2026-10-27T00:00:00Z` (5 129 comments),
so 26 October has no comment census.
² Through 10:00Z.

Peak windows:

| Window                 | Events                   | Composition                   |
| ---------------------- | ------------------------ | ----------------------------- |
| 2026-10-28 22:00Z hour | 511                      | 359 comments + 152 issues/PRs |
| 2026-10-27 03:00Z hour | 367                      | —                             |
| 2026-10-29 01:00Z hour | 252                      | the batch-2 program filing    |
| 2026-10-29 01:00Z min  | 39                       | highest minute                |
| 2026-10-27 03:44Z min  | 36                       | —                             |
| Ten-minute stretches   | 144, 129, 95, 95, 88, 84 | all under the 80/min ceiling  |

The 22:00Z hour sits at the documented 500/hour ceiling for this one
repository. `openchamber`, `opencode` and `nix` added roughly 288 more
created or updated items in the same evening on the same account, and
web-UI actions are not visible in this data at all.

The gap:

```
23:08:35Z  issue #1999 created  (the repo's 2 000th item, a heavy filing evening)
23:09–23:39  no issues, PRs or comments created in the repository
23:41:24Z  PR #2000 created, then steady ~2–4/min
00:58:54Z  #2013 created — reported as the probe that confirmed creation had cleared
00:59–01:04Z  batch-2 filing, 33 events in one minute, 252 in the hour
```

The gap length (31m48s) is consistent with a secondary-limit block, not
with an idle evening: work was queued and the fleet was active.

**Evidence caveat.** No client log of the 403 itself exists in this
worktree or on the laptop's fleet tooling; the 403 and the "gh exited 0
with empty output" behaviour come from the issue report and were not
reproduced here. The gap, the rates and the composition are measured
independently and do not depend on that report.

## Why one lane's burst blocks the whole account

1. **One identity.** Queue lanes A and B, both worker workers, the
   producers, the sweep jobs and the interactive agent all run `gh` as
   `dezren39`. 1 900 of 1 978 recently created items carry that login.
   The account budget is shared, but nothing is shared to enforce it.
2. **No pacing.** Filing is "file the batch now": a prepared 42-issue
   program becomes a 42-issue burst. The 01:00Z minute of 39 events is
   that pattern at work.
3. **Comments are the volume.** Agent-note discipline (picked up, every
   checkpoint, parked, blocked, done) turns each task into 3–6
   creations. On 2026-10-28 that was ~2 440 comments — five times the
   issue/PR count. The queue already stores checkpoints; the GitHub
   comment duplicates them.
4. **Nothing reads before writing.** No caller checks "did this
   actually get created", so the block produced no error the fleet
   could see, only silence.

## Failure mode: success that created nothing

`gh issue create` exits 0 in some conditions while the API call fails
(a secondary limit answered with a 403 that `gh` surfaces as no output,
or a retry layer swallowing it). The design rule is therefore blunt:
**an empty response is a failure**, and success is only what a
follow-up read confirms.

## Design

### 1. Creation ledger

One append-only ledger of content-creating calls, shared by every path
that writes to GitHub:

```
~/.local/state/fleet/content-ledger.sqlite
  events(ts_utc, actor, job, repo, kind, ref, outcome)
  kind ∈ issue | issue_comment | pr | pr_comment | review | label_write | other
```

- Written **before** the call (as a reservation) and updated with its
  outcome after the verifying read.
- Windows are computed from the ledger, not from memory in one process:
  last 60 s, last 3 600 s, per actor and account-wide.
- SQLite `BEGIN IMMEDIATE` gives cross-process atomicity. All fleet
  lanes run on one laptop today, so a local file is sufficient; a
  sandbox lane that files must reach the same ledger (open question 1).

### 2. Shared throttle

A wrapper every filing script calls instead of `gh ... create`:

```python
throttle.reserve(job="producer-batch-2", kind="issue")  # sleeps as needed
res = throttle.create_issue(title, body, labels)        # gh call + verify
```

Throttle parameters, chosen at ~60 % of the documented ceiling to leave
room for interactive and web-UI use:

| Control                  | Value                                    |
| ------------------------ | ---------------------------------------- |
| Account-wide sustained   | 45 creations/minute, 300/hour            |
| Minimum spacing per POST | 1 s (documented best practice)           |
| In-flight content calls  | ≤ 8 (documented concurrency ceiling 100) |
| Per-lane share           | 50/hour sustained, reclaimable when idle |
| Breakage reserve         | 75/hour held for priority-10 work only   |
| Bulk program batch       | ≤ 20 issues per 30-minute window         |

Fairness: lanes reserve against their own hourly share first; unused
share returns to a common pool at the next minute boundary, so an idle
lane never blocks a busy one. Priority-10 (breakage) work may draw on
the reserve and preempts batches, never interactive work.

### 3. Verify-by-read

`create_*` returns success only when a read confirms the object:

1. Parse the number/URL from stdout **and** require a non-empty body.
2. If empty or unparsable, do not retry blindly: search for a unique
   nonce embedded in the body (`<!-- fleet-nonce:… -->`) — a create
   that succeeded without printing its URL is common, a create that
   failed silently is indistinguishable without the read.
3. Record `outcome = created | verified_existing | failed` in the
   ledger. Only `created`/`verified_existing` count toward a batch's
   success tally.

### 4. Backoff and circuit breaker

- 403/429 with `retry-after`: sleep that many seconds, then one retry.
- 403/429 without it: sleep 60 s, then exponential (×2, five attempts).
- After five failures: **open the circuit** (no content creation for 15
  minutes), mark the calling task parked with a resume brief, alert.
- Never spin: the docs' ban warning is the reason the breaker exists.

### 5. Structural reduction (the cheapest fix)

- Replace per-checkpoint agent comments with **one live note per task**,
  edited in place (claim → the same comment gains parked/done lines).
  The queue checkpoint already records stage; the comment only needs to
  be the human-readable trail. Target: ≤ 2 comments per task.
- Producers file programs through the throttle in windows, not "now".

## Adoption

- The tooling lives outside this repository (`~/hatch/tools`,
  `repo-queue.py`, the producer scripts). This document is the design
  record; the implementation issue is filed against the fleet tooling
  with this file as its spec.
- Rollout: ledger + verify-by-read first (they are additive and fix the
  silent-success bug), then the throttle in warn-only mode (log what it
  _would_ have delayed for a week), then enforcing.
- Success measure: no 403 on creation for 14 days, and the per-hour
  account total staying under 300 while queue throughput is unchanged.

## Open questions

1. **Sandbox lanes.** If any lane files from the sandbox rather than the
   laptop, the ledger needs a small localhost service; a SQLite file on
   one host will not coordinate across hosts.
2. **Do PATCH/PUT count?** Editing a comment is content generation in
   spirit; assume yes for budgeting, but confirm against real behaviour
   before relying on "one live note" to be free.
3. **Per-token budgets.** The docs say limits are per user/client ID;
   it is not stated whether two tokens on one account get separate
   creation buckets. Treat as one budget until proven otherwise.
4. **The 42-issue program.** Whether its items were partly created and
   lost, or never sent, was not determined here; the ledger makes that
   answerable next time.
5. **Alerting owner.** The gap detector needs a home (fleet monitor is
   the natural one) and a threshold (proposal: > 5 min with queued
   filing work and no creation).

## References

- GitHub, [Rate limits for the REST API](https://docs.github.com/en/rest/using-the-rest-api/rate-limits-for-the-rest-api) (content creation, concurrency, points, ban warning).
- GitHub, [Best practices for using the REST API](https://docs.github.com/en/rest/using-the-rest-api/best-practices-for-using-the-rest-api) (serial requests, 1 s spacing).
- Issue [#2013](https://github.com/developing-today/code/issues/2013) — the request and the incident report.
- Parent epic [#2055](https://github.com/developing-today/code/issues/2055) — fleet expansion and placement program; #2013 is batch-2 item B11.
- Queue task 3334 (`repo-health-code-pilot`), this investigation.
