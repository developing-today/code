# World lease handoff at the actor boundary

## Request and intent

Implement the first phase-3 slice of #3805/#2819: serialize DO-to-native and native-to-DO lease handoff through the existing world actor and journal. A lease transition must become durable before the actor changes its in-memory lease state. Once the DO hands the lease away, the DO actor must fail closed before executing mutations.

## Initial design

- Add one actor command and `WorldHandle` control method for a handoff, carrying an expected resulting epoch, destination holder, and term expiry. The actor derives the source from its current lease; the caller cannot select an arbitrary source or epoch transition.
- Permit transfers involving the DO holder only. The host/control plane remains responsible for authenticating the control request and for draining the source / replaying the destination through the journal boundary before it activates that destination.
- Append `JournalEntry::LeaseHandedOff` first. Apply the transition to `WorldCore` only after append succeeds. A failed append returns an error and leaves the lease unchanged (the existing journal failure latch remains fail-closed).
- Gate the DO actor's mutation admission on the current lease holder and expiry before any program effects. A lease held by a native executor therefore leaves this DO actor read-only until a durable handback. This slice does not enable a native executor or transport.

## Deliberate scope boundary

The actor-level API is a serialization and safety seam, not a complete spill deployment. This slice does not add spill trigger detection, native-process startup, authenticated native requests, journal position receipts/mirror synchronization, or gate every command by a caller-presented writer/epoch/operation ID. In particular, source draining and the native replay barrier remain responsibilities of the not-yet-wired host adapter; do not claim production spill is enabled.

## Verification intent

Add actor/journal tests for both handoff directions, durability-before-state-update on append failure, and refusal by the DO actor after handoff without executing a program mutation. Exercise lease expiry through the existing lease state-machine oracle. Use targeted `id-worlds` world/lease tests under `~/tools/fleet-build-lock`.

---

## 2026-11-12T20-10-00Z Discovery: autonomous program turns must also stop

The actor can execute program work outside an input command: `after_change` runs capability requests and arms timers, and the actor loop can deliver ticks, location watches, and schedules. Gating only the external mutation methods would therefore leave a DO execution path active after spill. The implementation also gates these program-turn paths and the next wake deadline on a live DO lease. An expired or native-held lease stops the DO actor; expiry recovery remains revoke/grant and is deliberately not replaced with a handoff.
