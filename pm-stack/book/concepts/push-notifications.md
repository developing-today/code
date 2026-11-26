# Push notifications: detection to phone to answer

## The shape

The aggregator on the laptop is the only detector and decider for ordinary notification classes. It sends Web Push (VAPID) directly to subscribed browsers and phones. A Cloudflare relay exists in code as a dead-man's switch for the case where the laptop itself is silent; the relay is not deployed (§5, D5). The phone-side receivers are two service workers: the aggregator PWA worker (`aggregator/src/sw-source.ts`, served as `/sw.js`) and the console Android app's worker (`console/public/sw.js`, inside a TWA). Which of the two workers owns a given phone's subscription in production is UNVERIFIED from code alone.

## Detect

Detection has three inputs:

- **Incidents.** The stack sentinel (`aggregator/scripts/stack-sentinel.ts`), the watchdog and health modules (`aggregator/src/watchdog*.ts`, `stack-health.ts`, `host-health.ts`), the host-health collector (`pm-stack/host-health-collector.py`), and the health hooks (`pm-stack/health-hooks/run.py`). A health hook fires once on a transition to 3 consecutive failures and once on recovery.
- **State candidates.** `buildNotificationCandidates` in `aggregator/src/notifications.ts` builds candidates from attention, quota, and health snapshots.
- **External producers.** `POST /api/alerts` in `aggregator/src/server.ts` is the alert throat. It takes a bearer intake token resolved by secret name. Its store and engine live in `aggregator/src/alerts.ts`, backed by Turso `alerts.*` tables: incidents, `push_subscriptions`, and `notification_sends`.

## Decide

The engine in `aggregator/src/alerts.ts` (`runAlertTick`, `runDashboardNotificationTick`) applies, per subscription: class enablement, quiet hours, dedupe, a 6-hour cooldown (`NOTIFICATION_COOLDOWN_MS`), digest queueing and cancellation, and escalation.

- Quiet hours default to 23:00–07:00 America/Chicago (`DEFAULT_QUIET`). Quiet hours hold every class except breakthrough classes.
- Dedupe keys on subscription, class, object reference, and state fingerprint.
- A stored class map decides enablement per subscription. Legacy subscription rows default to `incident` only; new subscriptions default to all eight classes on.
- Every attempt is logged to `notification_sends` and to the notification history with one of these outcomes: `sent`, `suppressed-quiet`, `suppressed-dedupe`, `suppressed-cooldown`, `gone`, `error`, `queued`, `queue-cancelled`, `recovered`. The `/notifications` page measures that log.

Non-breakthrough candidates that pass the filters are queued and delivered as one digest push. A state that disappears, or a class that is muted, cancels its pending digest entry.

## The eight classes

Preference classes are the keys of `NOTIFICATION_CLASS_DEFAULTS` in `aggregator/src/notifications.ts`. The count is eight (D14). Issue #8882, now closed, described "six notification classes" in that file; the code has since added `section-risk` as the seventh and `prompt` as the eighth. This chapter enumerates the eight in the code, not the six in the issue.

1. `incident` — a watchdog reports something down. Always immediate; quiet hours never hold an incident. Breakthrough class. An incident also gets one escalation delivery. Acknowledge and close happen on `/alerts`.
2. `needs-drew` — an item enters the needs-Drew tier on the attention page. This class carries raised single questions with tappable answers and decision sets (§6).
3. `quota-wall` — a pool hits its limit well before reset, or a sole-door pool walls at all. "Well before" is `QUOTA_WELL_BEFORE_RESET_MS` = 24 hours in `notifications.ts`. Breakthrough for the sole-door case.
4. `quota-near-wall` — a sole-door pool crosses 85% of a window.
5. `section-risk` — a whole route section fails or is at severe risk (§4).
6. `stack-health` — a monitored stack service starts failing its probe. Sentinel state is authoritative over health-derived candidates when both exist. Health hooks post incidents with the `stackHealth` extension.
7. `blocked-age` — a needs-Drew item stays unacted for 24 hours, then 48 hours, then weekly. Digest-only, except the 48-hour step, which is a breakthrough. The thresholds are `BLOCKED_AGE_REMINDER_MS` (24 hours) and `BLOCKED_AGE_BREAKTHROUGH_MS` (48 hours).
8. `prompt` — a prompt submitted with notifications on reaches an answer or fails. Two gates apply: the per-submission opt-in and this subscription-side class toggle.

