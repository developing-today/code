# fleet queue: the fallback queue

The fallback queue is the GitHub-issue-comment queue: the queue used when the primary broker/store is down — and, via the permanent fallback switch, whenever a work verb's primary call fails. **All state lives in issue comments; there is no database and nothing to reconcile when the backend returns** — the broker system re-derives queue state from labels and its own tables as it already does after any drift.

Group usage:

```text
usage: repo-queue fallback <scan|status|claim|heartbeat|release|complete|unblock> ...
```

- **Work items** are open issues carrying the fleet's existing labels `queue/queued` (waiting), `queue/claimed` (held), `queue/blocked` (blocked) — the same labels the broker-backed system maintains; no parallel taxonomy. Blocked items sit outside the scan sweep and cannot be claimed.
- **Repos:** the sweep covers five, in pilot order — `developing-today/code`, `developing-today-forks/opencode`, `developing-today-forks/openchamber`, `dezren39/darwin-nix`, `dezren39/fleet`. Shortcuts: `code`, `opencode`, `openchamber`, `nix`, `fleet`. Full names are `developing-today/code`, `developing-today-forks/opencode`, `developing-today-forks/openchamber`, `dezren39/darwin-nix`, `dezren39/fleet`.
- **Worker identity:** every fleet worker posts to GitHub as the same account, so comment authorship cannot tell workers apart — the marker's worker id is the only identity the lease can be keyed on. Therefore `--worker` is **required, never defaulted** (or env `FLEET_FALLBACK_WORKER`); ids must match `[A-Za-z0-9._-]`, max 64 chars. Plain (unmarked) comments do NOT renew a claim.
- **Exit codes** mirror the primary client (see [core verbs](queue-core.md)): 0 ok, **2** usage, **3** the GitHub transport itself failed (gh missing/erroring), **5** the verb ran but the claim state forbids the action, **1** a `complete` whose disposition the gate rejected or whose record could not be read.
- All fallback verbs print JSON except where noted; every verb's JSON carries `"mode": "fallback"` on scan/status.

Sources: [fallback.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/fallback.rs), [client transport](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/client/transport.rs), [disposition.rs](https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/disposition.rs).

## The marker grammar

The claim is the _first line_ of a comment, exactly (the `fleet-claim v1` grammar):

```text
<!-- fleet-claim v1 kind=claim worker=<id> ts=<rfc3339-utc> -->
<!-- fleet-claim v1 kind=heartbeat worker=<id> ts=<rfc3339-utc> -->
<!-- fleet-claim v1 kind=release worker=<id> outcome=<done|free> ts=<rfc3339-utc> -->
<!-- fleet-claim v1 kind=unblock worker=<id> ts=<rfc3339-utc> -->
```

A human-readable line sits under the marker. The marker's embedded `ts` is authoritative. Marker reads paginate comments at 100/page, max 10 pages; a read that ends on a full 10th page **errors** rather than returning a truncated marker set.

## The lease and the claim-state fold

**The lease:** a claim stays live while the holder posts heartbeat markers. Default window **30 minutes**, overridable per-verb with `--lease-min` or globally with env `FLEET_QUEUE_FALLBACK_LEASE_MIN`. (30, not 20: worker checkpoints land at most ~15 min apart and the fix timebox is 45 min, so a 20-minute lease would let a live claim look dead between checkpoints.)

**Claim state fold** (a fold over the issue's marker events in comment order):

- `free` — no claim events. Claimable.
- `released {worker, outcome, ts}` — the holder released. Claimable.
- `live {worker, last_activity, age_seconds}` — last marker inside the lease. Not claimable by others.
- `stale {…}` — older than the lease; any worker may claim, superseding. Claimable.
- `contested {holder, challenger, ts}` — a claim posted while another's claim is live **contests** the issue: hands-off for everyone while the contest is younger than the lease. A contest ages out once every challenger has been silent a full lease window (a repeat claim refreshes that challenger's contest); a release from a challenger clears their contest at once. The holder may heartbeat throughout. Not claimable.

