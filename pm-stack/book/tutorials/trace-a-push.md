# Tutorial: trace a push notification from event to answer

This tutorial traces one push from the event that caused it to the answer that closes it. Work the trace in order. Each step names the file that owns the step, so a break in the chain has an address.

The shape, in one line: the aggregator on the laptop is the only detector and decider for the ordinary classes; it sends Web Push (VAPID) directly to subscribed phones and browsers; the Cloudflare notify relay is a dead-man's switch for total laptop silence, and it is not deployed (D5).

## Step 1: Start at the event

Pick the event class first. The class decides which detector produced the candidate.

| Event                                                  | Detector                               | File                                                                                                                                                                                                          |
| ------------------------------------------------------ | -------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| A stack service fails its probe                        | Stack sentinel, watchdog, health hooks | `aggregator/scripts/stack-sentinel.ts`; `aggregator/src/watchdog*.ts`; `aggregator/src/stack-health.ts`; `aggregator/src/host-health.ts`; `pm-stack/host-health-collector.py`; `pm-stack/health-hooks/run.py` |
| State changes on attention, quota, or health snapshots | Notification candidate builder         | `aggregator/src/notifications.ts`, `buildNotificationCandidates`                                                                                                                                              |
| An external producer files an incident                 | Alert throat                           | `POST /api/alerts` in `aggregator/src/server.ts`; store in `aggregator/src/alerts.ts`                                                                                                                         |
| A prompt is answered or fails                          | Prompt lifecycle                       | `aggregator/src/prompts.ts` and the `prompt` class, #10035                                                                                                                                                    |

The candidate builder constructs six classes: `needs-drew`, `blocked-age`, `quota-wall`, `quota-near-wall`, `section-risk`, `stack-health`. The classes `incident` and `prompt` arrive through their own producers, not through the snapshot builder.

## Step 2: Read the class registry

Open `NOTIFICATION_CLASS_DEFAULTS` in `aggregator/src/notifications.ts`. The registry defines eight preference classes, each default ON for a new subscription:

1. `incident`
2. `needs-drew`
3. `quota-wall`
4. `quota-near-wall`
5. `section-risk`
6. `stack-health`
7. `blocked-age`
8. `prompt`

Count them in the code, not in older issue text. Issue #8882 speaks of six classes; the code carries eight (D14). `section-risk` is the class #8882 added, and `prompt` followed it.

For `section-risk`, the trace continues in `aggregator/src/aa.ts`: route sections are exactly `workhorse`, `speed`, `specialist`, `summit`, `small`. A section fails when at least one pool serves and every serving pool is walled. A section is at severe risk when it has not failed and every non-walled serving pool is pending wall. The candidate's object ref is `section:&lt;name&gt;`, and its landing URL is `/quota`.

## Step 3: Follow the decision in the engine

Open `aggregator/src/alerts.ts`, in `runAlertTick` and `runDashboardNotificationTick`. For each subscription, the engine applies, in order:

1. Class enablement: `classEnabled` reads the subscription's stored class map. A stored choice always wins, including an explicit opt-out; a class with no stored choice — one added after the subscription was created — takes its reviewed default from `NOTIFICATION_CLASS_DEFAULTS` (the #13940 backfill for pre-class rows), and a class with no reviewed default stays off.
2. Quiet hours: the default window is 23:00–07:00 America/Chicago. Quiet hours hold every class except breakthrough classes. `incident` is a breakthrough class. A failed section in `section-risk` is a breakthrough. The 48-hour point in `blocked-age` is a breakthrough.
3. Dedupe: the key is subscription, class, object ref, and state fingerprint.
4. Cooldown: 6 hours, `NOTIFICATION_COOLDOWN_MS`.
5. Digest queueing: a non-breakthrough candidate enters the digest queue instead of sending at once. A state that disappears, or a class the subscription mutes, cancels the pending digest entry.

Every attempt lands in the `notification_sends` log with an outcome from the recorded set: `sent`, `suppressed-quiet`, `suppressed-dedupe`, `suppressed-cooldown`, `gone`, `error`, `queued`, `queue-cancelled`, `recovered`. That log is the measurement behind the `/notifications` page.

Set and check preferences on `/alerts/preferences`: per-class toggles, quiet hours, and a per-class test send for the subscription.

## Step 4: Follow the send

Open `aggregator/src/push.ts`, in `sendPush`. The aggregator sends Web Push with VAPID from the laptop. The public key is a constant in code. The private key arrives by secret name, `pm-aggregator.vapid-private-key`. The subject is a `mailto:` value in code.

Two test routes exercise this leg without a real event:

```text
POST /api/push/test
POST /api/push/test-class
```

