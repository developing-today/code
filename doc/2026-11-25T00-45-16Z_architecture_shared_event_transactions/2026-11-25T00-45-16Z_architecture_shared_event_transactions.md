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