Three delivery shapes sit outside the preference classes: `digest` (the queued delivery in §Decide), `test` (`POST /api/push/test`), and per-class test sends (`POST /api/push/test-class`, one landing URL per class). The relay-only `stack.silent` episode pushes in §5 ride the `incident` preference class.

The snapshot builder constructs candidates for six classes: `needs-drew`, `blocked-age`, `quota-wall`, `quota-near-wall`, `section-risk`, and `stack-health`. `incident` and `prompt` arrive through their own producers: alert intake and the prompt lifecycle.

Per-class toggles and quiet hours are set on `/alerts/preferences`. Subscription routes are `POST /api/push/subscriptions`, `POST /api/push/test`, `POST /api/push/test-class`, and `GET /api/sw-probe`.

## Section-risk in detail

Class `section-risk` is the #8882 class. Its label is "Section failure / severe risk". Drew's order on the issue, 2026-11-12: "update push on phone add a option that shows pools that are pending wall, only alert me when each major section of workhorse speed specialist summit or small fail or severe risk and in that scenario show compact what is failed or at risk which are walled and soonest related resets." The issue is closed.

Implementation, in `aggregator/src/notifications.ts` with the join in `server.ts` and leaf analysis in `aggregator/src/aa.ts`:

- The major sections are exactly `workhorse`, `speed`, `specialist`, `summit`, and `small`. Scout is excluded, per the issue.
- Section membership comes from the deployed OpenCode config's `experimental.model_routes`, joined to quota pools.
- A pool is walled when its quota state is `walled` or any measured window is at 100% or more. A pool in `error`, `stale`, or `lapsed` state never counts as serving. A pool is pending wall when it is not walled and any measured window is at 85% or more.
- A section is FAILED when it has at least one serving pool and all serving pools are walled. FAILED is a breakthrough: it bypasses quiet hours and the digest. A section is at SEVERE RISK when it is not failed and every non-walled serving pool is pending wall. SEVERE RISK is not a breakthrough.
- One candidate is raised per section per state, with object reference `section:\<name\>`. The body is the compact form from the issue: walled pools with soonest resets first, then pending pools with percent and reset, soonest first. The landing URL is `/quota`. Recovery uses the engine's disappearance and recovery semantics; no second mechanism exists.
- The issue records a subscription flip for Drew's phone: `section-risk` on, `quota-wall` and `quota-near-wall` off. Whether that flip was applied to the live subscription is UNVERIFIED: subscription state lives in the Turso alerts store, not in the repo.

## Send: the laptop path, and the relay that is not deployed

The normal send path is `aggregator/src/push.ts` `sendPush`: Web Push with VAPID, direct from the laptop aggregator to the subscription endpoint. The VAPID public key is a constant in code; the private key is resolved by secret name; the subject is a `mailto:` value in code.

The relay path does not exist in production (D5). The facts, in order:

- `pm-stack/notify-relay/` holds a Cloudflare Worker (`pm-notify`) built as a dead-man's switch. The aggregator client (`aggregator/src/notify-relay.ts`) posts a heartbeat every 60 seconds (`NOTIFY_RELAY_HEARTBEAT_MS`) and mirrors push subscriptions to the relay. The relay re-arms an alarm at last-beat plus 150 seconds; on silence it sends one `stack.silent` push, and on recovery one recovered push.
- `pm-stack/platform/workers.json` records `pm-notify` as "not deployed", with a null address. The relay README's deploy section is the step-1 remainder, not done by the build PR.
- With no relay URL configured, the aggregator side is inert. With no heartbeats arriving, the relay side is inert.
- The relay's event-mirror route (`POST /mirror/event`) and its answer routes (`GET /answer/:ledgerId`, `POST /api/needs-drew/:ledgerId/answer`) are **not routed**. The relay source header states it directly: those routes arrive with steps 2 and 3 and "are not routed yet"; every other path returns 404. The relay README's route table lists them alongside the live routes, separated only by a "Lands in" column. The code wins (D5): today the edge answer-intake and the dead-man's switch do not exist in production, and phone answers go to the laptop aggregator (§6).

