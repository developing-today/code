# PM push unification — one registry, one sender, the Android app as a first-class endpoint

Design record for [#14084](https://github.com/developing-today/code/issues/14084) (PM push unification).
Ordered by Drew, 2026-11-23: "start a lane now to unify push between the like 2 pm stack locations and the pm stack android app."
Lane PUSH-UNIFY, 2026-11-24. This document is the design of record; the build slices at §7 are its tail, not a separate plan.

Related records: [#14048](https://github.com/developing-today/code/issues/14048) (push delivery dark — zero subscriptions),
[#14061](https://github.com/developing-today/code/issues/14061) (send-ledger timestamps all unusable),
[#14043](https://github.com/developing-today/code/issues/14043) (answer write-back — adjacent territory, explicitly not redesigned here),
[#8882](https://github.com/developing-today/code/issues/8882) (section-risk class),
[#8569](https://github.com/developing-today/code/issues/8569) (Phase 2 edge relay / dead-man's switch),
[#13469](https://github.com/developing-today/code/issues/13469) (edge answer intake),
[#8570](https://github.com/developing-today/code/issues/8570) (Phase 3 service consolidation).

## 1. Inventory — what exists (verified 2026-11-23, code + live stores)

Four Web Push senders, three subscription stores plus one mirror, three VAPID keypairs, two class/preference models.

### 1.1 Aggregator (laptop; `pm-stack/aggregator`, `pm-aggregator.service`)

- **Store:** Turso (fleet-platform), attached schema `alerts.`: `push_subscriptions` (endpoint, p256dh, auth, per-subscription `classes` JSON map, `quiet` JSON, user_agent, gone_count, last_seen_at) and `notification_sends` (one row per send attempt). Live counts 2026-11-23: **subscriptions 0**; sends **271 rows, none after 2026-11-17T02:01:46Z**, the last two outcome `gone` (#14048's evidence).
- **Classes (8 toggleable):** `incident`, `needs-drew`, `quota-wall`, `quota-near-wall`, `section-risk`, `stack-health`, `blocked-age`, `prompt` — `NOTIFICATION_CLASSES` / `NOTIFICATION_CLASS_DEFAULTS` in `src/notifications.ts`. `digest` is a delivery shape (quiet-hours bundling), not a stored class; per-class test sends accept it additionally.
- **Sender:** `src/push.ts` over the `web-push` package. VAPID pair **A**: public key in code; private key resolved from Bitwarden by name at send time, held only in memory.
- **Engine:** producers raise incidents (broker raises via the alerts intake, watchdogs, sentinels) and the engine derives further candidates from snapshots (quota walls/near-walls, section risk, stack health, blocked age, prompt answers) in `buildNotificationCandidates`, with dedupe, cooldown, state fingerprints, quiet hours (per-subscription, breakthrough classes bypass), and digest bundling. Sends are written to the ledger by `AlertsStore.logSend` (`src/alerts.ts`); `acted_on_at` is stamped by the exactly-once Ack transition when Drew acts on the attention page.
- **Answerable pushes:** `needs-drew` payloads carry `{ledgerId, question, options[]}`; a tap lands on `/api/needs-drew/<ledgerId>/answer` under a per-question token and the resolution is recorded in the needs-drew ledger. Applying that answer to its source system is #14043's phase (design of record on that issue; applier aggregator-side on `recordAnswer`, signed decide envelope).
- **Surfaces:** `/alerts`, `/alerts/preferences` (per-subscription class toggles + quiet hours + test sends), `/notifications` (the #3319 sent-vs-acted-on roll-up), all at the tailnet URL `https://amd-6.raptor-pumpkinseed.ts.net`. Note the deployed aggregator build is stale (#14059: commit `7d3dcebe`, 2026-11-15) — the code inventoried here is main's, which is what any build slice starts from.

### 1.2 Console origin (laptop; `pm-stack/console`, `pm-console.service`)

- **Store:** a whole-document JSON file store. Push state at `~/.local/state/pm-console/push.json` — live tonight: **0 subscriptions**; keys `off` (store-global class off-list, currently empty), `subscriptions`, `vapid`. Notifier state in `notified.json` alongside it.
- **Classes (4):** `replies`, `needs`, `questions`, `trouble` (`CLASSES` in `src/push.ts`). Preferences are **store-global** (one off-list for the whole store), not per subscription.
- **Sender:** hand-rolled Web Push (`src/push.ts` — RFC 8291 aes128gcm encryption + RFC 8292 VAPID, platform crypto only). VAPID pair **B**, generated on first use and kept in the store file (`privateJwk` + `publicKey`).
- **Producer:** `src/notify.ts`'s Notifier — a diff engine over the console's state documents: it compares what the documents say with what has already been announced (`Seen`) and sends the difference (a fleet reply to one of Drew's prompts → `replies`; a new needs-hands item → `needs`; new questions → `questions`; a unit failed, with a 5-minute sustained-failure rule for timer units → `trouble`). First pass primes only, so enabling never floods.
- **Delivery feedback:** `Push.status()` (`GET /push`) reports the last send's taken/refused outcome; the app computes an **endpoint mark** (SHA-256 prefix of its endpoint) and compares it with the marks the server holds, "so the app can tell that this sender has lost it, without any address being sent back" — a sender-side subscription-loss self-check that already exists and that §5's migration reuses.
- **Devices:** `src/devices.ts` — a phone-link registry for the Android widget: challenge/verifier linking, the store keeps token _hashes_; a linked phone gets a read-only token for counts + top lines. This identity is unrelated to push subscriptions today.

### 1.3 Console edge (Cloudflare Worker; `pm-stack/console/edge`)

- The **same** `Push`/`Notifier` code, constructed over a **KV store** (`kvStore(this.kv, "push")`, `edge/src/edge.ts`). VAPID pair **C** lives in that KV store document (generated at the edge on first use; no seeding from pair B exists in code).
- This is the live serving path behind `https://pm.security.cab` — the site the Android app wraps, and one of the hosted-edition surfaces in flight under #9079.
- **Its subscription count is not readable from the laptop** — the edge exposes no census route for the push store. Whether Drew's phone currently holds a pair-C subscription cannot be answered from our own tooling; that unanswerability is itself one of the defects this design removes (§4.5).

### 1.4 Notify-relay (Cloudflare Worker + Durable Object; `pm-stack/notify-relay`, #8569 Phase 2 step 1)

- The aggregator POSTs a heartbeat every ~60 s and mirrors its **complete** subscription set (endpoints, keys, class maps, quiet hours) as one snapshot under a monotonically increasing `snapshotSeq`, on any subscription-store change (aggregator `src/notify-relay.ts`; shared secret by Bitwarden name on the laptop side, Worker secret at the edge).
- On silence > 150 s the Durable Object's alarm sends exactly one `stack.silent` Web Push ("PM stack is down"); the first heartbeat after silence sends exactly one recovered push. Both ride the `incident` preference class. The edge sends **only** this class by design invariant; the board never moves to the edge.
- **Sender:** its own `src/push.ts`, using the **aggregator's pair A** (public key copied in code; private key as a Worker secret — a second provisioned copy of the same key).

### 1.5 Android app (`pm-stack/console/android`; package `today.developing.pmconsole`)

- A **Trusted Web Activity** over the console site: "The site's notifications arrive as the app's own, with its mark in the status bar; the app asks for the permission itself." It has **no native push channel** (no FCM) — its notifications _are_ the console's Web Push, i.e. pair C / the edge store's subscriptions.
- Around the site: home-screen widget (polls the summary every ~30 min on the §1.2 device token), dictation/share intents into the Ask box, long-press shortcuts, `pm.security.cab` link interception. Mutual trust via asset statements + `/.well-known/assetlinks.json`.

### 1.6 The class union

Twelve distinct class ids exist across the stacks. Overlaps are near-misses, not duplicates: console `needs` = "a new item needs your hands" (census diff) vs aggregator `needs-drew` = "a question only Drew can answer, with answerable options" (attention tier + answer machinery). Console `trouble` (a unit failed) overlaps the _territory_ of `incident` (a watchdog reports something down) and `stack-health` (a monitored service starts failing its probe) but is produced by a different detector over different state. Console `replies` and `questions` have no aggregator counterpart; the quota family (`quota-wall`, `quota-near-wall`, `section-risk`), `blocked-age`, and `prompt` have no console counterpart.

### 1.7 What the fragmentation costs (the problem statement)

1. **The app Drew uses can never receive the stack's most important pushes.** Answerable `needs-drew` questions — the ones whose taps #14043 teaches the broker to apply — plus the whole quota family, `stack-health`, `blocked-age`, and `prompt` exist only in the aggregator's taxonomy, whose only delivery route is a browser subscription at a tailnet URL. Its table is empty.
2. **Delivery failure is silent and unmeasurable.** #14048: a week of dark delivery discovered by accident. #14061: the one measurement instrument reads a ledger whose rows it cannot date — `logSend`'s insert names no timestamp column, so the send time rides on a column default in a SQL-translated store, and the roll-up's `sent_at ?? created_at` parse rejects what the live rows carry (269/269 excluded). The console's only delivery signal is "the last send's outcome"; the edge's store cannot even be counted.
3. **Three VAPID identities** means three subscription universes: a subscription created under one pair is invisible to the others, and the sender that holds Drew's actual phone (pair C, edge) is the one with no answerable classes, no quiet hours, and no per-device preferences.
4. **Two preference models** (per-subscription vs store-global) means "turn off quota pushes" is a different operation — or impossible — depending on which stack a device happened to subscribe to.

## 2. Design principles

- **One registry, one sender, one taxonomy, one preference model** — stated as invariants, not aspirations; every component below either is the one, produces into it, or is retired.
- **Event-driven at the point of detection** (Drew's standing requirement, 2026-11-11): the stack pushes itself when it detects; producers emit at detection, and no new polling loop is introduced to move notifications around.
- **The exception that proves the rule is designed, not accidental:** the notify-relay remains a sender of last resort for exactly one class, because its purpose is the primary stack being down. It is a mirror _of the registry_, so its exception status costs no second registry.
- **Observability is a schema property,** not a page: timestamps are written by writers under a stated format, and an unmeasurable state alarms on a surface that does not depend on the thing being measured.

## 3. The unified design

### 3.1 One subscription registry

New tables in the **fleet-platform Turso store** (the aggregator's existing store; hosted, so the console edge reaches it over HTTPS the way it already reaches Turso for decision sets; the laptop components reach it as they do today):

- `push_endpoints`: `id`, `transport` (`webpush` in v1; the column is the extension point for a native transport), `client` (`browser` | `android-app`), `device_id` (nullable link to the console device registry), `endpoint`, `p256dh`, `auth`, `label`, `user_agent`, `created_at`, `last_seen_at`, `gone_count`. Endpoint key material never leaves the server side (the existing #3693 public-shape rule carries over: pages render everything except `p256dh`/`auth`).
- `push_preferences`: one row per (endpoint, class) with `enabled`, plus per-endpoint `quiet` (start/end/tz/enabled) — the aggregator's model, generalized.
- `push_sends`: the unified ledger (§3.5).

An endpoint is a _device-context_, not a browser accident: the Android app's TWA-held subscription registers with `client = android-app` and its `device_id` set, so the app is a first-class endpoint — named in the census, holding its own preferences, prunable on its own gone-streak — alongside browser endpoints. A future native (FCM) transport adds rows with `transport = fcm` behind the same preferences and ledger; nothing else in this design changes.

### 3.2 One sender

A single send service — in the aggregator today, in #8570's consolidated pm-core when that lands (this design does not wait for it) — owns **VAPID pair A** and is the only component that talks to push services for ordinary classes:

- Both existing engines become **producers** submitting candidates (class, object ref, title, body, landing URL, payload, breakthrough flag) to the sender's intake. The aggregator engine's dedupe/cooldown/fingerprint/quiet-hours/digest machinery operates at intake — it is the more complete machinery and is reused, not rewritten.
- The console Notifier's diff logic survives intact as a producer: its `Seen` state and sustained-failure rules are detection logic, and detection is exactly what a producer is.
- Console origin and edge `Push` instances **retire as senders**; pairs B and C retire with them.
- The notify-relay keeps pair A and its alarm behaviour, re-pointed to mirror **the registry** (§5). It is the designed exception of §2.

### 3.3 One class taxonomy

The union, each class with exactly one producer of record:

| Class                                           | Producer                                    | Notes                                                                     |
| ----------------------------------------------- | ------------------------------------------- | ------------------------------------------------------------------------- |
| `incident`                                      | aggregator intake (watchdogs/broker raises) | Breakthrough. `stack.silent`/recovered are payload variants at the relay. |
| `needs-drew`                                    | aggregator attention tier                   | Answerable; payload contract unchanged (§3.4).                            |
| `quota-wall`, `quota-near-wall`, `section-risk` | aggregator quota derivation                 | As today, incl. #8882's definitions.                                      |
| `stack-health`                                  | aggregator stack-health derivation          | Also carries §3.5's sentinel incidents (on non-push surfaces).            |
| `blocked-age`                                   | aggregator attention tier                   | As today.                                                                 |
| `prompt`                                        | aggregator prompt machinery (#10035)        | Per-submission opt-in still applies.                                      |
| `replies`, `questions`, `trouble`               | console Notifier                            | Detection logic unchanged; delivery via the one sender.                   |
| `needs`                                         | console Notifier                            | Kept distinct from `needs-drew` (see below).                              |

`needs` vs `needs-drew` stay **separate ids with separate labels** in v1 — "Needs me" (new items needing hands) vs "Needs Drew" (answerable questions). They make different promises and carry different payloads; merging them silently would change what Drew's toggles mean. Whether to merge is posed to Drew on #14084 as an explicit product question; the registry's class table makes either answer a data change, not a refactor. `digest` remains a delivery shape, never a class.

### 3.4 Answerable pushes

The `needs-drew` contract is preserved byte-for-byte in shape: payload `{ledgerId, question, options[]}`, tap → the answer page → `/api/needs-drew/<ledgerId>/answer` under the per-question token → resolution recorded in the needs-drew ledger. It must work identically when the endpoint's client is `android-app` — the TWA already intercepts `pm.security.cab` links, and the answer page is served from the same hosted surfaces the app wraps. Answer _application_ (the applier, its signed decide envelope, its exactly-once ledger) is #14043's design and is untouched; #13469's edge answer intake composes unchanged, because answers land in the same ledger regardless of which endpoint delivered the question.

### 3.5 Delivery observability (the #14061 fix, by construction)

- `push_sends`: one row per attempt — `endpoint_id`, `class`, `object_ref`, `incident_id`, `outcome` (`sent` | `gone` | `error` + push-service status), **`sent_at` stamped by the writer** as ISO-8601 UTC, schema-required (NOT NULL, no default), and `acted_on_at` stamped by the existing Ack transition.
- The `/notifications` roll-up (#3319's sent-vs-acted-on instrument) reads only this ledger from cutover.
- **Undated-row alarm:** any row the roll-up cannot date is a data defect that raises a visible incident — the alarm #14061 asks for, designed into the unified ledger. (The legacy ledger's repair/backfill and its own page alarm remain #14061's work; they are linked, not duplicated.)
- **Zero-endpoint sentinel** (#14048's detection ask): when the registry holds zero live endpoints while undelivered incidents exist — or no successful send in N days (N = 2 in v1) with incidents open — the stack-health sentinel raises a health incident on the **board and attention surfaces, never as a push**: a push about no-push is a dead letter by definition. The sentinel's home is the existing stack-health machinery (#8568's sentinel service ids).
- **Census:** one read — endpoints by transport/client, per-endpoint last-seen, last successful send — exposed on the aggregator's status surfaces and the console's `GET /push`-style status. "How many live endpoints exist?" gets exactly one answer, from the registry.

### 3.6 One preference surface

Preferences are managed where Drew and the app live: the console (pm.security.cab) serves the one preferences surface, reading/writing the registry — class toggles per endpoint, quiet hours, per-class test sends, endpoint labels, device links. The aggregator's `/alerts/preferences` keeps working while the aggregator still reads its own tables (migration window), then becomes a redirect to the console surface. Test sends exercise the one sender, so a test from either surface proves the same path a real alert takes.

## 4. Migration

1. **Aggregator subscriptions:** none live — nothing to move. Its table _shapes_ seed the registry schema (they are the richer model), and `AlertsStore` is re-pointed at the registry tables.
2. **Console edge subscriptions cannot be re-keyed.** A Web Push subscription is bound to the application-server key presented at `pushManager.subscribe()` time; rows written under pair C cannot be delivered to by pair A. Migration is therefore a **re-subscription**, not a copy: the console site/app compares its endpoint mark against the marks the registry holds (the existing §1.2 mechanism, built for exactly this detection), finds itself unknown to the one sender, and re-subscribes under pair A into the registry with `client` + `device_id` set. The KV push store freezes read-only for one release after cutover, then is dropped.
3. **Console origin store:** empty — retired outright; its `notified.json` producer state migrates to the producer's new home so the Notifier does not re-announce history.
4. **Relay mirror:** source switches from the aggregator's store to the registry (same snapshot protocol, same seq discipline, same shared secret by name). The mirrored set becomes _all_ live endpoints, so the dead-man's switch covers the app endpoint too.
5. **Sends history:** the legacy ledger stays as-is under #14061 (its repair is that issue's ask). The unified ledger starts at cutover; no backfill is attempted across the schema break — the roll-up's window simply begins at cutover, and the legacy page remains linked for the historical window.
6. **The device step in #14048 still stands** — Drew re-enabling notifications on his phone is the first live registration into the unified registry (via the app, `client = android-app`), and the census (§3.5) is how we prove it landed.

## 5. Boundaries and non-goals

- **#14043 (answer write-back)** owns answer application; this design changes nothing there and depends on nothing there.
- **#13469 (edge answer intake)** composes: however an answer reaches the ledger, the applier consumes it from there.
- **#8570 (service consolidation)** is the sender's natural eventual home; the unification is specified against the aggregator so it does not block on the consolidation.
- **Native FCM/APNs transport** is out of v1 scope; §3.1's transport column is its reserved seat.
- **The hosted-edition programme (#9079 slices)** is untouched except that the console edge's push store is one of the stores this design retires — S4 below is sequenced to land with, not against, that programme's edge work.

## 6. Risks and honest edges

- **Re-subscription depends on the shipped app/site actually running the endpoint-mark check on a cadence that fires without Drew doing anything.** If the check only runs when the app is opened, cutover completes the next time Drew opens the app — acceptable, but S4 must verify the trigger against the live build before the slice is specified (see §7).
- **Turso reachability from the edge** is assumed from the decision-set precedent (Worker + Turso is already the console's hosted data shape); S1 verifies the registry read/write path from the edge's runtime explicitly rather than assuming it.
- **The stale deployed aggregator (#14059)** means any aggregator-side slice includes a redeploy from current main; slices are specified against main, and their proofs run against the redeployed service.

## 7. Build-slice map (ordered — not packetised; see the note)

- **S1 — Registry + aggregator adoption.** `push_endpoints` / `push_preferences` in fleet-platform Turso; `AlertsStore` re-pointed; `/alerts/preferences` behaviour unchanged externally; edge registry read/write path verified. Proof: aggregator notification test suite green + a live subscribe → per-class test send on the laptop.
- **S2 — Unified ledger + observability.** `push_sends` with writer-stamped `sent_at`; roll-up on the unified ledger; undated-row alarm; zero-endpoint sentinel in the stack-health machinery; census read on both status surfaces. Lands #14048's detection ask. Needs S1.
- **S3 — Sender consolidation.** Console Notifier becomes a producer to the one sender; console origin `Push` retires; producer state (`Seen`) carried over. Proof: the four console classes delivered by the unified sender to a browser endpoint, incl. the sustained-failure rule firing once. Needs S1.
- **S4 — Edge cutover + app endpoint.** Edge `Push` retires as sender; subscribe path writes the registry with `client`/`device_id`; endpoint-mark re-subscription under pair A; the one preferences surface on pm.security.cab; `/alerts/preferences` redirect; KV store frozen. Needs S3.
- **S5 — Relay re-point + close-out.** Mirror source = registry; legacy stores dropped after their freeze windows; closure evidence posted for #14048 and #14061 (census shows ≥ 1 live endpoint per class path; a week of dated sends in the unified ledger). Needs S4.

**Why no slice is packetised yet:** two seams must settle on the #14084 record first. (i) The `needs`/`needs-drew` taxonomy call is Drew-facing — it changes what his toggles say — and is posed on the issue for his word; S1's schema does not depend on the answer, but S3's producer mapping does. (ii) S4 can only be specified exactly against the endpoint-mark flow's behaviour in the _shipped_ app build, which must be verified live before that slice is written. S1 and S2 are close to specifiable and packetise on this design's acceptance — the same design-first pattern #14043 used (its design of record landed, then its build slices were mapped from it).

## 8. Open questions (posed on #14084)

1. Class taxonomy: keep `needs` and `needs-drew` as separate toggles (recommended — different promises), or merge into one "Needs you" class?
2. Preferences surface home: the console (pm.security.cab) as the one surface with the aggregator page redirecting — as designed — or the reverse?
