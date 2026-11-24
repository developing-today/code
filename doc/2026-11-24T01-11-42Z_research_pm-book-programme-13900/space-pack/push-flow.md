# Project Management Space — Push flow: event to answer

> Pack file 4 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Sources: `survey/push.md` (primary), `survey/website-pages.md` §B28/§F,
> `survey/apps.md` §10–§12, `survey/disagreements.md` D5, D14.
> Ground truth: laptop clone `~/git/code` at `4d937b3` (2026-11-23);
> issue #8882 read via `fleet gh api` (closed, created 2026-11-12).
>
> **One-sentence shape:** the aggregator on the laptop is the only
> detector/decider for ordinary classes; it sends Web Push (VAPID) directly
> to subscribed browsers/phones; a Cloudflare relay exists in code as a
> dead-man's switch for "the laptop itself is silent" but is **not
> deployed**; the phone-side surfaces are the aggregator PWA (service worker
> in `aggregator/src/sw-source.ts`) and the console Android app (TWA +
> `console/public/sw.js`).

---

## 1. The flow as an ordered step sequence (diagram-ready)

Each step names its component and files. Steps 1–7 are the normal path;
step 8 is the laptop-dead path, which today sends nothing.

| Step | Stage                           | What happens                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | Component / files                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| ---: | ------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
|    1 | **Event**                       | Something happens: a watchdog probe fails, a stack service starts failing, a pool walls or nears its wall, an item enters the needs-Drew tier, a decision set is raised, a prompt is answered or fails, an external producer posts an incident.                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Probes/sentinel inputs; producers via the alert throat                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
|    2 | **Detection**                   | (a) _Incidents:_ the stack sentinel + watchdog + health hooks detect failures — the sentinel fires a health hook once on a 3-consecutive-failure transition and once on recovery. (b) _State candidates:_ the notification candidate builder derives candidates from attention/quota/health snapshots. (c) _External intake:_ producers POST to the alert throat, which stores incidents in the Turso-backed alerts store.                                                                                                                                                                                                                                                                                           | `aggregator/scripts/stack-sentinel.ts`; `aggregator/src/watchdog*.ts`, `stack-health.ts`, `host-health.ts`; `pm-stack/host-health-collector.py`; `pm-stack/health-hooks/run.py`; `aggregator/src/notifications.ts` (`buildNotificationCandidates`); throat: `POST /api/alerts` in `aggregator/src/server.ts` (bearer intake token by secret name `pm-aggregator.alerts-intake-token`), store/engine `aggregator/src/alerts.ts` (`AlertsStore`; Turso `alerts.*` tables: incidents, `push_subscriptions`, `notification_sends`) |
|    3 | **Decision**                    | The engine decides per subscription: is the class enabled? Are quiet hours holding it (default 23:00–07:00 America/Chicago — breakthrough classes bypass)? Is it a duplicate (dedupe by subscription + class + objectRef + stateFingerprint)? Is it inside the 6 h cooldown? Non-breakthrough candidates that pass are queued for one digest push; a disappeared state or muted class cancels its pending digest entry. Incidents get one escalation delivery. Every attempt is logged (`sent \| suppressed-quiet \| suppressed-dedupe \| suppressed-cooldown \| gone \| error \| queued \| queue-cancelled \| recovered`) — that log is what `/notifications` measures.                                             | `aggregator/src/alerts.ts` (`runAlertTick`, `runDashboardNotificationTick`; `NOTIFICATION_COOLDOWN_MS` = 6 h) + `aggregator/src/notifications.ts` (class registry, candidate fingerprints, breakthrough flags; `mergeClasses`/`classEnabled` — legacy subscription rows default only `incident` on)                                                                                                                                                                                                                            |
|    4 | **Send**                        | Web Push (VAPID) sent directly from the laptop to each subscribed browser/phone endpoint. Public key is a constant in code; the private key is fetched by secret name (`pm-aggregator.vapid-private-key`); subject is a `mailto:` in code.                                                                                                                                                                                                                                                                                                                                                                                                                                                                           | `aggregator/src/push.ts` (`sendPush`)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
|    5 | **Receive / tap**               | The phone's service worker shows the notification. Aggregator PWA: subscription routes `POST /api/push/subscriptions`, `POST /api/push/test`, `POST /api/push/test-class`, `GET /api/sw-probe`; client `src/assets/push-client.js`. Console app: push + notificationclick + offline kept answers. Which of the two SWs owns a given phone's subscription in production: UNVERIFIED from code alone. Tap behaviour depends on class — see §3.                                                                                                                                                                                                                                                                         | Aggregator PWA: `aggregator/src/sw-source.ts` served as `/sw.js`. Console: `console/public/sw.js`, `console/src/push.ts`, `console/src/notify.ts`, Android TWA in `console/android/`                                                                                                                                                                                                                                                                                                                                           |
|    6 | **Answer**                      | Three answer shapes: (a) _tap an action_ — a single needs-Drew question's first two options become notification actions; tapping POSTs the answer directly, no page open (§3a). (b) _open the page_ — a decision set's push opens `/decisions/<setId>?t=<token>`; the page is the answer surface (§3b). (c) _open the view_ — incident/quota/prompt taps open their state view; acting there is the answer (§3c). Console-app answers flow through the audited `POST /console/api/act` table, or are held by the edge and replayed when the laptop is unreachable.                                                                                                                                                   | `aggregator/src/sw-source.ts`; `aggregator/src/server.ts` (answer-page client); `console/src/server.ts`, `console/src/work.ts`                                                                                                                                                                                                                                                                                                                                                                                                 |
|    7 | **Write-back**                  | The answer lands in the same stores as a web answer: single-question and decision-set resolutions are recorded in the decisions machinery (first resolution wins per question; drafts pruned on resolve) and the dropped-thread catcher applies them to the Muse-side decisions ledger per question. Decision events fan out via the decision-events pipeline to Turso (fleet-platform `decision_set_events`) and the decision-events Worker (D1 + per-set Durable Object), which pushes new batches live to the set page's WebSocket subscribers. Incident ack/close writes the incident lifecycle (`open\|acked\|closed`) in the alerts store. Drew activity on an item stops blocked-age escalation for that ref. | `aggregator/src/decisions.ts`, `src/needs-drew.ts` (see D9 note in §3a); `aggregator/src/decision-events{,-sync}.ts`, `scripts/sync-decision-events.ts`, `pm-stack/decision-events-worker/`; `aggregator/src/alerts.ts`                                                                                                                                                                                                                                                                                                        |
|    8 | **Laptop-dead path (NOT LIVE)** | If the laptop dies, aggregator heartbeats to the notify relay stop (~60 s cadence); the relay's singleton Durable Object alarm (last-beat + 150 s) would send exactly one `stack.silent` Web Push, and exactly one recovered push on recovery. **Today this leg sends nothing:** the relay is not deployed (`platform/workers.json`: `pm-notify`, state "not deployed", address null) and with no `PM_AGG_NOTIFY_RELAY_URL` the aggregator side is inert (D5). Whether any other watchdog covers total laptop silence: UNVERIFIED.                                                                                                                                                                                   | Relay: `pm-stack/notify-relay/src/{index,relay,push}.ts`; aggregator client `aggregator/src/notify-relay.ts` (`NotifyRelayClient`, `NOTIFY_RELAY_HEARTBEAT_MS` = 60 s, snapshot mirror of subscriptions)                                                                                                                                                                                                                                                                                                                       |

