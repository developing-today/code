# pm-stack quota/throughput design contract (#851, #3933)

Design record for the joined quota/throughput model the pm-stack
aggregator renders, written 2026-11-08 as the #3933 remainder landed.
Issue: https://github.com/developing-today/code/issues/3933
Parent contract: https://github.com/developing-today/code/issues/851

## The joined model

One feed, one quota view (`QuotaView`, `pm-stack/aggregator/src/model.ts`):

- **Pools** (`QuotaPool[]`): identity (id, provider, plan), state
  derived from windows (`healthy` / `near` / `walled`, `src/derive.ts`
  `poolState`), and `windows` (`QuotaWindow`: name, unit, limit,
  used, resetsAt, percent).
- **Throughput**: `pool.burnTokensPerMin` (measured rate, below) and
  `pool.activeSessions` (sessions the OpenCode server holds active
  right now, with cumulative tokens — live attribution landed before
  #3933; collector `poolBurn` in `scripts/collect-fleet-feed.ts`).
- **History**: `quota.burn` (`BurnSegment[]`: provider, pool, from,
  to, tokens) — completed reset periods only, derived by
  `src/burn-history.ts` from the collector's persisted pool-window
  observations (#3933). The in-progress period is never emitted as
  history.
- **Guidance**: `quota.choices` (`src/quota-guidance.ts`
  `computeChoices`): `hasHeadroom`, `burnBeforeReset`, `doNotStart`.
- **Provenance/freshness**: the gatherer overlay summary (#1151),
  per-pool freshness, and the feed-load sanitization counts (#2429 /
  #3921 / #4546) ride on the same view, so an incomplete read is
  distinguishable from a successful empty one.

Consumers: `src/digest.ts` projects wall time from
`burnTokensPerMin` (remaining ÷ rate); `src/render.ts` renders pools,
history, and choices. Feed-load sanitization (`sanitizeQuotaView`,
`src/feed.ts`) validates every element: a measured rate survives
exactly when it is a finite non-negative number; invalid rates and
malformed segments drop individually and are counted, never
swallowed.

## Refresh cadence by source

| Source | Cadence | Collector behaviour |
| --- | --- | --- |
| Stewardship quota view (dated per-pool reads, `src/quota-extract.ts`) | Refreshed by the stewardship cycle; read at every collector tick | A pool whose read pattern stops matching renders identity + a "no current read" note |
| OpenCode server API (sessions + burn attribution) | Live at the collection instant | Unreachable server = empty attention section, marked `error`, feed still publishes |
| Quota gatherer (`quota-current.json`, keeper `check-usage`) | The keeper's own tick | Overlaid when present (#1151); absent file = stewardship reads stand alone |
| Usage snapshot outbox (hub §4 writers) | Drained at every collection | Records ride the feed's `snapshots` field; the laptop store dedupes on (service, metric, ts, value) |
| Burn-history observation state (#3933) | One observation per pool window plus one live token total per pool appended at every collection | Persisted beside the configured charges-surfaced state (`burn-history-state.json` in `dirname(CHARGES_SURFACED_STATE_PATH)`, `BURN_HISTORY_STATE_PATH` override); 35-day retention, capped per window; fail-soft — a lost state restarts history honestly empty |

The collector itself runs on the laptop as a user timer declared in
the pm-stack NixOS module (the sandbox producer was retired
2026-10-31, #3362/#3363).

## Measured-rate contract (#3933)

- `burnTokensPerMin` is **measured, never estimated**. Two
  derivations, both in `src/burn-history.ts`: (a) the token delta
  between the two most recent observations of one tokens-unit
  window inside the same reset period, divided by the minutes
  between them, only when that pair is at most `RATE_WINDOW_MS`
  (120 min) apart; (b) the live derivation — the delta between a
  pool's two most recent session-attribution token totals
  (cumulative tokens of the sessions `poolBurn` attributes to the
  pool at each collection). Today's quota sources emit only
  percent/USD windows, so (b) is the derivation that fires live
  and (a) is armed for the first tokens-unit window source. (b)
  diffs **matching session counters only** — the per-session
  cumulative counters recorded with each pool-token observation:
  a session that resumed between collections does not inject its
  historical tokens into the delta, a session that ended does not
  subtract its own, and a pair that cannot be compared that way
  (a missing per-session breakdown on either side, or every
  matching counter reset) yields no rate. (b) is also freshness-
  bounded: the newest observation of the pair must be within
  `RATE_WINDOW_MS` of the collection instant, so an idle pool or
  an unreachable server stops publishing its last measured rate
  instead of carrying it for the observation retention period.
- Pools without a qualifying pair **omit the field**. Fixture rates
  (`src/sources/fixtures.ts`) exist only in fixture data and never
  reach a live pool.
- Only tokens-unit windows produce rates or segments — a percent,
  USD, or requests window cannot honestly be expressed in
  tokens/min or `BurnSegment.tokens`, so it contributes neither.
- A reset boundary is a changed `resetsAt` or a drop in `used`
  between observations; the completed period's segment carries its
  final observed `used` as `tokens`, bounded by its first and last
  observation times.

## Per-pool wall / near-limit rules

From `poolState` (`src/derive.ts`) and `computeChoices`
(`src/quota-guidance.ts`), applied per pool over all its windows:

- **walled**: any window at ≥ 100% — pool lands in `doNotStart`;
  `wallLiftsAt` carries the reset time when known, and the digest's
  wall projection uses the measured rate when one exists.
- **near**: any window at ≥ 85% (`NEAR_PCT`) — also `doNotStart`:
  too close to the limit to start a second session.
- **healthy with headroom**: eligible for `hasHeadroom`, ordered by
  lowest peak-window usage first.
- **burn-before-reset**: a healthy pool whose soonest reset is
  within 24 h (`RESET_SOON_MS`) with unused headroom is listed in
  `burnBeforeReset`, soonest reset first — spend it before it
  expires.
- **no current read** (no windows): no guidance at all — the pool is
  in none of the three lists.
