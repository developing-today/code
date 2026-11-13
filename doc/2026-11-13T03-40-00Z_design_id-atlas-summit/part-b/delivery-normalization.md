# Part B delivery normalization

**Delivery unit:** 32 stable slices B01–B32; one child issue is one PR. No component bundle PRs. B01–B30 remain the transparent statement-derived reconstruction, B31–B32 add core statement coverage; this is not the missing original candidate list.

## Single crosscut interface owner

Issue #9110-02 alone owns the shared root/registry boundary:

- `pkgs/id/crates/id-sync/src/federation/mod.rs` — stable public root scaffolding.
- `pkgs/id/crates/id-sync/src/federation/slices/mod.rs` — registry inventory/scaffolding for the exact per-slice modules listed below.
- `pkgs/id/crates/id-sync/Cargo.toml` — Cargo/module/feature registration and test-target convention.

Minimum contract: registry records each unique path, but does **not** unconditionally export absent modules. The scaffolding lets each slice add its source/test without editing shared roots, Cargo, or crate `lib.rs`. Crosscut #9110-09 owns versioned public export wiring after module outputs exist. Every implementation row requires #9110-02; research-only rows do not.

## Unique implementation module and test target per issue

Each implementation slice owns exactly the named source module and exact new integration-test file below; Cargo target is the filename stem. Do not combine rows into one PR.

| Slice | Source module | Test file / target |
|---|---|---|
| B01 | `pkgs/id/crates/id-sync/src/federation/slices/b01_key_lifecycle.rs` | `pkgs/id/crates/id-sync/tests/federation_b01.rs` / `federation_b01` |
| B02 | `pkgs/id/crates/id-sync/src/federation/slices/b02_key_revocation.rs` | `pkgs/id/crates/id-sync/tests/federation_b02.rs` / `federation_b02` |
| B03 | `pkgs/id/crates/id-sync/src/federation/slices/b03_replay_guard.rs` | `pkgs/id/crates/id-sync/tests/federation_b03.rs` / `federation_b03` |
| B04 | `pkgs/id/crates/id-sync/src/federation/slices/b04_signature_suites.rs` | `pkgs/id/crates/id-sync/tests/federation_b04.rs` / `federation_b04` |
| B05 | `pkgs/id/crates/id-sync/src/federation/slices/b05_signer_binding.rs` | `pkgs/id/crates/id-sync/tests/federation_b05.rs` / `federation_b05` |
| B06 | `pkgs/id/crates/id-sync/src/federation/slices/b06_rejection_events.rs` | `pkgs/id/crates/id-sync/tests/federation_b06.rs` / `federation_b06` |
| B07 | `pkgs/id/crates/id-sync/src/federation/slices/b07_conflict_report.rs` | `pkgs/id/crates/id-sync/tests/federation_b07.rs` / `federation_b07` |
| B08 | `pkgs/id/crates/id-sync/src/federation/slices/b08_sync_scope.rs` | `pkgs/id/crates/id-sync/tests/federation_b08.rs` / `federation_b08` |
| B09 | `pkgs/id/crates/id-sync/src/federation/slices/b09_anti_entropy.rs` | `pkgs/id/crates/id-sync/tests/federation_b09.rs` / `federation_b09` |
| B15 | `pkgs/id/crates/id-sync/src/federation/slices/b15_peer_priority.rs` | `pkgs/id/crates/id-sync/tests/federation_b15.rs` / `federation_b15` |
| B16 | `pkgs/id/crates/id-sync/src/federation/slices/b16_fanout_budget.rs` | `pkgs/id/crates/id-sync/tests/federation_b16.rs` / `federation_b16` |
| B17 | `pkgs/id/crates/id-sync/src/federation/slices/b17_query_grants.rs` | `pkgs/id/crates/id-sync/tests/federation_b17.rs` / `federation_b17` |
| B18 | `pkgs/id/crates/id-sync/src/federation/slices/b18_query_privacy.rs` | `pkgs/id/crates/id-sync/tests/federation_b18.rs` / `federation_b18` |
| B19 | `pkgs/id/crates/id-sync/src/federation/slices/b19_signed_receipt.rs` | `pkgs/id/crates/id-sync/tests/federation_b19.rs` / `federation_b19` |
| B20 | `pkgs/id/crates/id-sync/src/federation/slices/b20_request_identity.rs` | `pkgs/id/crates/id-sync/tests/federation_b20.rs` / `federation_b20` |
| B21 | `pkgs/id/crates/id-sync/src/federation/slices/b21_result_dedup.rs` | `pkgs/id/crates/id-sync/tests/federation_b21.rs` / `federation_b21` |
| B22 | `pkgs/id/crates/id-sync/src/federation/slices/b22_loop_inform.rs` | `pkgs/id/crates/id-sync/tests/federation_b22.rs` / `federation_b22` |
| B23 | `pkgs/id/crates/id-sync/src/federation/slices/b23_routing_state.rs` | `pkgs/id/crates/id-sync/tests/federation_b23.rs` / `federation_b23` |
| B24 | `pkgs/id/crates/id-sync/src/federation/slices/b24_topology_probe.rs` | `pkgs/id/crates/id-sync/tests/federation_b24.rs` / `federation_b24` |
| B25 | `pkgs/id/crates/id-sync/src/federation/slices/b25_local_reputation.rs` | `pkgs/id/crates/id-sync/tests/federation_b25.rs` / `federation_b25` |
| B27 | `pkgs/id/crates/id-sync/src/federation/slices/b27_claim_log.rs` | `pkgs/id/crates/id-sync/tests/federation_b27.rs` / `federation_b27` |
| B28 | `pkgs/id/crates/id-sync/src/federation/slices/b28_takeover_response.rs` | `pkgs/id/crates/id-sync/tests/federation_b28.rs` / `federation_b28` |
| B29 | `pkgs/id/crates/id-sync/src/federation/slices/b29_receipt_contradiction.rs` | `pkgs/id/crates/id-sync/tests/federation_b29.rs` / `federation_b29` |
| B30 | `pkgs/id/crates/id-sync/src/federation/slices/b30_search_integrity.rs` | `pkgs/id/crates/id-sync/tests/federation_b30.rs` / `federation_b30` |
| B31 | `pkgs/id/crates/id-sync/src/federation/slices/b31_peer_classes.rs` | `pkgs/id/crates/id-sync/tests/federation_b31.rs` / `federation_b31` |
| B32 | `pkgs/id/crates/id-sync/src/federation/slices/b32_ingest_gate.rs` | `pkgs/id/crates/id-sync/tests/federation_b32.rs` / `federation_b32` |

