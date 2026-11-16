//! In-memory Kafka-like ordered-stream test double (slice
//! 9110-02, #10160).
//!
//! This is **contract coverage only**: it implements the
//! message-stream port contract in memory so conformance traces
//! (append order, duplicate/retry deduplication, redelivery of
//! unacknowledged records, ack) can run without a Kafka client,
//! a Kafka runtime dependency, or any deployment choice. It is
//! not a Kafka adapter and must not be wired as one.

use std::collections::HashMap;

use id_core::port::{
    CapabilityId, MessageStreamPort, OperationEnvelope, PORT_CONTRACT_VERSION, Port,
    PortDescriptor, PortError, PortKind, PortLimits, SemanticGuarantees, StreamOffset,
    StreamRecord,
};

/// An in-memory ordered stream with per-consumer ack cursors.
pub struct KafkaLikeOrderedStream {
    descriptor: PortDescriptor,
    log: Vec<StreamRecord>,
    idempotency_index: HashMap<String, StreamOffset>,
    acked: HashMap<String, StreamOffset>,
}

impl KafkaLikeOrderedStream {
    /// A stream advertising ordered, idempotent append with
    /// at-least-once delivery (explicitly *not* exactly-once).
    pub fn new(resource_id: &str, scope: &str) -> Self {
        Self {
            descriptor: PortDescriptor {
                kind: PortKind::MessageStream,
                contract_version: PORT_CONTRACT_VERSION,
                resource_id: resource_id.to_string(),
                scope: scope.to_string(),
                required_capabilities: Vec::new(),
                optional_capabilities: vec![
                    CapabilityId::ORDERED_STREAM,
                    CapabilityId::IDEMPOTENT_APPEND,
                ],
                guarantees: SemanticGuarantees {
                    ordered: true,
                    durable: false,
                    atomic_commit: false,
                    atomic_outbox: false,
                    exactly_once: false,
                },
                limits: PortLimits {
                    max_record_bytes: None,
                    max_batch_records: None,
                    max_message_bytes: Some(1_048_576),
                },
            },
            log: Vec::new(),
            idempotency_index: HashMap::new(),
            acked: HashMap::new(),
        }
    }

    /// Number of records actually appended (deduplicated).
    pub fn len(&self) -> usize {
        self.log.len()
    }

    /// Whether the stream holds no records.
    pub fn is_empty(&self) -> bool {
        self.log.is_empty()
    }
}

impl Port for KafkaLikeOrderedStream {
    fn descriptor(&self) -> &PortDescriptor {
        &self.descriptor
    }
}

impl MessageStreamPort for KafkaLikeOrderedStream {
    fn append(
        &mut self,
        envelope: &OperationEnvelope,
        payload: &[u8],
    ) -> Result<StreamOffset, PortError> {
        if let Some(max) = self.descriptor.limits.max_message_bytes {
            if payload.len() as u64 > max {
                return Err(PortError::LimitExceeded {
                    limit: "max_message_bytes",
                    declared: max,
                    requested: payload.len() as u64,
                });
            }
        }
        if let Some(key) = &envelope.idempotency_key {
            if let Some(existing) = self.idempotency_index.get(key) {
                return Ok(*existing);
            }
        }
        let offset = StreamOffset(self.log.len() as u64);
        self.log.push(StreamRecord {
            offset,
            operation_id: envelope.operation_id.clone(),
            payload: payload.to_vec(),
        });
        if let Some(key) = &envelope.idempotency_key {
            self.idempotency_index.insert(key.clone(), offset);
        }
        Ok(offset)
    }

    fn read(&self, consumer: &str, max: u32) -> Result<Vec<StreamRecord>, PortError> {
        let start = self
            .acked
            .get(consumer)
            .map(|acked| (acked.0 + 1) as usize)
            .unwrap_or(0);
        Ok(self
            .log
            .iter()
            .skip(start)
            .take(max as usize)
            .cloned()
            .collect())
    }

    fn ack(&mut self, consumer: &str, offset: StreamOffset) -> Result<(), PortError> {
        let entry = self.acked.entry(consumer.to_string()).or_insert(offset);
        if offset > *entry {
            *entry = offset;
        }
        Ok(())
    }
}
