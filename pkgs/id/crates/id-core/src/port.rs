//! Port contracts: the common interface every storage, stream and
//! transport adapter in the ID/Atlas program advertises against
//! (slice 9110-02, #10160; settled design: crosscut versioned-design,
//! "Port contract").
//!
//! A port advertises its `contract_version`, resource identity and
//! scope, required and optional capability IDs, semantic guarantees,
//! hard limits, and typed errors. An operation declares the
//! capabilities it requires *before* any side effect; the adapter
//! either satisfies them or returns a structured unsupported /
//! conditional result. A port must never silently emulate weaker
//! semantics than the ones it advertised.
//!
//! Port types are deliberately separate — record store, blob store,
//! event log, world store, object bucket, message stream, and
//! request/response transport are distinct contracts, and the
//! existing event/world/store authorities remain separately owned.
//! Nothing here merges them.

use std::fmt;

/// The port-contract version this module implements (design V2).
pub const PORT_CONTRACT_VERSION: u32 = 2;

/// A capability identifier, namespaced by port family
/// (for example `stream.ordered`). Capabilities are additive: an
/// adapter may advertise new optional capabilities under the same
/// contract version; a semantics-changing requirement needs a new
/// contract version and explicit negotiation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct CapabilityId(pub &'static str);

impl CapabilityId {
    /// Ordered delivery within a single stream partition.
    pub const ORDERED_STREAM: Self = Self("stream.ordered");
    /// Appends carrying an idempotency key are deduplicated.
    pub const IDEMPOTENT_APPEND: Self = Self("stream.idempotent-append");
    /// Commit and outbox append share one atomic scope.
    pub const ATOMIC_OUTBOX: Self = Self("commit.atomic-outbox");
    /// Range (offset/length) reads of stored blobs.
    pub const RANGE_READ: Self = Self("blob.range-read");
    /// Full scans of the record set, in the declared scan scope.
    pub const SCAN: Self = Self("store.scan");
    /// Responses carry a signed receipt bound to the request.
    pub const SIGNED_RECEIPTS: Self = Self("transport.signed-receipts");
}

impl fmt::Display for CapabilityId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0)
    }
}

/// The separate port types of the contract. Each kind has its own
/// trait below; kinds are never interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PortKind {
    /// Keyed record storage.
    RecordStore,
    /// Content-addressed blob storage.
    BlobStore,
    /// Append-only event log.
    EventLog,
    /// Durable world state store.
    WorldStore,
    /// Object bucket (bulk object placement).
    ObjectBucket,
    /// Ordered message stream.
    MessageStream,
    /// Request/response transport.
    RequestTransport,
}

/// Semantic guarantees an adapter advertises. Anything not declared
/// here must be treated as *not* guaranteed; in particular
/// `exactly_once` is never implied by `ordered` plus retries.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SemanticGuarantees {
    /// Records are delivered in append order within the scope.
    pub ordered: bool,
    /// Committed records survive restart of the adapter.
    pub durable: bool,
    /// A commit is atomic within its declared scope.
    pub atomic_commit: bool,
    /// Commit and outbox append are atomic together.
    pub atomic_outbox: bool,
    /// Delivery is exactly-once (almost never honestly claimable).
    pub exactly_once: bool,
}

/// Hard limits an adapter declares. `None` means the adapter
/// declares no finite limit for that dimension; an operation that
/// needs a bounded limit must negotiate explicitly rather than
/// assume one.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PortLimits {
    /// Largest single record, in bytes.
    pub max_record_bytes: Option<u64>,
    /// Largest batch, in records.
    pub max_batch_records: Option<u32>,
    /// Largest single stream message, in bytes.
    pub max_message_bytes: Option<u64>,
}

