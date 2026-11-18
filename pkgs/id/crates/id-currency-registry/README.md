# id-currency-registry

Currency control-plane registry — slice **F1** of the currency
control-plane design (#2062,
`doc/2026-10-29T12-42-21Z_design_currency_control_plane`), filed as
issue **#2317**. It unblocks F2 (account factory) and F4
(reconciliation + supply audit).

TigerBeetle's `ledger` is a bare non-zero `u32` with no metadata.
This crate is the authority for which u32 means what.

## What is here

- `migrations/0001_currency_registry.up.sql` — the Postgres schema:
  - `ledger_allocation` — the u32 allocator's ledger (design §2.1),
    with the ratified range split as a check constraint
    (`1..=0xFFFFF` tenancy/system, `0x100000..` currencies), the
    `allocate_ledger(kind, owner_ref)` function over monotonic
    sequences, and a trigger that refuses deletes — ledger ids are
    **never recycled** (design §6).
  - `currency` — the currency record (§2.2): `currency_id` globally
    unique, `(issuer_id, symbol)` unique (symbol is per-issuer,
    §2.3), `graph_object_id` linkage to the signed canonical record
    (§2.4).
  - `currency_issuer` — issuer/minter grants, drafted from §4.2.
  - `account` — the index of deterministic factory accounts,
    drafted from §3. (Derivation itself is F2.)
- `src/` — the allocator and registry semantics over a `Store`
  seam, with an in-memory store enforcing the same constraints, so
  the behaviour is testable without a database. Production writes
  go through the gateway's signed path (design §2.5); this crate
  states the registry rules that path must enforce.

## Decisions applied

From the ledger item `currency-registry-decisions-2317` (answered
2026-11-16, recorded on issue #2317):

1. Registry store: **Postgres**.
2. Ledger u32: **bigint + `0..4294967295` check**; the design §6
   range split is ratified.
3. `currency_id` **globally unique**; `symbol` unique **per issuer**.
4. The `currency_issuer` / `account` columns the design does not
   tabulate are **fleet-drafted** from design §2.1–2.2, §2.4, §3
   and §4.2 — they land here for ratification.

## Verify

```sh
cargo test -p id-currency-registry
scripts/test-migration.sh   # needs initdb/pg_ctl/psql on PATH
```
