//! Federation slice registry (slice 9110-02, #10160; wired for #12232).
//!
//! The B-registry inventory is exactly the 26 slice modules in
//! [`B_SLICE_REGISTRY`], recorded by their module names under
//! `federation/slices/`, so sequencing and coverage audits can enumerate
//! the program.
//!
//! The modules declared below are the slices that have landed. A slice
//! that adds its module file declares it here in the same change, in
//! registry order: `.github/scripts/check-rust-module-reach.sh` fails when
//! a source file is not reachable from its crate root. A slice whose
//! file does not exist yet is in the registry and is never declared.

pub mod b01_key_lifecycle;
pub mod b02_key_revocation;
pub mod b03_replay_guard;
pub mod b04_signature_suites;
pub mod b05_signer_binding;
pub mod b06_rejection_events;
pub mod b07_conflict_report;
pub mod b08_sync_scope;
pub mod b09_anti_entropy;
pub mod b15_peer_priority;
pub mod b16_fanout_budget;
pub mod b17_query_grants;
pub mod b18_query_privacy;
pub mod b19_signed_receipt;
pub mod b20_request_identity;
pub mod b21_result_dedup;
pub mod b22_loop_inform;
pub mod b23_routing_state;
pub mod b24_topology_probe;
pub mod b28_takeover_response;
pub mod b31_peer_classes;
pub mod b32_ingest_gate;

/// The record-join admission check (#12279) lives one level up, in
/// `federation::record_join`; re-exported here so slice modules can
/// reach it as `super::record_join` — the same relative path the
/// path-loaded federation tests use from their crate root.
pub use super::record_join;

/// The exact B-slice module names, in registry order. Paths are
/// `federation/slices/<name>` relative to the `id-sync` crate's
/// federation root.
pub const B_SLICE_REGISTRY: &[&str] = &[
    "b01_key_lifecycle",
    "b02_key_revocation",
    "b03_replay_guard",
    "b04_signature_suites",
    "b05_signer_binding",
    "b06_rejection_events",
    "b07_conflict_report",
    "b08_sync_scope",
    "b09_anti_entropy",
    "b15_peer_priority",
    "b16_fanout_budget",
    "b17_query_grants",
    "b18_query_privacy",
    "b19_signed_receipt",
    "b20_request_identity",
    "b21_result_dedup",
    "b22_loop_inform",
    "b23_routing_state",
    "b24_topology_probe",
    "b25_local_reputation",
    "b27_claim_log",
    "b28_takeover_response",
    "b29_receipt_contradiction",
    "b30_search_integrity",
    "b31_peer_classes",
    "b32_ingest_gate",
];
