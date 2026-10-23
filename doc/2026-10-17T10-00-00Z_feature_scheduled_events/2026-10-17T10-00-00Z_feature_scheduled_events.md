# Scheduled events: a world can ask for one fire at a set time

See the [Phase 9 plan](../../.opencode/plans/phase-9-portable-platform.md) (Phase 10 progress, P10.3). This record follows [roles and directory](../2026-10-17T08-00-00Z_feature_world_roles_directory/2026-10-17T08-00-00Z_feature_world_roles_directory.md) and [world capability ceilings](../2026-10-16T11-00-00Z_feature_world_capability_ceilings/2026-10-16T11-00-00Z_feature_world_capability_ceilings.md).

## Intent

The Phase 10 sketch asks for "a first-class `schedule` capability for tournaments and reminders, durable in the journal, delivered as ticks." Subscribed ticks are a repeating timer tied to presence: they stop when nobody is connected and carry no meaning beyond an interval. A program that needs "start round three at 20:00 UTC" or "remind me in an hour" has nothing durable to ask for. This slice adds two capability requests and a journaled set of pending fires. It does not add recurring schedules, cross-world scheduling, or any way for a participant to see the schedule.

## Requests

- `schedule.at` with args `{"id": "<text>", "at": <unix ms>}` returns `{"id", "fires_at"}`.
- `schedule.cancel` with args `{"id": "<text>"}` returns `{"cancelled": <bool>}`. An id that is not pending returns `false`.

Both are ordinary capability requests. They go through the capability ledger, so they are refused and counted when the world has not granted them or its ceiling excludes them. `decide_for` checks the acting participant's scopes through `allows_effect`; the default branch requires `join`, as for `records.read`. The program learns each outcome through the existing `result` event. No wire frame changes.

A fire reaches the program as a host event with participant 0. It is never sequenced, never sent to sessions, and never part of `events_after` or a snapshot:

```json
{ "cap": "schedule", "event": "fired", "id": "<id>" }
```

`"late":true` is added only when the schedule was already overdue when the world reopened.

## Decisions

1. **Schedules are world state.** They live on `WorldCore`, not in the program, and they survive a program install. A new program receives the fires of schedules the previous one pended. The install clears only what belongs to a program's own subscriptions and requests. Fires do not wait for presence, unlike ticks: a world with nobody connected still runs its tournament.

2. **Accept, journal the delivery, then consume the schedule.** A fire runs in this order:
   1. The program's `update` accepts the fire event. A refusal marks the program unavailable, as for any event, and nothing is journaled.
   2. The event is journaled as `JournalEntry::System`, the same entry ticks use.
   3. `JournalEntry::ScheduleFired { id }` removes the schedule from the pending set.

   The brief said "journaled before delivery, the same discipline as ticks". That does not match the code. `deliver` (used by ticks, presence, and results) and the participant input path both run `update` before they journal, because the journal must hold only what the program accepted. Journaling a fire first would record a delivery the program refused, and a restart would then lose the fire. This order keeps the schedule pending until its delivery is durable. The one window is a storage failure between steps 2 and 3. Then the fire is journaled but the schedule still pending, so a restart delivers it again, with `late:true`. That is at-least-once, and the program can dedupe on `id`.

3. **Restore never reads the clock.** Pending schedules are rebuilt from `ScheduleSet`, `ScheduleCancelled` and `ScheduleFired` entries. A replay feeds the program the `System` events that were journaled, byte for byte, so `late` is not recomputed during replay.

4. **Late means overdue at open.** When the actor starts, it records which restored schedules are already past their `at`. Their first fire carries `late:true`. A schedule requested in this run is never late, even if its `at` was in the past, and a cancel or fire removes the id from that set.

5. **Waiting only while deliverable.** The loop sleeps until the earliest pending `at` only while the program is healthy and storage is writable. Otherwise a due schedule that cannot be delivered would wake the loop without end. The rule is a pure function, `next_schedule_wake`, so it can be tested.

6. **Timer deadline uses the earliest tick.** The loop previously woke at `ticks.values().next()`, which is the deadline of the smallest interval, not the earliest deadline. It now takes the minimum over all armed ticks, so an interval that was armed later cannot be delivered late. This changes tick timing and is part of this slice because the deadline expression is shared.

7. **Refusals carry no reason on the wire.** `CapError` has a code and no message, and adding a message field would change the result frame. The program sees `bad_args`. The reason (malformed id, duplicate id, full schedule, or horizon) is in the core validation error and in a debug log.

8. **Wildcard grants reach the new names.** `*` grants every allowed catalog name, as it has for each name added since the catalog existed. A world granted `*` can schedule unless its ceiling excludes `schedule.at`.

9. **The checkpoint must agree with its entries.** A checkpoint carries `schedules`, and `ScheduleSet` entries precede it, as `CapsCeiling` does. Restore refuses a journal where the two disagree. A journal this writer produces never disagrees, so the check only catches corruption.

## Bounds

