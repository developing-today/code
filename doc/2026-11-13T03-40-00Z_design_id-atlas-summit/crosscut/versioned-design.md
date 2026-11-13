# Cross-cutting versioned design: primitives, adapters, hooks, modes

Scope: #9110 and #9111; requirement coverage S0.1–S0.17, S1.114–S1.120, S7.1–S7.16. This is the cross-cutting contract for sibling designs, not a substitute for their domain decisions.

## Decision summary

1. Use capability-oriented, versioned ports for stores, object buckets, streams/logs, and transports; do not define one universal database/transport interface.
2. Rust-native and Cloudflare/Turso remain fully featured tier-1 editions. Their implementations may differ where platform semantics require it, but required operation semantics and test vectors are shared. A backend gap is explicit capability/conformance data, not a product edition or silent weakening.
3. Hooks are typed, ordered lifecycle callbacks around all ordinary operation paths. Fixed signature/validation, replay, tenant-scope, authorization, and acceptance-state checks cannot be bypassed or accepted by a hook. Local producers sign before admission and pass the same gate.
4. Mechanism is primitive; behavior is config. Modes bundle validated defaults, with explicit scoped overrides and hard invariant ceilings. Peer, link, storage, data tier, and namespace registry remain separate configuration planes.
5. At-least-once delivery plus stable IDs/idempotent consumers is the cross-store/transport baseline. No global exactly-once promise.

## Stable APIs and ordering

### Port contract

Each port advertises `contract_version`, resource identity/scope, required and optional capability IDs, semantic guarantees, hard limits, and typed errors. An operation declares required capabilities before side effects. The adapter either satisfies them or returns a structured unsupported/conditional result; it must not silently emulate weaker semantics. Common envelope fields: operation ID/idempotency key, tenant and authorization scope, deadline/cancellation, trace/correlation ID, protocol version, and data classification. Secrets are not placed in observable context.

Port types are separate: record store, blob store, event log, world store, object bucket, message stream, request/response transport. Existing event/world/store authorities remain separately owned until a mapping is reviewed; “store abstraction” does not merge them. Adapter-specific APIs may exist behind an adapter boundary but cannot leak into core decisions.

### Operation pipeline (mandatory ordering)

1. Resolve/validate versioned config; check adapter capabilities and hard ceilings.
2. For a local producer only, trusted deterministic built-in code performs canonical normalization and signs before admission; this pre-sign step is not a plugin or hook. Bind caller, tenant, peer, operation ID and immutable signed envelope.
3. Run fixed admission checks: verify signature and signer binding; validate object semantics; check replay/idempotency; authorize signer/action and tenant/scope. These checks cannot be skipped or weakened. This gate applies to retained data/assertions from every source. A local read request is not retained accepted data: it requires caller authentication and tenant/scope authorization; state-changing requests additionally require replay/idempotency protection. Federated query envelopes follow their protocol signature/validation requirements.
4. Run deterministically ordered `prepare` hooks. Each sees only validated immutable input and read-only validation result. It may veto or return a namespaced non-authoritative annotation; it cannot mutate signed bytes or authority. Persisted enrichment must be a distinct derived object, signed by an authorized signer, and submitted through a new full admission operation.
5. Immediately before commit, reauthorize against the current policy revision/epoch and recheck relevant tenant, replay and acceptance constraints. Revocation during hook execution rejects commit.
6. Execute/commit within declared transaction/ordering scope; append outbox atomically only where adapter advertises that scope. If outcome is unknown, retain stable operation ID/status and read back to resolve; reauthorize before retry/new admission. Do not claim rollback or blindly replay. Otherwise use documented journal/reconciliation or reject operations requiring stronger atomicity.
7. Deliver post-commit hooks/outbox at least once, retry by stable event ID, and record pending/quarantined failures. Never report committed state as rolled back because delivery failed.