### Worked trace — outage example (named files, from `survey/push.md` §5)

`host-health-collector.py` / sentinel probes fail → `aggregator/scripts/stack-sentinel.ts`
(3-failure transition) → `health-hooks/run.py` (read-only diagnosis, JSONL
incident + best-effort POST) → `POST /api/alerts` (`server.ts`, intake bearer)
→ `aggregator/src/alerts.ts` `AlertsStore` (incident row, Turso) →
`runAlertTick` (class `incident`, breakthrough, dedupe) →
`aggregator/src/push.ts` `sendPush` (VAPID) → phone SW
`aggregator/src/sw-source.ts` (show; tap → `/alerts#incident-…`) →
`notification_sends` log → `/notifications` roll-up.

---

## 2. The eight notification classes (D14)

Preference classes are the keys of `NOTIFICATION_CLASS_DEFAULTS` /
`NOTIFICATION_CLASSES` in `aggregator/src/notifications.ts` — **eight**
classes, all default ON for new subscriptions, toggled per subscription on
`/alerts/preferences` (`/alerts/preferences` page + `/api/alerts/preferences.json`
twin; per-class test sends there too).

> **D14:** Issue #8882 (closed) describes "Six notification classes in
> `pm-stack/aggregator/src/notifications.ts`". The code now defines **eight**
> — `section-risk` itself is the seventh the issue added, and `prompt`
> (#10035) followed. Any text quoting "six classes" (or "a seventh class" as
> the end state) disagrees with `NOTIFICATION_CLASS_DEFAULTS` today. Code wins.

