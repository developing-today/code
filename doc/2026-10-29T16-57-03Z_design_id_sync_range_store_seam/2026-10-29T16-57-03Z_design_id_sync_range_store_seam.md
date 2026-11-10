# Wiring `SyncMethod::Range` under the `/id-sync/1` handshake: the per-call lock store seam

Related: [`#179`](https://github.com/developing-today/code/issues/179),
[`2026-10-29T16-21-00Z_design_id_sync_alpn_split`](../2026-10-29T16-21-00Z_design_id_sync_alpn_split/2026-10-29T16-21-00Z_design_id_sync_alpn_split.md)
(the ALPN split this change completes).

Dated 2026-10-29. This record says what was known when it was written. The
code it describes is the state of `queue/2970-179-sync-handshake` after this
change.

## Intent

The ALPN split record left one acceptance criterion of #179 open: a `Range`
request agreed under the `Request`/`Answer` handshake on `/id-sync/1` must
actually run range sync (`log_sync::run_agreed` returned
`SyncError::NotWired`), with tests for both methods against one peer/store.
That record named the blocker as a store seam: `log_sync` runs against
`SharedEventStore = Arc<RwLock<EventStore>>` (a `std::sync::RwLock`), while
the range session in `event_store::sync` took `&mut EventStore`. It listed
three candidate seams as an open question. This record answers it.

## Decision

**Per-call locking.** The range session is generalised over a small
store-access trait, `event_store::sync::RangeStore`, with exactly the three
operations a session performs:

- `sync_entries()` — the sync set, once, at the start;
- `sync_bytes(id)` — one event's bytes, per offer;
- `ingest_events(events, now_ms)` — one batch, per flush.

`EventStore` implements it by delegating to its inherent methods, so
`serve`'s path (`sync_over`) is behaviourally unchanged. `SharedEventStore`
implements it by taking its existing `RwLock` per call — read for the two
queries, write for one ingest batch — and never across an await.

This works because the range session never actually needed the store across
an await: `run` already wraps the store in a `std::sync::Mutex` and locks it
per step (`lock()`), for the same reason. The trait makes that discipline
the interface instead of an internal accident of the `&mut` version.

Two supporting changes:

- The handshake in `log_sync` (`sync_with` / `answer`) now runs on the raw
  stream pair, and only the `Log` arm wraps the streams in `Frames`. The
  handshake is one frame each way, strictly in turn, so the reader task
  `Frames` exists for (a read must never be cancelled mid-frame while
  `LogSync` selects between reading and writing) is not needed for it; the
  `Range` arm needs the raw streams because a range session sends and
  receives at once through its own halves.
- `event_store::sync::run` is split into `run` (opens streams, checks the
  pull gate — unchanged) and `run_session` (the session itself, on an open
  stream pair, generic over `RangeStore`). `log_sync` calls the new
  `sync_on_streams` entry. A `Range` session under the handshake therefore
  gets its gate check from `log_sync::answer`, exactly as a `Log` session
  does, and its deadline from the caller, as `Log` does.

`sync_method` gains `LogAndRange`, a `SyncSupport` that runs both methods;
`LogSyncOnly` is unchanged, so a responder that has not opted in still
refuses `Range` and the initiator still errors — the refusal semantics #179
requires stay tested.

## Alternatives considered

1. **Switch `SharedEventStore` to `tokio::sync::RwLock` and hold a write
   guard for the session.** The guard is held across network awaits for up
   to the 60-second session deadline: every other user of the store (other
   sessions, the web layer's reads) stalls behind one peer's silence, and
   the `LogSync` ingest worker — a blocking thread using the synchronous
   lock API — would have to be restructured around it.
2. **Give the session an owned `EventStore` handle.** `EventStore` owns a
   redb database; a second handle on the same file is not available, and
   splitting ownership out of `SharedEventStore` is a wider change than the
   dispatch it unblocks.
3. **Run the range session on a blocking thread holding the write guard,
   with a stream bridge.** Works, but the bridge (two frame channels plus
   back-pressure) is new machinery for a property the per-call seam already
   has, and the guard still blocks other store users for the session's life.

## Consequences

- `/id-sync/1` now carries exactly one session shape with both methods
  wired: `Request` → `Agree(method)`/`Refuse` → the agreed method. A first
  frame that is not a valid `Request` fails postcard decoding and is
  refused as a frame error (tested), never reinterpreted.
- Range sync's set is the store's whole set, as it has always been on
  `/id-range-sync/1`: the range protocol has no topic subset, and this
  change does not add one. The handshake's `topic` therefore names the
  session for the `Log` method; a `Range` session reconciles the full
  stores. Callers choosing methods with `choose()` already treat range as
  the whole-store path (its cost model is the difference of the stores).
- `serve`'s range sessions stay on `/id-range-sync/1`. The split record's
  second open question — retire that ALPN later, or keep two ALPNs with one
  shape each — is unchanged by this change and still open.
- `SyncError::NotWired` is removed: no agreed method is unwired now.
