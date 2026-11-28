# Time-based priority schedules for route pools (#8838)

Status: design record for implementation; no runtime code changed here.  
Date: 2026-11-17.  
Issue: [developing-today/code#8838](https://github.com/developing-today/code/issues/8838).  
Register: F4.18 in `pm-stack/design-register-2026-11-11.md`; tracking epic #8805.

## Intent and boundary

Allow an operator to adjust the relative preference for a provider, model, or
route pool during recurring local-time windows. This is a ranking preference,
not an availability rule: a low-priority target remains eligible as fallback.
When no schedule matches, the target's configured priority is `1` (neutral).
The feature does not alter credentials, quotas, provider health, stickiness, or
explicit user adjustments.

The runtime owner is the model-route selection boundary in `pkgs/mcpx/internal/model`:
add schedule parsing/evaluation to the route-pool selector there, before its
existing candidate ordering/sampling. `profileRouter` currently walks an
ordered `model.profiles` chain and is not itself a route-pool selector; do not
silently change profile ordering to approximate this feature. The selector
receives an injected clock (`Now func() time.Time`) so matching and ranking are
deterministic in tests. The provider call layer must not read wall-clock time.

## Proposed configuration

Add an optional `model.routes.schedules` array. A schedule applies to exactly
one target namespace; targets are provider IDs, `provider/model` IDs, or
`route/<pool-id>` references. Matching is case-insensitive only where existing
model/provider lookup is case-insensitive; route IDs otherwise retain their
configured spelling.

```json
{
  "model": {
    "routes": {
      "schedules": [
        {
          "target": "route/expensive-pool",
          "daysOfWeek": ["mon", "tue", "wed", "thu", "fri"],
          "daysOfMonth": [1, 15],
          "start": "09:00",
          "end": "17:00",
          "timezone": "America/Chicago",
          "priority": 0.2
        }
      ]
    }
  }
}
```

Fields:

- `target` (required): one provider, model, or route pool.
- `daysOfWeek` (optional): ISO weekday names (`mon` … `sun`).
- `daysOfMonth` (optional): integers 1–31. If both calendar selectors are
  present, both must match. If neither is present, every day matches.
- `start`, `end` (required): local `HH:MM`, start inclusive and end exclusive.
  Equal values are invalid; an overnight window is represented as a window
  whose `end` is earlier than `start` and is attributed to the day on which it
  began (e.g. Friday 22:00–02:00 also matches Saturday 00:00–02:00).
- `timezone` (required): IANA zone name, loaded with Go's timezone database.
  No machine-local or UTC implicit default.
- `priority` (optional): finite positive number, default `1`. Higher values
  rank ahead; values below `1` demote. Zero is not allowed, so a schedule
  cannot accidentally disable fallback.

For overlapping schedules on one target, the highest matching priority wins;
no matching schedule means `1`. Apply this as a multiplier to the target's
ordinary configured weight. This preserves existing weight semantics and
makes schedule behavior composable with static configuration. Do not multiply
multiple matching schedules together: overlap should not amplify unexpectedly.

## Selection precedence and failure behavior

1. An explicit user target/override wins and is not re-ranked.
2. A valid sticky target remains first while it is eligible under existing
   stickiness rules; schedules do not dislodge it.
3. For the remaining eligible candidates, order/sample using ordinary route
   weights multiplied by the effective scheduled priority. Existing quota,
   health, and availability exclusions remain authoritative; a schedule never
   revives an unavailable target.
4. If no schedule matches, use neutral priority `1` and preserve current
   behavior exactly.

Invalid schedule entries fail open to neutral priority for the affected entry:
ignore that entry, emit a startup/config diagnostic that identifies the field
and schedule index, and continue with base weights. A bad timezone must not
make every route unavailable. Structural errors that prevent decoding the
configuration continue to use the existing config loader's normal error path;
this rule concerns semantically invalid schedule entries that can be isolated.
There is no cache freshness/staleness state: schedules are evaluated from the
current injected wall clock at each route selection. A clock error is not a
runtime condition in Go; tests pass a fixed clock.

## Worked peak-hours example

At 10:30 on a Tuesday in `America/Chicago`, an operator wants the usual
`expensive-pool` to lose to `economy-pool` during weekday business hours. The
schedule above matches `expensive-pool` and multiplies its base weight by
`0.2`; `economy-pool`, with no matching schedule, remains at `1`. Both remain
eligible, but the route selector prefers `economy-pool` according to the
selection mode. At 18:00, or on Saturday, the schedule does not match and the
expensive pool returns to its ordinary configured weight. A sticky eligible
expensive-pool remains selected during the window, and an explicit user
selection always wins.

## Test and acceptance plan for implementation

Use a fake clock in `internal/model` tests; no sleeps and no host timezone
assumptions. Cover exact start/end boundaries, each weekday, month days 1 and
31, both calendar selectors combined, overnight windows and attribution,
IANA/DST transitions, overlapping entries (maximum wins), neutral default,
provider/model/pool targets, composition with base weights, sticky and
explicit override precedence, unavailable-target filtering, malformed fields
and unknown timezone fail-open diagnostics. Add a deterministic worked example
asserting the economy target ranks before the demoted pool during the window
and the original ordering returns outside it. Existing routing behavior with
no schedules must remain byte-for-byte equivalent in its selected target
sequence for a fixed seed/clock.

## Decision record and implementation handoff

This record fixes the schema, precedence, invalid-entry behavior, evaluation
owner, and fake-clock strategy. Implementation should be a separate packet
against this record. It should first confirm the runtime route-pool selector
and its current stickiness/user-override APIs; if the target repo's selector
is not `pkgs/mcpx/internal/model`, update the implementation location without
moving time evaluation into provider calls. The design does not require a
Drew-only decision: it specifies a fail-open, fallback-preserving preference
mechanism and leaves existing user controls dominant.

## Acceptance mapping

1. Schema: `daysOfWeek`, `daysOfMonth`, local window, IANA timezone, target
   namespace, and default priority `1` are specified above.
2. Precedence: explicit override > eligible stickiness > scheduled candidate
   ranking; invalid entries fail open to neutral priority.
3. Evaluation: model-route selection boundary with injected `Now` clock;
   test cases are enumerated above.
4. Implementation handoff: separate implementation task should be enqueued
   from this record; this design is not an implementation authorization.
5. Worked example: weekday peak-hours demotion of `expensive-pool` in favor
   of `economy-pool` is shown in schema and prose.
