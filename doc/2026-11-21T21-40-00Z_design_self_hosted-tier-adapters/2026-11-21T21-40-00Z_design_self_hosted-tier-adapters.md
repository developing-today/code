# Self-hosted tier: storage, transport, and placement adapter design

Status: proposed V1 design for component I of #9078. This is a backend design, not a cluster deployment specification. The self-hosted ID service remains a peer participant; only its implementation and operational topology differ.

## Context and constraints

The hosted-editions intake establishes Cloudflare + Turso and Rust binaries as tier-1 platforms, with parity and deliberate divergence rather than a “port down” hierarchy. Component I adds the longer-term self-hosted Kubernetes tier: RustFS is the S3-compatible counterpart to R2, NATS JetStream is the streams/messaging counterpart to hosted sync transport, and local disk classes add capacity/latency tiers. See [`../2026-11-12T23-04-00Z_design_hosted-editions-workers/note.md`](../2026-11-12T23-04-00Z_design_hosted-editions-workers/note.md), statements S16–S20.

Binding constraints:

- A self-hosted service is the same protocol participant as a hosted-CF or Rust-binary service. No client-visible protocol, identity, authorization, or data-model branch may depend on “self-hosted.”
- RustFS, NATS, Kubernetes and local disks are replaceable backend implementations. The domain owns references, authorization, retention policy and placement intent.
- This document specifies contracts and a recommended policy. It does not provision a cluster, choose hardware, or make hosted services depend on NATS.
- Object bytes remain content-addressed; database/reference state remains authoritative. A storage tier is a location/availability class, never a new content identity.

## Decisions

1. Define backend-neutral object, event-stream, and placement interfaces at the ID/content service boundary. Hosted implementations use R2 and the hosted sync transport; self-hosted implementations use RustFS and NATS JetStream. Conformance tests target the interface, not provider quirks.
2. Keep metadata and authorization out of object-store keys and transport subjects. Resolve an opaque object ID through authorized metadata before issuing a short-lived data-plane capability. Do not treat a presigned URL or NATS subject as authorization by itself.
3. Use a three-class logical tier model (`hot`, `warm`, `archive`) with policy-driven placement. Physical media and provider names are deployment configuration, not protocol values. Begin with hot/warm; archive is optional and must be explicitly enabled.
4. Make placement asynchronous and recoverable. Database state records desired placement and observed replicas; a reconciler performs idempotent copy/verify/publish/delete transitions. Do not move the only verified copy before destination verification.
5. Separate retention from placement. Retention determines whether bytes may be deleted; tier policy determines where live bytes should reside. Garbage collection requires both expired retention and no live authorized references, plus a safety grace period.
6. Treat NATS as at-least-once transport. Events carry stable IDs and aggregate sequence/version, consumers deduplicate, and durable business state is committed independently of broker delivery. Do not require global ordering across tenants.
7. Model ID-service clustering as stateless peer nodes over a shared transactional metadata/control store plus replicated object storage. Node membership and routing are deployment concerns; no node is a privileged protocol leader. Consensus/partitioning is deferred until measured write contention or availability requirements justify it.

## Object adapter contract (R2 ↔ RustFS/S3)

The domain interface should be expressed in terms of operations and invariants, not S3 SDK request shapes:

- `put_staged(object_id, bytes, checksum, metadata)` creates an immutable staged object or returns the already-existing matching object. The adapter verifies length and checksum before reporting success. `object_id` is derived from canonical content identity, not a tenant secret or backend path.
- `head(object_id)` returns existence, verified size/checksum, and opaque backend version; it does not grant read access.
- `read(object_id, byte_range?, capability)` streams bytes only after the service has authorized the caller. Range support is optional at the adapter boundary; callers must tolerate a full read fallback.
- `copy_to(object_id, destination_class)` creates and verifies a destination replica while preserving object identity. `delete_replica(object_id, class, expected_version)` is conditional/idempotent and must not delete another version.
- Large uploads may use multipart internally. Multipart completion is not publication: only the service's verification step makes an object referenceable. Abort abandoned uploads after a bounded staging TTL.

