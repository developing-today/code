# Design: TigerBeetle LedgerBackend binding — fallible failure contract

**Supersedes:** the "Status mapping and panic stance" section (and the
`wire_code` / `poc_code` panic notes) of
[`2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding/2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding.md`](2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding.md).
Everything else in that record still describes the binding.

**Context:** issue [#5950](https://github.com/developing-today/code/issues/5950)
(the clippy remainder of main-broken [#5612](https://github.com/developing-today/code/issues/5612))
denies `clippy::unwrap_used`, `clippy::expect_used` and `clippy::panic` in
production code and requires production diagnostics to be fixed rather than
allowed. The `tb_ledger` binding was the last production module on the
unmodified path: every transport failure, unmodelled status and lock
poisoning was an `expect`/`panic!`. Rewriting those sites into typed
errors changed the trait's failure contract, so this record states the new
contract and retires the old panic stance.

## 1. Two lookup forms on `LedgerBackend`

The trait now separates *critical* reads from *best-effort* reads:

| Method | Semantics |
| --- | --- |
| `try_lookup_transfer`, `try_lookup_account`, `try_account_balance`, `try_transfers_with_user_data_128` | Fallible. `Ok(None)` means "absent"; `Err` means "the backend could not answer". |
| `lookup_transfer`, `lookup_account`, `account_balance`, `transfers_with_user_data_128` | Defaulted best-effort wrappers over the fallible form. On `Err` they log at error level and return `None` / an empty vector. |

**Rule:** recovery, preflight and any path that must distinguish a missing
record from an unreachable ledger uses the `try_` form. The best-effort
form exists for tests and inspection, where "not found" and "backend
down" have the same effect on the caller. A backend outage must never be
read as "the transfer was never posted" — recovery that did so would
re-submit under fresh IDs. The in-memory backend returns `Ok(...)` from
every method; it has no failure channel.

## 2. Typed creation and wire-code errors

`AccountError` and `TransferError` gain three variants:

- `CodeOutOfRange` — the PoC `code` cannot be encoded (`u16::MAX`, whose
  `+1` shift wraps to `0`). Previously this was a documentation-level
  programming error and a panic. It is now a per-item rejection returned
  before any request reaches the server, so a batch containing it fails
  item by item rather than aborting the caller.
- `BackendFailure` — the request failed in transport, or the server
  returned a status count that does not match the batch. This is reported
  for every item in the affected batch.
- `UnmodelledStatus` — the server returned a status this PoC does not
  model. Previously a `panic!` naming the status; the status is now logged
  at error level and mapped to this variant.

`wire_code` returns `Option<u16>`, `poc_code` returns `Result<u16>`, and
`account_code`/`transfer_code` return `Result<u16>` propagated to callers.
A poisoned code-registry mutex is logged and degrades to decoding the wire
code; it no longer panics, because the registry is an optimisation over
`poc_code`, not the source of truth.

## 3. Idempotent linked-chain retry under failure

The linked-chain translation (first leg `Exists`, later legs
`LinkedEventFailed`, all stored legs verified identical → all legs
`Exists`) now verifies stored legs through `try_lookup_transfer`. A lookup
error fails verification for that leg, so the batch keeps its raw mapping
instead of claiming idempotent success on unverified state.

## 4. Consequences

- Callers that only need "best effort" keep the same `Option`/`Vec` API
  they had; no exhaustive match changes are needed outside the trait.
- Callers that can lose money (recovery, conversion preflight) must adopt
  the `try_` form. `recovery::reconcile` and the conversion executor do.
- The status mapping table itself is unchanged: the same statuses still
  map to the same PoC variants. Only unmodelled statuses and transport
  failures changed, from panic to typed error.

## 5. Verification

Unit tests in `currency_poc::tb_ledger::tests` pin the two wire-code
boundaries (`wire_code(U+FFFF) == None`, `poc_code(0).is_err()`). The
scenario suite in `currency_poc::tests` continues to run against the
in-memory backend by default and the live `TestCluster` when ignored
tests are enabled; the live cases cover the registry-preserving rejected
retry, exact zero `user_data_128` matching, and the linked-chain retry
above.

## Change log

- 2026-11-09: Created (PR for issue #5950) to record the fallible failure
  contract that replaced the panic stance in the 2026-11-03 binding
  design.
