# Design: TigerBeetle LedgerBackend binding (`tb_ledger`)

Issue: #2289 (conversion-executor PoC, acceptance item 1). Code:
`pkgs/id/src/currency_poc/tb_ledger.rs`, behind the `currency-poc-tb`
feature. PR: #5072.

This record documents the as-built design of the live binding. Per
`pkgs/id/AGENTS.md` ("Documenting Design & Architecture Decisions")
it should have been written before implementation; it was filed
during PR #5072 review after a Codex finding (2026-11-03) pointed
out the gap, and records the decisions and constraints the module
comments only summarized.

## Intent

`currency_poc` defines a synchronous `LedgerBackend` trait whose
reference implementation, `InMemoryLedger`, encodes the TigerBeetle
semantics the conversion executor depends on (atomic linked chains,
field-sensitive idempotent transfer IDs, balance-invariant flags).
The binding implements the same trait over the official TigerBeetle
Rust client (`tigerbeetle` crate, pinned to the harness server
version 0.17.9) so the executor scenarios run against a real
single-replica test cluster (`currency_poc::harness::TestCluster`),
not only the in-memory model.

## Decisions

### Feature gate

The binding is behind `currency-poc-tb` so default builds of the
`id` crate do not link the client's native `tb_client` library.
The harness and live tests are gated the same way; live tests are
additionally `#[ignore]`d and need the pinned binary
(`TIGERBEETLE_POC_BINARY` or `tigerbeetle` on `PATH`).

### Private runtime, synchronous surface

The trait is synchronous; the client is async but runtime-agnostic
(it runs its own off-thread event loop). The binding owns a private
current-thread Tokio runtime and `block_on`s each request. It must
therefore be driven from plain synchronous code (as the PoC executor
is) — never from inside a Tokio runtime, where `block_on` would
panic. One client (and runtime) per ledger instance.

### Cluster identity

The harness formats its data file with cluster ID 0, so the client
connects with cluster ID 0. This is a test-harness binding, not a
production deployment story.

### Reversible wire-code encoding

TigerBeetle rejects `code = 0` on accounts and transfers, while the
PoC types use 0 as a supported chart-of-accounts code. Codes are
sent shifted by one (`wire_code`: `c -> c + 1`; `poc_code` is the
inverse). The shift is a bijection on `0..=u16::MAX - 1`, so every
supported code stays distinct on the wire. The earlier design —
substituting a constant for 0 only — serialized codes 0 and 1
identically, so a retry with the other code reported `Exists`
instead of `ExistsWithDifferentFields`, breaking the backend's
field-sensitive idempotency contract (Codex review, PR #5072).
`u16::MAX` has no wire representation under the shift (it would
wrap to 0) and panics as a programming error; the PoC's codes are
small chart-of-accounts values.

### Code registries

TigerBeetle stores only the wire code, so the binding records the
original code of every account/transfer it creates and restores it
on lookups (a looked-up leg must compare equal to the persisted
intent leg the recovery scanner checks). Registries are updated
**only after** TigerBeetle accepts a creation (`Created`, or
`Exists` confirming identical fields): a rejected retry must not
overwrite the registry, or later lookups would misreport the stored
record (Codex review, PR #5072). For records the registry does not
know (e.g. created by an earlier binding instance), lookup decodes
the wire code with `poc_code`.

### Status mapping and panic stance

Result statuses are mapped onto the PoC error enums for the subset
the PoC models. An unmodelled status — or a transport-level client
error, for which the trait has no channel — panics with the
status/error named rather than being silently folded into a wrong
variant. One exception is modelled explicitly: `IdAlreadyFailed`
maps to `TransferError::IdAlreadyFailed`. TigerBeetle remembers
failed transfer IDs, so a recovery resubmission of a previously
failed conversion meets this status by design; as a regular error
it lets `recovery::reconcile` report `ResubmissionRejected` and
continue the scan instead of aborting it (Codex review, PR #5072).

### Exact `user_data_128` queries

The trait promises exact matching on `user_data_128` (the
conversion-ID lookup). TigerBeetle treats a zero `QueryFilter`
field as "no filter", so a zero query would return every transfer.
The binding therefore pages through query results (advancing
`timestamp_min` past each full page; timestamps are unique per
transfer) and post-filters on exact equality for **every** value,
zero included, so the result stays exact even if server filter
semantics change (Codex review, PR #5072).

## Verification

The scenario suite in `currency_poc::tests` runs against both
backends: in-memory by default, live against `TestCluster` when
ignored tests are enabled with the pinned binary. Live coverage
includes the three behaviours this record describes: distinct
0/1 codes with registry-preserving rejected retries, exact zero
`user_data_128` matching, and failure-then-recovery reporting
`ResubmissionRejected`.

### Linked-chain retry

TigerBeetle breaks a linked chain on any non-`Created` status, so
resubmitting an already-posted chain reports `Exists` for its first
leg and `LinkedEventFailed` for every later leg, while the
in-memory backend reports `Exists` per leg (and the executor
accepts that as idempotent success). The binding translates the
pattern back: when a whole chain shows `Exists` then
`LinkedEventFailed`s AND every stored leg verifies identical to
the submitted leg, all legs report `Exists`. A genuinely failed
chain never starts with `Exists`, and a differing stored leg fails
verification, so both keep the raw mapping. Covered live by
`live_same_request_twice_posts_once` (Codex review, PR #5072).
