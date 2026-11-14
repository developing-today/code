//! Federation slice registry (slice 9110-02, #10160).
//!
//! The B-registry inventory is exactly the 26 slice modules below,
//! recorded by their module names under `federation/slices/`. The
//! registry records the paths so sequencing and coverage audits
//! can enumerate the program, but it deliberately does **not**
//! declare or export the modules: each slice's module file is
//! added by its own implementation slice when it lands, and an
//! absent module must never be exported from this root.

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
