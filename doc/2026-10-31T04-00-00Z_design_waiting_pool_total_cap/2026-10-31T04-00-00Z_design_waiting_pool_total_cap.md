# Waiting Pool Total Cap — Bounding Memory and Disk Waiting Rows Against Sybil Floods

- Date: 2026-10-31
- Status: **implemented**
- Issue: [#1567](https://github.com/developing-today/code/issues/1567)
- Pull Request: [#2940](https://github.com/developing-today/code/pull/2940)

## 1. Intent and Background

In `pkgs/id`, an event waits in `WaitingPool` when a dependency has not yet arrived:
a backlink, an `after` hash, or an auth dependency. The pool holds the event in
memory and mirrors it to durable on-disk storage (`redb` `waiting_rows` table)
until its dependencies arrive or it expires.

Prior to this change, `WaitingLimits` enforced only two caps:
1. `per_author`: maximum waiting events for a single author (default: 256)
2. `max_age_ms`: maximum age before an unreleased event is dropped (default: 7 days)

While the module documentation claimed that these two caps bounded what a peer could
make a node hold, author keys are self-certifying 32-byte Ed25519 public keys. A peer
can generate arbitrary numbers of author keys at zero cost. Consequently, an attacker
could flood a node with waiting events under unique author identities, causing
unbounded memory growth and unbounded on-disk storage in `redb` across the 7-day
window.

## 2. Total Cap Specification

To establish a true resource bound, `WaitingLimits` gains a pool-wide total cap:

- **Constant**: `pub const DEFAULT_TOTAL: usize = 32 * DEFAULT_PER_AUTHOR;` (8,192 events).
- **Rationale**: 8,192 events accommodates 32 concurrent authors fully saturating
  their 256-event per-author quotas, providing ample headroom for legitimate multi-author
  activity while strictly bounding memory footprint and on-disk waiting rows.
- **Configurability**: A world or embedder can adjust the cap via builder method:
  - `WaitingLimits::with_total(self, total: usize) -> Self`
  - `WaitingLimits::total(self) -> usize`
  - `WaitingLimits::new(per_author, max_age_ms)` initializes with `total: DEFAULT_TOTAL`.

## 3. Admission and Refusal Policy

Admission is gated in `WaitingPool::hold()`:

```rust
if self.len() >= self.limits.total() {
    return Err(WaitRefusal::PoolFull);
}
if self.waiting_count(author) >= self.limits.per_author() {
    return Err(WaitRefusal::AuthorFull);
}
```

- **Priority**: The total cap is checked **first**, prior to the per-author cap.
  When the pool is saturated, all incoming waiting events are refused regardless of author.
- **Refusal Distinction**: `WaitRefusal::PoolFull` is introduced alongside `WaitRefusal::AuthorFull`.
  This allows internal callers and telemetry to differentiate between single-author quota
  exhaustion and pool-wide saturation (e.g. Sybil attacks).
- **Ingest Handling**: Ingest and sync paths treat both refusals identically: the event
  is not acknowledged to the peer, leaving sync to retry once missing dependencies have
  arrived.

## 4. Restore-on-Open and Excess Row Pruning

When opening an existing `EventStore`, waiting events stored in `redb` are restored into
`WaitingPool` in traversal order:

- If a database was written under a higher cap (or an earlier un-capped release) and is
  subsequently opened under a lower total cap, `pool.restore()` returns
  `Err(WaitRefusal::PoolFull)` once the cap is reached.
- `EventStore::open` handles both `WaitRefusal::AuthorFull` and `WaitRefusal::PoolFull`
  by durably deleting the excess row from `redb` (`waiting_rows::delete(&txn, &row.id)?`).
- Earlier arrivals are preserved; excess rows are pruned cleanly without wedging the store,
  ensuring `check_invariants()` continues to hold.