| Bound                       | Value                                                                                                           | Where                                           |
| --------------------------- | --------------------------------------------------------------------------------------------------------------- | ----------------------------------------------- |
| Pending schedules per world | 64                                                                                                              | `MAX_PENDING_SCHEDULES`                         |
| Id length                   | 1 to 64 bytes, printable ASCII (0x20 to 0x7e)                                                                   | `validate_schedule_id`, `MAX_SCHEDULE_ID_BYTES` |
| Horizon                     | `at` at most 30 days past the host clock at request time; any past `at` is allowed and fires on the next wakeup | `MAX_SCHEDULE_HORIZON_DAYS`                     |
| Fires per wakeup            | 64, earliest first                                                                                              | `MAX_SCHEDULE_FIRES_PER_WAKEUP`                 |

A refused request leaves the pending set and the journal unchanged; the capability usage counter still records the attempt. A duplicate pending id is refused, and the existing schedule is never overwritten. The horizon and count checks run only on a new request. Restore does not re-check the horizon, because the request time is not journaled.

## Durability

- `ScheduleSet { id, at_ms }` is journaled when a request is accepted, before its result is delivered.
- `ScheduleCancelled { id }` is journaled before a cancel's result is returned.
- `ScheduleFired { id }` follows the fire's `System` entry (decision 2).
- A checkpoint writes one `ScheduleSet` per pending schedule, before the checkpoint entry, and carries `schedules` as `(id, at_ms)` pairs. The field is omitted when nothing is pending, so older checkpoints and their JSON are unchanged.
- The state entries do not advance `since_checkpoint`. Their `System` event does, through `deliver`, so compaction cadence still follows delivered events.

**Forward-only.** New entries mean an older binary cannot read a journal that holds them. This is the rollback limitation tracked in issue #227. The claim has a gap, found while writing this record. `FileJournal::open` treats an unparsable final line as a torn append and truncates it. So an older binary would silently drop a schedule entry that is the last line of a journal that was not compacted. A compacted journal is safe, because the `ScheduleSet` entries sit before the checkpoint and are not last. Closing the gap means `FileJournal::open` must refuse an unparsable line that ends in a newline. That changes the torn-tail rule for every entry type, so it is a follow-up and not part of this slice.

## Security

- Scheduling adds no authority. It needs the `schedule.at` grant and the ceiling must allow it. A fire delivers only to the program that already holds the world's state.
- The host clock decides when a fire happens, and that decision is journaled. A replay never reads the clock.
- Bounds cap memory and journal growth per world: 64 pending, 30 days ahead, and 64-byte ids.
- Each accepted or refused request still produces a journaled `result` event, so churn (schedule, cancel, schedule) costs journal space as `chat.say` does. Compaction trims it.
- A participant can cause a schedule only by sending input that makes the program ask. That input requires the `input` scope, and the request requires `join`, as for any effect a program takes on input.

## Follow-ups

- Recurring schedules and a `schedule.list` request, if an app needs them.
- Cross-world scheduling. Not designed.
- A re-requested pending id, after a restart or program install, gets `bad_args` instead of its original result. The program would see an error for a schedule that is pending with the same `at`. Idempotent re-request needs a design decision.
- Refuse unparsable complete journal lines in `FileJournal::open` (see Durability), to make rollback safe for every entry type. Issue #227.
- Host events never enter the event stream, so a joiner cannot see a fire. Issue #242.
- Whether `schedule.at` should need a scope beyond `join`, for example `chat`. Not decided here.

---

## 2026-10-17T12-10-00Z Update: implemented as designed, with the deviations listed

Shipped:

- `world_caps.rs`: `schedule.at` and `schedule.cancel` in the catalog, the bounds (`MAX_PENDING_SCHEDULES`, `MAX_SCHEDULE_ID_BYTES`, `MAX_SCHEDULE_HORIZON_DAYS`, `MAX_SCHEDULE_FIRES_PER_WAKEUP`), and `validate_schedule_id`.
- `world.rs`: the three journal entries, the checkpoint `schedules` field with its consistency check, `WorldCore::check_schedule`, the two request handlers, `fire_schedules`, `next_schedule_wake`, and the overdue-at-open set. Its tests are in `schedule_tests`.
- `world_store.rs`: a file-journal round trip for the new entries.
- `examples/roc-world/RocCap.roc`: `schedule_at_request` and `schedule_cancel_request`, checked by a compile-and-run test in `world_compile.rs`. The README lists the capabilities and builders.

Deviations from the brief, for review:

1. **Fire order.** The fire is accepted, then its `System` delivery is journaled, then `ScheduleFired` is journaled (decision 2). The brief's "journaled before delivery" does not describe the code: `deliver` and the participant input path both journal after the program accepts. The README and the `world_caps` module comment repeated the same wording and now match the code.
2. **Tick deadline.** The loop wakes at the minimum over armed ticks. The previous expression read the deadline of the smallest interval key (decision 6).
3. **Checkpoint consistency.** Restore refuses a checkpoint whose `schedules` disagree with the `ScheduleSet` entries before it (decision 9).
4. **Refusal reason.** The program sees `bad_args`. The reason goes to the core error and a debug log (decision 7).

Found while testing:

- `FileJournal::open` truncates an unparsable final line as a torn append, so an older binary can silently drop a trailing schedule entry from an uncompacted journal. Compaction protects only compacted journals (see Durability). The fix is a follow-up.
- A pending schedule survives a program install (decision 1). A re-requested pending id after an install or restart gets `bad_args` (see Follow-ups).
