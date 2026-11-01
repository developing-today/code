# Design: PWA + push notifications for the PM aggregator

- Date: 2026-11-01
- Status: **design** — implementation is filed as sub-issues of #1184
  (listed in §7); nothing here is built yet.
- Issue: #1254 (epic #1184, PM stack coherence design series).
  Triage task 2514 confirmed the greenfield on main `a1aa07d8b`:
  no PWA manifest, no real service worker (`/sw.js` serves an
  unregister kill switch), no Web Push code, no design note.
- Branches on: #1182 (one public URL behind Cloudflare Access).
  That access model is settled but its implementation is gated on
  Drew's recorded go (`user/escalated`); this design therefore
  specifies both branches and recommends starting on the branch that
  exists today (§4).

## The shape

```
existing JSON surfaces          notification engine           Web Push
attention view       ─┐         (in the aggregator)           (RFC 8030 + VAPID)
  needs-drew tier     ├─ poll ─► diff vs engine state ─┬─► vendor push service ─► phone
/api/quota.json      ─┤         dedupe · quiet hours   │    (FCM / Apple / Mozilla)
/api/health          ─┘         batching · escalation  └─► send log (jsonl)
                                                              │ click
subscriptions.json ◄── /notifications page (per-class ◄───────┘
(state dir)             toggles, quiet hours)         deep link to the item view
```

Everything new lives inside the aggregator and its runtime state
dir — the same pattern as `comments.json` / `registry.json`
(`src/config.ts` state files, live copies under
`~/.local/state/pm-aggregator/`). No new service, no new host.

## 1. Notification classes and defaults

Classes are sourced from surfaces that already exist; the engine
invents no new detection.

| Class | Source | Trigger | Recommended default |
|---|---|---|---|
| `needs-drew` | the full attention view (below); the `needs-drew` tier is derived in `src/derive.ts:308-324` | an item *enters* the needs-drew tier | **on** |
| `quota-wall` | `/api/quota.json` + the quota registry | a pool reaches 100% / closed before its reset | **on** |
| `quota-near-wall` | same | a pool crosses 85% with time left to reset | **off**, except sole-door pools (below) |
| `stack-health` | `/api/health` section diagnoses | a stack section flips ok → failing | **on** |
| `blocked-age` | the full attention view, item ages | a needs-drew item is still unacted after the age threshold (§5) | **digest only** (never a per-item push) |

**The engine's attention source is the complete view, not the
flat API array.** The engine runs in-process and consumes the same
attention view the dashboard renders — `view.all` **plus**
`view.drewExtra` (`src/derive.ts:379,463`): the non-session
needs-drew items (decisions, tasks, gaps with no matching session)
that `src/render.ts:2402` includes in the displayed queue.
`/api/attention.json` (`src/server.ts:780-789`) returns only
`view.all` + watchdog entries, so an engine naively polling the API
would never see those items — no entry push, no blocked-age
escalation, for exactly the items most likely to be waiting on
Drew. Slice (c) therefore also extends `/api/attention.json` to
carry the drewExtra entries, so external consumers see the same
queue the HTML renders.

Two rules keep `quota-*` honest against the standing quota doctrine
(2026-10-29: pools are *supposed* to reach ~100% at reset; routine
exhaustion is not an anomaly):

- `quota-wall` fires only for a wall hit **well before** the pool's
  reset, or for a **sole-door** pool (no alternative route) at any
  point. A pool exhausting on schedule near its reset is silent.
- `quota-near-wall` defaults off for multi-route pools for the same
  reason; for sole-door pools it is the early warning and defaults
  on with the class.

**Per-class subscription:** the subscription record (§4) carries a
per-class on/off map, edited on the `/notifications` page and via
`POST /api/push/subscriptions` (same actions-store pattern as other
dashboard writes). Changing a default changes it only for new
subscriptions; existing subscriptions keep their map until Drew
edits them — a default change must never silently re-subscribe a
class he muted.

**The defaults themselves are Drew's call** — open question Q1 (§8).
The table above is the recommendation he is ruling on.

## 2. Quiet hours and batching

- Default quiet window: **23:00–07:00 America/Chicago** (Q2, §8).
  The window is a per-subscription setting, stored with the
  subscription, evaluated in the subscription's timezone at send
  time.
- During quiet hours, non-urgent classes (`quota-near-wall`,
  `blocked-age`, `stack-health` *recovery* notices) are not sent;
  they accumulate into **one digest notification** at window end.
  A second batch point at 12:00 collects the morning's non-urgent
  items so nothing waits a full day.
- **Breakthrough set** (sent immediately, quiet hours or not):
  `needs-drew` items Drew himself marked urgent is not a concept
  that exists yet, so the breakthrough set is exactly:
  1. `quota-wall` on a **sole-door** pool (work is about to stop
     with no alternative route), and
  2. `blocked-age` escalation at the 48-hour step (§5) — one push,
     then back to digest.
  `stack-health` does **not** break through: if the stack is down,
  the engine sending the notification is the thing that is down;
  its recovery digest reports the outage window after the fact.