## Research-only issue artifacts

Each retains its own already-created report and validator as **existing read-only evidence**; no production Rust module is owned by these slices. Each future one-issue PR instead creates its own distinct JSON result artifact. The shared delivery validator CLI/schema is being normalized by the delivery owner; the manifest records its intended `--input` interface without editing that shared file here.

| Slice | Research scope | Existing read-only evidence | New owned result JSON / validator command |
|---|---|---|---|
| B10 | `crdt-automerge` | `B10-research-automerge.md`, `B10-research-automerge-validate.py` | `research-results/B10-result.json` / `python3 -I doc/2026-11-13T03-40-00Z_design_id-atlas-summit/delivery/validate_research_report.py --input doc/2026-11-13T03-40-00Z_design_id-atlas-summit/part-b/research-results/B10-result.json` |
| B11 | `crdt-diamond-types` | `B11-research-diamond.md`, `B11-research-diamond-validate.py` | `research-results/B11-result.json` / shared validator `--input` that path |
| B12 | `crdt-willow` | `B12-research-willow.md`, `B12-research-willow-validate.py` | `research-results/B12-result.json` / shared validator `--input` that path |
| B13 | `crdt-ocapn` | `B13-research-ocapn.md`, `B13-research-ocapn-validate.py` | `research-results/B13-result.json` / shared validator `--input` that path |
| B14 | `crdt-common-matrix` | `B14-research-crdt-matrix.md`, `B14-research-crdt-matrix-validate.py` | `research-results/B14-result.json` / shared validator `--input` that path |
| B26 | `reputation-abuse` | `B26-research-reputation-abuse.md`, `B26-research-reputation-abuse-validate.py` | `research-results/B26-result.json` / shared validator `--input` that path |

B19 is a standalone V2-crucial receipt-schema PR requiring only signer identity/gate foundations B05/B32 plus #9110-02 module registration; V3 fanout consumes it later. B28 is also a standalone V2-crucial emergency takeover-disable/quarantine issue/PR; advanced cross-peer attribution remains V4.

Existing validators check the present Markdown evidence and remain untouched; they are not future issue tests. The new JSON results are separately owned and checked by the delivery validator. CRDT matrix may conclude “adopt none.” B04's V2 Ed25519 suite allowlist is crucial because no crucial V2 gate may be held behind recommended work. B19 and B28 remain standalone V2-crucial PR units.