R2 and RustFS/S3 may differ in consistency, multipart limits, conditional operations and URL behavior. The adapter must surface capabilities (conditional-write strength, range reads, multipart bounds, direct-download support) and use a safe fallback when absent. It must not promise stronger semantics than the selected backend provides. Publish a metadata reference only after read-after-write verification; a temporary visibility delay is a retryable storage result, not a protocol failure.

If direct access is enabled, issue narrowly scoped, short-lived capabilities after authorization. The capability binds object, operation, expiry and (where supported) byte/range constraints. Never persist a presigned URL as the object's identity or expose a bucket path as a stable API.

## Event/stream adapter contract (hosted sync ↔ NATS JetStream)

The service publishes domain events through an interface such as `append(tenant, stream, event_id, sequence, schema_version, payload)` and consumes with durable cursors. Requirements:

- Delivery is at least once. Every event has a stable idempotency key; consumers persist dedupe state with their effects. Retrying an append with the same event ID must not create a second logical effect.
- Ordering is required only within a tenant/entity stream and is expressed by sequence/version. A gap triggers replay or snapshot repair; the transport cannot silently reorder and claim success.
- The service owns an outbox (or equivalent transactional handoff): commit domain change and outbox row together, then publish/retry. A NATS acknowledgement means accepted by the configured stream, not processed by every consumer.
- Durable retention, replay windows, maximum payload, consumer acknowledgement deadline and dead-letter/retry policy are explicit deployment settings. Large object bytes travel through the object adapter; events carry references and metadata, not payload blobs.
- Hosted transport and NATS need not share wire protocol or operations. They implement the same domain event contract and common conformance suite. No client needs to know which transport is active.

## Tiered placement and retention

### Logical model

Each live object has immutable content identity, one or more replica records, and a placement policy. A replica record includes logical class, provider-local locator/version (private to the adapter), verification status, byte count, timestamps and lifecycle state. Tenant-visible APIs expose neither physical locator nor provider name.

Suggested initial classes:

| Class | Intent | Typical media | Read behavior | Retention |
|---|---|---|---|---|
| `hot` | frequent/latency-sensitive access | NVMe / fast object tier | immediate | policy minimum; no automatic eviction while pinned |
| `warm` | ordinary durable content | HDD-backed RustFS / standard object tier | online, possibly slower | policy minimum; default durable live copy |
| `archive` | infrequent, capacity-optimized data | optional cold disk/object tier | restore may be delayed | explicit policy; never silently selected for latency-sensitive reads |

Class names describe service guarantees, not a required disk count or product. A deployment maps classes to one or more failure domains. A class is eligible only if its configured durability, capacity and restore-time bounds meet the policy. The initial safe default is at least one verified warm copy; hot is an optional additional copy, and archive is opt-in.

### Placement and movement

Policy inputs may include tenant policy, sensitivity/access class, explicit pin, object size, recent access frequency, age and available capacity. Deterministic precedence: legal/security restrictions and explicit pins first; tenant policy next; automatic cost/temperature policy last. Reference count alone must not authorize movement or deletion. The control plane stores policy version and desired classes so decisions are auditable and repeatable.

A reconciler executes `requested → copying → destination_verified → metadata_switched → source_retained → source_deleted`:

1. Select a destination satisfying the desired class and failure-domain constraints.
2. Copy from a verified source; retry safely using object identity and transfer ID.
3. Verify destination checksum and size, then atomically mark that replica verified.
4. Update the preferred placement in metadata. Reads can fall back to another verified replica during transition.
5. Keep the source for a rollback/grace interval. Delete only if another required durable copy remains and no legal hold/pin blocks deletion.

Promotion is the same state machine in the opposite direction. Failures leave the last verified replica readable; reconciliation resumes from recorded state. Capacity pressure may defer best-effort promotion/demotion but cannot violate retention, durability floor, legal hold or authorization.

### Retention and deletion

Retention is expressed per tenant/object class as a minimum age, optional maximum age, and legal/administrative holds. Expiration only makes an unreferenced object eligible for collection; it does not immediately delete it. Collection requires: no live authorized references in the authoritative metadata, retention expiry, no hold/pin, and a configurable grace interval. Reference removal must account for concurrent writes and replicated metadata; use a tombstone/epoch or equivalent safe mark-and-sweep protocol. A stale counter is not sufficient evidence to delete. Object-store lifecycle rules may clean abandoned staging uploads, but must not independently expire published content.

