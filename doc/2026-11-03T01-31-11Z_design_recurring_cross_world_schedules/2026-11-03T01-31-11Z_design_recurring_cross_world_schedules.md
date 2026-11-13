# Recurring and cross-world scheduled events

**Status:** Initial design for issue [#1355](https://github.com/developing-today/code/issues/1355), 2026-11-03.

## Intent

Keep one-shot schedules compatible while adding finite recurring schedules and delivery to another world only with explicit consent. `time.tick` remains a separate, unchanged mechanism.

## Durable recurrence and bounds

A schedule remains one pending record keyed by its existing ID. The record carries its next due time, an optional fixed interval, and a finite remaining-fire count; cross-world records also carry the destination world. Recurrence is fixed-rate: after firing at planned time `t`, the next due time is `t + interval`, never `now + interval`. This prevents late delivery and restart time from drifting the schedule. A fire's `System` event and its journaled fire transition record each occurrence; replay advances from those entries and never consults the wall clock. A finite count is required so a recurring schedule cannot live forever without a bound.

Each active schedule consumes one `MAX_PENDING_SCHEDULES` slot, regardless of how many fires remain. Re-arming changes that record in place and does not reserve a second slot; the slot is released only when the final occurrence fires or the schedule is cancelled. Admission/replay must enforce the cap, and checkpoint writing/restoration must serialize and validate the same complete set of active records. Replacing an ID validates the replacement first, then journals cancellation and the new set, preserving the existing atomic-refusal behavior.

Cancellation by ID cancels the whole series. Actor serialization orders a cancel against a due fire: already-journaled deliveries remain, while a cancellation that wins prevents all later occurrences. One-shot behavior is the same state machine with no interval and one remaining fire.

## Cross-world grant shape

The F12.14 design in `.opencode/plans/phase-12-platform-flexibility.md` calls for a portal capability granted by both worlds' admins. That implementation is not present yet: current `update_caps` grants are local capability names, and `cross_world_write` is a host file-policy gate, not portal consent. Reuse the F12.14 shape by requiring reciprocal, journaled permission for the named source/destination pair; do not treat `cross_world_write` as a substitute or create a schedule-only authorization model. A scheduled delivery carries only the bounded schedule ID and late flag, not arbitrary payload. The target must durably accept a delivery keyed by source world, schedule ID, and occurrence before the source consumes that occurrence, so a retry after restart is idempotent. Missing, revoked, isolated, or unavailable target/grant means refusal and the source occurrence stays pending.

## Verification

Extend the existing schedule bounds/refusal, cancellation, restart/replay, and checkpoint tests. Add bounded recurrence tests that verify the exact fire count and interval anchoring, replay without wall-clock drift, cap behavior through re-arming and compaction, and cross-world refusal/delivery with and without reciprocal grants.
