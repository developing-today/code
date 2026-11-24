# Android push notifications — end to end (PM-SURVEY, 2026-11-23)

Ground truth: laptop clone `~/git/code` at `4d937b3`, via `~/tools/laptop-ssh.sh`. Issue #8882 read via `fleet gh api repos/developing-today/code/issues/8882` (state: closed, created 2026-11-12).

**One-sentence shape:** the aggregator on the laptop is the only detector/decider for ordinary classes; it sends Web Push (VAPID) directly to subscribed browsers/phones; a Cloudflare relay exists in code as a dead-man's switch for "the laptop itself is silent" but is **not deployed**; the phone-side surfaces are the aggregator PWA (service worker in `aggregator/src/sw-source.ts`) and the console Android app (TWA + `console/public/sw.js`).

## 1. Components: detect → decide → send

| Role | Component | Files |
|---|---|---|
| Detect (incidents) | Stack sentinel + watchdog + health hooks | `aggregator/scripts/stack-sentinel.ts`; `aggregator/src/watchdog*.ts`, `stack-health.ts`, `host-health.ts`; `pm-stack/host-health-collector.py`; `pm-stack/health-hooks/run.py` (fires once on 3-consecutive-failure transition and once on recovery, per `health-hooks/README.md`) |
| Detect (state candidates) | Notification candidate builder | `aggregator/src/notifications.ts` (`buildNotificationCandidates` over attention/quota/health snapshots) |
| Intake (external producers) | Alert throat | `POST /api/alerts` in `aggregator/src/server.ts`, bearer intake token resolved by name (`pm-aggregator.alerts-intake-token`, via `src/secrets.ts`); store/engine in `aggregator/src/alerts.ts` (`AlertsStore`, Turso-backed `alerts.*` tables: incidents, `push_subscriptions`, `notification_sends`) |
| Decide | Alert/notification engine | `aggregator/src/alerts.ts` (`runAlertTick`, `runDashboardNotificationTick`: class enablement, quiet hours, dedupe, 6 h cooldown `NOTIFICATION_COOLDOWN_MS`, digest queueing/cancellation, escalation) + `notifications.ts` (class registry, candidate fingerprints, breakthrough flags) |
| Send (normal) | Web Push from the laptop | `aggregator/src/push.ts` (`sendPush`, VAPID; public key constant in code, private key by secret name `pm-aggregator.vapid-private-key`, subject `mailto:` in code) |
| Send (laptop dead) | Notify relay (edge) | `pm-stack/notify-relay/src/{index,relay,push}.ts`; aggregator client `aggregator/src/notify-relay.ts` (`NotifyRelayClient`, heartbeat 60 s `NOTIFY_RELAY_HEARTBEAT_MS`, snapshot mirror of subscriptions). **Not deployed** — `pm-stack/platform/workers.json`: `pm-notify`, state "not deployed", address null; relay README's deploy section is explicitly "the step-1 remainder, not done by the build PR". With no `PM_AGG_NOTIFY_RELAY_URL` the aggregator side is inert and with no heartbeats the relay side is inert (same README). |
| Receive (phone) | Service workers | Aggregator PWA: `aggregator/src/sw-source.ts` served as `/sw.js`, client `src/assets/push-client.js`, subscription routes `POST /api/push/subscriptions`, `POST /api/push/test`, `POST /api/push/test-class`, `GET /api/sw-probe` (`server.ts`). Console app: `console/public/sw.js` (push + notificationclick + offline kept answers), `console/src/push.ts`, `console/src/notify.ts`, Android TWA in `console/android/` (assetlinks served at `/.well-known/assetlinks.json` by `console/src/server.ts`). Which of the two SWs owns a given phone's subscription in production: UNVERIFIED from code alone. |
| Record | Decision-events pipeline (for decision answers, not push itself) | `aggregator/src/decision-events{,-sync}.ts`, `scripts/sync-decision-events.ts`, `pm-stack/decision-events-worker/` (D1 + per-set Durable Object + Turso fleet-platform `decision_set_events`) |

## 2. Event classes

Preference classes are the keys of `NOTIFICATION_CLASS_DEFAULTS` / `NOTIFICATION_CLASSES` in `aggregator/src/notifications.ts` — **eight** classes, all default ON for new subscriptions, toggled per subscription on `/alerts/preferences` (see `website-pages.md` §B28):