## Component E/F boundaries

### Requirements for E (#9083: tenancy/control plane)

- Own tenant/object reference records, authorization, retention policy, holds, placement policy/version, and desired-versus-observed replica metadata.
- Scope reference queries and event streams by tenant; enforce public publication as an explicit reference/visibility state. A physical replica is not proof that a tenant can read it.
- Provide transactional update/outbox or an equivalent replayable handoff for reference, policy and placement changes.
- Define how reference removal is serialized against concurrent writes and GC. Retention and legal hold apply across every replica.

### Requirements for F (#9084: content/data plane)

- Keep content IDs and reference semantics provider-neutral. Accept immutable object operations with checksum/size verification and staged publication; never assume R2-specific consistency, URL, multipart or lifecycle behavior.
- Represent replicas and placement transitions separately from logical content identity. Reads resolve authorized metadata to a verified replica and can fall back during movement.
- Keep object bytes out of sync events; publish references and versioned metadata only. Support idempotent retries, resumable large transfers, and a GC safety protocol that consults authoritative references and retention/hold state.
- Provide adapter conformance tests for hosted R2 and self-hosted RustFS, including delayed visibility, interrupted multipart, duplicate request, conditional delete, checksum mismatch, and movement rollback.

## ID-service clustering sketch and lab-cluster relationship

For the first self-hosted deployment, multiple stateless ID-service peers serve the same API and share a transactional metadata store (the selected store abstraction from D #9082), an outbox/stream adapter, and RustFS-backed object replicas. A Kubernetes service routes requests; readiness excludes nodes unable to reach required metadata or a usable durable replica. Keep signing keys and tenant secrets in the deployment's secret-management boundary. Start with one metadata writer per shard/store unless the selected database provides a stronger documented multi-writer model; asynchronous object replication does not make metadata writes multi-master-safe.

Partitioning, leader election and cross-region consensus are not prerequisites for the adapter contract. The metadata store remains the serialization authority for references, sequence numbers, policy versions and GC epochs. If later scale requires sharding, partition by tenant with explicit ownership/epoch fencing; node identity remains absent from client-visible protocol. A cluster upgrade must preserve schema/event compatibility and recover in-flight placement work.

Relationship to the lab-cluster program: treat this design as a software workload/tenant of the shared hardware and cluster-management program, not as a second competing cluster architecture. The lab program owns physical provisioning, Kubernetes lifecycle, monitoring and capacity inventory; component I owns workload contracts, storage semantics and service SLOs. Reuse the lab cluster when it satisfies the workload's isolation, durability and operational requirements; otherwise document a separate deployment profile, not a new protocol. This boundary is a recommended default pending the lab program's actual capacity and tenancy contract.

## Risks, observability and acceptance

- **Backend semantic drift:** shared adapter conformance suite; expose adapter capability matrix and retryable-vs-terminal failures.
- **Data loss during movement/GC:** verify-before-switch, retained source grace interval, authoritative references, GC epochs, audit events and restore drills.
- **NATS mistaken for source of truth:** transactional outbox, idempotent consumers, lag/gap alarms and replay tests.
- **Cost policy harms availability:** policy cannot demote/delete the last required verified copy; expose placement backlog and per-class capacity/restore metrics.
- **Peer principle erosion:** architectural/API tests assert no backend kind or node role appears in peer protocol schemas or authorization decisions.

Acceptance for this design: R2/RustFS implement the same object contract; hosted transport/NATS implement the same event contract; tier policy, per-tier retention and movement follow the invariants above; E and F requirements are linked from their design issues; no cluster provisioning is implied. Before production, measure actual RustFS consistency and failure behavior, NATS retention/replay behavior, and the selected metadata store's write/partition guarantees against these contracts.

## Deferred implementation decisions

Exact storage classes per installation, numeric retention windows, replication factors, restore-time SLOs, RustFS version/configuration, NATS topology, metadata store, and Kubernetes sizing require workload and operations measurements. Defaults above are conservative interface requirements, not claims about a currently deployed cluster. Revisit these values when the lab-cluster capacity/SLO contract and component E/F schemas are available.