Claimable = free | released | stale.

## The permanent fallback switch (exit 6)

The GitHub fallback is **not** an outage-only mode anymore. For the work verbs — **claim, heartbeat, checkpoint, complete, health** — _any_ primary-store failure (broker unreachable, or an `{ok:false}` envelope from the broker) switches the caller to the GitHub fallback for that work instead of dying:

- stderr: `repo-queue: primary queue store failed on <verb> (exit <n>): <err>; switching this work to the GitHub fallback`
- stdout: one JSON object `{"ok": false, "fallback": true, "switch_to": "github-fallback", "verb": <verb>, "mode_line": {"verb": …, "mode": "db-degraded", "route": "github-markers"}, "broker_exit": <n>, "error": <err>, "hint": "continue this work with: fleet queue fallback scan|claim|heartbeat|release|unblock"}`
- **exit 6**, so a worker loop can branch on it without parsing prose.
- Escape hatch: env `REPO_QUEUE_NO_FALLBACK_SWITCH=1` restores plain die-on-error.

The primary claim path also guards in the other direction — the no-double-serve guard: after a successful primary claim, if the issue is `live` or `contested` in the fallback under a different worker, the primary task is blocked back with reason-class `fallback_held` and the claim answers exit 5 (see [core verbs](queue-core.md)).

## fleet queue fallback scan

```text
usage: repo-queue fallback scan [--repo <repo>] [--limit <1..100>] [--lease-min <minutes>]
```

| Flag            | Argument         | Default                                      | Meaning                                                                                                     |
| --------------- | ---------------- | -------------------------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `--repo`        | repo or shortcut | all five sweep repos                         | Restrict the sweep to one repo (shortcut names resolve: `code`, `opencode`, `openchamber`, `nix`, `fleet`). |
| `--limit`       | integer          | 15                                           | Max items; range 1..=100 (outside → usage error).                                                           |
| `--lease-min`   | integer minutes  | 30 (or env `FLEET_QUEUE_FALLBACK_LEASE_MIN`) | Lease window used to fold claim states.                                                                     |
| _(positionals)_ | —                | **none accepted**                            | `scan` takes **no positional arguments** — see discrepancy note.                                            |

**Behaviour.** Sweeps the fallback repos' labelled issues (`queue/queued`, `queue/claimed`, `queue/blocked`) and folds each issue's claim state. Output JSON: `{"mode":"fallback", "generated_at", "lease_seconds", "items": [{"repo","number","title","url","labels","blocked","claim": <state JSON>}], "summary": {"total", "claimable"}, "notes": […]}`. Items are ranked claimable first (free/released/stale), then live, then contested/blocked; a blocked item never ranks claimable even if its marker fold reads free — the label is the block. Per-issue comment-read failures land in `notes` and the issue is folded with no markers rather than failing the sweep.

Help: the group usage line lists `scan [--repo <repo>] [--limit <1..100>] [--lease-min <minutes>]` — `fallback scan` takes **no positionals at all** (they are a usage error); its repo filter is the `--repo` flag. The top-level usage lists the full fallback subcommand set, `complete` included. This chapter documents the implementation behaviour.

Exit codes: 0 ok · 2 usage (including positionals, out-of-range `--limit`) · 3 GitHub transport failed.

```text
$ fleet queue fallback scan --repo code --limit 5
{"mode": "fallback", "generated_at": "2026-11-23T04:30:00Z", "lease_seconds": 1800, "items": [{"repo": "developing-today/code", "number": 13165, "title": "Write the fleet book", "url": "https://github.com/developing-today/code/issues/13165", "labels": ["queue/queued"], "blocked": false, "claim": {"state": "free", "claimable": true}}], "summary": {"total": 1, "claimable": 1}, "notes": []}
```

## fleet queue fallback status

```text
usage: repo-queue fallback status <repo> <issue-number> [--lease-min <minutes>]
```

