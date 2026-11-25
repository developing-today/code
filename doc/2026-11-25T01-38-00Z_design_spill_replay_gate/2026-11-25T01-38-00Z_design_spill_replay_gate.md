# Spill successor replay admission (#7572)

The spill v2 failover execution journals a successor grant and returns its
position. Checking the returned `resume_ready` helper is optional today, so
the spill admission boundary can admit a successor that has not replayed the
grant. Comparing the actor's lease watermark with that same lease's grant
position would not fix this: the authority has already applied the grant.

Require the successor's verified replay position at `evaluate_spill`, alongside
the authority's current lease. Derive the required position from the latest
grant or handoff for the current epoch, not a caller-selected barrier and not
the latest renewal/rejection audit. Refuse dispatch below that position even
when identity, epoch, expiry and residency pass. Bootstrap and unstamped legacy
ownership boundaries retain position zero semantics. The host must obtain the
watermark from verified replay, not an unauthenticated node assertion.

Verified when: the public admission gate refuses a matching successor below
the grant position, dispatch sends no copy, and admission succeeds exactly at
the position and above it. A later renewal must not move the ownership barrier.
The oracle is design #5787 section 1 step 4; property tests vary replay positions
around a stamped ownership boundary. Focused tests run through Fleet dispatch.

This is a bounded admission slice, not an end-to-end host controller. There are
currently no production callers of `evaluate_spill`; scheduling observations,
authenticated cursor delivery, native executor integration and real game-day
latency evidence remain outside this slice. Do not report full #7572 completion
based on these unit tests, or tune the 30s/10s defaults from synthetic timings.

## References

- [Spill v2 design](../2026-11-08_design_id5787.md)
- `crates/id-worlds/src/world/spill_gate.rs`
- `crates/id-worlds/src/world/spill_pool.rs`

---

## 2026-11-25T01-52-00Z Update: Admission implementation

The gate now derives the current epoch's grant/handoff position and returns
`SpillDeny::ReplayPending` below it. An acceptance case checks refusal and zero
dispatches below the boundary, and a property test varies handoff/grant boundaries
and cursors while renewing the term. The red checkpoint is commit
`31eeaf90bbab26a7085a4565919d8c2f565b2c1b`; remote execution is still pending
confirmation, not evidence of a passing test. The first dispatch failed checkout
because the abbreviated SHA was treated as a branch name; a full-SHA retry was
submitted. Synthetic unit-test timing is not a game-day measurement.
