# Incremental world-set receiver sessions

Status: implemented as the receiver-side foundation for developing-today/code#13747 slice 1. This does not yet produce or transport incremental export files; it establishes the safe commit seam the producer will use.

## Decision

An incremental update is a signed tail batch over the exact journal chain already committed at the recipient. A recipient must not accept a base cursor supplied by the sender. `DurableReceiver::open` recomputes the committed cursor from the local journal bytes under the configured world id and pinned source key; `begin_incremental_session` seeds a fresh nonce-bound tracker from only that verified cursor. The new session's frame-digest chain starts at zero, while the journal cursor continues from the committed ordinal and digest.

The first frame in an incremental session may therefore be a `Tail`, never a `Snapshot` or `Terminal`. Tail payload bytes remain staged and invisible until the signed terminal commits the batch. Existing append rollback/fsync behavior applies. Missing base, stale/forked cursor, wrong source, wrong nonce, bad signature, or missing terminal cannot advance the committed cursor or journal.

Reset/first-sync sessions remain snapshot-first. A snapshot continues to replace the journal atomically; incremental sessions append to it. Both paths use the existing signed replication envelope and frame encoding rather than introducing a second serialization format.

## Invariants and evidence

- A session base is derived from the recipient's verified disk state, never from untrusted incremental metadata.
- The chain cursor is continuous across sessions; each session nonce and frame digest chain is fresh.
- Partial tails do not modify the committed journal; the terminal frame is the commit point.
- Existing snapshot sets remain self-contained P3 exports. The follow-on producer work must capture a suffix at a caller-known cursor, bind it to the current signed chain, and pair it with the existing whole-set retention policy in `host_snapshots.rs`.
- Follow-on issue work still needs to choose and document the off-host file-copy workflow and the replica-versus-backup contract. This PR is not evidence that scheduled incremental export or external transport is complete.

The new receiver oracle is `cargo test -p id-worlds --lib incremental_session`: a committed snapshot followed by a signed tail-only session appends the exact entry bytes and advances to the expected cursor; a receiver with no committed base refuses incremental start.
