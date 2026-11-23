# id-browser — IndexedDB request-batch adapter

The browser record-store profile is adopted for the checked request-batch subset in #9082. Chromium runs the shared port-v1 fixture (contract version 2) against real IndexedDB. Applications opt in explicitly.

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

## Supported scope

The shared fixture names the active indexeddb profile and gives each
divergent step its own expected observation. The suite checks the adapter
descriptor against that report, compares the two complete traces and separately
forces a transaction abort. Writes across stores and the operation-ID record
must roll back together. Retrying the same ID with valid values then commits.

The measured persistence scope is a database close/reopen and a version upgrade
in Chromium. The origin may lose its storage through eviction or explicit
clearing. The suite reports the permission returned by the browser. It supplies
no guarantee of power-loss durability or protection against quota pressure.
Firefox, WebKit and a user-facing store selection require their own evidence.