One consequence is stated plainly: if the laptop dies, the heartbeat leg sends nothing today. Whether any other watchdog covers total laptop silence is UNVERIFIED.

## The answer writes back

### A single question: tap an answer

1. A producer raises a question in the Muse-side decisions ledger. The aggregator's needs-Drew candidate carries it: ledger id, question, and options.
2. The push payload includes the answer block. The service worker turns the first two options into notification actions and stores the ledger id, the answer token (parsed from the answer-page URL's `?t=` parameter), and the answer URL in the notification data.
3. On `notificationclick` with an action, the service worker posts directly to `POST /api/needs-drew/\<ledgerId\>/answer` with the token and the tapped option id. No page opens. Errors in the service worker are silent; the answer page is the fallback and calls the same endpoint.
4. The endpoint records the resolution against the ledger item's question, and the dropped-thread catcher applies resolutions to the Muse-side ledger per question. The single-question answer is written to the needs-Drew question store (`recordAnswer` in `needs-drew.ts`; D9), treated in [Question sets and the answer-back flow](questions-and-answers.md).

### A decision set: the page is the answer surface

1. A set is raised through the alert throat (`POST /api/alerts`, `decisionSet` extension) or through `POST /api/decision-sets`. The engine sends one push for the whole set. The tap opens `/decisions/\<setId\>?t=\<token\>`. A decision-set push carries no tappable answers by design: the page is the answer surface.
2. Submit, defer, or request-info on the page records resolutions in the decisions store, first resolution wins, and prunes drafts.
3. `scripts/sync-decision-events.ts` drains the decision-event outbox to Turso and to the decision-events Worker, which pushes new batches live to the set page's WebSocket subscribers. The page also polls `/api/decisions/\<setId\>/events`.

### Other taps

- An incident tap opens `/alerts#incident-\<id\>`; acknowledge and close there write the incident lifecycle in the alerts store.
- A quota or section-risk tap opens `/quota`. No answer-back exists; the page is a state view.
- A needs-Drew or blocked-age tap opens `/attention`, anchored to the item. Acting on the item is recorded as Drew activity, and that activity stops blocked-age escalation for the item's reference.
- A prompt tap opens `/chat`. The answer itself was written by the responding session through the prompt thread.
- A console-app notification tap opens the addressed console view. Console answers then flow through the audited `POST /console/api/act` table, or are held by the edge and replayed, and land in the same stores as the two flows above.

## Worked trace: an outage

`host-health-collector.py` and sentinel probes fail → `aggregator/scripts/stack-sentinel.ts` registers the 3-failure transition → `health-hooks/run.py` runs read-only diagnosis, appends a JSONL incident, and posts to the alert throat → `POST /api/alerts` in `server.ts` checks the intake bearer → `alerts.ts` writes the incident row in Turso → `runAlertTick` classifies it `incident`, breakthrough, deduped → `push.ts` `sendPush` delivers by VAPID Web Push → the phone service worker shows the notification; a tap opens `/alerts#incident-…` → the attempt is logged in `notification_sends` → `/notifications` rolls the log up. The health surfaces behind the first steps are in [Health, quota and usage surfaces](health-surfaces.md).

Sources. Survey `push.md` §§1–5; survey `website-pages.md` §§B28, F1–F2; survey `disagreements.md` D5, D9, D14; issue #8882 (closed) as quoted in the survey; code at laptop clone `~/git/code` at `4d937b3` as cited inline.
