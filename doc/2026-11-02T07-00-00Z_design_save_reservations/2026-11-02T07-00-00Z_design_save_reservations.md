# Save reservations: in-flight exclusion for same-file saves

Related: issue #2466 (duplicate same-file saves racing to the filename tag)
and PR #4193, which implements this design in `pkgs/id/src/web/`.

## Intent

The web save handler rate-limits saves per filename with a cooldown
(`SaveRateLimiter::check` / `record`), but the cooldown only sees
_completed_ saves. Two overlapping saves of the same file — two
collaborators, or one client retrying while its first attempt is still
running — both passed the check and raced to write the filename tag,
so the later finisher silently overwrote the earlier one's snapshot
ordering. The limiter needed to refuse a second save while one is in
flight, not just after one has finished.

## Decision

1. **Reservation at the boundary.** `SaveRateLimiter::acquire(name)`
   combines the existing cooldown check with an in-flight reservation,
   and re-validates the cooldown after taking the reservation: the
   check and the reservation use different locks, but `complete()`
   records the save before `Drop` releases the reservation (decision
   2), so any save completing between an `acquire`'s check and its
   reservation is observed by the re-validation and refused, with the
   just-taken reservation released — the admission decision is atomic
   with respect to record + release (#9587). If the name is already
   reserved, `acquire` returns `Err(cooldown)` and the handler refuses
   with the same `429 / "Try again in Ns"` contract clients already
   handle for the cooldown, so no client change is needed to understand
   the refusal — clients schedule their normal rate-limit retry.
2. **RAII permit.** On success `acquire` returns a `SavePermit` that
   holds the reservation until it is dropped. The handler holds the
   permit for its whole run:
   - `SavePermit::complete()` (success path only) records the save for
     the cooldown, then the reservation is released by `Drop` as the
     permit goes out of scope.
   - Dropping without `complete()` — every early error return in the
     handler — releases the reservation and records nothing, so a
     failed save strands neither the filename nor a cooldown it did
     not earn.
3. **Synchronous std mutex for the in-flight set, deliberately.** The
   set is touched only for O(1) insert/remove and is never held across
   an await, and `Drop` cannot await — releasing the reservation from
   `Drop` requires a synchronous lock. The cooldown map keeps its
   `tokio::sync::Mutex` because it is only used from async contexts.
   Poisoning is recovered with `into_inner()` on the acquire path
   and on the permit's `Drop` path alike (#9587): a poisoned set must
   not wedge saves for a filename forever, and a `Drop` that skipped
   the release on a poisoned lock would wedge it silently.
4. **Client retry survives the winner's broadcast.** The refusal makes
   the losing client schedule a rate-limit retry; the winning save's
   `notify_new_version` broadcast must not cancel that retry, because
   the broadcast version by construction does not contain the rejected
   snapshot. `AutoSaveManager.onNewVersion()` therefore leaves a
   pending rate-limit retry (and the dirty state) untouched; the retry
   persists the newer local snapshot when it fires. If the edits had
   in fact already propagated, the retry is one redundant save — the
   safe direction.

## Alternatives considered

- **Serialize instead of refuse** (queue the second save behind the
  first): saves carry whole-document snapshots, so a queued stale
  snapshot landing after the fresher one is exactly the overwrite
  being fixed; the loser must re-read and re-decide, which is what the
  client's retry after refusal already does.
- **Per-file async mutex held across the handler:** blocks rather than
  refuses (same staleness problem), and cleanup on cancellation is
  implicit in ways the explicit permit makes auditable.

## Consequences

- One in-flight save per filename per server process; a second is a
  429 with the cooldown duration, indistinguishable on the wire from a
  cooldown refusal.
- The reservation lifecycle is: `acquire` → handler runs → `complete`
  (record + release) or drop (release only). Tests in
  `pkgs/id/src/web/mod.rs` pin all three outcomes plus concurrent
  acquires admitting exactly one save; `pkgs/id/web/src/auto-save.test.ts`
  pins the client retry surviving a NewVersion broadcast.
