# Currency, tokens and metering

ID currencies let a world, a group or a service keep score in units it defines: a game currency, a community token, a metered credit. The shipped system splits that job in two. A control plane decides what the units mean — which ledger number is which currency, who issues it, what its supply policy is. A data plane (TigerBeetle) moves the units between accounts. This chapter covers the shipped control plane, the proofs of concept that connect it to a ledger, and the metering budgets that bound how much of anything a node will accept. It also corrects, in place, a wrong address: the listing, suspension and delisting lifecycle does not live where a reader would first look for it.

## The registry: id-currency-registry

The control plane is the crate `id-currency-registry` (`crates/id-currency-registry`), built as F1 of #2062 (issue #2317), with the account factory as F2 (issue #2318). Its own statement of purpose is exact: the registry is the authority for which `u32` means what. TigerBeetle ledgers are bare non-zero `u32` values; the currency's name, symbol, scale, issuer and supply policy, and its status, live in the registry, not in the ledger.

Ledger numbers are allocated monotonically and are never recycled. Ledger 0 is never allocated. The ratified split of the number space is: `0x00000001` to `0x000FFFFF` for tenancy and system ledgers, and `0x00100000` upward for currencies (`CURRENCY_RANGE_START`). The allocator is the `Allocator` type, with `allocate` and `reserve_specific`, and its error type `AllocError`. The registry proper is `Registry`, over `CurrencyRecord`, `CurrencyStatus`, `SupplyPolicy`, `RegisterCurrency` and `RegisterAccount`, with `RegistryError`.

Persistence is a Postgres schema with migrations for four tables — `ledger_allocation`, `currency`, `currency_issuer` and `account` — with the migration text carried in the crate as the constants `MIGRATION_0001_UP` and `MIGRATION_0001_DOWN`. The crate also ships a `Store` trait with an `InMemoryStore`: the in-memory store enforces the same constraints as the Postgres migration, so acceptance behaviour is testable without a database.

The crate has five public modules: `allocator`, `factory`, `grants`, `registry` and `store`. It depends on nothing else in the workspace, and nothing at the facade's public surface re-exports it: its only production consumer is the facade's own currency gateway proof of concept, which names it by its crate path (`id_currency_registry::…`). A library consumer who wants the registry names the crate directly.

## The account factory

The factory (F2) creates the accounts a currency needs on a ledger cluster. Its types are `Factory` and `FactoryError`, `CurrencyAccounts`, the `AccountCluster` trait (the seam to a real cluster), `AccountSpec`, `AccountKind`, `CreateOutcome` and `ClusterError`, plus `factory::transfer_code` and the constants `PLATFORM_OWNER_ID` and `CURRENCY_RANGE_START`. Account creation is specified against the registry record: the gateway proof of concept builds an account spec from a record's ledger ID with `AccountKind::Holder`, an owner, and an empty memo string.

## What is proof of concept, and what is not shipped

Three currency artefacts exist beyond the registry crate, and all three are proofs of concept. None of them is a running currency service.

