# Part A: graph, attributes, social objects, and signer authority

- **Parent:** design summit #9106; Part A issue #9107.
- **Children in scope:** #9125–#9132; statement inventory S5.1–S5.64.
- **Status:** proposed component contracts; not an implementation authorization or wire-format freeze.
- **Source scaffold:** [`note.md`](../../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md), §5, retrieved 2026-11-12.
- **Research companion:** [`research.md`](research.md).
- **External dependency boundaries:** #9077 owns SSO; #9083 owns tenancy; #9084 owns ref-vs-copy (still unresolved). Existing Iroh docs, worlds, groups and ID content models retain their domain owners. Current baseline says #6277/#6287–#6290 are live staged designs with #6312 as plan-of-record, not settled architecture; exact producer-side Atlas `content_hash` contract is absent in this checkout and needs an explicit follow-up. This part does not claim ownership or infer a hash algorithm.

## 1. Decisions and explicit deferrals

1. A triple is an immutable assertion carried as signed content; the graph is a projection over validated assertions plus server-local state. No consumer may treat storage order, network arrival, or a query result as authority.
2. “Everything breaks down into triples” is an extensible object model, not a demand that every blob or every operation be destructured into triples. Content bytes remain immutable, content-addressed objects; signed associations and metadata remain separate mutable/revisioned records.
3. Core API starts with bounded triple patterns and paged cursors. No write-capable query language and no arbitrary server-side code. Named language syntax and property paths are later adapters, not a parallel store.
4. Cycles are legal in asserted graph data. Tree rendering detects revisits and represents them as references; traversal has explicit depth/node/byte budgets. No recursive expansion by default.
5. Append submission and authoritative acceptance are distinct: producers sign before submission; the server verifies before durable admission; a server-assigned monotonic sequence determines accepted-list order. Pending moderation is not accepted content.
6. Version lineage is a DAG, not a mutable `latest` pointer. A projection picks one head under explicit policy; concurrent branches remain visible. Comments do not change identity or bytes of the content they discuss.
7. A cursor is an opaque, server-authenticated snapshot/frontier token with a bounded lifetime and an epoch/generation. A stale token returns `CursorExpired` (restart required), never a false “no changes.” Deletion/moderation changes invalidate the relevant projection generation.
8. True deletion means removal from this server’s controlled stores/backups according to declared retention, not global erasure or revocation of already copied bytes. Signed deletion/revocation requests are evidence and policy inputs, not magic erasers.
9. Signer-bound spaces prove a key signed an address-bound value under a specified encoding and verification rule. They do not prove a human, account, tenant, or equivalence to another key. SSO (#9077), tenant authorization (#9083), and client trust policy are separate checks.
10. Delegation grants narrowly-scoped authority; transfer replaces authority only through an explicit, accepted transfer record. Cross-signing establishes a mutual key-link assertion, not automatic identity equivalence or account merging.
11. Universal signature + validation gate is a primitive invariant, not a configurable behavior. A local producer signs an event before passing it to the same validator used for remote ingress.

## 2. Version placement

| Version | Part A scope |
|---|---|
| **V1 — existing** | Existing domain-specific signed events/chains, Iroh document namespace and sync/relay mechanisms, group/world authorization, peer discovery, query-local/remote behavior, redb-backed blob-store mode plus distinct authoritative event/world stores, and Atlas's separate content-hash/search pipeline. There is **no demonstrated uniform cross-domain namespace registry, generic federation protocol, or general triples/append-list API**. Do not flatten current mechanisms into “no namespaces/federation” or “ID is on redb”; see summit baseline audit for bounded source evidence. |
| **V2 — primitives** | Canonical triple term model and storage adapter contract (#9125); bounded triple-pattern API and query cursor (#9126); signed attribute objects and append admission state machine (#9127); `succeeds` edge/DAG core and competing-head projection (#9129-01); client-verifiable signer-bound space derivation (#9131). V2 does not require a social application model. |
| **V3 — social substrate** | Nested topic/comment objects, complete-feed projection ledger and freshness cursor (#9128); default ancestor-comment carry projection over the V2 version DAG (#9129-02); moderation workflows, deletion policy adapter and scoped delegation / accepted transfer (#9130, #9132). Cross-service federation/SSO/tenant policy are integrations, not silently included. |
| **V4 — horizon** | Optional SPARQL-like syntax, broader property paths/aggregates, graph visualization clients at scale, richer moderation interoperability, cross-signing UX and policy bundles. This is not a reason to block the V2 triple-pattern API. |

## 3. Shared vocabulary and authority

### 3.1 Types and nonrecursive identity (logical contract; concrete wire encoding owned by #9125)

```text
ContentId = { profile_id, digest: Hash(exact_profile_bytes) }
PayloadId = Hash(raw_payload_bytes)
LogicalGraphId = stable signed logical identifier (not derived from assertions)
EnvelopeId = Hash(exact_complete_canonical_signed_envelope_including_signature)
AssertionId = EnvelopeId(assertion envelope)
SnapshotId = EnvelopeId(snapshot envelope)
ObjectRef = ContentRef(profile_id, digest) | SignedEnvelopeRef(EnvelopeId)
SignerId = PublicKeyId
Term = { type_tag: uvarint, bytes: opaque byte string } | ObjectReference(ObjectRef)
Triple = { subject: Term, predicate: Term, object: Term }
Assertion = { graph: LogicalGraphId, triple: Triple, source_record?: EnvelopeId }
Frontier = opaque canonical set of (signer, sequence, head_envelope_id)
```

Terms are a general typed-bytes foundation, not RDF-only. `ObjectRef` is an explicit separately tagged reference, never a string coercion. Equality is `(type_tag, bytes)` equality; a byte string that looks like an IRI is not an IRI. `source_record` references already identified source evidence, never the enclosing assertion's own ID. An optional RDF codec adapter may map a declared subset to RDF IRIs/literals/blank nodes; its blank-node labels are scoped to one signed payload/graph and must be skolemized or remain local when exported. Do not make RDF typing mandatory for S4 array-address values.

Identity preimage rules are shared with the summit cross-part contract: `ContentId` hashes the exact bytes under its named profile; `PayloadId` hashes raw payload bytes; a new signed record has canonical `Body` committing to payload ID, length, domain and context, with detached signature over domain-prefix + canonical Body. Body excludes its own signature and envelope ID. `EnvelopeId`/`AssertionId`/`SnapshotId` hash the complete canonical signed envelope including signature; IDs are never included in their own signed preimage. `LogicalGraphId` and signer-bound space address are stable logical identifiers separate from assertion content hashes. Use deterministic CBOR for the new profile; legacy artifact JSON bytes/signature profile remains untouched with compatibility vectors, including the documented-vs-implemented preimage mismatch tracked by its owner. A conflict is surfaced, never coerced or silently re-signed.

### 3.2 Validation pipeline

`Receive(bytes, source, tenant_context) -> Result<ValidatedObject, Reject>`:

1. Enforce the selected initial deployment profile’s envelope and byte ceilings before allocation; parse canonical versioned encoding.
2. The local producer signs the object **before** attempting admission. Verify content hash and signature over the canonical envelope/body; bind author to the verifying key. Local producers use the same gate; there is no unsigned local bypass.
3. Validate schema, term-position constraints, references and namespace/profile version.
4. Authorize signer against the destination server/tenant and relevant graph/attribute policy; authentication is not authorization.
5. Admit durably as signed evidence, then update derived indexes/projections transactionally or mark projection lag explicitly.

No mode, trusted peer, server-admin, local loopback, or adapter bypasses steps 1–4. A reject is typed, observable, and does not partially mutate accepted state. Duplicate valid object IDs are idempotent; same ID with differing bytes is `HashCollisionOrCorruption` and fail-closed.

## 4. #9125 — triples encoding and storage

### 4.1 Chosen seam, not premature engine lock-in

The API is an ordered stream of independently verifiable records; storage adapters implement `append`, `scan(pattern, cursor, limits)`, `get(object_id)`, and `checkpoint`. A record boundary is independently decodable. The logical triple model does not inherit one engine’s private file format. The research bake-off is a tiny stdlib experiment and is not a production redb/SQLite/JSONB benchmark.

### 4.2 Required contracts

- Canonical signed payload includes `schema_version`, stable `LogicalGraphId`, a count or end marker, and zero or more triples; each term is explicit `type_tag + length-delimited opaque bytes` or a typed object reference. Canonicalization fixes map/key ordering and integer form; it does not normalize opaque bytes. An optional RDF adapter owns RDF-specific normalization and scoped blank-node conversion.
- Signed payload bytes are immutable. Indexes (SPO, POS, OSP at minimum) are rebuildable derived data. A missing/corrupt index cannot change signed truth.
- Each triple/record has a stable identity or deterministically derived offset; resumable reads do not require materializing all records.
- Querying absent predicate/subject filters may require a bounded scan and must report scanned rows/bytes and limit status.
- Schema evolution is additive: unknown tagged terms can be preserved opaquely; unknown required schema version is rejected with `UnsupportedSchema`.

### 4.3 Alternatives evaluated

See [`research.md`](research.md) for dated sources and toy measurement. Candidates: newline/pretty JSON (human-friendly but not canonical signed storage and poor size/escape costs), canonical compressed streamable binary (preferred for signed exchange; still requires separate indexes), SQLite JSONB (portable SQL engine, but SQLite marks JSONB private and most operations O(N)), normalized byte-key→byte-value map (efficient lookup/indexing, but atomic multi-index commits and key layout complexity). Keep content encoding distinct from persistent DB layout.

## 5. #9126 — graph query and visualization

### 5.1 V2 query API

```text
TriplePattern = { graph?: LogicalGraphId, subject?: TermPattern, predicate?: TermPattern|Variable, object?: TermPattern }
Query = { patterns: NonEmptyList<TriplePattern>, projection: List<Variable>, order: StableOrder?, limit: u32, cursor?: Cursor }
QueryPage = { rows: List<Binding>, next_cursor?: Cursor, snapshot: Frontier, complete: bool, cost: { scanned: u64, returned: u32 } }
```

Initial semantics are conjunctive basic graph patterns only; no updates, property paths, remote federation, implicit entailment, joins against private/unvalidated data, or unbounded sort. Deterministic joins return a stable total order `(logical_graph_id, typed_subject_key, typed_predicate_key, typed_object_key, assertion_id)` unless caller specifies a supported indexed order. An operator-validated **initial deployment profile** supplies finite ceilings (initial suggestion: page default 100, row ceiling 1,000, response 8 MiB, 250 ms CPU-equivalent, 1,000,000 index steps); these are not protocol maxima. Operators may raise finite ceilings only with workload/security evidence and deployment validation; query requests cannot raise them. A system that cannot cap an abstract triple count or represent an arbitrarily large logical graph uses chunked objects plus signed manifests and paged operations rather than a protocol-level “infinite message.” Exceeding profile budget returns `BudgetExceeded` with no claim of completeness.

Errors: `InvalidPattern`, `UnsupportedQueryFeature`, `UnauthorizedGraph`, `CursorExpired`, `CursorMismatch`, `BudgetExceeded`, `UnsupportedSchema`, `IndexUnavailable` (safe scan allowed only if within budget). Read-only API is separate from append/update surfaces.

### 5.2 Language and visualization options

Evaluate direct library triple API, a small typed JSON query AST, SPARQL subset, and Cypher-like property-graph syntax. Recommendation: library + typed AST in V2; do not yet promise full SPARQL semantics or language lock-in. Visualization consumes `QueryPage`/provenance, never a second graph store. UI emits explicit node/edge references, distinguishes asserted vs projected edges, identifies cycles, missing references, hidden-by-policy records, and incomplete pages. Layout is best-effort, not a signed or canonical graph result.

## 6. #9127 — user attributes and append-list protocol

### 6.1 Object classes

`AttributeDefinition` is a signed owner/group object at `(owner_space, target_ref, attribute_name)` with kind, schema, audience, append mode (`closed | open | moderated`), admission rule, retention/deletion policy ID and revision. It is not Atlas metadata or content-location metadata. Attribute values/submissions are separate signed content-addressed objects; relation edges are mutable signed associations/projections. `name` on a user’s path and `comments` on a path are examples, not reserved global properties.

### 6.2 State machine and ordering

```text
Submit(signed_object, payload_id, attribute_ref, idempotency_key)
 -> verify signature/hash/schema and tenant + attribute scope
 -> Rejected(reason) | Pending(validated_submission_id) | Accepted(server_epoch, server_sequence, admission_id)
```

The producer signs locally before submit; invalid signatures never enter pending or accepted state. `open` means an eligible signed submission may be submitted, not that it bypasses moderation. **Every non-group submission MUST enter `Pending`, including on an open attribute.** Direct acceptance is only for an in-group submitter who is eligible under the attribute’s configured member/admin policy. Pending is validated signed evidence but is not an accepted/published list item. `closed` rejects append. Approval atomically rechecks current membership, policy, revocation state and attribute epoch at the commit frontier, then uses compare-and-swap (CAS) to commit exactly one decision plus an accepted sequence; a concurrent policy change/decision returns `DecisionConflict`/`PolicyChanged` and requires review again. Historical sequence/order is never rewritten. Duplicate idempotency key with same object returns prior state; same key with different payload is `IdempotencyConflict`.

Accepted order is `(server_epoch: u64, sequence: u64)` per attribute authority; neither field wraps. At sequence exhaustion, an atomic durable epoch transition advances epoch and resets sequence only if the previous epoch is sealed and its final sequence is recorded. If epoch itself exhausts, stop admission with `SequenceExhausted` and require a signed new-log-identity transition; never reuse an order token. This order is local to that server/attribute, not a global federation order.

Pending is not visible in default accepted feeds. It is visible to authorized reviewers and submitter with state `Pending`; approval/reject/retract are signed actions. Rejection does not erase submitted signed bytes from audit/retention automatically. Errors: `UnknownAttribute`, `Closed`, `SchemaViolation`, `BadSignature`, `HashMismatch`, `TenantDenied`, `NotReviewer`, `AlreadyDecided`, `DecisionConflict`, `PolicyChanged`, `SequenceExhausted`, `IdempotencyConflict`, `CapacityExceeded`.

## 7. #9128 — social object graph

Social shapes are application profiles over nested object references: `Topic`, `Comment`, `Reaction`, `FeedEntry`, optional `Profile`. A topic/comment is immutable content plus signed relation to parent and optional append admission. Every object carries `object_id`, `author`, `created_claim` (untrusted signer claim, not trusted wall clock), `parent_ref?`, `content_ref`, `schema_version`, and optional `succeeds_ref`. Feed ordering and moderation are named projections with provenance, not an inherent order in the graph. Infinite nesting is representable; readers impose a default max display depth 32, max visited nodes 2,000 and max 8 MiB response; continued traversal uses bounded cursors. Cycle-safe traversal uses a visited set and displays back-references rather than recursing.

Freshness is based on one durable projection ledger for each complete feed scope `(server_id, tenant_id, query_fingerprint, projection_generation)`. Every feed entry gets a monotonically increasing `last_projection_seq` and stable `entry_id` from that ledger, even when the entry came from another source authority. The entry also retains source provenance `(source_authority, source_epoch, source_sequence, source_object_id)`; those source positions are not the merged cursor position. A durable per-feed high-water mark continues across projection-generation changes, so `last_projection_seq` is never reused; exhaustion stops allocation rather than wrapping.

`FeedCursor = MAC(server_key, tenant_id, query_fingerprint, projection_generation, last_projection_seq, last_entry_id, snapshot_cutoff_frontier, expires_at)`. The cutoff is the pinned vector of source frontiers included in the complete feed snapshot. `latest` verifies cursor scope/generation; `since(cursor)` returns entries after `(last_projection_seq, last_entry_id)` up to the cursor's pinned cutoff; `before(cursor)` pages older entries by the same stable total order. On restart, the ledger and high-water mark are durable; loss/rollback rotates generation and returns `CursorExpired`, never “unchanged.” Access, moderation, deletion or query-policy changes advance generation and invalidate prior cursors. Two source attributes may each be at source epoch/sequence `(e,1)`; their entries still receive distinct projection-ledger sequences, so neither is omitted or conflated. The cursor proves freshness only for this server, tenant, query and pinned source-frontier vector; it says nothing about disconnected remote peers. [`feed-cursor-oracle.py`](feed-cursor-oracle.py) exercises source-attribute collisions, pinned cutoffs, stable paging, restart and generation invalidation as a deterministic model fixture, not a runtime verification.

## 8. #9129 — `succeeds` versioning

S5.42 direction is explicit: version 1 → version 2 is the triple `(predecessor_v1, succeeds, successor_v2)`. A reverse index from v2 to v1 may accelerate predecessor/ancestry lookup; it does not change stored edge direction. Existing `Supersede` is an adapter mapping to this edge only when its target and successor semantics are proven equivalent; it is not rewritten in place. Edges cite versions with the same logical object key and compatible schema; self-edge is invalid. The relation graph may branch; cycles are rejected at admission if ancestry is complete, otherwise flagged `VersionLineagePending` until referenced ancestry is fetched and verified. A head is a version with no accepted successor in the selected view; multiple heads are valid. Default view chooses a head by explicit policy: owner-signed selection if present; otherwise show branch set (no timestamp or server-arrival tie-break winner). A product may choose a presentation winner but must label it and retain branch alternatives.

Comments attach to the exact version ID. **V3 social projection** carries ancestor comments forward onto a selected descendant with an old-version marker and `origin_version` preserved transitively; this does not rewrite the comment target or imply approval for revised text. Access, moderation, deletion and tenant policy continue to filter carried comments. Branch projections include only comments in that branch’s ancestry; deduplicate by immutable comment ID so transitive projection never duplicates a comment. V2 defines only the `succeeds` edge/version-DAG core; V3 owns this carry-forward view behavior. Editing requires owner/admin authorization under relevant graph policy, plus a new valid signature. Errors: `WrongLogicalObject`, `InvalidSuccessor`, `LineageCycle`, `LineagePending`, `UnauthorizedEdit`, `ConcurrentHeads` (informational result, not rejection).

## 9. #9130 — moderation, deletion, and GC

Keep four distinct states: `DeleteRequested` (author/group request), `ModerationQueued` (review workflow), `HiddenInProjection` (policy/view outcome), and `TrueDeletedOnThisServer` (storage lifecycle). Requests and decisions are append-only signed evidence. Server admins may true-delete under published server policy; GC may remove only when reference graph says unreferenced under a named retention/replication horizon; owner/group users can request deletion for next batch. A request never deletes immediately. `please_delete` / `please_moderate` is a bounded, rate-limited queue with stable ticket ID and audit status.

True deletion procedure uses a fencing protocol shared with #9137 GC and #9084 content references: create durable `Deleting(object_id, generation, policy_id)` tombstone with a monotonic fence; stop new reads and reference creation through that fence; atomically validate reference-index generation + retention horizon and claim the delete generation; delete bytes/caches only after compare-and-swap confirms no write/reference can cross the fence. A fresh reference check alone is racy. Stale workers with older fences cannot delete a newly retained object. Unresolved #9084 ref-vs-copy semantics must be consumed before enabling physical GC of shared content. Version deletion can remove serving bytes only under policy; it cannot erase signed/evidence records whose retention policy requires them. Keep a non-content deletion receipt where allowed. This cannot revoke bytes already downloaded, erase unknown copies, or prove global deletion. A host must not advertise global revocation as fact. `Referenced` blocks ordinary GC; admin override must record reason and may leave dangling refs, returned as `MissingObject` not silently substituted. Tenant boundary is checked at reference traversal and GC scope; do not let a cross-tenant pointer authorize access.

## 10. #9131 — signer-bound spaces and petnames

Address derivation uses an explicit signed profile: `SpaceBindingV1 = [null, typed_public_key_bytes]` in the namespace address core; it does not imply a UUID. Well-known control lookup is a typed array path, e.g. `[typed_text("id"), typed_text("space-control"), typed_object_ref(space_id), typed_text("events")]`, not an untyped string path or guessed UUID. Client recomputes/decodes under the exact profile version, verifies signature and namespace rules, and checks the designated signer key. The address binding is immutable: rotating/transferring control cannot replace the key at the same coordinates. A successor either proves a signed chain at the original address (without changing the original binding) or creates a new address and publishes an explicitly signed redirect/transfer link from old to new. Client-side validation proves only key control over that binding record. A server may store, index, or serve it but cannot manufacture proof. Petnames are local client mappings `(local_owner, canonical_object_ref -> display_label)`; they are not globally unique and never affect equality, authorization, or signature validation.

Identity resolution returns `KeyIdentity(key_id)`, `Unlinked`, or `LinkClaim { status: pending|mutual|policy-accepted }`; it never collapses keys by name, SSO account, endpoint, tenant, or profile similarity alone. A client UI must display trust level/source separately from the identity key.

## 11. #9132 — delegation, transfer, and cross-signing

Delegation record: `{space, grantor_key, grantee_key, scope, permissions, not_before, expires, nonce, predecessor?, status}` signed by grantor; timestamps are UTC Unix seconds in the signed body. `scope` includes exact typed array-address prefixes; permissions enumerate read/append/edit/moderate/delegate; delegation may not exceed grantor's authority; no implicit transitive delegation unless `delegate` is explicit. Verify each link at one authority frontier; reject cycles and enforce a finite operator-profile chain-depth/work budget (initial suggested maximum depth 16, not a protocol limit); over-budget is `DelegationChainTooDeep`, never partial authority. Replay is rejected by uniqueness of `(grantor, domain, nonce)` at a frontier; exact same signed envelope is idempotent, but same nonce with different body is `ReplayConflict`. Validate `not_before <= verifier_utc + 300s` and `expires >= verifier_utc - 300s` using the trusted verifier clock; after expiry plus 300s the grant is invalid. Revocation is a signed record referencing grant ID, effective at the latest verified authority frontier. Offline clients surface frontier age; the initial deployment profile caps revocation staleness at 24h for **all delegated writes** (operator may lower). Beyond the bound, fail closed for writes; old read attribution may be shown as stale, not current. This freshness policy does not establish global finality while disconnected.

Transfer record replaces the controlling key only after old controller signs offer and new controller signs acceptance over the same transfer ID, scope, nonce, challenge/domain and expiry; transfer is distinct from adding a delegate and never mutates the immutable public-key address binding. If old key is unavailable, recovery is not inferred: use an explicit namespace recovery policy, or create a new space and preserve old as unresolved. Cross-signing requires both keys to sign the **same challenge bytes** binding ordered key pair, purpose/domain, exact scope, nonce and expiry; unrelated unilateral statements cannot be paired. Bilateral publication proves both keys approved that link statement, not that keys belong to one person or should share privileges. `sameAs` triples are data claims and do not create authentication, authorization, or SSO equivalence.

## 12. Tenant, authority, and resource checks

Every object reference is authorized in `(server, tenant, graph, principal, operation)` context. Tenant namespace partitions storage/index/cursor/cache keys. Cross-tenant refs are denied by default; #9083 controls any deliberate sharing. Object existence, query predicates, cursor validation, moderation queues, delegation lookup and GC all use the same tenant gate to prevent side channels. The server validates signature and content regardless of trusted author/peer; group membership or delegation is then evaluated from a selected verified authority frontier. Local owner/admin policy can restrict serving; it cannot make an invalid signature valid.

The protocol has no hard maximum graph size or total triple count. Large bodies are chunked as separately signed payload objects with a signed manifest; interfaces page through them. Deployments must select a validated finite **initial resource profile** for max envelope/chunk, parser depth, visited nodes/page, result rows/bytes, CPU and index steps. Initial suggested values are 8 MiB envelope, 1 MiB chunk, parser depth 64, 2,000 visited nodes, 1,000 rows / 8 MiB per query page, 250 ms CPU-equivalent and 1,000,000 index steps. Operators may raise a ceiling only with workload/security evidence and validation; tenant config may narrow, never exceed its server ceiling. These are operational defaults, not wire/protocol maxima or claims of benchmarked safety.

## 13. Migration, parity, and invariants

- V1 event IDs and signed envelopes remain readable. New triple/attribute records have versioned tagged kinds; do not rewrite prior events or reinterpret old bodies as triples. Preserve legacy artifact JSON signed preimage bytes and compatibility vectors; resolve the docs/code mismatch by an explicit additive profile, never by silently changing/re-signing old artifacts.
- Legacy projections can be dual-read during migration; new write path signs canonical new objects before acceptance. Backfill emits derived index entries with source object ID and is repeatable; it does not fabricate authorship or append-order evidence.
- redb is initial native adapter; SQLite/Turso/hosted adapters conform to same validation and result oracle. Storage engine differences are not a capability tier. If an adapter cannot enforce atomic append/admission, it reports conformance failure rather than approximating silently.
- PBT/MBT invariants: canonical encode/decode round-trip and stable IDs; malformed/noncanonical signatures rejected; equal triples yield equal term keys; indexes rebuilt from evidence equal indexed query; query output independent of insertion permutation at fixed frontier; cursor pages concatenate to same ordered snapshot with no duplication/omission; append sequence is unique/monotonic under concurrent submissions; moderation/deletion transitions preserve audit evidence permitted by retention; version `succeeds` graph is acyclic and all concurrent heads retained; signer-bound proof verifies offline and fails for wrong key/encoding; delegation cannot exceed scope and revocation affects only observed frontier; tenant A cannot read or mutate B through any traversal path.
- Oracles: canonical reference encoder/decoder; pure query matcher over in-memory set; model state machine for append/moderation/version/deletion/delegation; independent signature verification; database index rebuild as query oracle. Benchmarks alone are not correctness oracles.

## 14. Open calls / recorded defaults

No blocking question for Drew is inferred by this part. Defaults above favor backward-safe and explicit authority. **Recorded for summit batching only if it chooses otherwise:** whether V2 requires an RDF codec adapter, whether V2 must ship a query language rather than typed AST, delete policy retention/legal envelope, transfer recovery policy, and the initial operator-profile revocation staleness value. #9077 SSO equivalence and #9083 tenant-sharing are owned elsewhere. #9084 ref-vs-copy remains open; until settled, references are immutable object IDs and physical shared-object GC is fenced/disabled. Current baseline says #6277/#6287–#6290 are open staged designs linked to #6312 plan-of-record, not settled architecture. The Atlas `content_hash` producer is absent from checked-in production code; create an explicit owner-assigned follow-up to define source bytes/profile and compatibility vectors before any producer depends on it. The baseline also notes the signed artifact docs/code preimage mismatch; keep legacy exact bytes stable pending its owner’s additive compatibility resolution.