| Flag                          | Argument         | Default                                      | Meaning                                    |
| ----------------------------- | ---------------- | -------------------------------------------- | ------------------------------------------ |
| `<repo>` (positional)         | repo or shortcut | required                                     | The issue's repo.                          |
| `<issue-number>` (positional) | integer          | required                                     | The issue number.                          |
| `--lease-min`                 | integer minutes  | 30 (or env `FLEET_QUEUE_FALLBACK_LEASE_MIN`) | Lease window used to fold the claim state. |

**Behaviour.** Shows the folded claim state for one fallback issue. Output JSON: `{"mode":"fallback", "repo", "number", "url", "lease_seconds", "blocked": bool, "last_unblock": {"worker","ts"}|null, "claim": <state JSON>}` where state JSON is one of `{"state":"free","claimable":true}`, `{"state":"released","claimable":true,"worker","outcome","released_at"}`, `{"state":"live"|"stale","claimable":bool,"worker","last_activity","age_seconds"}`, `{"state":"contested","claimable":false,"worker"(holder),"challenger","contested_at"}`.

Exit codes: 0 ok · 2 usage · 3 GitHub transport failed.

```text
$ fleet queue fallback status code 13165
{"mode": "fallback", "repo": "developing-today/code", "number": 13165, "url": "https://github.com/developing-today/code/issues/13165", "lease_seconds": 1800, "blocked": false, "last_unblock": null, "claim": {"state": "live", "claimable": false, "worker": "repo-worker-code", "last_activity": "2026-11-23T04:28:00Z", "age_seconds": 120}}
```

## fleet queue fallback claim

```text
usage: repo-queue fallback claim <repo> <issue-number> --worker <id> [--lease-min <minutes>] [--note <text>]
```

| Flag                          | Argument         | Default                                       | Meaning                                                                                                                         |
| ----------------------------- | ---------------- | --------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------- |
| `<repo>` (positional)         | repo or shortcut | required                                      | The issue's repo.                                                                                                               |
| `<issue-number>` (positional) | integer          | required                                      | The issue number.                                                                                                               |
| `--worker`                    | worker id        | **required** (or env `FLEET_FALLBACK_WORKER`) | Claiming worker identity — the marker's worker id is the lease key. Must match `[A-Za-z0-9._-]`, max 64 chars. Never defaulted. |
| `--lease-min`                 | integer minutes  | 30 (or env `FLEET_QUEUE_FALLBACK_LEASE_MIN`)  | Lease window.                                                                                                                   |
| `--note`                      | text             | unset                                         | Human-readable note placed under the claim marker.                                                                              |

**Behaviour.** Refuses (exit 5) when the issue is blocked (label), closed, or under a live/contested claim by another worker. Otherwise posts the claim marker comment ("Picked up by `<worker>` — working this now. … after &lt;lease&gt; minutes of silence any other worker may pick this up."), then **re-reads the markers**: races are detected on read-back (if the fold now shows a contest or another holder, the claim did not take). On success it sets labels `queue/claimed` + `work/assigned`, removing `queue/queued`.

No-double-serve note: this is the fallback side of the guard described in [core verbs](queue-core.md) — a fallback claim on an issue the primary queue currently holds does not merge the two claims; workers must resolve the contest through the fold (the primary claim path blocks its own task back when it detects a live fallback hold by a different worker).

Exit codes: 0 claimed · 2 usage / missing or malformed `--worker` · 3 GitHub transport failed · 5 issue blocked, closed, or live/contested under another worker; or the read-back shows the claim did not take.

```text
$ fleet queue fallback claim code 13165 --worker repo-worker-code
{"mode": "fallback", "repo": "developing-today/code", "number": 13165, "claim": {"state": "live", "claimable": false, "worker": "repo-worker-code", "last_activity": "2026-11-23T04:30:00Z", "age_seconds": 0}}
```

## fleet queue fallback heartbeat

```text
usage: repo-queue fallback heartbeat <repo> <issue-number> --worker <id> [--lease-min <minutes>] [--note <text>]
```

