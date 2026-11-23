# Design record: pm-stack notifications Phase 2 — CF data-source sync topology + edge relay (post-ruling)

- **Date:** 2026-11-17
- **Status:** Design record (queue task 20170). Records the Phase-2 decisions the parent design left to this record; it implements nothing.
- **Issue:** [#8569](https://github.com/developing-today/code/issues/8569) (Phase 2 of the pm-stack notifications program)
- **Parent design:** `pm-stack-notifications-design-2026-11-11.md` (goal workspace, keep-the-four-repos-healthy) §3 (event model), §4 (transport + the Cloudflare ruling), §5 (answerable choices), §7 (Phase-2 acceptance), §8 (cron narrowing). Phase 1 ([#8568](https://github.com/developing-today/code/issues/8568)) is CLOSED — the throat, answer endpoint, answers outbox, and ledger write-back this record builds on all exist.
- **The ruling being recorded:** board/aggregator to Workers is **NO-GO** (every primary source is laptop-local and tailnet-only; an edge board is a cache of a dead source at exactly the moment it matters). A small edge notification relay is **GO**: one Worker + one Durable Object (`pm-notify`) owning the mirrored subscriptions, the event mirror log, pending answers, and the heartbeat alarm. This record decides the five things the ruling left open.

## 1. Sync topology (decided)

Three flows, three different shapes — chosen by data size and loss tolerance, not by uniformity.

| Flow                                           | Shape                                                                                                                                                                                                                                                                                                                   | Why                                                                                                                                                                                                                                                                                                                                                                                                          |
| ---------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| **Event mirror** (aggregator → DO)             | Append-only, one authenticated POST per push-worthy event at raise/transition time, carrying the throat's existing event payload plus a mirror envelope `{mirrorId, objectRef, transition, raisedAt}`. `mirrorId` is the idempotency key: the DO dedupes on it and acks duplicates as success. No batching, no polling. | Events are few (a handful a day) and each is precious; per-event POST at the detection point is the same shape as the Phase-1 throat intake, and idempotent retry makes the laptop's delivery obligation simply "retry until acked" with no ordering protocol.                                                                                                                                               |
| **Subscription mirror** (aggregator → DO)      | Full-snapshot replace: on any subscription-store change (subscribe, class toggles, quiet hours), the aggregator POSTs the complete mirrored set (endpoints, keys, class map, quiet hours) with a monotonically increasing `snapshotSeq`; the DO keeps only the highest seq.                                             | The set is tiny (one user, a handful of devices) and is the DO's send authority — a partial/delta sync that diverges silently is the worst failure here. Snapshot replace is self-healing: any missed update is corrected by the next change, and a seq regression is rejected loudly.                                                                                                                       |
| **Heartbeat + answer drain** (aggregator ↔ DO) | Heartbeat POST every ~60s carrying `{stackEpoch, seq, at}`. The response carries the count of pending edge answers; the aggregator then pulls them (GET answers, ack by answer id) in the same contact. Answers are therefore drained on _every_ contact, not just recovery.                                            | The heartbeat already exists as the dead-man's switch input; making it bidirectional costs nothing and bounds answer latency while the stack is up to one beat, while the pull-with-ack shape means a crash mid-drain re-offers the answer instead of losing it. The Phase-1 catcher remains the single writer to the pending-questions ledger — the edge path changes transport, never the place of record. |

**Stack epoch:** each aggregator start mints a `stackEpoch` (UUID). Heartbeats and mirrors carry it. The DO treats an epoch change as a recovery boundary (§3), which is what makes "exactly one recovered push" decidable at the edge.

## 2. Relay / Durable Object shape (decided)

- **One Worker, one DO class (`pm-notify`), one instance** (singleton by name). The state is inherently singular — one stack, one subscriber set — so sharding buys nothing and costs ordering.
- **DO storage:** (a) the current subscription snapshot + its seq; (b) the event mirror log, append-only, pruned to a rolling 30 days (it is a mirror, not the record — the throat's store stays authoritative); (c) pending answers, keyed by answer id, each `{ledgerId, optionId | freeText, answeredAt, answerTokenHash}`; (d) heartbeat state `{stackEpoch, lastSeq, lastBeatAt, silentSince | null}`.
- **Alarm discipline:** every heartbeat sets the DO alarm to `lastBeatAt + 150s` (~2 missed beats + margin). Alarm fire with no intervening beat ⇒ the `stack.silent` episode opens (§3). The alarm is the only scheduler; there is no cron anywhere in the design (parent §8 narrows the existing page-health-push cron to a daily canary when this ships).
- **Worker routes:** `POST /mirror/event`, `POST /mirror/subscriptions`, `POST /heartbeat` (all shared-secret authenticated, laptop → edge); `GET /answer/:ledgerId` + `POST /api/needs-drew/:ledgerId/answer` (answer pages/intake, per-question token auth per parent §5); `GET /status` (minimal stack state page: up/silent + since — deliberately not board data, §3 of this record). Everything else 404s.

## 3. Board read path (decided)

**The board does not move, and the edge serves no board data.** `/board`, `/api/board.json`, and the other aggregator APIs remain laptop-only behind Tailscale Serve, exactly as today — that is the NO-GO half of the ruling, and this record adds no exception to it. The Worker's only read surfaces are the answer pages (question + full option list from DO state, parent §5) and the minimal `/status` page above. If the laptop is down, Drew gets the push and the answer page; he does not get a degraded board, by design.

**Episode semantics (the acceptance terms, made mechanical):**

- _Open:_ alarm fires ⇒ if `silentSince` is null, set it and send exactly one `stack.silent` push (class `incident`, breakthrough per parent §3). Re-fires while silent send nothing.
- _Close:_ first heartbeat with a **new `stackEpoch`** ⇒ send exactly one recovered push naming the downtime (`now − silentSince`), clear `silentSince`. A heartbeat with the _same_ epoch after a silent episode cannot occur (a live process keeps beating; silence means it died), so epoch change is the recovery signal, not a flag in the payload.
- _Answers while down:_ accepted by the Worker against DO state, stored pending, drained on the next contact (§1), applied once by the Phase-1 catcher (answer ids dedupe the drain; the ledger's answered shape is unchanged).

## 4. Secrets — by name only (decided)

No secret value appears in this record, the repo, or the DO's stored state beyond its role below. Custody is by name, following the stack's existing pattern (the VAPID private key is already fetched by name from Bitwarden Secrets at first send):

| Secret                      | Role                                                            | Custody                                                                                                                                                                                                                                                          |
| --------------------------- | --------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| Throat↔Worker shared secret | Authenticates the three laptop → edge POSTs (mirror, heartbeat) | Bitwarden Secrets, by name, on the laptop side (same pattern as the throat intake token); Worker secret on the edge side. One secret for all three routes — they share one trust direction.                                                                      |
| VAPID private key           | Signs edge Web Push sends                                       | The existing keypair — custody moves, the keypair does not change (parent §4): the private key becomes a Worker secret; the laptop keeps fetching its copy by name from Bitwarden for its own sends during dual-send. Subscriptions therefore never re-register. |
| Per-question answer token   | Authorizes one answer page / answer POST                        | Not a stored secret: minted at raise time (Phase 1), carried only in the push payload / answer URL; the DO stores only its hash beside the mirrored question, expiring when the question is answered or withdrawn.                                               |
| Cloudflare API token        | Deploys the Worker/DO (wrangler)                                | The existing day-to-day CF API token, Bitwarden by name (parent §4); deploy-time only, never a Worker secret.                                                                                                                                                    |

## 5. Ordered implementation split (decided)

Each step is independently shippable and leaves the system correct if the next never lands.

1. **Relay skeleton + dead-man's switch.** Worker + `pm-notify` DO; heartbeat intake + alarm; subscription snapshot sync; `stack.silent` send on episode open, recovered send on epoch change. Dual-send discipline starts here: the edge owns `stack.silent` from this step on; the laptop never sends that class. _Acceptance: kill the stack (or the tunnel) → down push < 2 min; restart → exactly one recovered push._
2. **Event mirror.** Aggregator mirrors push-worthy events (incident transitions, needs-drew raises, quota walls) with the §1 envelope + retry-until-ack. Edge remains a silent mirror for these classes (no sends) — this step proves the sync before any cutover is contemplated. _Acceptance: mirror log in the DO matches the throat store across a soak window, dedupe verified by forced retries._
3. **Edge answer intake.** Answer pages + answer POST on the Worker from DO state; heartbeat-response drain + pull/ack; catcher application unchanged. _Acceptance: answer from a push while the laptop is down → on return, ledger answered, task unblocked, nothing double-applied._
4. **Cron narrowing + cutover review.** Page-health-push cron retires its page checks and becomes the daily DO canary (parent §8). Separately reviewed (not in this split's authority): whether any mirrored class beyond `stack.silent` ever cuts over to edge sending.

## Invariants

1. **One place of record:** the pending-questions ledger and the throat store stay authoritative; the DO is a mirror and a mailbox, never a second ledger.
2. **The edge sends one class:** `stack.silent` (plus its recovery) until a separately reviewed cutover says otherwise.
3. **No cron in the path:** DO alarms are the only scheduler Phase 2 introduces.
4. **No board at the edge:** answer pages + a status stub are the entire edge read surface.
5. **Secrets by name:** values live in Bitwarden / Worker secrets; code and docs name them, never carry them.
