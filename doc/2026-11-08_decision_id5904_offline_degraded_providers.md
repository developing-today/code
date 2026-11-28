# Decision: offline & degraded operation for capability providers

- **Date:** 2026-11-08
- **Status:** decision record — no implementation is included in this change.
- **Issue:** [#5904](https://github.com/developing-today/code/issues/5904),
  "id: design: offline & degraded operation for providers (from #5686)".
- **Parent design:** [#5686](https://github.com/developing-today/code/issues/5686)
  (client-side capability providers) and its in-tree design record
  `doc/2026-10-25T05-37-57Z_design_client_capability_providers/`
  (F13.7: `side: client` providers, the ceiling model, two-key opt-in,
  journaled results vs ephemeral routing, bounded replies, stable refusal
  codes). The parent's decisions are **not re-decided here**; §1 lists what
  this record takes as given.
- **Inputs cited:**
  - [#5900](https://github.com/developing-today/code/issues/5900)
    (capability grants for providers) — still open. This record decides
    only the *offline semantics* of grants (caching and expiry, §5); grant
    scoping, sensitivity classes, and approval flows remain #5900's to
    settle, and the expiry fields decided here ride on whatever grant
    shape #5900 lands.
  - [#5931](https://github.com/developing-today/code/issues/5931)
    (field survey) — still open; an inventory, not a decision. This record
    does not wait for it: the per-class rules in §3 are stated against the
    parent's target classes, and §8 says what the survey could change.
  - Triage scoping on #5904 (task 10306, 2026-11-05; re-triage task 12388,
    2026-11-08): the four questions this record must answer — buffering,
    backfill/gap semantics, cached-grant expiry, per-class offline
    actuation — and the ordering note that cached-grant decisions consume
    #5900.
- **Evidence baseline:** `origin/main` at `25210239f3`, worktree
  `~/git/fleet/12984`, 2026-11-08. Verified at that baseline: no doc/
  record settles offline or degraded provider operation; the F13.7 record
  covers a vanished client only for replay (replay never contacts a
  client) and for reopened pending requests (refused `actor_denied`,
  reason `restarted`). Live request routing to an unreachable provider,
  store-and-forward delivery, and buffered observation data are
  undecided there — that gap is this record.

## Decision summary

| Question | Decision |
| --- | --- |
| Presence states | Three — `online`, `degraded`, `offline` — derived from heartbeat/deadline signals (§2), never from a single transport error |
| Who buffers | The **provider side only**, and only observation data, within manifest bounds (§4). The host never queues requests for an unreachable provider |
| Actuation while unreachable | **Never queued for later execution.** An actuation that cannot be delivered inside its deadline fails `provider_unavailable`; store-and-forward actuation exists only where the manifest and the grant both explicitly declare it (§3) |
| Backfill | Provider sequence-numbered; gaps are delivered as explicit gap markers, never silently filled or interpolated (§4) |
| Cached grants | A provider may act on a cached grant snapshot only for observation, only until the earlier of grant expiry and the manifest cache TTL, and **never for actuation** (§5) |
| Degraded operation | A *negotiated* reduced agreement (lower rate, coarser data) declared in the manifest's degraded profile — never a silent quality drop (§6) |
| Discovery while offline | Stale marking and tombstone exactly as the parent's discovery rules; staleness is metadata, it does not extend any grant or deadline (§2) |

## 1. What the parent already fixed (not re-decided here)

From #5686 / F13.7, taken as given:

1. One provider model for every target class (ESP32/MQTT bridge, terminal,
   Bevy, smartphone, web). If a rule below needs a per-target special case
   outside the class table in §3, the rule is wrong, not the table.
2. A provider offers nothing above its ceiling; a consumer receives
   nothing above its grant; client providers default to
   `default_ceiling: excluded`.
3. Two keys for client providers: the world grant **and** the client's
   own per-world opt-in, checked on every request.
4. Results consumed by `update` are journaled; routing state (pending
   table, nonces, advertisements, deadlines) is ephemeral. Replay never
   contacts a client, and revocation stops future reads without erasing
   history.
5. Refusals carry stable error codes (F11.13); an unreachable provider at
   a request's deadline surfaces as `provider_unavailable`.

## 2. Presence: online, degraded, offline

- A provider session heartbeats at the interval its manifest declares.
  **Online:** last heartbeat within 2 intervals. **Degraded:** heartbeats
  arriving but the session has declared a reduced profile (§6), or
  heartbeat jitter exceeds 2 intervals while requests still complete
  inside their deadlines. **Offline:** no heartbeat for 3 intervals, or
  the session ended (tab closed, game quit, device asleep) — the parent's
  tombstone path.
- Presence is a property of the *session advertisement*, which is
  ephemeral (§1.4). It is never journaled and never, by itself, changes a
  grant, a ceiling, or a deadline.
- A single failed request or transport error changes nothing: presence
  moves only on heartbeat evidence. This kills the flap mode where one
  dropped MQTT packet marks a sleeping-but-healthy device offline and
  the next request refuses against a provider that would have answered.

## 3. Per-class actuation rules

Actuation = any capability whose use changes state outside the journal
(an output, a write, a physical effect). Observation = a bounded read or
a sensor sample whose result is data for `update`.

| Class | Examples | Observation while offline | Actuation while offline |
| --- | --- | --- | --- |
| Bridge / store-and-forward | ESP32 over MQTT | Provider buffers samples (§4); delivered as backfill on wake | Refused at the deadline, **unless** manifest declares `offline: queueable` *and* the grant names queued actuation; then the request rides the bridge with its original deadline and is dropped, audited, if the deadline passes before delivery |
| Session-bound interactive | terminal, web, smartphone | No provider-side buffer exists (the session is gone); consumer sees `provider_unavailable`; subscriptions resume at rejoin with a gap marker (§4) | Refused at the deadline. Never queued — there is no store the request could honestly wait in |
| In-process | Bevy / engine plugin | Provider and consumer share a process; "offline" means the plugin is disabled or the engine paused — samples are simply not produced; gap marker on resume | Refused immediately; the request's deadline machinery is not involved |

The deadline is part of the request (F13.7 wire sketch: `deadline_ms`).
Offline handling may never extend it. A request that expires undelivered
is finished — refused and audited — not retried under a new identity;
a consumer that still wants the effect asks again, as the parent's
at-least-once rule already requires after a restart.

## 4. Buffering and backfill (observation only)

- Only the provider buffers, only its own observation output, and only
  up to the manifest `bounds` (count and bytes). Overflow drops the
  **oldest** samples and is reported as part of the next gap marker;
  a buffer that silently discards is a falsified record.
- Every buffered sample carries the provider's monotonic sequence
  number and the provider-side timestamp. On reconnect, backfill is
  delivered in sequence order as ordinary result events; only results
  `update` consumes are journaled (parent §1.4) — backfill gets no
  special journal path.
- **Gaps are explicit.** Where sequence numbers jump — overflow, a
  session-bound provider that was closed, an in-process provider that
  was paused — the consumer receives a gap marker naming the missing
  sequence range and the reason class (`overflow`, `session_ended`,
  `paused`). Consumers must never be able to mistake a gap for a quiet
  period, and no layer interpolates, synthesises, or back-dates samples
  to hide one.
- Dedup is by `(provider, sequence)`: a sample redelivered after a
  reconnect is dropped as a duplicate, audited, and never reaches
  `update` twice. (Redelivery is expected on store-and-forward bridges;
  exactly-once is claimed at the journal boundary, not on the wire.)

## 5. Cached grants and expiry

The case that matters is the bridge class: a device wakes to requests
that were queued while it slept, and must decide — possibly before it
can revalidate anything — whether the grant behind a request still
holds. Session-bound and in-process providers make this decision
against the live host and cache nothing.

- A provider may cache a **grant snapshot**: the grant's scope, its
  absolute expiry, and a snapshot timestamp, learned while online.
- A cached snapshot authorises **observation only**, and only until the
  earlier of (a) the grant's own expiry and (b) snapshot time + the
  manifest's grant-cache TTL. Expiry is absolute: use does not renew it
  (no sliding window), and presence (§2) does not extend it.
- A cached snapshot **never authorises actuation**. Queued actuation
  (§3, bridge class) is decided only after the device has revalidated
  the grant live on wake; if it cannot revalidate, the request expires
  by its deadline like any other.
- **Revocation beats backfill.** On reconnect, before any buffered data
  is delivered, the provider checks for revocation learned since the
  snapshot. Data gathered under a since-revoked grant is not delivered;
  it is dropped and the drop is audited on the client (the parent's
  two-sided audit rule). Data already journaled before revocation is
  history and stays (§1.4).
- Every offline decision made from a cached snapshot is audited on the
  client with the snapshot timestamp, so a later review can tell a live
  grant decision from a cached one.

## 6. Degraded operation

- Degraded is a **negotiated** state, not a failure the consumer
  discovers from bad data. A provider that cannot meet the negotiated
  agreement (rate, resolution, freshness) offers its manifest's
  **degraded profile** — the reduced rate/quality it *can* hold —
  through the parent's negotiation path: partial agreement or a clear
  refusal. Silent quality reduction under an unchanged agreement is a
  protocol violation.
- A degraded profile may reduce rate, resolution, or freshness bounds.
  It may never widen scope, raise a ceiling, or stretch a grant: the
  degraded agreement is the intersection of the original grant and the
  degraded profile, nothing more.
- Observation backfill (§4) from a degraded period is marked with the
  profile in force, so journaled data records the quality it was
  gathered at.

## 7. Invariants implementation must hold

1. No actuation executes after its request deadline, from any queue,
   buffer, or bridge, under any class.
2. No layer between provider and journal can make buffered data
   indistinguishable from live data: sequence, timestamps, gap markers,
   and degraded-profile marks survive to the consumer.
3. A grant decision made offline is never stronger than the snapshot it
   was made from: observation-only, absolutely expired, actuation never.
4. Revocation learned at reconnect is applied before any backfill from
   the revoked grant is delivered.
5. Presence changes (§2) never mutate grants, ceilings, deadlines, or
   journal content.
6. Every offline outcome — buffered, dropped, gap-marked,
   refused-at-deadline, decided-from-cache — leaves the parent's
   two-sided audit entries (host and client).

## 8. Alternatives rejected, and what could reopen this

- **Host-side request queueing for offline providers.** Rejected: the
  host would be storing actuation intent it cannot deadline honestly,
  and a queued host request blurs the parent's routing rule (requests
  route to a live advertised session). The bridge already is the
  store-and-forward layer for the one class that needs it.
- **Queue all actuation on the bridge.** Rejected: a physical effect
  executed minutes after its context vanished is the failure this
  record exists to prevent; opt-in per manifest *and* per grant keeps
  it possible only where a design has explicitly accepted it.
- **Sliding grant-cache expiry (use renews).** Rejected: a busy device
  would never let a cached grant lapse, which is a revocation bypass
  with extra steps.
- **Silent gap filling / interpolation.** Rejected: journaled data is
  the replay source of truth; synthesised samples would be replayed as
  fact.
- **Refuse everything while offline, including bridge observation.**
  Rejected: it throws away exactly the sensor data the bridge class
  exists to collect, at no security gain — observation under an
  unexpired grant is already parent-sanctioned.
- The **#5931 survey** could add a target class or show a class above
  mis-assigned (e.g. a phone that is sometimes a bridge). That changes
  rows of the §3 table, not its rules; this record is written so the
  table is the only place a class change lands. **#5900** landing a
  different grant shape renames the §5 snapshot fields, not their
  semantics.

## 9. Splits

No new splits are filed. Implementation of this record is ordinary
follow-on work against #5686's slices (the shell-contract v2 frames need
a presence/gap-marker addition; that belongs to the F11.12 frame-kind
decision the parent already waits on, and is noted here rather than
filed as a separate design).