| Flag                          | Argument         | Default                                       | Meaning                                                  |
| ----------------------------- | ---------------- | --------------------------------------------- | -------------------------------------------------------- |
| `<repo>` (positional)         | repo or shortcut | required                                      | The issue's repo.                                        |
| `<issue-number>` (positional) | integer          | required                                      | The issue number.                                        |
| `--worker`                    | worker id        | **required** (or env `FLEET_FALLBACK_WORKER`) | Holder identity. Never defaulted.                        |
| `--lease-min`                 | integer minutes  | 30 (or env `FLEET_QUEUE_FALLBACK_LEASE_MIN`)  | Lease window the renewed claim is folded under.          |
| `--note`                      | text             | unset                                         | Human-readable progress note under the heartbeat marker. |

**Behaviour.** Posts a heartbeat marker, renewing _your_ lease. Only the current holder's markers renew; heartbeating someone else's live claim does not take it over (the fold keeps the holder; your marker at most contests if it were a claim — a heartbeat is not).

Exit codes: 0 heartbeat posted · 2 usage / missing or malformed `--worker` · 3 GitHub transport failed · 5 the claim state forbids the action.

```text
$ fleet queue fallback heartbeat code 13165 --worker repo-worker-code --note "chapters 2 of 6 drafted"
{"mode": "fallback", "repo": "developing-today/code", "number": 13165, "claim": {"state": "live", "claimable": false, "worker": "repo-worker-code", "last_activity": "2026-11-23T04:45:00Z", "age_seconds": 0}}
```

## fleet queue fallback release

```text
usage: repo-queue fallback release <repo> <issue-number> --worker <id> [--lease-min <minutes>] [--outcome done|free] [--summary <text>]
```

| Flag                          | Argument         | Default                                       | Meaning                                                               |
| ----------------------------- | ---------------- | --------------------------------------------- | --------------------------------------------------------------------- |
| `<repo>` (positional)         | repo or shortcut | required                                      | The issue's repo.                                                     |
| `<issue-number>` (positional) | integer          | required                                      | The issue number.                                                     |
| `--worker`                    | worker id        | **required** (or env `FLEET_FALLBACK_WORKER`) | Holder identity. Never defaulted.                                     |
| `--lease-min`                 | integer minutes  | 30 (or env `FLEET_QUEUE_FALLBACK_LEASE_MIN`)  | Lease window.                                                         |
| `--outcome`                   | `done\|free`     | unset                                         | `done` keeps the terminal state; `free` hands the work back unworked. |
| `--summary`                   | text             | unset                                         | Human-readable summary under the release marker.                      |

**Behaviour.** Posts a release marker with the outcome. `done` keeps the terminal state (labels: `queue/claimed`/`work/assigned` removed, nothing restored); `free` hands the work back unworked and restores `queue/queued`. Plain `release --outcome done` remains the way to hand back work that carries no disposition (contrast `complete` below). A release from a challenger clears their contest at once.

Exit codes: 0 released · 2 usage / missing or malformed `--worker` / bad `--outcome` · 3 GitHub transport failed · 5 the claim state forbids the action.

```text
$ fleet queue fallback release code 13165 --worker repo-worker-code --outcome free --summary "cannot take this now; back to the pool"
{"mode": "fallback", "repo": "developing-today/code", "number": 13165, "claim": {"state": "released", "claimable": true, "worker": "repo-worker-code", "outcome": "free", "released_at": "2026-11-23T04:50:00Z"}}
```

## fleet queue fallback complete

```text
usage: repo-queue fallback complete <repo> <issue-number> --worker <id> --summary <text> --result <path> [--pr <n[,n...]>] [--lease-min <minutes>]
```