/// What a port advertises: identity, scope, capabilities,
/// guarantees and limits, all fixed before any operation runs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PortDescriptor {
    /// Which separate port type this is.
    pub kind: PortKind,
    /// Contract version the adapter implements.
    pub contract_version: u32,
    /// Stable resource identity (adapter-local, opaque outside it).
    pub resource_id: String,
    /// Tenant/authorization scope the resource is bound to.
    pub scope: String,
    /// Capabilities the adapter requires callers to accept.
    pub required_capabilities: Vec<CapabilityId>,
    /// Capabilities the adapter can grant when an operation asks.
    pub optional_capabilities: Vec<CapabilityId>,
    /// Declared semantic guarantees.
    pub guarantees: SemanticGuarantees,
    /// Declared hard limits.
    pub limits: PortLimits,
}

impl PortDescriptor {
    /// Whether the adapter advertises `capability` at all
    /// (required or optional).
    pub fn advertises(&self, capability: CapabilityId) -> bool {
        self.required_capabilities.contains(&capability)
            || self.optional_capabilities.contains(&capability)
    }
}

/// Data classification carried on the operation envelope. Secrets
/// are never placed in the envelope or any other observable
/// context; classification labels data, it does not carry it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DataClassification {
    /// Freely shareable.
    Public,
    /// Internal to the operating scope.
    Internal,
    /// Restricted; handling follows the scope's policy.
    Confidential,
}

/// The common envelope every operation declares before side
/// effects: identity, idempotency, scope, deadline, trace and
/// classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationEnvelope {
    /// Stable operation ID; retries keep it.
    pub operation_id: String,
    /// Idempotency key, when the operation is replayable.
    pub idempotency_key: Option<String>,
    /// Tenant scope the operation acts within.
    pub tenant_scope: String,
    /// Authorization scope asserted for the caller.
    pub authorization_scope: String,
    /// Deadline as unix milliseconds, when bounded.
    pub deadline_unix_ms: Option<u64>,
    /// Trace/correlation ID for observability.
    pub trace_id: String,
    /// Protocol version the caller speaks.
    pub protocol_version: u32,
    /// Classification of the data the operation touches.
    pub data_classification: DataClassification,
}

/// What an operation requires from a port, declared up front.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationRequest {
    /// Capabilities the operation cannot proceed without.
    pub required_capabilities: Vec<CapabilityId>,
    /// Protocol/contract version the operation was written against.
    pub protocol_version: u32,
}

/// The outcome of a successful negotiation: the profile the
/// operation may rely on. A capability is in `granted` only when
/// the adapter declared it — negotiation never invents semantics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedProfile {
    /// Port kind negotiated against.
    pub kind: PortKind,
    /// Contract version in force.
    pub contract_version: u32,
    /// Capabilities granted for this operation.
    pub granted: Vec<CapabilityId>,
}

impl NegotiatedProfile {
    /// Whether `capability` was granted by this negotiation.
    pub fn grants(&self, capability: CapabilityId) -> bool {
        self.granted.contains(&capability)
    }
}

/// Typed port errors. Capability and limit failures are structured
/// values, never stringly-typed log lines, so callers can react
/// programmatically (choose another adapter, narrow the operation,
/// or refuse) instead of guessing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PortError {
    /// The port does not advertise a capability the operation
    /// requires. Returned *before* any side effect.
    UnsupportedCapability {
        /// The missing capability.
        capability: CapabilityId,
        /// The port kind that refused it.
        port: PortKind,
    },
    /// The capability exists only under a stated condition the
    /// operation did not meet.
    Conditional {
        /// The conditional capability.
        capability: CapabilityId,
        /// The unmet condition, in the adapter's terms.
        condition: String,
    },
    /// The operation exceeded a declared hard limit.
    LimitExceeded {
        /// Which declared limit was exceeded.
        limit: &'static str,
        /// The declared bound.
        declared: u64,
        /// What the operation requested.
        requested: u64,
    },
    /// The operation's protocol version does not match the port's
    /// contract version. Downgrades that would drop a required
    /// capability are refused here, never silently accepted.
    VersionMismatch {
        /// The port's contract version.
        declared: u32,
        /// The version the operation requested.
        requested: u32,
    },
}

