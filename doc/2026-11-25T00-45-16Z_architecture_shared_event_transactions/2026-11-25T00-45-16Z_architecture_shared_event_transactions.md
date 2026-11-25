# Shared event transactions for native and tenant stores (Refs #12801)

## Request and intent

Continue E6 by making the Turso `TenantStore` an `EventStoreBackend` without
creating a hosted-only ingest path. Event verification, fork decisions,
waiting-event transitions, resolution, and synchronization semantics must
remain shared. Native redb remains the behavioral oracle.

The existing `RowRead`/`RowWrite` adapters cover persistence helpers and
resolution reads, but `id-events` still opens and manipulates redb transactions
directly across ingest, recovery, compaction, and sync. A row adapter alone
therefore cannot implement the full backend contract.

## Initial design

Add the smallest transaction-driver boundary that lets the existing event
semantics execute against either storage engine. The native implementation
continues to own redb transaction creation and commit; the tenant implementation
owns its Turso transaction lifecycle. Shared event-store logic receives a
transaction-scoped storage interface and remains the sole implementation of
protocol decisions. Row-level operations stay backend adapters and expose owned
values so redb guards or Turso session details do not escape.

The exact driver shape is intentionally constrained by an inventory of current
redb operations before implementation. It must preserve one-transaction batch
atomicity, rollback/error behavior, and read-your-writes semantics. Avoid
introducing a parallel `TenantEventStore` policy implementation or caller-side
backend branching. If a direct generic transaction context is too invasive, an
equivalent boundary is acceptable only if the same semantic functions run for
both backends.

## Alternatives considered

- **Port ingest and sync method-by-method to `TenantStore`:** rejected because
  it duplicates behavior and can drift from the native oracle.
- **Keep only the row helper seam:** insufficient; current event logic still
  depends on redb transaction/table APIs outside the shared helpers.
- **Generic event-store/transaction context:** initial direction because it
  can preserve one semantic implementation while allowing backend-owned
  transaction lifecycle. Its smallest viable operation set is to be confirmed
  from the direct-redb inventory before code is changed.

## Observable acceptance and evidence

Verified when all of the following are observable:

1. The tenant adapter implements `id_store::store_traits::EventStoreBackend`.
2. One shared conformance suite passes against the native and tenant backend
   implementations, with the native implementation serving as the oracle.
3. Ingest, batching, fork handling, waiting rows, resolution and sync are not
   reimplemented in tenant-specific callers or policy code.
4. Transaction failures preserve rollback and typed error behavior; no partial
   batch becomes visible after a rejected/failed operation.

Evidence will be the same remotely dispatched `id-store` conformance command
run with both backend fixtures, plus targeted native/tenant parity tests for
transactional paths and the relevant crate checks. Required laptop evidence is
tracked separately; the laptop was unreachable at the start of this continuation
(2026-11-25).

## Current status

This is a proposal for the next implementation slice, not proof of E6
completion. At creation, the pushed branch is
`code-f/23653-tenant-backend-seam` at `1b9644b712b`; remote test and check runs
45519717589 and 45519720346 are in progress. PR #14642 remains draft and issue
#12801 remains open.

---

## 2026-11-25T00:59:52Z Update: transaction-driver inventory

The row seam now includes shared waiting-row writes and resolution reads, and
the latest pushed head is `4efb0bf4666`. A direct-redb inventory confirms that
the remaining work spans ingest/recovery, compaction, lifecycle, and query
modules; a row-only driver would not let the tenant adapter implement the
backend contract. The next seam therefore needs transaction-scoped reads and
writes plus backend-owned begin/commit/abort, while retaining one shared
semantic implementation. Remote verification #45521245212 is still running;
the laptop and Codespace are unreachable. E6 acceptance remains unmet, PR
#14642 remains draft, and #12801 remains open.

## 2026-11-25T01:10:00Z Update: backend blockers and bounded scans

The focused transaction-driver test was dispatched as
`developing-today-forks/fleet#45525854255`. An earlier dispatch exists as
`#45521245212`, but its status lookup returned HTTP 403. The PR still targets
`19335d25195` and is draft.

