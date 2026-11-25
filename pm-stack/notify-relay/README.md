# pm-notify relay — the pm-stack dead-man's switch at the edge

Phase 2 of the pm-stack notifications program
([#8569](https://github.com/developing-today/code/issues/8569)),
built to the design record
`doc/2026-11-17_design_pm8569_notifications_phase2.md`.
**This package is step 1 of the design's ordered split: the relay
skeleton + dead-man's switch.** Steps 2 (event mirror), 3 (edge
answer intake) and 4 (cron narrowing) are separate issues; their
routes are deliberately not routed yet.

## What it does

The aggregator on the laptop POSTs a heartbeat every ~60s and
mirrors its push-subscription set here. A single Durable Object
(`PmNotify`, singleton) holds that mirror and one alarm:

- Every heartbeat re-arms the alarm at last-beat + 150s.
- An alarm fire with no intervening beat opens a **silent
  episode** and sends exactly one `stack.silent` Web Push ("PM
  stack is down", with the silence start) to the mirrored
  subscriptions — no cron, no laptop, no Muse in the path.
- The first heartbeat after silence closes the episode and sends
  exactly one **recovered** push naming the downtime.

The edge sends only this one class (design invariant 2), riding
the `incident` preference class (default-on, breakthrough — quiet
hours never hold it). The board never moves to the edge (the
NO-GO half of the ruling); the only read surface is `GET /status`.

## Routes (closed list)

| Route                                                            | Auth               | Lands in                                                                                 |
| ---------------------------------------------------------------- | ------------------ | ---------------------------------------------------------------------------------------- |
| `POST /mirror/subscriptions`                                     | shared secret      | step 1 (this package)                                                                    |
| `POST /heartbeat`                                                | shared secret      | step 1; returns the pending-answer count                                                  |
| `GET /status`                                                    | none (minimal)     | step 1 (this package)                                                                    |
| `POST /mirror/event`                                             | shared secret      | step 2; append-only, deduped by `mirrorId`, pruned after 30 days, never sent at the edge |
| `POST /mirror/questions`                                         | shared secret      | step 3; replaces open question mirror, stores only token hashes                         |
| `GET /answer/:ledgerId`, `POST /api/needs-drew/:ledgerId/answer` | per-question token | step 3; edge page + durable pending answer mailbox                                       |
| `GET /pending-answers`, `POST /pending-answers/:answerId/ack`    | shared secret      | step 3; aggregator pull/ack drain                                                       |

Everything else 404s. The heartbeat response carries
`pendingAnswers`. The aggregator pulls and ACKs each answer only after
the Phase-1 `recordAnswer` writer returns, so a crash before ACK reoffers
the same answer id and the local first-answer-wins writer dedupes it.

`test/event.test.ts` is the event-mirror comparison harness: it keeps a
source-ledger fixture for incident, needs-Drew, and quota-wall events,
posts those events (including a concurrent duplicate), and compares the
source IDs with the Durable Object's stored mirror log. It also verifies
that the event path leaves edge delivery state untouched.

## Recorded deviation from the design letter

Design §3 names an epoch change as the recovery signal, on the
stated ground that a same-epoch heartbeat after silence "cannot
occur". A suspended laptop is the counterexample: it stops
beating without dying and resumes with the same epoch. This
implementation closes the episode on the **first beat after
silence, epoch change or not** — the design's invariant (exactly
one down push, exactly one recovered push per episode) holds
under both signals, and a same-epoch resume can never strand an
episode open (which would suppress the next episode's down
push). The heartbeat state records both epochs so the two cases
stay distinguishable in the delivery record.

## Secrets — by name only

No secret value lives in this repo. Custody (design §4):

| Secret               | Role                                  | Where it lives                                                                                                                                                      |
| -------------------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Relay shared secret  | Authenticates the laptop → edge POSTs | Bitwarden `pm-aggregator.notify-relay-shared-secret`; set on the Worker as `RELAY_SHARED_SECRET`                                                                    |
| VAPID private key    | Signs the edge's Web Push sends       | The existing aggregator keypair (custody moves, the keypair does not change): Bitwarden `pm-aggregator.vapid-private-key`; set on the Worker as `VAPID_PRIVATE_KEY` |
| Cloudflare API token | Deploys (wrangler)                    | Bitwarden, the existing day-to-day CF token; deploy-time only                                                                                                       |

## Deploy (the step-1 remainder, not done by the build PR)

From this directory, with the day-to-day CF API token available
to wrangler:

```sh
bunx wrangler deploy
bunx wrangler secret put RELAY_SHARED_SECRET   # value: the Bitwarden-named secret above
bunx wrangler secret put VAPID_PRIVATE_KEY      # value: the Bitwarden-named key above
```

Then point the aggregator at it: set `PM_AGG_NOTIFY_RELAY_URL`
to the deployed Worker URL in the aggregator's environment and
restart the aggregator. Until both halves exist, each side is
inert: the relay with no heartbeats never opens an episode (an
alarm with no beat ever seen opens nothing), and the aggregator
with no URL configured never sends.

Acceptance after deploy (Drew's terms, #8569): stop the laptop
stack (or kill the tunnel) → the down push reaches the phone in
under 2 minutes; restart → exactly one recovered push naming
the downtime.

## Development

```sh
bun test          # oracle tests for the state machine + push crypto
bun run typecheck
bun run dev       # wrangler dev (local DO + alarm)
```