Hook sequence is deterministic: phase order, configured priority, stable registration ID. Same-operation reentrant writes are rejected; a callback can request a separate queued operation with a fresh ID. Callback timeout/budget exhaustion has phase-specific outcome (pre-commit veto/failure vs post-commit pending delivery). Hooks are observable and auditable but cannot rewrite acceptance decisions.

Before claiming extensibility coverage complete, inventory every normal ingress/read/write/sync/query/stream/bucket path and identify its adapter, gate location, hooks, bypass tests, and recovery semantics. V2 migrates paths incrementally, but its exit gate is a complete path inventory plus proof each retained-data admission path reaches the fixed gate. A representative path is a prototype, not completion.

## V1–V4 progression

### V1 — existing behavior (evidence, not retroactive claims)

ID includes Iroh `FsStore` persistent mode (redb metadata plus blob data files) and ephemeral `MemStore`; the event log is a separate redb DB and durable worlds are fsynced `journal.jsonl` plus modules. ID also has domain-local Iroh Docs namespace registry, specialized signed sync and relay/query mechanisms. Atlas `content_hash` is declared/consumed but has no checked-in production producer. No uniform cross-domain namespace/federation contract, universal port/capability registry, universal hook lifecycle or cross-runtime config model is demonstrated. Existing primitives remain supported; do not rename specialized behavior into a general contract without migration evidence.

### V2 — stable seams and conformance foundation (first implementation target)

Define port contract and capability schema; map existing ID store/event/world/blob seams to ports; add Rust traits and TypeScript interfaces/fixtures; make redb and Turso/SQLite adapter profiles honest about transaction, durability, retry and scan scope; evaluate IndexedDB by the same suite. Establish a minimal V2 validated config schema (mandatory invariant IDs, authority/resource ceilings, schema/protocol versions, adapter capabilities, deterministic canonicalization) before core gates depend on it. Inventory all normal paths, then migrate hooks/gates across them; representative-path work is only the first slice. Ship conformance reports and versioned shared vectors. Keep NATS as an additive adapter prototype, not a production dependency.

**V2 exit:** same required ID operation fixtures pass in native redb and hosted Turso paths; any non-shared semantic is explicitly conditional and guarded; IndexedDB verdict is evidence-backed; a complete normal-path inventory shows every retained-data admission path reaches the fixed gate; hook faults cannot bypass acceptance/security checks; adapter addition requires no core rewrite.

V2 also includes minimal key revocation/disable and replay-window enforcement required for safe admission. Advanced reputation scoring and broad key-compromise reputation recovery remain V4.

### V3 — composable configuration and federation integrations

Expand the V2 schema into validated mode bundles, per-peer/link/storage/data-tier bindings, behavior override precedence, capability negotiation and resolved-config digest. Integrate trust/peer classes from #9141, tenant policy from #9083, namespace registry from #9116, and transport profile from #9085 without duplicating those owners. Add per-operation custom connector selection. NATS/Kafka/Postgres remain independently selectable adapter families; implementations can be deferred while contracts and test doubles exist.

**V3 exit:** independent deployment can reproduce the same protocol-visible behavior from equivalent config; mandatory invariants survive malformed, downgraded, or hostile config; config changes are staged/rollback-capable with clear external-effect limits.

### V4 — long-tail implementations and operations

Add IndexedDB if evaluation passes; Postgres, NATS JetStream, Kafka, RustFS/S3, and tiered placement as independently justified adapters. Add migration tooling, schema evolution, capability telemetry, restart/outbox reconciliation and performance/cost evidence. Do not weaken the port contract to increase adapter count. Add optional capabilities under compatible minor versions; semantics-changing requirements need a new major protocol/port version and explicit migration/negotiation.

## Configuration data model and precedence

Top-level `ConfigDocument` includes schema version, config ID/revision, named mode reference, protocol-version allowlist, hard resource ceilings (non-overridable), peer policies, link bindings, storage bindings, data-tier placement/retention, namespace registry reference, and optional namespaced extensions. Values carry source/provenance in resolved output; secrets are separate references, not serialized inline.