A fresh code map confirms the transaction driver alone does not make
`EventStoreBackend` portable: `EventStore` owns a concrete redb `Database`,
most ingest helpers take `&WriteTransaction`, and tenant row reads currently
require mutable access. Also, `RowRead::scan_all` lets the tenant implementation
materialize an entire table with an effectively unbounded limit, which conflicts
with the native recovery path's bounded-memory behavior. The next small storage
seam is a cursor-based `scan_page(table, after_key, limit)` operation. Existing
`scan_all` remains a convenience API; consumers with large tables must move to
pages before claiming bounded-memory parity. The backend ownership/read-session
shape remains a separate architectural decision and blocks full parity.

The laptop and Codespace remain unreachable. This update does not change E6
acceptance: the tenant backend and shared native/tenant conformance suite are
still unimplemented; PR #14642 and issue #12801 remain open.

## Bounded recovery port — 2026-11-25

Device-operation recovery will use the shared row seam, retaining the native
entry point as an adapter. It pages event rows in exclusive-key order, looks up
each retained body on demand, and writes recovered operations in the same
transaction. Only one page of event bytes and the capped waiting rows are held.
The recovery marker is written last; errors still leave the enclosing
transaction responsible for rollback. This preserves the native recovery policy
without adding a tenant-only startup policy. A shared SQLite/redb helper test
must cross a page boundary and compare recovery results and persisted bytes.

Plain-Cargo verification of `727de7ab0d4` was dispatched as
`developing-today-forks/fleet#45527102123`; it reached compilation and failed:
redb 5's `range` takes `impl KeyRange` rather than an explicit generic argument.
Removed the unsupported turbofish in all three native paging adapters. Earlier plain
Cargo run `45519717589` reached compilation (rather than failing Nix setup),
but predates the compiler fixes and is not evidence for the current head.

## Next ownership seam — 2026-11-25

Selected direction after read-only review: `EventStore<B = Database>`, with a
backend trait exposing closure-scoped `read(&self, op)` and `write(&self, op)`
operations over row handles. The write closure returns the existing change bit;
the backend owns begin/commit/abort through the shared transaction runner.
Native constructors and file-maintenance methods remain specialized to
`EventStore<Database>`; shared ingest, waiting state and generation/cache handling
stay in the existing event-store implementation rather than a second outer store.

The tenant adapter owns `Mutex<TenantStore<T>>`. It holds the lock for the entire
closure, with a read transaction spanning all pages and multi-table reads, not
one independent snapshot per SQL request. Closure scope avoids a returned guard
that must borrow a tenant session it also owns. It preserves `EventStoreBackend`
reads through `&self` and the current `Send + Sync` shared handle contract.
An enum adds transport erasure/variant dispatch without reducing helper-porting
work; a separate core with two outer orchestrators risks policy divergence.

Before making ingest generic, the next reviewable slice must prove native/tenant
read-snapshot parity, read cleanup and transaction rollback under this ownership
seam. Then port the existing `transact`/`reload` and ingest helpers, not a new
hosted ingest path. An uncertain COMMIT retires the baton; only a fresh session
and waiting-pool/cache recovery may restore usability, with no blind retry.
A mutex is only local exclusion: authoritative single-writer ownership per
tenant must also be enforced before claiming restart/concurrency parity.

The earlier lifecycle test also needs the allowlisted `meta` table rather than
the invented `driver_rows` name, and explicit error-only closure result types.
These are fixture/compiler corrections, not a relaxation of table validation.

Inspection of the previously uncompiled native transaction-driver call site
also found a mismatched shared borrow for `Mirror<WriteTransaction>` and a tail
`?` that produced a tuple instead of the closure's `Result`. The native row
borrow is now mutable, `attempt` explicitly reborrows its underlying transaction,
and the operation result is returned directly. Read-only review agrees this
keeps commit/abort, error-drop rollback, pool reload and generation behavior.
Runtime evidence for these corrections remains required.