1. `incident` — "A watchdog reports something down. Always immediate — quiet hours never hold an incident." Breakthrough class (#3676). Incidents also get one escalation delivery (`alerts.ts` `buildIncidentPayload(…, escalation)`); ack/close on `/alerts`.
2. `needs-drew` — "An item enters the needs-Drew tier on the attention page." Includes raised single questions with tappable answers (§4) and decision sets (§4).
3. `quota-wall` — "A pool hits its limit well before reset, or a sole-door pool walls at all." (`QUOTA_WELL_BEFORE_RESET_MS` = 24 h in `notifications.ts`.) Breakthrough for the sole-door case per the #8882 record.
4. `quota-near-wall` — "A sole-door pool crosses 85% of a window."
5. `section-risk` — the #8882 class; §3 below.
6. `stack-health` — "A monitored stack service starts failing its probe." Sentinel state is authoritative over health-derived candidates when present (`NotificationSnapshot.stackHealthStates` in `notifications.ts`); health hooks POST incidents with the `stackHealth` extension (`health-hooks/README.md`).
7. `blocked-age` — "A needs-Drew item stays unacted for 24/48 hours, then weekly (digest-only except the 48-hour breakthrough)." (`BLOCKED_AGE_REMINDER_MS` 24 h, `BLOCKED_AGE_BREAKTHROUGH_MS` 48 h.)
8. `prompt` — "A prompt submitted with notifications on (#10035) reaches an answer or fails." Per-submission opt-in *and* this subscription-side gate both apply.

Plus non-preference delivery shapes/classes in code: `digest` (queued non-breakthrough candidates delivered as one digest push; a disappeared state or muted class cancels its pending digest entry — `alerts.ts`), `test` (`POST /api/push/test`, object ref `test:push`), per-class test sends (`POST /api/push/test-class`, refs `test:<class>`, landing URLs per class in `notifications.ts` `notificationTestLanding`), and the relay-only `stack.silent` episode pushes, which ride the `incident` preference class (`notify-relay/src/relay.ts`: `SILENT_CLASS`).

Candidate classes actually constructed by `buildNotificationCandidates` are the union type in `notifications.ts`: `needs-drew | blocked-age | quota-wall | quota-near-wall | section-risk | stack-health` — `incident` and `prompt` arrive through their own producers (alert intake / prompt lifecycle), not the snapshot builder.

Decision mechanics (engine, `alerts.ts`): per subscription, candidates are filtered by `classEnabled(sub, cls)` (stored class map; legacy rows default only `incident` on — `mergeClasses`/`classEnabled` comments); quiet hours default 23:00–07:00 America/Chicago (`DEFAULT_QUIET`) and hold everything except breakthrough classes; dedupe is by (subscription, class, objectRef, stateFingerprint); cooldown 6 h; every attempt is logged to `notification_sends` and to the notification history (`sent | suppressed-quiet | suppressed-dedupe | suppressed-cooldown | gone | error | queued | queue-cancelled | recovered`), which is what `/notifications` measures.

## 3. The section-risk class (#8882)

Issue #8882 (Drew's order, 2026-11-12 ~01:58 CST, quoted in the issue): *"update push on phone add a option that shows pools that are pending wall, only alert me when each major section of workhorse speed specialist summit or small fail or severe risk and in that scenario show compact what is failed or at risk which are walled and soonest related resets."* The issue is **closed**.

As implemented (`aggregator/src/notifications.ts` class meta + candidate derivation; join in `aggregator/src/server.ts`; leaf analysis in `aggregator/src/aa.ts`):

- Class id `section-risk`, label "Section failure / severe risk", blurb: "A whole route section (workhorse, speed, specialist, summit, small) fails or is at severe risk. Shows the pools pending wall, which are walled, and the soonest resets." Default ON.
- Major sections are exactly `workhorse, speed, specialist, summit, small` (scout deliberately excluded, per the issue). Membership: `sectionPoolIdsFromConfig` / `routeLeafSets` over the deployed OpenCode config's `experimental.model_routes`, joined to quota pools (`NotificationSnapshot.sectionPools`, whose comment in `notifications.ts` cites #8882 and `aa.ts`).
- Pool walled = quota state `walled` or any measured window ≥100%; pools in `error`/`stale`/`lapsed` never count as serving. Pending wall = not walled and any measured window ≥85%. **Section FAILED** = ≥1 serving pool and all serving pools walled (breakthrough — bypasses quiet hours/digest). **Section SEVERE RISK** = not failed, and every non-walled serving pool is pending wall (not a breakthrough).
- One candidate per section per state, objectRef `section:<name>`; body is the compact form from the issue (walled pools with soonest resets — the exhausted window's `wallLiftsAt` — and pending pools with percent + reset, soonest first); landing URL `/quota`; recovery uses the engine's existing disappearance/recovery semantics, no second mechanism.
- Subscription flip required by the issue (Drew's phone: `section-risk` ON, `quota-wall`/`quota-near-wall` OFF, recorded on the issue): whether that flip was actually applied to the live subscription is UNVERIFIED from this repo (it is subscription state in the Turso alerts store, not code).

## 4. The answer-writes-back mechanic

### 4a. Single needs-Drew question — tap an answer on the phone
1. Raise: a producer raises a question in the Muse-side decisions ledger; the aggregator's needs-Drew candidate carries it (`NotificationCandidate.needsDrew = {ledgerId, question, options}`, `notifications.ts` #8568).
2. Payload: `alerts.ts` push payload includes the answer block; the service worker (`aggregator/src/sw-source.ts`) turns the **first two options** into notification actions and stashes `answerLedgerId`, `answerToken` (parsed from the answer-page URL's `?t=`), and `answerUrl` in the notification data.
3. Tap: on `notificationclick` with an action, the SW POSTs directly — `fetch('/api/needs-drew/' + ledgerId + '/answer', {body: {token, optionId: event.action}})` — no page open needed. Errors are silent in the SW; the answer page remains the fallback (same endpoint, generated client in `server.ts`).
4. Write-back: the endpoint records the resolution against the ledger item's question (decisions machinery, `aggregator/src/decisions.ts` / `needs-drew.ts`); the dropped-thread catcher then applies resolutions to the Muse-side ledger per question (`decisions.ts` header). See D9 in `disagreements.md` for the store ambiguity.

### 4b. Decision set — the page is the answer surface
1. Raise: `scripts/raise-decision-set.ts` POSTs the set through the alert throat (`POST /api/alerts`, `decisionSet` extension) or `scripts/submit-decision-set.ts` via `POST /api/decision-sets`; the engine sends **one push for the whole set** whose tap opens `/decisions/<setId>?t=<token>` — deliberately no tappable answers (`notifications.ts`: "the page is the answer surface").
2. Answer: submit / defer / request-info on the page (or the same sets in console `#needs/questions`) → resolutions recorded first-resolution-wins in the decisions store (`decisions.ts` `recordResolutions`), drafts pruned, events emitted.
3. Fan-out: `scripts/sync-decision-events.ts` drains the decision-event outbox to Turso (fleet-platform) **and** the decision-events Worker (D1 + per-set Durable Object), which pushes new batches live to the set page's WebSocket subscribers (`decision-events-worker/README.md`; page polls/subscribes via `/api/decisions/<setId>/events` in `server.ts`).

### 4c. Other taps
- Incident push tap → `/alerts#incident-<id>`; ack/close there writes the incident lifecycle in the alerts store.
- Quota/section-risk tap → `/quota` (no answer-back; it is a state view).
- Needs-Drew/blocked-age tap → `/attention` anchored to the item; acting on the item (dashboard action/comment, §F5–F6 of `website-pages.md`) is recorded as Drew activity (`NotificationSnapshot.drewActivity`) and stops blocked-age escalation for that ref (`actedRefs` in `notifications.ts`).
- Prompt tap → `/chat`; the answer itself was written by the responding session through the prompt thread.
- Console-app notifications: `console/public/sw.js` `notificationclick` opens the addressed console view; console answers then flow through the audited `POST /console/api/act` table (or are held by the edge and replayed — `console/src/server.ts`), landing in the same stores as §4a/4b (`console/src/work.ts` `Work.submit()` calls the dashboard's own `/api/decisions/<setId>/submit`).

## 5. Flow trace (named files, outage example)

`host-health-collector.py` / sentinel probes fail → `aggregator/scripts/stack-sentinel.ts` (3-failure transition) → `health-hooks/run.py` (read-only diagnosis, JSONL incident + best-effort POST) → `POST /api/alerts` (`server.ts`, intake bearer) → `aggregator/src/alerts.ts` `AlertsStore` (incident row, Turso) → `runAlertTick` (class `incident`, breakthrough, dedupe) → `aggregator/src/push.ts` `sendPush` (VAPID) → phone SW `aggregator/src/sw-source.ts` (show; tap → `/alerts#incident-…`) → `notification_sends` log → `/notifications` roll-up. If instead the **laptop dies**: `aggregator/src/notify-relay.ts` heartbeats stop → (relay not deployed, §1 — today this leg sends nothing; UNVERIFIED whether any other watchdog covers total laptop silence).
