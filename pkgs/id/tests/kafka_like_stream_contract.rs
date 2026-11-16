//! Kafka-like ordered-stream contract tests (slice 9110-02,
//! #10160) — evidence first, per the issue.
//!
//! Oracle: capability negotiation chooses the ordered-stream
//! profile only when declared; in-memory duplicate/retry traces
//! conform without a Kafka client or runtime dependency. A small
//! independent ordered-stream reference state machine in this
//! file checks sequence, redelivery, ack, and
//! unsupported-capability outcomes against the test double.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "support/kafka_like_ordered_stream.rs"]
mod kafka_like_ordered_stream;

use id_core::port::{
    CapabilityId, DataClassification, MessageStreamPort, OperationEnvelope, OperationRequest,
    PORT_CONTRACT_VERSION, Port, PortError, StreamOffset,
};
use kafka_like_ordered_stream::KafkaLikeOrderedStream;

/// Independent reference model: a plain log plus per-consumer ack
/// cursors, written directly from the contract's stated semantics
/// (append order, idempotent-append dedup, read-from-first-unacked,
/// ack advances). The double must match it step for step.
#[derive(Default)]
struct ReferenceStream {
    log: Vec<Vec<u8>>,
    keys: std::collections::HashMap<String, u64>,
    acked: std::collections::HashMap<String, u64>,
}

impl ReferenceStream {
    fn append(&mut self, key: Option<&str>, payload: &[u8]) -> u64 {
        if let Some(key) = key {
            if let Some(existing) = self.keys.get(key) {
                return *existing;
            }
        }
        let offset = self.log.len() as u64;
        self.log.push(payload.to_vec());
        if let Some(key) = key {
            self.keys.insert(key.to_string(), offset);
        }
        offset
    }

    fn read(&self, consumer: &str, max: usize) -> Vec<(u64, Vec<u8>)> {
        let start = self
            .acked
            .get(consumer)
            .map(|acked| (*acked + 1) as usize)
            .unwrap_or(0);
        self.log
            .iter()
            .enumerate()
            .skip(start)
            .take(max)
            .map(|(i, payload)| (i as u64, payload.clone()))
            .collect()
    }

    fn ack(&mut self, consumer: &str, offset: u64) {
        let entry = self.acked.entry(consumer.to_string()).or_insert(offset);
        if offset > *entry {
            *entry = offset;
        }
    }
}

fn envelope(operation_id: &str, idempotency_key: Option<&str>) -> OperationEnvelope {
    OperationEnvelope {
        operation_id: operation_id.to_string(),
        idempotency_key: idempotency_key.map(str::to_string),
        tenant_scope: "tenant:test".to_string(),
        authorization_scope: "member".to_string(),
        deadline_unix_ms: None,
        trace_id: format!("trace-{operation_id}"),
        protocol_version: PORT_CONTRACT_VERSION,
        data_classification: DataClassification::Internal,
    }
}

#[test]
fn negotiation_chooses_ordered_profile_only_when_declared() {
    let stream = KafkaLikeOrderedStream::new("stream:test", "tenant:test");
    let profile = stream
        .negotiate(&OperationRequest {
            required_capabilities: vec![CapabilityId::ORDERED_STREAM],
            protocol_version: PORT_CONTRACT_VERSION,
        })
        .unwrap();
    assert!(profile.grants(CapabilityId::ORDERED_STREAM));

    // The double does not advertise an atomic outbox; asking for
    // it is a typed refusal, before any side effect.
    let err = stream
        .negotiate(&OperationRequest {
            required_capabilities: vec![CapabilityId::ATOMIC_OUTBOX],
            protocol_version: PORT_CONTRACT_VERSION,
        })
        .unwrap_err();
    assert_eq!(
        err,
        PortError::UnsupportedCapability {
            capability: CapabilityId::ATOMIC_OUTBOX,
            port: id_core::port::PortKind::MessageStream,
        }
    );
    assert!(stream.is_empty(), "a refused negotiation appends nothing");
}

#[test]
fn duplicate_and_retry_traces_conform_to_the_reference_model() {
    let mut stream = KafkaLikeOrderedStream::new("stream:test", "tenant:test");
    let mut model = ReferenceStream::default();

    // Append a, b; retry a with the same idempotency key (a
    // duplicate delivery of the producer request); append c
    // without a key twice (distinct operations, both land).
    let script: [(&str, Option<&str>, &[u8]); 5] = [
        ("op-a", Some("k-a"), b"a"),
        ("op-b", Some("k-b"), b"b"),
        ("op-a-retry", Some("k-a"), b"a"),
        ("op-c", None, b"c"),
        ("op-c2", None, b"c"),
    ];
    for (op, key, payload) in script {
        let got = stream.append(&envelope(op, key), payload).unwrap();
        let want = model.append(key, payload);
        assert_eq!(got, StreamOffset(want), "append offset for {op}");
    }
    assert_eq!(stream.len(), 4, "the retry was deduplicated");
    assert_eq!(model.log.len(), 4);

    // Read before ack: everything, in order, matching the model.
    let read = stream.read("consumer-1", 10).unwrap();
    let model_read = model.read("consumer-1", 10);
    assert_eq!(read.len(), model_read.len());
    for (record, (want_offset, want_payload)) in read.iter().zip(model_read.iter()) {
        assert_eq!(record.offset, StreamOffset(*want_offset));
        assert_eq!(&record.payload, want_payload);
    }

    // Redelivery: reading again without acking returns the same
    // records (at-least-once).
    let redelivered = stream.read("consumer-1", 10).unwrap();
    assert_eq!(redelivered, read);

    // Ack the first two; the consumer now starts at offset 2,
    // while a fresh consumer still starts at 0.
    stream.ack("consumer-1", StreamOffset(1)).unwrap();
    model.ack("consumer-1", 1);
    let after_ack = stream.read("consumer-1", 10).unwrap();
    let model_after = model.read("consumer-1", 10);
    assert_eq!(after_ack.len(), model_after.len());
    assert_eq!(after_ack.first().unwrap().offset, StreamOffset(2));
    assert_eq!(stream.read("consumer-2", 10).unwrap().len(), 4);
}

#[test]
fn oversized_messages_hit_the_declared_limit() {
    let mut stream = KafkaLikeOrderedStream::new("stream:test", "tenant:test");
    let payload = vec![0u8; 1_048_577];
    let err = stream
        .append(&envelope("op-big", None), &payload)
        .unwrap_err();
    assert_eq!(
        err,
        PortError::LimitExceeded {
            limit: "max_message_bytes",
            declared: 1_048_576,
            requested: 1_048_577,
        }
    );
    assert!(stream.is_empty());
}
