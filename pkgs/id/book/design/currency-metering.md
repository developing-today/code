# Design: currency, tokens and metering

This chapter is design. It describes the currency, token and metering system as designed in 26 records homed here in the archive survey for this book, plus the receipt contract (#3231, audit #3232) whose records are homed with the identity records and cross-linked here because every anchored currency operation depends on it. The design has three layers that the records keep separate: graph-native tokens, whose balances are derived by folding signed graph events; ledger-anchored currencies, whose balance truth lives in TigerBeetle behind a gateway; and metering, which prices computation and function use without being a currency itself. The final section states what exists today.

Related chapters: [Design: foundations — objects, graph and the file core](foundations.md), [Design: capabilities and providers](capabilities-providers.md), [Currency, tokens and metering](../concepts/currency.md).

## Currencies v1 (#2043)

The governing record is `2026-11-01_design_id2043.md` (2026-11-01), the currencies v1 design under epic #2056, with Drew's 2026-11-08 policy rulings folded in. Its decision, in five parts:

1. A currency is a signed graph definition whose stable identity is the pair (issuer public key, definition event/hash). Name and symbol are labels, not identity.
2. Each definition selects exactly one binding: graph-native (offline, peer-to-peer, balances derived by folding events) or ledger-anchored (online, gateway-mediated, TigerBeetle holds balance truth).
3. The graph is the authorization and audit record for anchored operations. The control plane is a rebuildable operational index. TigerBeetle is the balance authority. Graph-native transfers are claims: conflicts and double-spends can be detected, and the design states plainly that they cannot be prevented offline.
4. Real-service purchases use the pending-hold lifecycle and only anchored currencies. A payment receipt records an effect; a receipt is never itself a balance movement.
5. Anchored conversions use one signed, directional rate authorisation, a short-lived event-pinned quote, and one atomically linked TigerBeetle batch. A posted conversion is corrected by a new compensating conversion, never edited.

The public naming is decided in the same record: the neutral label is "units", naming three currencies — units, pending units, revoked units. `2026-11-08T14-05-49Z_design_id2043_extension.md` (2026-11-08) extends the v1 design for the token rulings of 2026-11-08. The wire format underneath the graph-native side is `2026-10-30T13-25-00Z_design_graph_token_formats_snapshot_anchoring.md` (graph token object formats and snapshot anchoring), and the issuance analysis is `2026-10-30T19-18-00Z_investigation_token_issuance_id_graph` (per-ID issuance, issuer snapshots, authorized conversion rates). `2026-11-07T03-55-49Z_feature_graph_currency_contracts` (2026-11-07) is the implementation-slice record for the graph-native contracts under #5522: immutable definitions and the three primary value events — issue, transfer, burn. Its own status line states that it is not a claim that currencies v1 is complete.

An example with invented data: a definition event by issuer key `issuer-example` defines a currency labelled `test units` with a graph-native binding. A transfer event moves `50` units from account `alice-example` to account `bob-example`. Any node that folds the definition, the issuance and the transfer derives the same two balances; no server is consulted, and a second conflicting transfer of the same units is detectable in the fold, not prevented at the moment of signing.

## The receipt contract (#3231) and its audit (#3232)

Anchored operations and graph artifacts share one validity question, settled by `2026-11-01_design_id3231.md` (2026-11-01): what does it mean for a signed artifact to be valid when it arrives, and where is that contract enforced? The record's decision: a known-kind artifact is acceptable on receipt only when it satisfies the structural rules that the crate's supported producers are meant to create. Signature authenticity, chain position and unknown-kind forward compatibility stay distinct from known-kind body validation. Receipt must not rewrite signed bytes to make them pass. Unknown kinds may be stored; they are never applied. The record's status line states that implementation is represented by the origin issues it lists and that none is implemented by the record itself.

The Phase 3 audit of that contract is `2026-11-04_audit_id3232_receipt_validation.md` (2026-11-04), the receipt-validation audit for `pkgs/id` artifacts under #3232. The two records together are the receipt contract the currency design cites: a currency definition, a rate authorisation and a grant are each signed artifacts, and each is judged at receipt by the #3231 rules, not by the producer's intent. The signing-side counterpart — sign only what receipt would accept — is designed in the identity records and stated in the facade library header; it is cross-linked from [Design: identity, directory, groups and delegation](identity-directory-groups.md).

## Conversion and RateAuthorisation (#2063, #2290)

`2026-11-01_design_id2063.md` designs the conversion engine: linked transfer pairs and rate authorisation. `2026-11-01_design_id2290.md` designs the authorisation itself as a signed ID-graph object: a `currency.rate_authorisation` event, signed and directional, that names the pair of currencies, the rate and the authority under which a conversion may post. A conversion then runs as one atomically linked TigerBeetle batch — the debit leg and the credit leg post together or not at all — pinned to a short-lived quote that references the authorisation event. `2026-11-05_design_id5828.md` (2026-11-05) carries conversion and RateAuthorisation across the staged backends of #5676 (Redis, multi-Turso, TigerBeetle), so the contract survives each backend change.

## The TigerBeetle line

Four records work out the anchored backend:

- `2026-10-28T23-34-15Z_investigation_tigerbeetle_currency_backend` (2026-10-28): the investigation of TigerBeetle as the accounting backend for dynamic ID currencies.
- `2026-10-29T11-57-00Z_investigation_tigerbeetle_cluster_topology` (2026-10-29): the cluster topology decision — 3 replicas against 6, site placement, commit latency.
- `2026-11-03T20-36-00Z_design_tigerbeetle_ledger_binding` (2026-11-03): the LedgerBackend binding, `tb_ledger`, that connects the currency port to a TigerBeetle cluster.
- `2026-11-14T06-22-00Z_poc_currency_gateway_boundary_results` (2026-11-14): proof-of-concept results for the currency gateway boundary, #2298.

Two boundary records keep the anchored service in its place. `2026-10-29T12-20-00Z_design_currency_centralised_exception` designs currency as the centralised exception: the hosted edition's one centralised service boundary is the currency service, and the record draws that boundary. `2026-10-29T12-42-21Z_design_currency_control_plane` designs the control plane — registry, account factory, gateway. The registry is the authority for which `u32` means what: TigerBeetle ledgers are bare non-zero `u32` values, and name, symbol, scale, issuer, supply policy and status live in the registry. The ratified split of the ledger number space is `0x00000001` to `0x000FFFFF` for tenancy and system ledgers, and `0x00100000` upward for currencies; ledger 0 is never allocated.

## Grants, holds and refunds

Real-service value moves through grants, designed in three records. `2026-10-30T11-55-00Z_design_grant_lifecycle.md` is the grant lifecycle implementation specification: pending holds, a fingerprint registry, clawback and shortfall handling, and revocation policy. `2026-10-29T12-10-00Z_design_refundable_grants_pending_transfers` designs refundable grants via pending transfers — premium tokens that survive a refund, because the value sits in a pending transfer that a refund can void instead of a posted transfer that a refund would have to reverse. The v1 design binds the two together: purchases use pending holds, and only for anchored currencies.

## Scaling stages and the backend port

The staged path from the first backend to TigerBeetle is four records:

- `2026-11-14_design_id5823.md` (2026-11-14): the currency backend port and its invariant test-suite contract — one port, with the invariants every backend must pass stated as tests.
- `2026-11-16_design_id5825.md`: currency autoscaling — a stateless front end, hot-account mitigations, scaling triggers.
- `2026-11-16_design_id5826.md`: Stage B — multi-Turso sharding by currency.
- `2026-11-16_design_id5827.md`: Stage C — the TigerBeetle adapter and the B-to-C cutover. One currency is one `ledger: u32`. The cutover migrates one currency at a time: readiness, a shadow dual-run with a zero-divergence ledger comparison, then freeze, saga drain, fence, copy and verify, directory flip, and retirement of the Stage B copy as read-only audit.

`2026-11-08_design_id5829.md` designs the load and correctness verification programme those stages must pass: property tests, crash injection, and per-stage performance envelopes. `2026-11-08_decision_id5906_kdl_json_contract.md` is the decision record for the KDL-to-JSON canonical conversion and round-trip contract used by currency configuration surfaces.

One survey classification note: the survey homes `2026-03-21T05-47-48Z_feature_content_modes` (content modes, format conversion and media viewing, 2026-03-21) in this theme. It belongs to the file core's format-conversion history, not to currency conversion; the full index in [Roadmap, issue map and the design-record index](roadmap-and-issue-map.md) keeps the survey's mapping and this chapter does not cite it as a currency record.

## Metering

Metering is priced computation, designed separately from currency so that a host can charge for function use without issuing money.

- `2026-11-09T00-10-25Z_research_id5966_gas_timeouts_multithreading` (2026-11-09): research on per-function gas metering, timeouts and multithreading for user functions.
- `2026-11-10T03-10-00Z_design_id_two_level_metering` (2026-11-10): the two-level metering design — a burst bucket plus a hard per-tick ceiling, per function. The burst bucket absorbs short spikes; the ceiling is the value no single tick may exceed.
- `2026-11-11_design_id6016_metering_record_flow.md` (2026-11-11): the per-function metering record and its flow — the record's fields and its route, designed for the case where servers charge by function class.

World execution already has one metering value in force in the shipped runtime, stated here because the metering design builds on it: the default guest fuel budget is 10,000,000 fuel per call, and the world CLI defaults include `--world-fuel` with default `10000000`, `--world-memory-mib` with default `16`, and `--world-total-memory-mib` with default `64`. Fuel is the unit the two-level design meters at the function level; the per-function records of #6016 are the design for accounting that fuel to a function class. The interaction between metering records and currency — a host converting metered use into an anchored charge — is not settled in any record read for this chapter; that claim is unverified and no record is cited for it.

## What exists today

Grounding, from the library and web surveys for this book (checkout `1ccb4a19457`, 2026-11-23):

- Shipped: the `id-currency-registry` crate — the control-plane registry of the design, built as F1/F2 of #2062 (#2317, #2318). It contains the Postgres schema and migrations for ledger allocation, currency, currency issuer and account, the monotonic never-recycled ledger-`u32` allocator, the account factory, and an in-memory store that enforces the same constraints as the Postgres migration. Its only production dependent is the facade.
- Shipped as proof of concept: the facade's `currency` and `currency_poc` modules and the gateway PoC that composes the registry (`src/currency/gateway_poc.rs`, #2321), plus the optional `currency-poc-tb` feature, which pulls in the official TigerBeetle Rust client for the #2289 live-cluster PoC. A hosted-edition currency gateway PoC also exists in the Worker code (`src/currency/`, #2298) and its own text states it is not the permanent currency service: it is an in-process TigerBeetle-style ledger stub with gateway-signed operations.
- Shipped as a contract slice: the graph-native currency event contracts of `2026-11-07T03-55-49Z_feature_graph_currency_contracts` — definitions, issue, transfer, burn — under #5522.
- Still design: currencies v1 as a complete system. The v1 record is a design recommendation with policy rulings, not an implementation claim; the conversion engine as a running service, the staged backends B and C as production deployments, per-function metering records (#6016) and two-level metering as a charging system (#5966 research, 2026-11-10 design) have no shipped implementation named in the records read for this chapter.