impl fmt::Display for PortError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedCapability { capability, port } => {
                write!(
                    f,
                    "port {port:?} does not advertise capability {capability}"
                )
            }
            Self::Conditional {
                capability,
                condition,
            } => {
                write!(f, "capability {capability} is conditional: {condition}")
            }
            Self::LimitExceeded {
                limit,
                declared,
                requested,
            } => {
                write!(
                    f,
                    "limit {limit} exceeded: declared {declared}, requested {requested}"
                )
            }
            Self::VersionMismatch {
                declared,
                requested,
            } => {
                write!(
                    f,
                    "contract version mismatch: port declares {declared}, operation requested {requested}"
                )
            }
        }
    }
}

impl std::error::Error for PortError {}

/// Negotiate an operation against a port descriptor. Succeeds only
/// when the version matches and every required capability is
/// advertised; the granted profile contains exactly the required
/// capabilities the adapter declared — no more, so an ordered
/// profile is chosen only when `stream.ordered` was declared.
pub fn negotiate(
    descriptor: &PortDescriptor,
    operation: &OperationRequest,
) -> Result<NegotiatedProfile, PortError> {
    if operation.protocol_version != descriptor.contract_version {
        return Err(PortError::VersionMismatch {
            declared: descriptor.contract_version,
            requested: operation.protocol_version,
        });
    }
    let mut granted = Vec::with_capacity(operation.required_capabilities.len());
    for capability in &operation.required_capabilities {
        if !descriptor.advertises(*capability) {
            return Err(PortError::UnsupportedCapability {
                capability: *capability,
                port: descriptor.kind,
            });
        }
        granted.push(*capability);
    }
    Ok(NegotiatedProfile {
        kind: descriptor.kind,
        contract_version: descriptor.contract_version,
        granted,
    })
}

/// The base contract every port implements.
pub trait Port {
    /// The descriptor this port advertises.
    fn descriptor(&self) -> &PortDescriptor;

    /// Negotiate an operation against this port's descriptor,
    /// before any side effect.
    fn negotiate(&self, operation: &OperationRequest) -> Result<NegotiatedProfile, PortError> {
        negotiate(self.descriptor(), operation)
    }
}

/// Record-store port (separate contract; see [`PortKind`]).
pub trait RecordStorePort: Port {}
/// Blob-store port (separate contract; see [`PortKind`]).
pub trait BlobStorePort: Port {}
/// Event-log port (separate contract; see [`PortKind`]).
pub trait EventLogPort: Port {}
/// World-store port (separate contract; see [`PortKind`]).
pub trait WorldStorePort: Port {}
/// Object-bucket port (separate contract; see [`PortKind`]).
pub trait ObjectBucketPort: Port {}
/// Request/response transport port (separate contract).
pub trait RequestTransportPort: Port {}

/// Offset of a record within a stream partition.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct StreamOffset(pub u64);

/// One record read from a stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StreamRecord {
    /// The record's offset in the stream.
    pub offset: StreamOffset,
    /// The operation that appended it.
    pub operation_id: String,
    /// The record payload.
    pub payload: Vec<u8>,
}

/// Message-stream port: append/read/ack over an ordered log.
/// Reading does not advance a consumer; only `ack` does, so
/// unacknowledged records are redelivered — at-least-once delivery
/// with consumer-side deduplication by operation/idempotency key.
pub trait MessageStreamPort: Port {
    /// Append a record under `envelope`, returning its offset.
    /// With an advertised idempotency capability, a retry carrying
    /// the same idempotency key returns the original offset and
    /// does not append a duplicate.
    fn append(
        &mut self,
        envelope: &OperationEnvelope,
        payload: &[u8],
    ) -> Result<StreamOffset, PortError>;

    /// Read up to `max` records for `consumer` starting at its
    /// first unacknowledged offset.
    fn read(&self, consumer: &str, max: u32) -> Result<Vec<StreamRecord>, PortError>;

    /// Acknowledge records up to and including `offset` for
    /// `consumer`; acknowledged records are not redelivered.
    fn ack(&mut self, consumer: &str, offset: StreamOffset) -> Result<(), PortError>;
}
