# One `/id-sync/1` handshake, and why serve's range flights moved to
# `/id-range-sync/1`

Related: [`#179`](https://github.com/developing-today/code/issues/179) (the
issue this record answers), [`2026-10-07T00-20-00Z_architecture_roc_worlds`](../2026-10-07T00-20-00Z_architecture_roc_worlds/2026-10-07T00-20-00Z_architecture_roc_worlds.md)
(the worlds architecture whose event store `id serve` syncs).

Dated 2026-10-29. This record says what was known when it was written. The
code it describes is the state of `main` at `5a34fa70` plus the change on
`queue/2970-179-sync-handshake`.

## Intent

`/id-sync/1` was meant to carry **one** session shape: the first frame is a
`Request { topic, method }`, the peer answers `Agree(method)` or `Refuse`, and
the agreed method runs on the same bidirectional stream. Range sync was
intended to join as a second method of that session (`sync_method.rs`:
`choose` already returns `SyncMethod::Range` for a store that is behind on
ranges, not just logs), so a peer never has to guess which shape an existing
stream is speaking.

At the time of writing the unification is half done:

- `log_sync.rs` speaks the handshake, with refusal semantics implemented and
  tested: the responder answers `Refuse` for a method its `SyncSupport` does
  not allow, and the initiator errors when the `Answer` names a method other
  than the one it asked for (`SyncError::Mismatch`).
- `run_agreed` still dispatches only `SyncMethod::Log`;
  `SyncMethod::Range` returns `SyncError::NotWired`.
- `commands/serve.rs` registers the node's own `LocalEvents` store for range
  flights. That acceptor used to accept the *same* `/id-sync/1` ALPN but
  started a range flight immediately, with no `Request`/`Answer` first frame
  — a second, unlabelled session shape on one ALPN (#792).

Two shapes on one ALPN is the defect #179 names: a peer opening with the shape
the acceptor does not expect is misread rather than refused, and a `Range`
request under the handshake is a dead end.

## Decision

For this change: **serve's range flights move to their own ALPN,
`/id-range-sync/1`** (`event_store/sync.rs::ALPN`), and the handshake ALPN
stays `/id-sync/1` for method sessions. The split is documented in both
modules (`log_sync.rs` module docs, `event_sync.rs` module docs,
`event_store/sync.rs::ALPN`).

Reason: each ALPN now names exactly one first frame, so a peer that opens the
wrong one is refused by the protocol's own framing instead of having its
bytes parsed as a different message type. That is the property #179 asks for
("do not leave two silent shapes on `/id-sync/1`"), and it is reachable
without a store-ownership redesign (below).

Alternatives considered:

1. **Bring serve's acceptor under the `Request`/`Answer` first frame.** This
   is the eventual shape the design wants, but it forces the two stores into
   one session type. `log_sync` runs against `SharedEventStore`
   (`Arc<RwLock<EventStore>>`), while a range session needs `&mut EventStore`
   for its whole life, because events are applied while the data phase is
   still running (`event_store/sync.rs::run`). Taking the write guard for a
   session would have to happen across awaits, so the session future stops
   being `Send` unless the lock becomes `tokio::sync::Mutex` or the session
   holds an owned store handle. `id serve` does not use `SharedEventStore` at
   all: it shares `Arc<Mutex<LocalEvents>>` between the web layer and the
   handler, and `LocalEvents` owns its `EventStore`. That is a locking and
   ownership change to the sync seams, not a dispatch arm, and it deserves
   its own dated record and tests.
2. **Leave both shapes on `/id-sync/1` and sniff the first frame.** Gives up
   the refusal property: a mis-typed frame is still parsed as *something*, and
   the acceptor has to guess.
3. **Wire `Range` under the handshake now and leave serve on the old ALPN.**
   This is the direction the design intends, but it is not implementable in
   this change without decision 1's store seam, so it would leave `NotWired`
   in place anyway and claim an acceptance criterion it does not meet.

## Consequences

- A peer that opens `/id-sync/1` and writes a range flight as its first frame
  is now refused as a malformed `Request` (postcard decode failure on the
  first frame), not served as a range session. An older peer that only knows
  the shared ALPN therefore cannot range-sync with this node through
  `/id-sync/1`; it must dial `/id-range-sync/1`. This is a wire-visible break
  between peers of different versions and is acceptable pre-1.0: the two
  shapes could not interoperate correctly before either.
- Peers that both run this change keep working: serve's store syncs through
  `/id-range-sync/1`, and method sessions run on `/id-sync/1`.
- `SyncMethod::Range` under `/id-sync/1` remains unimplemented
  (`SyncError::NotWired`). #179's remaining acceptance criterion — a Range
  request agreed under the handshake runs range sync for the topic, with tests
  for both methods against one peer/store — is **not** met by this change and
  needs decision 1 above: one store seam that can lend `&mut EventStore` to a
  session running under `Request`/`Answer`.

## Open questions

- Which store seam should carry `Range` under the handshake: switch
  `SharedEventStore` to `tokio::sync::Mutex` (cheap, but `log_sync`'s blocking
  ingest worker must change with it), give the session an owned
  `EventStore` handle (cleaner, wider change), or run the range session on a
  blocking thread that owns the guard (keeps the session future `Send`, but
  needs a stream bridge)?
- Once `Range` is wired, should serve's `/id-range-sync/1` acceptor be retired
  in favour of the handshake acceptor, or kept as the low-level path that
  `serve` uses while method sessions stay for `id sync`? Retiring it later is
  another wire break; keeping it means two ALPNs forever, each with one shape.