| Flag                          | Argument                   | Default                                       | Meaning                                                                                                                                              |
| ----------------------------- | -------------------------- | --------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| `<repo>` (positional)         | repo or shortcut           | required                                      | The issue's repo.                                                                                                                                    |
| `<issue-number>` (positional) | integer                    | required                                      | The issue number.                                                                                                                                    |
| `--worker`                    | worker id                  | **required** (or env `FLEET_FALLBACK_WORKER`) | Holder identity. Never defaulted.                                                                                                                    |
| `--summary`                   | text                       | **required**                                  | Completion summary posted with the release-done marker.                                                                                              |
| `--result`                    | file path                  | **required**                                  | A disposition record file — the output of `fleet queue verify-disposition <issue> --repo <repo> --json` (see [lifecycle verbs](queue-lifecycle.md)). |
| `--pr`                        | comma-separated PR numbers | unset                                         | The completion's linked PRs, for the gate's coverage check.                                                                                          |
| `--lease-min`                 | integer minutes            | 30 (or env `FLEET_QUEUE_FALLBACK_LEASE_MIN`)  | Lease window.                                                                                                                                        |

**Behaviour — offline gate parity.** Broker parity: `--result` must pass the broker's **offline** disposition gate (`disposition::check` — the same pure gate the broker applies, including the 1800 s freshness rule and the five-verdict rules in [lifecycle verbs](queue-lifecycle.md)) for this issue _before anything is posted_; a rejection posts nothing, the claim stays live, and the verb exits **1**.

On a pass it posts the release-done marker with the summary and the **full record** in a ` ```fleet-disposition ` fenced block, so the record stays consumable from the marker stream when the broker returns.

**UNCHECKED follow-ups.** The one broker-only check — a `follow_up`'s `task_id` against the live queue — cannot run from a fallback seat: such a record is completed with that check flagged **UNCHECKED** in the comment and the verb output, never silently treated as checked.

Exit codes: 0 completed · 1 disposition gate rejection or unreadable record (nothing posted; claim stays live) · 2 usage / missing required flag / malformed `--worker` · 3 GitHub transport failed · 5 the claim state forbids the action.

```text
$ fleet queue verify-disposition 13165 --repo developing-today/code --json > /tmp/disp.json
$ fleet queue fallback complete code 13165 --worker repo-worker-code --summary "Book chapters merged" --result /tmp/disp.json
{"mode": "fallback", "repo": "developing-today/code", "number": 13165, "completed": true, "verdict": "merged_verified", "follow_up_task_check": "n/a"}
```

## fleet queue fallback unblock

```text
usage: repo-queue fallback unblock <repo> <issue-number> --worker <id> --reason <text>
```

| Flag                          | Argument         | Default                                       | Meaning                                                                                   |
| ----------------------------- | ---------------- | --------------------------------------------- | ----------------------------------------------------------------------------------------- |
| `<repo>` (positional)         | repo or shortcut | required                                      | The issue's repo.                                                                         |
| `<issue-number>` (positional) | integer          | required                                      | The issue number.                                                                         |
| `--worker`                    | worker id        | **required** (or env `FLEET_FALLBACK_WORKER`) | Overriding worker identity — the override is worker-scoped and recorded. Never defaulted. |
| `--reason`                    | text             | **required**                                  | Why the block is overridden; recorded in the unblock marker. Missing → exit 2.            |

**Behaviour.** The fallback form of the broker's manual override: posts an unblock marker recording the override + reason, clears the `queue/blocked` label and restores `queue/queued`, then verifies the label transition landed. **Refusals (exit 5):** an issue that is closed, not blocked, or under a live or contested claim.

Exit codes: 0 unblocked · 2 usage / missing `--worker` or `--reason` · 3 GitHub transport failed · 5 closed, not blocked, or live/contested claim.

```text
$ fleet queue fallback unblock code 13165 --worker repo-worker-code --reason "blocking PR merged; override the stale block"
{"mode": "fallback", "repo": "developing-today/code", "number": 13165, "blocked": false, "last_unblock": {"worker": "repo-worker-code", "ts": "2026-11-23T04:55:00Z"}, "claim": {"state": "free", "claimable": true}}
```
