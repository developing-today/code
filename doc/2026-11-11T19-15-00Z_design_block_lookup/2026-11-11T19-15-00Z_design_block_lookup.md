# Design: block-list lookups that can say "the lookup failed" (#8750)

## Intent

`BlockList::is_blocked` (crates/id-core/src/block_list.rs) returned only
`bool`. Both store implementations erased lookup failures to `false` —
`EventStore` with `unwrap_or(false)`, `SharedEventStore` with a second
erasure for poisoned locks — and every enforcement consumer
(`AbuseEvaluator`, `PullGate`, the blob gate) treated that `false` as a
successful "not blocked". A corrupt block row, a transient storage
failure, or a poisoned lock therefore silently allowed connections and
pulls, indistinguishable from an ordinary unblocked result, with no
operator diagnostic anywhere.

The ask from #8750: failure observability and an explicit per-surface
indeterminate-result policy. Not a unilateral change to fail-closed —
the documented fail-open default is an availability choice and stays.

## Approach

- **One distinguishing outcome type at the trait boundary.**
  `BlockLookup` = `Blocked` | `Unblocked` | `Indeterminate(BlockLookupFailure)`,
  added to the `BlockList` trait as `lookup()` with `peer_lookup` /
  `topic_tag_lookup` / `ip_lookup` helpers mirroring the bool helpers.
  The default `lookup()` derives from `is_blocked`, so implementations
  that cannot report failures keep compiling and keep their semantics;
  only the two real stores override it.
- **Failure as categories, never storage details.**
  `BlockLookupFailure` = `Corrupt` | `Storage` | `Unavailable`. The
  category is all that crosses the seam: a remote peer must never learn
  store internals (file layout, lock state, error text) from an
  enforcement decision. The full store error goes to the operator log
  only, via `tracing::warn!` at the implementation that saw it.
- **The bool seam cannot diverge.** Each store's `is_blocked` is now
  `lookup(...) == Blocked`, one code path for both answers. The bool
  keeps its documented meaning: indeterminate fails open.
- **Store mapping.** `EventStore`: `StoreError::Corrupt` -> `Corrupt`,
  any other store error -> `Storage`. `SharedEventStore`: the same for
  errors from the backend, and a poisoned lock (or any other
  trait-layer failure to run the read) -> `Unavailable`.
- **Operator surfaces.** `LocalEvents::lookup_block` exposes the
  distinguishing answer next to `is_blocked`, and `PullGate` gains
  `peer_lookup` / `topic_tag_lookup` returning `Option<BlockLookup>`
  (`None` = no list attached, as before).

## Decisions and rationale

- **Fail-open at every enforcement seam, stated explicitly.** The
  connection seam (`AbuseEvaluator::open_connection_inner`), the pull
  gate (`PullGate::admits`), blob serving (`decide_with_blocks`), and
  the log-sync topic check each match on the lookup: `Blocked`
  refuses; `Indeterminate` logs a `tracing::warn!` diagnostic naming
  the failure category and proceeds; `Unblocked` proceeds. Rationale:
  a block store that cannot answer must not become a silent,
  fleet-wide connection/pull refusal (availability), but the failure
  must stop posing as a clean "not blocked" (observability). A future
  seam that needs fail-closed can choose it on the same type without
  another API change.
- **No change to definite-answer semantics.** Active, absent, and
  expired lookups behave exactly as before; the pre-existing block
  tests pass unmodified, which is the regression oracle.
- **Diagnostics at two layers.** The store implementations log the
  failure with the underlying error; each enforcement seam logs its
  own policy application. Either layer alone would leave a gap (a
  failure with no decision context, or a decision with no cause).

## Tests (the oracle)

A failed lookup must be distinguishable from an ordinary unblocked
result, per surface:

- id-events: a raw undecodable block row written into the store yields
  `Indeterminate(Corrupt)`; a healthy row yields `Blocked`; an absent
  one `Unblocked`; the bool seam still reads `false` for the corrupt
  row.
- id-store: `SharedEventStore` distinguishes `Blocked` / `Unblocked`,
  and a poisoned lock yields `Indeterminate(Unavailable)`.
- id-core abuse: an always-indeterminate list fails open at the
  connection seam; definite blocks still refuse.
- id-events pull policy: the gate surfaces `Some(Indeterminate(..))`,
  the bool seam reads false, and `admits` follows the stated policy.

---

## 2026-11-11T19-15-00Z Update: record formalized during the implementing PR

The design above was settled in the #8750 issue body and the queue
task packet before implementation (the packet's acceptance criteria
are the test oracle listed here). This file formalizes that record in
the repository's prescribed location; it was added to PR #8761 after
the Codex review (finding on block_list.rs) pointed out the missing
dated record. No design content changed in response — only its
location.
