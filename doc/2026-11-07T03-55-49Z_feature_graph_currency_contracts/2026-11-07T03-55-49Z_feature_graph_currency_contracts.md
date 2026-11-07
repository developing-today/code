# Graph-native currency event contracts

- **Issue:** [#5522 — implement ID currencies v1](https://github.com/developing-today/code/issues/5522)
- **Governing design:** [`2026-11-01_design_id2043.md`](../2026-11-01_design_id2043.md)
- **Wire-format specification:** [`2026-10-30T13-25-00Z_design_graph_token_formats_snapshot_anchoring.md`](../2026-10-30T13-25-00Z_design_graph_token_formats_snapshot_anchoring.md)
- **Status:** implementation slice; not a claim that currencies v1 is complete.

## Request and scope

Issue #5522 asks for production currency behavior beyond `currency_poc`, governed by the v1 design. That design explicitly phases the work and says a partial implementation must not advertise guarantees it does not provide. This change establishes the graph-native wire/API contract for immutable definitions and the three primary value events: issue, transfer, and burn.

This slice does not implement balance/supply folds, ancestry and fork resolution, snapshots, anchored registry/gateway or TigerBeetle execution, conversion, payment receipts, purchase grants, or client UX. Those remain required before claiming the full v1 acceptance criteria.

## Decisions

- A semantic currency identity is the issuer verifying key paired with the BLAKE3 hash of the canonical-CBOR definition body. Its compact hash is only a lookup/reference form; the pair remains authoritative.
- Definition names and symbols are display labels, never identity fields outside the definition hash. A changed definition creates a different identity.
- V1 definitions require a finite positive issuance cap, following the newer #2043 recommendation over the earlier `open` option in the graph-token wire spec. Open supply is not accepted by this contract.
- Definition scale is limited to 0–18 and is immutable because it participates in the definition body. Definitions require an NFC, non-empty name.
- Token payloads use canonical CBOR with strict decoding and closed schemas. Amount-bearing issue/transfer/burn objects reject zero amounts.
- Signing helpers bind each object to the existing `id.events.v1` chained-event envelope. Issue signing requires the definition identity and issuer key to agree; transfer and burn are signed by the holder.
- A ledger binding may be recorded in a definition, but this module provides no ledger-backed execution or balance guarantee.

## Verification plan

- Unit tests cover canonical body round trips, closed-schema rejection, definition identity, validation boundaries, signer binding, and positive amount requirements.
- Property tests exercise the definition identity's stable-pair rule and canonical encode/decode round trips over generated valid definitions.
- Run the focused `id` library tests and formatting/lint checks; the PR's `id` workflow remains the authoritative package validation.

## Remaining work

The graph-native fold and its causal/fork/cap rules, plus every anchored and real-service phase, remain outside this slice. No API here should be treated as exposing settled balances or accepted payments until those layers are implemented and verified against the governing design.
