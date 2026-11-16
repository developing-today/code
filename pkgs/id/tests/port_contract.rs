//! Port-contract conformance tests (slice 9110-02, #10160).
//!
//! The oracle under test: capability negotiation chooses a
//! profile only when the adapter declared the capabilities it
//! requires; refusals are typed and happen before side effects.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use id_core::port::{
    CapabilityId, DataClassification, OperationEnvelope, OperationRequest, PORT_CONTRACT_VERSION,
    PortDescriptor, PortError, PortKind, PortLimits, SemanticGuarantees, negotiate,
};
use id_sync::federation::slices::B_SLICE_REGISTRY;

fn descriptor(kind: PortKind, optional: Vec<CapabilityId>) -> PortDescriptor {
    PortDescriptor {
        kind,
        contract_version: PORT_CONTRACT_VERSION,
        resource_id: "test-resource".to_string(),
        scope: "tenant:test".to_string(),
        required_capabilities: Vec::new(),
        optional_capabilities: optional,
        guarantees: SemanticGuarantees::default(),
        limits: PortLimits::default(),
    }
}

fn operation(required: Vec<CapabilityId>) -> OperationRequest {
    OperationRequest {
        required_capabilities: required,
        protocol_version: PORT_CONTRACT_VERSION,
    }
}

#[test]
fn ordered_stream_profile_chosen_only_when_declared() {
    let with = descriptor(
        PortKind::MessageStream,
        vec![
            CapabilityId::ORDERED_STREAM,
            CapabilityId::IDEMPOTENT_APPEND,
        ],
    );
    let profile = negotiate(&with, &operation(vec![CapabilityId::ORDERED_STREAM])).unwrap();
    assert!(profile.grants(CapabilityId::ORDERED_STREAM));
    assert_eq!(profile.kind, PortKind::MessageStream);

    let without = descriptor(
        PortKind::MessageStream,
        vec![CapabilityId::IDEMPOTENT_APPEND],
    );
    let err = negotiate(&without, &operation(vec![CapabilityId::ORDERED_STREAM])).unwrap_err();
    assert_eq!(
        err,
        PortError::UnsupportedCapability {
            capability: CapabilityId::ORDERED_STREAM,
            port: PortKind::MessageStream,
        }
    );
}

#[test]
fn negotiation_grants_exactly_what_was_declared_and_required() {
    let d = descriptor(
        PortKind::BlobStore,
        vec![CapabilityId::RANGE_READ, CapabilityId::SCAN],
    );
    let profile = negotiate(&d, &operation(vec![CapabilityId::RANGE_READ])).unwrap();
    assert!(profile.grants(CapabilityId::RANGE_READ));
    // Advertised but not required by this operation: not granted.
    assert!(!profile.grants(CapabilityId::SCAN));
}

#[test]
fn version_mismatch_is_a_typed_refusal() {
    let d = descriptor(PortKind::RecordStore, Vec::new());
    let mut op = operation(Vec::new());
    op.protocol_version = PORT_CONTRACT_VERSION + 1;
    assert_eq!(
        negotiate(&d, &op).unwrap_err(),
        PortError::VersionMismatch {
            declared: PORT_CONTRACT_VERSION,
            requested: PORT_CONTRACT_VERSION + 1,
        }
    );
}

#[test]
fn port_kinds_are_separate_contracts() {
    let kinds = [
        PortKind::RecordStore,
        PortKind::BlobStore,
        PortKind::EventLog,
        PortKind::WorldStore,
        PortKind::ObjectBucket,
        PortKind::MessageStream,
        PortKind::RequestTransport,
    ];
    for (i, a) in kinds.iter().enumerate() {
        for b in kinds.iter().skip(i + 1) {
            assert_ne!(a, b);
        }
    }
}

#[test]
fn envelope_carries_the_common_fields() {
    let envelope = OperationEnvelope {
        operation_id: "op-1".to_string(),
        idempotency_key: Some("key-1".to_string()),
        tenant_scope: "tenant:test".to_string(),
        authorization_scope: "member".to_string(),
        deadline_unix_ms: Some(1_700_000_000_000),
        trace_id: "trace-1".to_string(),
        protocol_version: PORT_CONTRACT_VERSION,
        data_classification: DataClassification::Internal,
    };
    assert_eq!(envelope.operation_id, "op-1");
    assert_eq!(envelope.idempotency_key.as_deref(), Some("key-1"));
    assert_eq!(envelope.data_classification, DataClassification::Internal);
}

#[test]
fn b_slice_registry_is_exactly_the_settled_inventory() {
    assert_eq!(B_SLICE_REGISTRY.len(), 26);
    assert_eq!(B_SLICE_REGISTRY.first(), Some(&"b01_key_lifecycle"));
    assert_eq!(B_SLICE_REGISTRY.last(), Some(&"b32_ingest_gate"));
    assert!(B_SLICE_REGISTRY.contains(&"b19_signed_receipt"));
    assert!(B_SLICE_REGISTRY.contains(&"b22_loop_inform"));
    // The settled inventory skips b10-b14 and b26 by design.
    assert!(!B_SLICE_REGISTRY.iter().any(|name| name.starts_with("b10")));
    assert!(!B_SLICE_REGISTRY.iter().any(|name| name.starts_with("b26")));
    let mut sorted = B_SLICE_REGISTRY.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    assert_eq!(sorted.len(), 26, "registry names are unique");
}

#[test]
fn graph_and_triples_roots_exist_without_exporting_absent_modules() {
    // The roots are wired into id-core and addressable; their
    // contract is that they declare no child modules until
    // consumer slices land (the root files carry documentation
    // and this name only).
    assert_eq!(id_core::graph::ROOT_NAME, "graph");
    assert_eq!(id_core::triples::ROOT_NAME, "triples");
}