Subscriptions arrive through `POST /api/push/subscriptions`. `GET /api/sw-probe` probes the service-worker side.

## Step 5: Follow the receive on the phone

Two service workers exist:

- The aggregator PWA: `aggregator/src/sw-source.ts`, served as `/sw.js`, with the client in `aggregator/src/assets/push-client.js`.
- The console app: `console/public/sw.js`, with `console/src/push.ts` and `console/src/notify.ts`, wrapped as an Android TWA from `console/android/`.

Which of the two service workers owns a given phone's subscription in production is UNVERIFIED from the code surveyed. Trace a concrete delivery through the subscription row in the alerts store, not through an assumption about the app.

## Step 6: Follow the tap to the answer

The tap target depends on the class:

- `needs-drew`, single question: the service worker turns the first two options into actions and sends `POST /api/needs-drew/&lt;ledgerId&gt;/answer` with `{token, optionId}` on the tap, with no page open. The full procedure is in [Tutorial: answer a waiting question end to end](answer-a-question.md).
- `needs-drew`, decision set: the tap opens `/decisions/&lt;setId&gt;?t=&lt;token&gt;`. The page is the answer surface.
- `incident`: the tap opens `/alerts#incident-&lt;id&gt;`. Acknowledge or close there; the action writes the incident lifecycle (`open`, `acked`, `closed`) in the alerts store. Incidents also receive one escalation delivery.
- `quota-wall`, `quota-near-wall`, `section-risk`: the tap opens `/quota`. There is no answer-back; `/quota` is a state view.
- `blocked-age`: the tap opens `/attention`, anchored to the item. Acting on the item through a dashboard action or comment records owner activity and stops blocked-age escalation for that ref.
- `prompt`: the tap opens `/chat`. The responding session wrote the answer in the prompt thread itself.

## Step 7: Trace the dead-man's switch leg, and stop at its edge

The relay leg exists in code:

- Client: `aggregator/src/notify-relay.ts`, `NotifyRelayClient`. The aggregator sends a heartbeat every 60 seconds (`NOTIFY_RELAY_HEARTBEAT_MS`) and mirrors push subscriptions to the relay.
- Relay: `pm-stack/notify-relay/src/index.ts`, `relay.ts`, and `push.ts`. A singleton Durable Object re-arms an alarm at last-beat plus 150 seconds. On silence it sends exactly one `stack.silent` Web Push, riding the `incident` preference class. On recovery it sends exactly one recovered push.

The leg ends there today. `pm-stack/platform/workers.json` records the `pm-notify` Worker as "not deployed", with a null address (D5). With no `PM_AGG_NOTIFY_RELAY_URL` set, the aggregator side is inert. With no heartbeats, the relay side is inert.

Two relay routes in its README's route table are not routed in code: `POST /mirror/event` arrives with step 2 of the relay build, and `GET /answer/&lt;ledgerId&gt;` and `POST /api/needs-drew/&lt;ledgerId&gt;/answer` arrive with step 3. Everything else in the relay returns 404. Phone answers therefore go to the laptop aggregator, as in Step 6. Any document that presents edge answering as available is ahead of the code.

Whether any other watchdog covers total laptop silence is UNVERIFIED in the surveys.

## Worked trace: one outage, end to end

1. `pm-stack/host-health-collector.py` and the sentinel probes record a failure.
2. `aggregator/scripts/stack-sentinel.ts` fires on the 3-consecutive-failure transition.
3. `pm-stack/health-hooks/run.py` runs a read-only diagnosis, appends a JSONL incident, and posts to the alert throat on a best-effort basis.
4. `POST /api/alerts` in `aggregator/src/server.ts` accepts the post under the intake bearer token, resolved by the secret name `pm-aggregator.alerts-intake-token`.
5. `aggregator/src/alerts.ts` writes the incident row in the Turso-backed alerts tables and runs the tick: class `incident`, breakthrough, dedupe applied.
6. `aggregator/src/push.ts` sends the push.
7. The phone service worker shows the notification. The tap opens `/alerts#incident-…`.
8. The `notification_sends` log records the attempt, and the `/notifications` page rolls it into the sent-vs-acted-on measurement.

## You are done when

- Each leg of the delivery names its file: detector, candidate builder, engine, send, service worker, tap target.
- The `notification_sends` outcome for the delivery is one of the recorded outcomes, and the outcome matches the leg where the trace stopped.
- For a laptop-silence trace, the trace stops at the relay edge in Step 7, with D5 cited. No answer route through the relay is presented as live.

Sources. survey/push.md §1–§5; survey/website-pages.md §B28, §F1–F2; survey/disagreements.md D5, D14. Ground truth: laptop clone at 4d937b3, 2026-11-23.