Resolve immutable primitive invariants and protocol/resource ceilings first; then deployment behavioral defaults → mode behavioral defaults → tenant/peer/link/storage behavioral overrides → permitted operation-scoped overrides. Authority ceilings (maximum tenant/peer scope) and resource ceilings (maximum hops/bytes/time/fan-out) can only narrow at lower scopes. Behavioral values can be replaced in either direction by later authorized scopes, provided results stay within every applicable ceiling and preserve invariants. Example: deployment `sync_scope=metadata`, mode `sync_scope=full`, tenant authority allows only `metadata` resolves to `metadata`; deployment `tier=cold`, mode `tier=hot`, tenant `tier=warm` resolves to `warm` if allowed; hard `max_hops=5`, mode default 3, peer override 1 resolves to 1 while 6 is rejected. Validation is fail-closed. Unknown mandatory field/version rejects the document; namespaced optional extensions may be retained but are not executed unless understood and authorized. Activation is two-phase: validate against all selected adapter capabilities and peer protocol constraints, then commit a revision; on failure retain previous revision. Rollback restores prior config, not already-completed external effects. Canonical effective config is deterministic; provenance is recorded separately.

Mode is a coherent behavior bundle (e.g. `single-node`, `hosted-tenant`, `federated-relay`) and never a code fork. A mode cannot confer identity or privilege. Peer class is directional per relationship. Link kind (sync, query/fan-out, send-only, control) is a transport capability choice. Storage kind declares durability/retention/sync participation. Tiering is native in the data model: retained records reference signed, versioned `data_tier` and/or `storage_tier` identifiers with scope/provenance; policy maps identifiers to placement/retention. Reclassification creates a new signed version/derived record, not mutation of prior signed classification. Tier labels do not authorize access. Peer, link, storage, data-tier, and namespace axes remain distinct. Config supports per-operation connection selection but forbids an operation override from bypassing auth, signature, replay, or tenant-boundary checks.

## Failure, recovery, and version rules

- Capability mismatch and invalid config are rejected before operation side effects.
- Before-commit hook veto means no committed mutation. Storage timeout with unknown outcome returns operation ID and requires idempotent readback/retry. Post-commit hook/outbox error leaves committed state and pending delivery.
- Restart replays outbox by stable ID, preserving declared order scope; duplicate delivery is expected. Failed permanent hooks enter configured quarantine/dead-letter workflow with operator visibility.
- A store+stream boundary without shared atomic commit uses outbox/journal and reconciliation; exactly-once is not claimed.
- Port/protocol negotiation is signed and identity-bound. Negotiate highest mutually supported allowlisted version. Reject downgrade that removes a required signature, validation, authorization, replay, tenant, or operation capability. Never fall back silently.
- Acknowledgment contradiction evidence compares signed claims only when scope, time, authorization and applicable retention/completeness obligations are comparable. A receipt is not a perpetual retention promise; claims are evidence, not verdicts.
- Revocation/disable and replay-window changes take effect at V2 admission/commit checks. They do not rewrite prior signature bytes; retained historical records preserve signer/key identity, profile and acceptance-time evidence.
- Additive optional capability is minor-compatible; changing interpretation, ordering, or mandatory guarantees is a major version. Migrations are explicit, resumable, idempotent and retain old readers until cutover/rollback window closes.

## Conformance and test gates

