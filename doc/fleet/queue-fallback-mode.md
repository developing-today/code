# Queue fallback mode — GitHub-native claiming when the backend is down

**Status:** ACTIVE whenever the queue backend is unreachable (Drew's
order, 2026-11-06). **Tooling:** `fleet queue fallback`
(`pkgs/fleet/crates/fleet-queue/src/fallback.rs`). **Why:** the
queue-broker → Supabase PostgREST path failed for a full day on
2026-11-06 (code#6619 / #6464) and every queue worker starved with
it. The work items were never in the database in any meaningful
sense — they are GitHub issues. Fallback mode coordinates on the
issues themselves until the backend returns.

## The protocol

- **Work items** are open issues carrying the fleet's existing
  labels: `queue/queued` (waiting) or `queue/claimed` (held) — the
  same taxonomy the broker system maintains (worker instruction §9;
  `pm-stack/dispatch` `LABEL_DEFS`). No new taxonomy. Any repo in
  the fleet's working set: developing-today/code,
  developing-today-forks/opencode, developing-today-forks/openchamber,
  dezren39/darwin-nix, dezren39/fleet.
- **Claim** = a comment whose first line is a machine-readable
  marker, with a human line under it ("Picked up by …"):

  ```
  <!-- fleet-claim v1 kind=claim worker=<id> ts=<rfc3339-utc> -->
  <!-- fleet-claim v1 kind=heartbeat worker=<id> ts=<rfc3339-utc> -->
  <!-- fleet-claim v1 kind=release worker=<id> outcome=<done|free> ts=<rfc3339-utc> -->
  ```

- **Lease** = 30 minutes (Drew's 20–30 range; 30 because worker
  checkpoints land ≤ ~15 min apart on long work and the fix
  timebox is 45 min — a 20-minute lease would let a live claim
  look dead between checkpoints). While the holder posts heartbeat
  markers, the claim is live and the issue is hands-off. After 30
  minutes of marker silence the claim is **stale** and any worker
  may claim the issue, superseding it. Override per call with
  `--lease-min`, or fleet-wide with `FLEET_QUEUE_FALLBACK_LEASE_MIN`.
- **Release** = a release marker with the outcome and a summary:
  `done` (work landed — summary names the PRs) or `free` (handed
  back unworked; restores the `queue/queued` label).
- **All state lives in the comments.** No database, no broker,
  nothing to reconcile afterwards — when the backend returns, the
  normal system re-derives state from labels and its tables as it
  already does after any drift.
- **The marker is the worker identity.** Every fleet worker posts
  as the same GitHub account, so authorship cannot key the lease;
  `--worker` is therefore required (never defaulted), and plain
  unmarked comments do **not** renew a claim.
- **Contested** = a second claim marker landed while the holder's
  claim was live (someone bypassed the tool). Neither worker
  proceeds while the contest is younger than the lease. A contest
  is not permanent (#6668): it ages out one lease window after it
  lands if the challenger posts no further claim, and a release
  from the challenger clears it at once. The holder may heartbeat
  while contested; once the contest has aged out, the holder's
  last activity governs — live if the holder kept heartbeating,
  stale and claimable if everyone went silent.

## The verbs

```bash
fleet queue fallback scan [--repo <repo>] [--limit <n>] [--lease-min <m>]
fleet queue fallback status <repo> <issue> [--lease-min <m>]
fleet queue fallback claim <repo> <issue> --worker <id> [--note <text>]
fleet queue fallback heartbeat <repo> <issue> --worker <id> [--note <text>]
fleet queue fallback release <repo> <issue> --worker <id> \
    --outcome done|free --summary "<what landed, with PR links>"
```

`<repo>` accepts owner/repo or the shortcuts `code`, `opencode`,
`openchamber`, `nix`, `fleet`. Output is JSON, like the rest of the
queue client. Exit codes: 0 ok · 2 usage · 3 the `gh` transport
failed · 5 the claim state forbids the action (live claim held by
another worker, contested, no claim to heartbeat/release). `claim`
re-reads the issue after posting and reports `race_lost` instead of
pretending a contested pickup succeeded. Claim/release also mirror
the §9 label transitions for the labels they own (`queue/claimed`,
`work/assigned`, `queue/queued`) on a best-effort basis — label
failures surface as warnings; the comments are the state.

## When a worker uses it

Detection is one call: `fleet queue health` failing (exit 3 / a
broker error rather than a counts report) means the backend is
down. The queue worker instruction's §0a carries the fallback loop
for worker shifts: scan for claimable work in your queue's repo,
claim with your lane id as the worker id, work the issue exactly as
the normal runbook prescribes (§2 revalidation, worktrees,
checkpoints — checkpoints become heartbeat markers), and finish
with a release instead of `complete`. Feeders keep filing issues
with `queue/queued` as usual; in fallback mode the label *is* the
enqueue, and an enqueue failure is expected, not a blocker. Every
fallback shift names the mode in its report's first line.