- Every batched digest deep-links per item (§3): the digest
  notification's payload lists each item, and the click lands on
  the attention view filtered to those refs — still never a bare
  home page.

## 3. Deep-link contract

Every notification payload is `{class, objectRef, url, title, body}`
and the contract is:

- `url` is **the item's own triage-capable view** — the same URL the
  dashboard card for that item links to (item view for board items,
  `/sessions?id=<id>` for sessions, `/discuss/<objectRef>` where the
  comment thread is the triage surface). Triage-capable means the
  landed view exposes the item's actions (the dashboard actions API),
  not a read-only rendering.
- **Never** the origin root or a generic list. The only list-shaped
  landing allowed is the §2 digest case (attention view filtered to
  the digest's refs).
- The notification `body` always names the item in plain text (repo
  + number or session label), so the information survives even when
  the link cannot be opened (e.g. tailnet down, §4 branch A).
- Implementation acceptance per class: a test notification for each
  class lands on a view whose actions complete a triage round-trip
  (act on the item → item leaves the needs-drew tier → engine sees
  it acted-on, §5).

## 4. Delivery path and subscription storage — branched on #1182

**Common to both branches.** Delivery is the Web Push protocol
(RFC 8030) with VAPID authentication:

- Keys: one VAPID keypair for the aggregator. The public key ships
  in the page/service worker; the private key is a secret, fetched
  by name from the existing secrets flow (Bitwarden Secrets
  Manager / SOPS) at runtime — never in the repo, never in the Nix
  store.
- Sending is **outbound only**: the aggregator POSTs to the
  subscription's vendor push endpoint (FCM / Apple / Mozilla). No
  inbound public route is required to *send* — this is what makes
  branch A viable today.
- Subscription storage: `push-subscriptions.json` in the runtime
  state dir, one record per browser installation:
  `{endpoint, keys:{p256dh, auth}, classes:{...}, quiet:{start,end,tz},
  createdAt, lastSeenAt, goneCount}`. A 404/410 from the push
  service marks the subscription gone; two consecutive gone
  responses prune it. The store follows the comments/registry
  pattern (single JSON file, atomic replace, aggregator-owned).
- iOS constraint shaping the order of work: Web Push on iOS
  requires the site to be **installed** as a PWA (iOS 16.4+), so
  installability (§7 slice a) is a hard prerequisite for push on
  Drew's phone, not a nicety.

**Branch A — tailnet-only (today's reality).** The aggregator binds
`127.0.0.1:4870` + the tailnet name; there is no public route
(#1182 triage, verified at `f00dde689`).

*Prerequisite — a trusted HTTPS origin.* The canonical tailnet URL
today is **plain HTTP** (`pm-stack/aggregator/README.md:38`:
`http://amd-6.raptor-pumpkinseed.ts.net:4870`), and plain HTTP on a
remote host is not a browser secure context: service workers and
`PushManager` are both restricted to secure contexts, so **no
subscription can be created on the current origin at all**,
regardless of the outbound-only sending path. Branch A therefore
starts by serving the aggregator through **Tailscale Serve**
(`https://amd-6.<tailnet>.ts.net`, Tailscale-issued certificate) —
or an equivalent trusted certificate — and that HTTPS origin is the
PWA origin (`PM_AGG_PUBLIC_ORIGIN` defaults to it on this branch).
The plain-HTTP listener stays for local/plumbing use; nothing
PWA-related ever points at it. Subscriptions are created
from the tailnet origin while the phone is on the tailnet, pushes
are delivered by the vendor service over the phone's normal
connectivity, and the click opens the tailnet URL — which needs
Tailscale up on the phone to load. The plain-text body (§3) is the
fallback when it is not. **Recommendation: build on this branch
now.** It needs nothing from #1182.

**Branch B — behind Cloudflare Access (`pm.security.cab`).** If
Drew gives #1182 its go: the public origin becomes the PWA origin.
Differences, all configuration-level if branch A is built
origin-configurable from the start:

- The page, manifest, and service worker are served from the public
  origin; the subscription POST rides Drew's Access session cookie
  (he is logged in when he subscribes). The push *receive* path in
  the service worker needs no Access session — pushes arrive from
  the vendor, not from our origin.
- A click with an expired Access session redirects through the
  Access login and must preserve the deep-link path post-auth —
  an explicit acceptance check in the push-plumbing slice, because
  losing the path silently breaks §3.
- The engine and subscription store do not change at all.

The origin is therefore a single config value (`PM_AGG_PUBLIC_ORIGIN`,
defaulting to the tailnet origin); branch B is a config flip plus
the Access acceptance check, never a redesign.

## 5. Anti-fatigue

Fatigue is the failure mode that kills notification systems, so the
engine is conservative by construction:

- **Dedupe:** the engine keys sent notifications by
  `(class, objectRef, state-fingerprint)`. Entering a state pushes
  once; staying in it never re-pushes. Only an escalation step or a
  state change (e.g. near-wall → wall) sends again.
- **Cooldown:** per class per item, a minimum 6 hours between any
  two pushes about the same item outside escalation steps.
- **Escalation cadence for unacknowledged `needs-drew`:**
  entry push → +24h one reminder, batched into the midday batch →
  +48h one breakthrough-eligible push (§2) → after that, weekly
  digest only. Acted-on stops the ladder immediately.
- **Acted-on detection:** an item counts as acted-on when it leaves
  the needs-drew tier, or when Drew posts a comment/action on it
  through the dashboard stores — both already observable by the
  aggregator; no new tracking of Drew is introduced.
- **Weekly sent-vs-acted-on measure:** every send is appended to
  `notifications.jsonl` in the state dir (`{at, class, objectRef,
  outcome}` with outcome filled when acted-on is detected). A weekly
  roll-up on the `/notifications` page shows, per class: sent,
  acted-on within 24h, ratio. Any class under 10% acted-on over a
  rolling 4 weeks is *proposed* for default-off in the roll-up —
  a proposal on the page, never an automatic change (defaults are
  Drew's, Q1).

## 6. The `/sw.js` kill switch — disposition

Today `/sw.js` (`src/server.ts:815-819`, `SW_KILL_SWITCH` at
`src/server.ts:227`) serves a worker whose whole job is to
`registration.unregister()` itself and navigate clients away: it
exists to evict a service worker from **opencode** that was
stranded on this origin. It unregisters *any* registration —
including, if naively replaced in the wrong order, the real
aggregator worker, and including, if naively *kept*, our worker
forever.

**Decision: two-step replacement, probe-gated.**

1. **Installability first, no worker change.** Slice (a) ships the
   manifest and icons while `/sw.js` stays the kill switch. (iOS
   install works without our own worker; push waits for step 2.)
2. **Probe, then replace.** The aggregator gains a tiny probe: pages
   report `navigator.serviceWorker.getRegistrations()` script URLs
   to `/api/sw-probe` (counted, no content). When 14 consecutive
   days of probe data show **no foreign registration** on any
   client, the stranded opencode worker is gone and slice (b)
   replaces `/sw.js` with the real worker outright. If a foreign
   registration *is* still seen, the real worker's activate handler
   carries the kill switch's job forward for one release: it
   unregisters any registration whose script URL is not its own
   before claiming clients — eviction preserved, then removed in
   the following release once the probe is clean.

Either way the invariant holds: **no release ever serves a `/sw.js`
that neither evicts the stranded worker nor is the real worker**,
and no release silently drops the eviction behaviour.

## 7. Implementation slices (filed as sub-issues of #1184)

a. **PWA installability** — Tailscale Serve HTTPS mapping for the
   aggregator (the §4 branch-A prerequisite: config + README/topology
   docs), manifest, icons, iOS meta, install affordance; `/sw.js`
   untouched.
b. **Web Push plumbing** — VAPID secret wiring, subscription store
   + API, service worker push/notification-click handlers, §6
   step-2 kill-switch migration, §4 branch-B Access deep-link
   check (when #1182 lands).
c. **Notification engine** — pollers over the full attention view
   (incl. drewExtra, §1), quota, and health; class triggers per §1
   (incl. the quota-doctrine rules), dedupe, quiet hours, batching,
   escalation ladder, send log; extends `/api/attention.json` with
   the drewExtra entries (§1).
d. **Preferences + deep links** — `/notifications` page (per-class
   toggles, quiet hours), per-class deep-link acceptance per §3.
e. **Measurement** — weekly sent-vs-acted-on roll-up on
   `/notifications`, default-off proposals per §5.

Slices a–d are the critical path (a before b before c; d with c).
Slice e follows once sends exist to measure.

## 8. Open questions for Drew (explicit)

- **Q1 — Defaults.** Is the §1 recommended default set right
  (on: needs-drew, quota-wall, stack-health; sole-door near-wall
  on; blocked-age digest-only), or should the first version ship
  with *only* needs-drew on until the fatigue measure exists?
- **Q2 — Quiet hours.** Is 23:00–07:00 America/Chicago the right
  window, and is the §2 breakthrough set (sole-door quota wall,
  48h blocked-age) the right one — anything else that should wake
  the phone, or anything there that should not?
- **Q3 — Blocked-age threshold.** First reminder at +24h and
  breakthrough at +48h (§5): right cadence, or should the ladder
  start at +48h/+96h for a dashboard reviewed daily anyway?
- **Q4 — VAPID contact.** Web Push wants a `mailto:` subject in
  the VAPID claim; which address should it carry?
- **Q5 — Second device.** Is Drew's phone the only subscription
  target for v1, or should a second device (tablet/desktop browser)
  be in the acceptance set for slice (b)?

Nothing else in this note waits on Drew: slices (a) and (b) can be
built against the recommended defaults, and a Q1/Q2 ruling changes
config values, not the design.

## Non-goals

- No native app, no SMS/email/pager fallback channel.
- No change to #1182 itself; this design consumes its outcome.
- No notification content beyond what the dashboard already shows
  the item's owner — pushes carry the item's public-to-Drew facts
  only (title, repo, class), never secrets or comment bodies.
