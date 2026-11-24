# Adopt the IndexedDB request-batch record-store profile

Decision for [#9082](https://github.com/developing-today/code/issues/9082),
following the [#10213 evidence](https://github.com/developing-today/code/issues/10213#issuecomment-7099619478).
The active indexeddb entry in pkgs/id/tests/fixtures/port-v1.json describes
the TypeScript adapter in pkgs/id/browser. Chromium is the verified engine.
Applications still choose the adapter explicitly.

## Contract and authority

The shared port supports keyed puts, reads, deletes, ordered scans,
capability negotiation and atomic request batches. A batch supplies its
entire write set up front. The adapter queues writes and its operation-ID
marker inside one database transaction. An observed local abort rolls
them all back. The caller can retry using the same operation ID. The
adapter supplies no transaction handle across a caller's asynchronous work.
Scan and multi-record/whole-database transactions are optional capabilities;
the base contract neither requires a watch stream nor supplies a remote replica.

A browser origin owns its local IndexedDB records. The tested durability
scope covers closing/reopening the database and upgrading its schema.
Storage estimates and a persistence-permission query report the current
browser policy. Neither survives an explicit origin wipe by contract.
Quota, eviction and crash/power-loss guarantees need separate evidence.
Firefox and WebKit have no result in the Chromium suite. A durable backup
belongs on another eligible store under the user's replication policy.

The native redb profile keeps local authority and a definitive commit
outcome. The Turso/libSQL-over-HTTP profile keeps hosted authority,
supports whole-database batches only inside an explicit interactive
session, and requires readback after an ambiguous outcome. Adopting the
browser profile does not change either one. Keep signed events, accepted
lineage and scopes at the application boundary. A store's success receipt
alone cannot authorize a new signed graph operation.

## Relationship to the hosted edition

[#5681](https://github.com/developing-today/code/issues/5681) and
[#5863](https://github.com/developing-today/code/issues/5863) concern the
serverless edition and its storage contract. The Worker/Turso deployment and
the origin-local browser adapter have different custody and failure domains.
Applications use the port's negotiated operations in either case. A native
redb-to-hosted migration should pin a source frontier, export the authorized
records and event IDs, import idempotently, independently verify identities,
counts and replayed state, then explicitly switch authority. An interrupted
import or ambiguous HTTP outcome requires reconciliation before cutover.
This describes the migration boundary and grants no live deployment authority.
The browser adapter uses platform IndexedDB via TypeScript. Selecting a Rust
WASM database runtime would be a distinct implementation.

## Evidence and outstanding gates

- [#10160](https://github.com/developing-today/code/issues/10160) owns the contract, [#10174](https://github.com/developing-today/code/issues/10174) the native/hosted declarations, [#10191](https://github.com/developing-today/code/issues/10191) the portable vectors, and [#10213](https://github.com/developing-today/code/issues/10213) the engine evaluation. [#9110](https://github.com/developing-today/code/issues/9110) owns the cross-cutting contract.
- The browser suite executes the vectors twice and compares observations to the independent reference model. Each divergent vector now has its own indexeddb expectation. Run the suite with Node and headless Chromium in pkgs/id/browser.
- A separate atomicity probe places a genuine unique-index constraint in one test database. An abort on a second store must leave the first store and operation marker unchanged. A corrected batch reusing the ID must commit. The oracle names both stores' exact contents before and after retry.
- Product-default selection, additional engines, live hosted migration and stronger durability guarantees require separate changes and evidence.