|   # | Class id          | Meaning (code blurb, condensed)                                                                                                                                                       | Breakthrough?                                          | How it arrives                                                                 | Tap lands on                                                                                                                   |
| --: | ----------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ | ------------------------------------------------------------------------------ | ------------------------------------------------------------------------------------------------------------------------------ |
|   1 | `incident`        | A watchdog reports something down. Always immediate — quiet hours never hold an incident (#3676). One escalation delivery per incident.                                               | **Yes**                                                | Own producer (alert intake), not the snapshot builder                          | `/alerts#incident-<id>`; ack/close there                                                                                       |
|   2 | `needs-drew`      | An item enters the needs-Drew tier on the attention page. Includes raised single questions with tappable answers and decision sets.                                                   | No (answers ride the class; see §3)                    | Snapshot builder (`buildNotificationCandidates`) + decision-set producers      | Single question: notification actions / answer page. Decision set: `/decisions/<setId>?t=<token>`. Item: `/attention` anchored |
|   3 | `quota-wall`      | A pool hits its limit well before reset (`QUOTA_WELL_BEFORE_RESET_MS` = 24 h), or a sole-door pool walls at all.                                                                      | **Yes** for the sole-door case (per the #8882 record)  | Snapshot builder                                                               | `/quota`                                                                                                                       |
|   4 | `quota-near-wall` | A sole-door pool crosses 85% of a window.                                                                                                                                             | No                                                     | Snapshot builder                                                               | `/quota`                                                                                                                       |
|   5 | `section-risk`    | A whole route section (workhorse, speed, specialist, summit, small) fails or is at severe risk. Shows the pools pending wall, which are walled, and the soonest resets. Detail in §4. | **Yes** when a section has FAILED; no when SEVERE RISK | Snapshot builder                                                               | `/quota`                                                                                                                       |
|   6 | `stack-health`    | A monitored stack service starts failing its probe. Sentinel state is authoritative over health-derived candidates when present.                                                      | No (incidents raised via health hooks ride class 1)    | Snapshot builder; health hooks POST incidents with the `stackHealth` extension | `/alerts` / health surface                                                                                                     |
|   7 | `blocked-age`     | A needs-Drew item stays unacted for 24/48 hours, then weekly. Digest-only except the 48-hour breakthrough (`BLOCKED_AGE_REMINDER_MS` 24 h, `BLOCKED_AGE_BREAKTHROUGH_MS` 48 h).       | **Yes** at 48 h only                                   | Snapshot builder                                                               | `/attention` anchored to the item                                                                                              |
|   8 | `prompt`          | A prompt submitted with notifications on (#10035) reaches an answer or fails. Per-submission opt-in _and_ this subscription-side gate both apply.                                     | No                                                     | Own producer (prompt lifecycle), not the snapshot builder                      | `/chat`                                                                                                                        |

**Non-preference delivery shapes in code (not classes):** `digest` (queued
non-breakthrough candidates delivered as one digest push; a disappeared state
or muted class cancels its pending digest entry), `test` (`POST /api/push/test`,
object ref `test:push`), per-class test sends (`POST /api/push/test-class`,
refs `test:<class>`, landing URLs per class in `notifications.ts`
`notificationTestLanding`), and the relay-only `stack.silent` episode pushes,
which ride the `incident` preference class (`notify-relay/src/relay.ts`:
`SILENT_CLASS`).

**Candidate classes actually constructed** by `buildNotificationCandidates`
(the union type in `notifications.ts`): `needs-drew | blocked-age |
quota-wall | quota-near-wall | section-risk | stack-health` — `incident` and
`prompt` arrive through their own producers.

**Decision mechanics recap:** per subscription, candidates are filtered by
`classEnabled(sub, cls)`; quiet hours default 23:00–07:00 America/Chicago
(`DEFAULT_QUIET`) and hold everything except breakthrough classes; dedupe by
(subscription, class, objectRef, stateFingerprint); cooldown 6 h; every
attempt logged to `notification_sends` and the notification history — the
record `/notifications` measures (weekly sent-vs-acted-on roll-up; display-only
default-off proposals under 10%).

---

## 3. The answer-writes-back mechanic

### 3a. Single needs-Drew question — tap an answer on the phone

1. **Raise:** a producer raises a question in the Muse-side decisions
   ledger; the aggregator's needs-Drew candidate carries it
   (`NotificationCandidate.needsDrew = {ledgerId, question, options}`,
   `notifications.ts`, #8568).
2. **Payload:** the `alerts.ts` push payload includes the answer block; the
   service worker (`aggregator/src/sw-source.ts`) turns the **first two
   options** into notification actions and stashes `answerLedgerId`,
   `answerToken` (parsed from the answer-page URL's `?t=`), and `answerUrl`
   in the notification data.
3. **Tap:** on `notificationclick` with an action, the SW POSTs directly —
   `fetch('/api/needs-drew/' + ledgerId + '/answer', {body: {token, optionId:
event.action}})` — no page open needed. Errors are silent in the SW; the
   answer page remains the fallback (same endpoint, generated client in
   `server.ts` ~line 884).
4. **Write-back:** the endpoint records the resolution against the ledger
   item's question (decisions machinery, `aggregator/src/decisions.ts` /
   `needs-drew.ts`); the dropped-thread catcher then applies resolutions to
   the Muse-side ledger per question (`decisions.ts` header).
   **D9:** which file a single-question (non-set) answer ultimately writes is
   not unambiguously documented in the code surveyed — UNVERIFIED; describe
   the endpoint and the ledger application, do not invent a store.

### 3b. Decision set — the page is the answer surface

1. **Raise:** `scripts/raise-decision-set.ts` POSTs the set through the alert
   throat (`POST /api/alerts`, `decisionSet` extension) or
   `scripts/submit-decision-set.ts` via `POST /api/decision-sets`; the engine
   sends **one push for the whole set** whose tap opens
   `/decisions/<setId>?t=<token>` — deliberately no tappable answers
   (`notifications.ts`: "the page is the answer surface").
2. **Answer:** submit / defer / request-info on the page (or the same sets in
   console `#needs/questions`) → resolutions recorded first-resolution-wins
   in the decisions store (`decisions.ts` `recordResolutions`), drafts
   pruned, events emitted. Resolution kinds: `answered` (optionId/optionIds +
   freeText + comment), `deferred` (lean `none|skeptical|approve`),
   `request-info`. Bulk answers: page submit-all; console "Accept \<n\>
   recommended" per set (batched at 200, dashboard cap 256/request).
3. **Fan-out:** `scripts/sync-decision-events.ts` drains the decision-event
   outbox to Turso (fleet-platform) **and** the decision-events Worker (D1 +
   per-set Durable Object), which pushes new batches live to the set page's
   WebSocket subscribers (page polls/subscribes via
   `/api/decisions/<setId>/events` in `server.ts`).

### 3c. Other taps

- **Incident** tap → `/alerts#incident-<id>`; ack/close there writes the
  incident lifecycle in the alerts store.
- **Quota / section-risk** tap → `/quota` (no answer-back; it is a state view).
- **Needs-Drew / blocked-age** tap → `/attention` anchored to the item; acting
  on the item (dashboard action/comment) is recorded as Drew activity
  (`NotificationSnapshot.drewActivity`) and stops blocked-age escalation for
  that ref (`actedRefs` in `notifications.ts`).
- **Prompt** tap → `/chat`; the answer itself was written by the responding
  session through the prompt thread.
- **Console-app notifications:** `console/public/sw.js` `notificationclick`
  opens the addressed console view; console answers then flow through the
  audited `POST /console/api/act` table (or are held by the edge and
  replayed), landing in the same stores as §3a/§3b (`console/src/work.ts`
  `Work.submit()` calls the dashboard's own `/api/decisions/<setId>/submit`).

### D5 — the edge caveat (must appear in any Space rendering)

`notify-relay/README.md`'s route table lists `POST /mirror/event` and
`GET /answer/:ledgerId` / `POST /api/needs-drew/:ledgerId/answer` alongside
the live routes. The code is explicit that they are **not routed**:
everything else 404s. And `platform/workers.json` records `pm-notify` as
**"not deployed"** with a null address. Today the edge answer-intake and the
dead-man's switch **do not exist in production**; phone answers go to the
laptop aggregator (§3a). Any doc describing edge answering as available is
ahead of the code.

---

## 4. The section-risk class (#8882) — detail

Issue #8882 (Drew's order, 2026-11-12 ~01:58 CST, quoted in the issue):
_"update push on phone add a option that shows pools that are pending wall,
only alert me when each major section of workhorse speed specialist summit or
small fail or severe risk and in that scenario show compact what is failed or
at risk which are walled and soonest related resets."_ The issue is **closed**.

As implemented (`aggregator/src/notifications.ts` class meta + candidate
derivation; join in `aggregator/src/server.ts`; leaf analysis in
`aggregator/src/aa.ts`):

- Class id `section-risk`, label "Section failure / severe risk", default ON.
- Major sections are exactly `workhorse, speed, specialist, summit, small`
  (scout deliberately excluded, per the issue). Membership:
  `sectionPoolIdsFromConfig` / `routeLeafSets` over the deployed OpenCode
  config's `experimental.model_routes`, joined to quota pools
  (`NotificationSnapshot.sectionPools`).
- **Pool walled** = quota state `walled` or any measured window ≥100%; pools
  in `error`/`stale`/`lapsed` never count as serving. **Pending wall** = not
  walled and any measured window ≥85%.
- **Section FAILED** = ≥1 serving pool and all serving pools walled
  (breakthrough — bypasses quiet hours/digest). **Section SEVERE RISK** = not
  failed, and every non-walled serving pool is pending wall (not a
  breakthrough).
- One candidate per section per state, objectRef `section:<name>`; body is
  the compact form from the issue (walled pools with soonest resets — the
  exhausted window's `wallLiftsAt` — and pending pools with percent + reset,
  soonest first); landing URL `/quota`; recovery uses the engine's existing
  disappearance/recovery semantics, no second mechanism.
- Subscription flip required by the issue (Drew's phone: `section-risk` ON,
  `quota-wall`/`quota-near-wall` OFF, recorded on the issue): whether that
  flip was actually applied to the live subscription is UNVERIFIED from the
  repo (subscription state lives in the Turso alerts store, not code).