1. **Store adapter suite:** redb, Turso/SQLite, adopted IndexedDB. Oracle is an abstract reference state machine. Check CRUD, boundary scans, declared atomic batch scope, capability mismatch-before-write, idempotent retries, reopen/restart, fault/unknown outcomes.
2. **Hook model tests:** generated traces against `normalize/sign→gate→prepare→reauthorize→commit→outbox→delivery`. Oracle is a transition model. Inject mutation attempts, policy revocation, crashes, duplicate callbacks, unknown commit outcomes and timeouts; assert immutable validated hook input, no unauthorized commit, reauthorization before retry, and no rollback fiction.
3. **PBT:** config resolution is deterministic; behavior overrides replace in either direction within ceilings; authority/resource ceilings never widen; primitive gates never disable; canonical encoding round-trips; unsupported capability never mutates. Oracle: independent canonical resolver plus invariant predicates.
4. **Cross-runtime fixtures:** Rust and TypeScript consume/emit identical signed envelopes/config vectors. Oracle: golden vectors verified by an independent implementation, not the serializer under test.
5. **Transport/stream suite:** ordering scope, ack/redelivery, bounded replay, cancellation and retention. Oracle: explicit state machine; at-least-once allowed, exactly-once not expected.
6. **Interop/non-specialness:** two independent nodes with equivalent public config get equivalent authorization and operation outcomes; no first-party bypass. Oracle: paired black-box trace comparison.

## Cross-part identity profiles consumed by the ports

Use distinct identities: `PayloadId` hashes raw payload bytes; initial `ContentId` preserves Iroh BLAKE3-256 over exact profile bytes, while every `ContentId` carries/dispatches its explicit profile; `EnvelopeId`/`AssertionId`/`SnapshotId` hash the complete canonical signed envelope including signature, never including that ID in its own preimage; `LogicalGraphId` and space addresses are stable logical identities separate from assertion content hashes. A new signed record has canonical `Body` committing payload ID, length, domain and context, plus detached signature over domain-prefix + canonical Body; Body excludes signature and EnvelopeId. Use deterministic CBOR for the new profile. Existing artifact JSON bytes remain a legacy profile: add compatibility vectors for actual serde JSON bytes and the docs/code mismatch, without silently changing preimages, re-signing, or rehashing existing records. Report profile conflicts, never implicitly coerce. Derived cache/index records are rebuildable projections over validated signed authority; a derived object independently retained or transported needs a signed service-produced envelope. Atlas `content_hash` has checked-in consumers/schema but no checked-in production producer; a follow-up must define exact producer input bytes, canonicalization, profile and oracle before interoperability claims. No V1 IANA namespace-scheme claim is made.

## Relationship boundaries

- #9082 owns ID backend selection, backend semantics, IndexedDB verdict and migration design; #9110 owns the cross-cutting primitive/capability contract. Amend #9082 with the contract link; do not restate its detailed backend design here.
- #9078/#9083 own hosted deployment and tenancy control plane. This design carries scope through port operations without restating their decisions.
- #9084 owns content addressing/reference/copy and object lifecycle; #9110 specifies the object bucket seam only.
- #9085 owns TS transport/runtime scope; #9110 defines transport-port capabilities, not the transport protocol inventory.
- #9111 defines shared configuration shape and safe precedence; #9141 peer semantics, #9116 namespace registry, and storage/content owners retain their domain meaning.
- Preserve the two namespace cores. SSO remains owned by #9077; tenancy by #9083; content-plane behavior by #9084. In particular, reference-vs-copy remains open with #9084; this crosscut does not settle it.

## Remaining evidence gaps and conservative defaults

- The artifact signature documentation/code preimage mismatch still needs owner-level byte-for-byte reconciliation. Until #9110-R1 completes, treat current verified serde JSON bytes as immutable legacy behavior; do not correct old signatures in place or label the documented preimage as interoperable.
- No checked-in production Atlas `content_hash` producer was found. Until #9110-R2 locates and pins one and provides a known-answer vector, make no cross-system content-hash interoperability claim and do not infer an algorithm from schema, consumers, fixture strings, or vector joins.
- IndexedDB remains evaluation-only per #9082. Do not promise it as an equal persistent backend until browser-engine transaction/quota/reopen conformance evidence supports the needed capability subset.
- Exact deployment-pinned redb/Turso durability and ambiguous-commit behavior remain to be established by the adapter conformance tests. Until then advertise only explicit transaction APIs/capabilities and conditional durability; unknown remote outcome stays unknown pending operation-ID readback.