- The facade's currency modules: the root crate owns `currency` and `currency_poc` modules. The gateway proof of concept (`src/currency/gateway_poc.rs`, #2321) composes the registry with a ledger backend. The facade feature `currency-poc-tb` adds the official TigerBeetle Rust client for the #2289 live-cluster proof of concept; the feature has no CLI surface.
- The PoC harness reads one environment variable: `TIGERBEETLE_POC_BINARY` (in `src/currency_poc/harness.rs`). Unset, there is no external TigerBeetle binary. It is a harness setting, not a service configuration.
- The hosted worker carries a second gateway proof of concept (`src/currency/{gateway,ledger,relay,crypto}.ts`, #2298), described in [The hosted edition](hosted.md): an in-process ledger stub modelled on TigerBeetle, gateway-signed balance snapshots, and a UserDO relay reconciling by request-ID lookup. Its own text states it is not the permanent currency service.

One format from this family is named in the shipped tree: the currency receipt domain, `id.currency.receipt.v1`. Beyond that, the surveys do not enumerate a shipped transfer, issuance or conversion command surface, and this chapter claims none. The design records for the rest — ID currencies v1 (#2043) and its token extension rulings, the conversion engine with linked transfer pairs and RateAuthorisation as a signed graph object (#2063, #2290), refundable grants via pending transfers, graph-native currency contracts, and the TigerBeetle topology investigations — are carried by the design chapter currency, tokens and metering (26 records in the survey's mapping).

A toy example, with invented data only: a registry operator registers a currency with symbol `EXM` (an invented example symbol) and is allocated the next free ledger number at or above `0x00100000`; the number, once allocated, is never handed to a second currency, even if the first is retired. The example is an illustration of the allocation rule, not a record of a real currency.

## Metering: the server budgets

Metering that does ship, and that every operator meets, is the `id serve` budget family in the server configuration TOML (`[budgets]`, in `crates/id-cli/src/server_config/`). Budgets bound inbound traffic per application, per peer and per IP. The surveyed defaults are:

| Key | Default |
|---|---|
| `app_inbound_bytes_per_hour` | 20 GiB per hour (21,474,836,480 bytes) |
| `per_peer_bytes_per_hour` | 100 MiB per hour (104,857,600 bytes) |
| `per_ip_bytes_per_hour` | 250 MiB per hour (262,144,000 bytes) |

Two further keys are implemented in the parser (`parse.rs`) but missing from the module's own doc table — survey disagreement WEB-14: `app_inbound_bytes_per_second`, default 33,554,432 (32 MiB, 1-second window), and `per_peer_burst_bytes_per_minute`, default 8,388,608 (8 MiB, 60-second window), both from `BudgetPolicy::default` in `crates/id-core/src/abuse.rs` (verified in the source for this book; the reference chapter on configuration is their home). A per-counterparty override table, `[[budgets.counterparty]]` with `peer`, `bytes_per_window`, `window_seconds` and optional `messages_per_window`, replaces — it does not stack on — the per-peer limits for that peer.

World execution is metered separately, at the guest boundary: the sandbox runs guests under fuel budgets, worlds carry per-account charge configuration (`id-worlds` `account_charge`, with `WorldChargeConfig`, `WorldAccountRegistry` and `ChargeMode`), and guest burn budgets are their own module (`guest_burn`). The two-level metering design — a burst bucket plus a hard per-tick ceiling, per function — is a design record (2026-11-10), not a shipped mechanism this chapter can point to in the budget table.

Agent actions are metered a third way, by quota: the MCP policy point charges tool calls, frames and program invocations against a grant's quota before execution, and parks the agent when the quota is exhausted. That mechanism is covered in [Agents in worlds: the MCP bridge and policy](mcp-and-agents.md).

## Listing, suspension and delisting: the corrected address

The commissioning assumption for this chapter placed the listing, suspension and delisting family in `id-directory`. The survey checked, and that is wrong: no listing, suspension or delisting record kind exists in `crates/id-directory`. The directory journal's 17 entry kinds contain none of them. The family lives in three other places:

- Signed registry lists, in `crates/id-sync/src/registry.rs`. A registry list body is canonical CBOR, signed by the registry key (Ed25519). Each entry is a `RegistryEntry`: the announcement verbatim (signed by the world key), an `included_at` time, and a `registry_seq`. A delisting is a `Tombstone{world_id, removed_at, reason, registry_seq}`, with the reason at most 200 characters. The registry's policy statement is at most 2000 characters. Unlisted worlds are refused at inclusion and skipped on import.
- The provider declaration lifecycle, in `crates/id-caps/src/declaration.rs`: `LifecycleState` runs `Filed → Admitted → Live → Suspended / Superseded / Stale → Tombstoned`. Suspension is registry state; the declared contents are unchanged. The signing domains are `id.provider-manifest.v1`, `id.provider-withdrawal.v1` and the `id.host-attested-provider:` prefix.
- Gallery listing decisions, in `crates/id-worlds/src/program_gallery_admission.rs`: admission decisions with a `ReasonClass`, and the transparency entries — removal, refusal, reinstatement — served by the web shell's gallery transparency route, with the operator's admission posture.

A fourth, narrower member of the family sits in federation: key suspension records in `crates/id-sync/src/federation/slices/b02_key_revocation.rs`, covering operator maintenance suspension, bounded or indefinite. The residency side of suspension is noted in [Residency, jurisdiction and boundaries](residency.md).

The correction matters because the three homes have three different authorities: a registry key lists and tombstones worlds, a declaration lifecycle governs providers, and a gallery operator admits and removes programs. A reader sent to `id-directory` would find none of them, and would miss that listing is a signed, sequenced, tombstone-bearing record — not a flag in a directory entry.
