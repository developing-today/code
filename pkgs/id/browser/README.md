# id-browser — IndexedDB conformance prototype

Slice 9110-05 of #9110, tracked by #10213. An **evaluation-only**
IndexedDB adapter that exercises the declared browser-store
capability subset of the port contract (port-v1, contract version
2) against the shared conformance fixtures from slice 9110-04
(`pkgs/id/tests/fixtures/port-v1.json`). Nothing in the product
enables this adapter; whether IndexedDB is adopted is #9082's
decision, informed by the verdict this package produces.

## The adapter

`src/indexeddb_store.ts` implements a **request-batch** store over
raw IndexedDB:

- Every commit hands the adapter the full write set up front; the
  adapter places all requests — the operation-ID dedupe probe, the
  writes, and the operation-ID record — synchronously inside a
  single transaction that may span every object store of the
  database.
- No transaction handle ever escapes to the caller, so the adapter
  never advertises — and cannot express — a long-lived async
  transaction. (An IndexedDB transaction goes inactive when its
  task ends; the conformance suite demonstrates the engine's
  refusal — an `InvalidStateError` — for a request placed too
  late.)
- Commits are deduplicated by stable operation ID inside the same
  transaction as the writes; the commit outcome is definitive; a
  lost commit is retried as a new operation under the same ID.
  Capability negotiation refuses before any write, mirroring the
  Rust port contract (`id-core::port::negotiate`).

## Running the evaluation

```sh
npm test        # or: pnpm test / bun test — the script is plain node
```

The test needs a Chromium binary on `PATH` (or `CHROMIUM_BIN`
set); it drives headless Chromium over CDP using only Node
builtins, serves the adapter and driver type-stripped from their
TypeScript sources, and runs the shared fixture vectors twice
(determinism), checking every observation three ways: adapter,
fixture expectation, and an independent reference model. It also
runs the engine probes for transaction lifetime, upgrade, reopen,
and quota/eviction, and prints the verdict (engine, proposed
capability report, probe results) as JSON.

## Verdict shape

The proposed `indexeddb` capability report — per-batch
whole-database atomic scope, definitive commit ambiguity,
retry-as-new-operation, `store.scan` / `store.txn.multi-record` /
`store.txn.whole-database` as optional capabilities — is asserted
in the test and recorded as evidence on #10213. The fixture's own
`indexeddb` entry deliberately stays `reserved`: flipping it to
`active` is part of #9082's adoption decision.

Per-engine matrix: Chromium is the engine under evaluation here.
Firefox and WebKit rows are not run by this package and remain
open evidence for #9082.
