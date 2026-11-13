# Part A: ID/Atlas addressing primitives — versioned design

Status: additive design expansion for #9106 Part A, components #9113–#9124. Companion evidence matrix: [`research.md`](research.md). This file does not supersede the intake proto. Normative statement IDs are from [`../../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md`](../../2026-11-13T00-41-00Z_design_id-atlas-primitives-federation-namespaces/note.md).

## Scope and non-goals

This design covers shape dispatch, content IDs, sidecars, acknowledgements/locations, namespace registries and URI syntax, array addresses, shorthand and streaming interfaces, names/lenses, numeric derived registries, stream addresses/lifecycle, and signed coordinate worlds. It establishes exactly two address cores: immutable content-addressed objects and mutable array-addressed records. String-path names, numeric registries, streams and signed worlds are derived namespaces/lenses over those cores, never a third core.

This is not the design for SSO (#9077), tenancy (#9083), or the reference-versus-copy decision in #9084. It specifies seams: tenant/authority context is mandatory input to authorization; storage returns opaque references and the higher-level policy decides reference/copy. No content location or URI implies authorization.

## V1 — current behavior (bounded source audit and snapshot evidence)

**V1.1** The bounded baseline audit (`../baseline/v1-audit.md`, checkout SHA `a0f3a529d3e167df0f614a598dc0c9f7aa532b0c`) establishes several distinct V1 surfaces. `pkgs/id/crates/id-store/src/store.rs:7-16,48-60` uses Iroh `FsStore`/`MemStore`; the hash root is unkeyed BLAKE3 over exact input bytes and 32 bytes (`id-store/src/documents.rs:83-145` plus the pinned upstream `iroh-blobs 0.103.1` source). Event payload commitments, signed artifact objects and world module IDs/signatures use distinct typed commitments (`id-events/src/event_store/ingest.rs:452-462`, `id-sync/src/artifact.rs:107-159`, `id-worlds/src/world_session/mod.rs:64-99`). Do not collapse these as one hash profile. File names/tags map to hashes; they are not content identity. `pkgs/id/ARCHITECTURE.md:57-80,180-201` describes tag indexes and tuple encoding.

**V1.2** Existing signed events/documents/artifacts have family-specific signing and receipt validation; local authoring signs before receipt. Neither that evidence nor transport authentication establishes a universal signed-and-validated acceptance gate across every ingress (`../baseline/v1-audit.md:13,31`). The new gate is an invariant of this design: every admission, including locally produced content, verifies a signature, signer binding, schema and required policy before commit. Local producers sign first and then use the same gate; no local bypass. `pkgs/id/src/key_envelope.rs:1-5,24-35,83-110` bounds untrusted KDF parameters before Argon2 work, a useful resource-cap precedent.

**V1.3** V1 already has bounded domain-local namespace, identity, group/world authorization, sync, peer discovery, relay and remote-query mechanisms; these are not a uniform cross-domain namespace registry or generic federated-query protocol (`../baseline/v1-audit.md:15,35-39`; `../baseline/seam-map.md:7-14`). The new `id://` and array-address designs must coexist with Iroh doc IDs, world IDs, account keys, group scopes and name/tag APIs. Existing ID sync is protocol-specific and is not itself generic query federation. Hosted worker routing is separate from addressing: `pkgs/id/hosted-worker/src/worker.ts:3-11,31-47` trusts internal authenticated context, strips user-provided internal headers, then routes; `user-do.ts:8-23,70-104` fails closed on absent/mismatched binding or jurisdiction. New addressing APIs accept authenticated context but independently check namespace/tenant binding.

**V1.4** No claim is made that existing code implements complete signed metadata sidecars, the proposed generic namespace registry, `id://`, a general array-address namespace, numeric registry, stream namespace, or signed coordinate worlds. Existing tuple encoding is a storage-key codec, not the general array-address core; existing event/world/group mechanisms retain their own contract. The checked-in Atlas source has `content_hash` schema/consumers but no producer. Read-only external snapshot inspection found 101,471/101,471 `corpus.db` chunks and 6,941/6,941 `topup-chunks.db` chunks match lowercase SHA-256 of exact UTF-8 chunk text; this is a fact about those snapshots, not proof of the writer implementation or its stability (`../baseline/v1-audit.md:18,22-24`). Atlas embeddings are derived vectors and do not receive `content_hash`. #9113 must retain this Atlas snapshot profile as a distinct typed profile/foreign ID seam; it must not conflate the Atlas text chunk ID with raw-byte Iroh ContentId or vector output identity.

## V2 — namespace registry, both cores, shape dispatch, sidecars and locations

**V2 binding:** #9116 registry, #9117 URI, #9118 array core and #9120 content core are V2 work, alongside #9113 shape dispatch and #9114–#9115 sidecars/locations. This follows the proto's V2 “cores” plan. #9119's shorthand and bounded query interface is also a V2 consumer seam. Names/lenses and numeric registry are V3; streams are V3; signed worlds are V4.

### Ownership boundary: #9113 vs #9118 vs #9120

- **#9113 owns shape/profile registry and dispatch only:** parse type, version, static/configured profile selection, portability and backward compatibility. It does not own a second content storage core.
- **#9120 owns the immutable content-addressed core:** exact bytes, content insertion/retrieval, digest recomputation, immutable lifecycle and the storage adapter seam. Initial profile is fixed below.
- **#9118 owns the mutable array-address core:** nullable ordered byte slots, ordering/equality, value encoding and array store semantics.
- **#9117 owns syntax transport only**; it serializes the above typed IDs but does not mint IDs or establish authority.

### #9113 — Extensible ID shapes and content portability

**A-9113.1 (S3.1–S3.3)** Parse IDs into a tagged sum, not a string-prefix heuristic: `Id = ContentId | ArrayId | DerivedId`. Each has an explicit `shape_id` and version; unknown shape/version returns `unsupported_shape`, not a guessed fallback. Adding a shape is additive and does not change the interpretation of already stored bytes.

**A-9113.2 (S3.4–S3.10)** Initial `ContentId` profile is `raw-blake3-256`: Iroh's unkeyed BLAKE3-256 over the exact bytes, 32-byte digest, no application prefix, normalization, metadata, location or media fields. Profile dispatch is static/configured parsing, not a per-object translation map or registry lookup. Preserve existing Iroh binary/text forms unchanged. SHA-256 signed-artifact commitments remain a separate domain-specific ID, not a content-profile conversion. Atlas snapshot data has a separate observed `atlas-chunk-sha256-utf8` profile (lowercase SHA-256 over exact UTF-8 chunk text); its producer source/stability contract remains unverified, so this is not a universal profile for future Atlas writes. Other profiles are additive and explicit.

**API:** `parse_id(text|bytes, profile_policy) -> Id | IdError`; `verify_content(ContentId, byte_stream) -> Verified | DigestMismatch | UnsupportedProfile`; `format_id(Id, encoding_profile) -> text`.

**Errors:** malformed, unsupported shape/profile, digest mismatch, invalid profile parameters, resource limit. No “unknown digest equals absent” shortcut.

### #9114 — Complete metadata sidecars and mutable head

**A-9114.1 (S3.11–S3.14)** Sidecars are separate signed records, never input bytes for ContentId. A sidecar snapshot is a complete metadata view within its explicit namespace/scope. Suggested envelope:

```text
SidecarBodyV1 {
  subject_content_id, namespace_id, tenant_scope,
  generation: unsigned integer, parent_snapshot_id: Option<SnapshotId>,
  created_at: timestamp (advisory), domain_context, signer_id, key_id, suite,
  payload_profile, payload_id: PayloadId, payload_length: unsigned integer
}
SidecarEnvelopeV1 { body: SidecarBodyV1, raw_payload_bytes, signer, signature }
SnapshotId = EnvelopeId = ContentId(canonical_cbor(SidecarEnvelopeV1))
SidecarHeadV1 { subject_content_id, generation, snapshot_id, signer, signature }
```

**A-9114.2 (S3.15–S3.20)** Settled storage form: immutable `hash.metadata.<snapshot-digest>` signed snapshots plus mutable `hash.metadata` head. `PayloadId` hashes exact `raw_payload_bytes` under an explicit profile. The canonical `Body` commits `payload_id`, payload length, domain/context, signer/key/suite and scoped subject; `Signature = Sign(domain_prefix || canonical_cbor(Body))`. The Body excludes raw payload, signature and its own enclosing envelope ID; subject, payload and predecessor IDs remain signed references. The envelope carries payload bytes alongside the signed body. `EnvelopeId`/`SnapshotId` hashes the complete canonical signed envelope including payload and signature; it is never embedded in the bytes it hashes. Thus there is no recursive self-ID. Filename snapshot digest is this signed-envelope ID, never a timestamp. Head replacement requires CAS on prior `(generation,snapshot_id)`; generation increments exactly once and `parent_snapshot_id` equals prior head. This refines intake's pointer-to-dated-version default: content-derived signed IDs avoid timestamp collisions and make immutable snapshot names verifiable, while the mutable head remains explicit. A timestamp-only, non-CAS mutable file is non-conforming because lost updates and ABA are undetectable. Sidecar subject identity does not define Atlas file/chunk/version identity and cannot substitute for Atlas signed ingest evidence or version objects.

**A-9114.3** New records use a specified deterministic CBOR profile and exact byte-level compatibility vectors. `PayloadId = {profile, digest: H_profile(raw_payload)}`: the raw BLAKE3 profile adds no prefix to the hashed payload. A canonical signed `Body` commits payload ID, payload length, domain and context; detached signature covers the domain prefix plus canonical Body. `EnvelopeId`/`AssertionId`/`SnapshotId = {profile, digest: H_profile(canonical signed envelope including signature)}`; the enclosing ID is never part of its own Body/preimage. `LogicalGraphId`/space address is stable identity separate from assertion content hash. Legacy artifacts remain their existing profile: SHA-256 over `serde_json::to_vec(SignedBody)` and Ed25519 over those bytes. Source docs' claimed domain-prefix/length framing differs from inspected code; a dedicated compatibility/reconciliation slice must document vectors and source mismatch without changing/re-signing/re-hashing old records. Unknown critical fields reject. Derived indexes/caches are rebuildable projections over validated signed authority; any derived record independently retained or transported has a signed service-produced envelope.

**Store seam:** `put_immutable(key, bytes)`, `get(key)`, `compare_and_swap(key, expected_etag_or_generation, bytes) -> Applied | Conflict`. Adapter declares CAS support and compare token semantics. A provider ETag is opaque and not assumed to equal a content hash. If CAS unavailable, single-writer serialization is explicit or update fails `concurrency_unsupported`.

### #9115 — Existence acknowledgements, location harvester, privacy

**A-9115.1 (S3.23–S3.25)** Reuse Part B / **B19's V2 shared receipt contract**; B11 is Diamond Types research, not a receipt schema. `ExistsAck` uses `verified_present` and binds responder identity, subject digest, tenant/namespace scope, policy epoch and observation time. A signed `stored_unverified` inventory observation is a distinct assertion about unverified storage, never proof of accepted/validated content. The assertion itself must pass the signature/authority gate. Observation promises no future retention, availability, authorization or public location. `NotPresent` and `Indeterminate` are separate internal results; policy may coalesce them externally.

**A-9115.2 (S3.26–S3.38)** A valid, authorized, visible location in a complete signed sidecar is always inserted into the corresponding per-scope location index; there is no opt-in field switch that silently omits otherwise-visible locations. Every per-scope sidecar states its completeness scope. Location assertion: `{content_id, occurrence_ref?, source_version_ref?, locator: TypedLocator, namespace, tenant_scope, observed_at, access_class, signer}`. Initial locator variants include `R2 {account,bucket,key}`, `Laptop {hostname,path}`, and `Drive {account,file_id,url}`; each is typed to avoid pretending a local path and cloud URL resolve identically. Updates carry signed stale/delete tombstones and reconciliation cursor; stale or deleted locations are not erased without a tombstone. Indexes are rebuildable projections from validated sidecars and tombstones. Results expose `fresh | stale | deleted | unverified`, provenance and occurrence/version lineage. This is a seam with Atlas lineage/retention/deletion owners, not a replacement (`../baseline/seam-map.md:11`).

**Privacy rule:** default visibility is private within caller-authorized namespace/tenant. Queries filter authorization before ranking or returning candidate counts; public enumeration is prohibited unless explicitly configured. Avoid object keys containing raw tenant IDs, user paths, petnames or coordinates. Cache and logs redact query identifiers according to deployment policy. Configurable retention and export controls do not alter the identity semantics.

## V2 — registry and address-facing interfaces

### #9116 — Local namespace registry

**A-9116.1 (S4.1–S4.10)** A namespace definition binds a stable local `namespace_id` to `{core_id, handler_id, encoding_profile, read_policy, write_policy, shorthand_policy, schema_version}`. Registry is server-local and may differ by server; the same display name may map differently. API input includes resolver context `server_id` or an explicit registry handle. No protocol path silently consults a global registry.

Registry mutation requires authority over the local registry, is versioned, and is audited. Extension handlers receive a typed core address and validated context; they cannot change core identity, signature requirements or authorization ordering. Plugin failure is typed and fail-closed for writes.

### #9117 — URI syntax (`id://`) and encoding

**A-9117.1 (S4.11–S4.18)** Internal, unregistered URI syntax is settled as:

```abnf
id-uri = "id://" resolver-hint "/" namespace "/v1/" id-body
resolver-hint = (lower-alpha / DIGIT) *62(alias-char)
alias-char = lower-alpha / DIGIT / "-" / "." / "_"
namespace = 1*(unreserved / pct-encoded) ; decoded bytes MUST be valid UTF-8
id-body = content-id / array-id
content-id = "content/" hash-profile-token "." digest-text
hash-profile-token = 1*(lower-alpha / DIGIT / "-")
digest-text = 64lower-hexdig
array-id = "array" *( "/" array-slot )
array-slot = "~" / "b." *( unreserved / pct-encoded )
lower-alpha = %x61-7A
lower-hexdig = DIGIT / %x61-66
```

Examples:

```text
id://local/main/v1/content/raw-blake3-256.0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef
id://local/main/v1/array
id://local/main/v1/array/~
id://local/main/v1/array/b.
id://local/main/v1/array/b.%2F
id://local/main/v1/array/~/b.%00%FF
id://local/wiki/v1/array/b.us%20president.md
```

`resolver-hint` is REQUIRED, lowercase ASCII, and parsed as the URI authority (`//` is not repurposed as a path prefix). It is a registry alias, not DNS authority. The caller selects a local resolver context; its configured alias must match the URI hint exactly or resolution returns `resolver_context_mismatch`. There is no implicit DNS, network discovery, global registry, or fallback resolver. Alias lookup is local configuration, and no per-object hash translation table is involved.

Namespace is non-empty UTF-8 text percent-encoded as one path component. Reject raw non-unreserved bytes, malformed escapes, invalid UTF-8, and `.`/`..` namespace components (to avoid URI path normalization ambiguity). Array `~` means null; `b.` followed by octets means bytes, including empty bytes. Keep ASCII unreserved octets literal for human readability (`b.us%20president.md`); encode all others as uppercase `%HH`. Reject redundant escapes for unreserved slot octets and noncanonical spellings. Thus null, bytes-empty, zero-slot array and absent slot differ. Parse path boundaries before decoding, decode exactly once, never treat `+` as space, and never dot-normalize or re-split decoded payload bytes. `/array` is the empty array address: no trailing slash is permitted. Unknown `/vN/` versions or shape tokens are critical and reject; query/fragment are not part of V1 and reject.

String-family human names are values handled/rendered by namespace registry handlers; stable core URI serialization remains array/content and is not replaced by a second string-path grammar. `uri_codec_reference.py` is the executable reference and deterministic property corpus. This syntax is internal and unregistered; no IANA registration claim is made.

### #9118 — Array-address core

**A-9118.1 (S4.23–S4.28)** `ArrayAddress = list<Option<Bytes>>`, with order significant. `[]` is valid zero-length address. `None` is a meaningful null slot; `Some([])` is empty bytes and distinct from `None`; omitted list position does not exist. No fixed semantic maximum. Addresses compare by componentwise typed ordering (null before bytes, bytes unsigned lexicographic then length) and then shorter equal-prefix list first. Serialization is versioned, lossless and round-trips exactly. URI tokens are `~` for null and `b.` plus percent-encoded octets for bytes.

**A-9118.2 (S4.34–S4.40)** `Value = { type_id, payload_bytes }`. Type ID is a stable registered URI-like identifier or numeric code scoped by serialization profile; type tag participates in equality. Distinct types with identical payload are unequal (e.g., integer 1 and text "1"). Arbitrary object values are serialized by an explicit codec profile; unknown non-critical type may be preserved opaque, unknown critical type rejects. No implicit stringification.

**Property:** `decode(encode(x)) == x` for all admitted typed values, and `encode(decode(bytes)) == bytes` for canonical bytes. Encode null, empty bytes, empty tuple, zero integer, string `""` as distinct vectors.

### #9119 — shorthand and streaming query API

**A-9119.1 (S4.29–S4.33)** Bare string shorthand expands using namespace configuration `{default_slot: index, prefix_fill: null, text_encoding: UTF8}`. `default_slot` is per namespace, can be overridden per server, unset by default. Example slot 2 yields `[null,null,utf8("name")]`; this is example configuration only. Never hard-code slot 2. Missing config yields `missing_shorthand_policy`, not fallback.

**A-9119.2 (S4.41–S4.44)** `query(address, max_items, cursor?, snapshot?) -> AsyncStream<ResultPage>` where page has rows, next cursor, `truncated`, snapshot token and terminal reason. `max_items` is a result-page cap only, not a cap on valid stored addresses or total matching results. Cursor is opaque, authenticated, bound to tenant, namespace, query, sort profile and snapshot. Stale cursor returns `cursor_expired`; changed policy returns `cursor_context_changed`; cancellation is observable. Backpressure propagates; production has configurable byte/page/deadline budgets.

### #9120 — content-address core

**A-9120.1 (S4.19–S4.22, S4.20)** Core 1 is immutable bytes addressed by ContentId; Core 2 is array address. Registry families do not instantiate other cores. Content ingest computes the declared hash while streaming and commits only after complete digest match. Partial upload is staging state, never an accepted object.

## V3 — names, numbers and streams (derived families)

### #9121 — String-path names, lenses and authority groups

**A-9121.1 (S4.45–S4.53)** A name is a sequence of UTF-8 path components plus a namespace-specific comparison profile; retain original spelling. It resolves to a set of candidate assertions, not a hidden single winner. Search results return candidates, source/provenance, signer, scope, freshness, and selected lens. Unicode normalization/case-folding is opt-in per namespace profile and never mutates stored text; collisions remain representable.

**A-9121.2 (S4.54–S4.63)** Lenses are named, versioned projection functions over authorized evidence: `local_overrides`, `total_popularity`, `interaction_cohort`, `standing_cohort`, `authority_group(group_id)`. Candidate set is stable; ordering tie-breaks by declared score, then signer ID, then candidate ID. “Standing” is an opaque, issuer-scoped claim queried through policy, not a global numeric trust metric. Groups sign curation assertions and are voluntary; multiple groups may overlap; a client may select a group without granting it global authority. Write authority remains namespace policy and signer authorization.

### #9122 — Numeric registry family

**A-9122.1 (S4.64–S4.73)** Numeric key is typed `NumericValue = Integer(BigInt) | Decimal(coefficient: BigInt, exponent10: BigInt) | BinaryFloat(sign, significand: BigInt, exponent2: BigInt)`. Decimal normalizes by stripping base-10 factors from coefficient and adjusting exponent; binary float strips factors of two from nonzero significand and adjusts `exponent2`. Both canonicalize all signed zero to `+0`. NaN and infinities are rejected in the ordered numeric registry; an optional IEEE-754 compatibility profile may preserve them only as typed, non-ordered values outside that registry. This honors infinitely growable magnitude/precision while keeping ordered numeric keys.

Kinds compare by stable type tag first (`Integer < Decimal < BinaryFloat`), then within-kind numeric reference comparator; there is no implicit cross-kind coercion. BigInt comparison is sign then magnitude. Decimal/binary-float magnitude comparison first compares effective digit/bit length, then lexically compares virtual zero-padded significands; implementation must not allocate `10^exponent` or `2^exponent` for ordering. If an adversarial exponent exceeds configured work/byte budget, return operational `resource_limit`, not a semantic range error. Domain has no fixed maximum magnitude/precision; operational limits are configurable. Conversion between kinds is explicit and carries rounding/loss policy.

### #9123 — addressable live streams

**A-9123.1 (S4.74–S4.82)** `StreamDescriptor = {stream_id, namespace_id, media_type, creator, created_at, state, current_seq, final_ref?, generation}` stored at an array address. States: `open`, `paused`, `finalizing`, `completed`, `aborted`, `expired`. Chunks are immutable content-addressed objects; each signed chunk record binds stream ID, tenant scope, sequence, previous record ID, payload ID, media metadata and signer. Multiwriter streams require namespace-configured sequencer/lease authority; no unordered “last write wins.”

Text, typed chat contributions, voice and video use the same lifecycle, with modality-specific chunk codecs. Read API can subscribe from cursor, replay retained chunks, receive heartbeat/watermark and terminal event. Resume cursor is sequence + chain-record ID, not timestamp. Duplicate chunk is idempotent only for same `(stream,sequence,record ID)`; differing record is conflict/equivocation evidence.

Completion CASes `open -> finalizing -> completed`, with final content/manifest reference and signed completion record. A finished stream resolves via typed `Completed { final_id }`; following is client-selected, not implicit HTTP 3xx. Resolver checks current tenant/scope policy again on final object. Cycles/redirect loops are impossible by typed result; if URI adapters expose HTTP redirect, cap hops and preserve origin security checks as HTTP requires. Aborts/expiry do not claim final content.

## V4 — signed coordinate worlds

### #9124 — signer-local worlds and horizons

**A-9124.1 (S4.83–S4.93)** `WorldDefinitionV1 = {world_id, signer_id, signer_key_id, core=array, coordinate_profile, root_ref, presentation_profile, version, parent_definition?, issued_at, valid_from?, valid_until?, key_history_ref?, signature}`. Signature covers deterministic serialized payload and domain-separation string `id-world-definition-v1`; verifies the exact bytes, signer binding and required namespace/tenant scope. World coordinates are array addresses in that world. A definition commits to an immutable root/manifest; updates are new signed versions, not in-place edits.

**A-9124.2** Authority is signer-local: a valid signature means “canonical according to this signer for this definition,” not globally canonical, not trusted by every client, and not a privilege over other servers. A remote server can serve its own mapping and label it distinctly. Read-only for others means a client cannot mutate the signed definition or signer-owned root; local replicas can add local pointers/annotations without rewriting signer data.

**A-9124.3** Verifier API returns separate dimensions: `signature_valid`, `signer_identity`, `definition_currentness`, `key_status`, `trust_policy_result`, `root_available`, `tenant_authorized`. Historical signature validity is evaluated against historical key state/proof; current revocation/compromise may invalidate current authority without erasing historical evidence. Unknown key status is not “valid”. Key rotation creates a signed successor/history event; revocation does not silently rebind old definitions.

Presentation/rendering is a client/profile capability. Resolve returns raw data plus declared presentation profile; renderer is sandboxed, has no implicit network fetch, and honors content/tenant policy. Signed definitions cannot cause arbitrary active content to execute.

## Cross-cutting invariants and configuration boundaries

### Invariants (not configurable)

1. Acceptance gate always verifies a cryptographic signature and validates signer binding, schema, namespace/tenant authority, scope and policy before commit. Local producers sign before entering that gate; no unsigned-local exception. (#9106 principles; S1.12–S1.20; #9113–#9124 ingestion surfaces)
2. Content hashes are recomputed on receipt; immutable content identity does not include metadata or locations. IDs preserve shape/profile across transports.
3. Exactly two cores only; derived families are configurations or projections.
4. Null, empty bytes, empty address and absent address component remain distinct.
5. Registry resolution is local-context-bound; no global registry lookup, hidden fallback, or server specialness.
6. Authorization is checked before read/write, harvesting, enumeration, lens scoring and stream finalization; requested tenant/scope must match authenticated context.
7. Signed bytes use a declared deterministic serialization profile and domain separation; unknown critical fields, invalid signatures and malformed encodings reject.
8. Mutable heads use CAS or explicit single-writer serialization; no timestamp-only last-writer-wins.
9. Cursor and stream state is finite operationally and exposes truncation, expiration, cancellation, error and conflict; no silent partial success.

### Configurable behavior (cannot weaken invariants)

Default shorthand slot/encoding; allowed namespace handlers; scope access/visibility and location retention; lens ranking policy and selected groups; numeric presentation/rounding; resource budgets (URI bytes, address bytes/count, stream chunk/window/retention, page size, numeric parse effort); accepted cryptographic profiles during agility windows; and peer/tenant authorization rules. Settings are tenant/server scoped and audit/versioned. None disables signature verification, address type distinction, CAS requirements for concurrent mutable heads, completeness of authorized visible locations, or authority checks.

## Cross-edition, migration and conformance

**Edition parity:** Rust binary and Cloudflare/Turso hosted edition implement identical core parse/validate/verify/accept semantics and typed error outcomes. An adapter may lack a capability such as CAS or streaming; it reports a conformance gap/capability refusal, never a weaker core invariant. Use shared golden vectors and cross-edition property suites.

**Migration:** retain legacy BLAKE3 IDs and existing mutable tag/name references as V1 adapters. A migration record maps legacy reference to new typed ID and is signed/validated; do not rewrite bytes or force reissue. Dual-read, single-new-write is the default bridge; conversion must be reversible or explicitly marked lossy. Sidecar adoption starts by generating signed snapshot from a consistent source read; if no consistent read/CAS possible, preserve legacy read-only until adapter is upgraded. Namespace URI version/profile is explicit and version upgrades add parser support before switching writers.

**Error envelope:**

```text
AddressError {
  code: malformed | unsupported_shape | unsupported_profile | digest_mismatch |
        signature_invalid | signer_mismatch | unauthorized | tenant_mismatch |
        namespace_unknown | missing_shorthand_policy | ambiguous_name |
        resource_limit | concurrency_unsupported | conflict | cursor_expired |
        cursor_context_changed | stream_terminal | key_status_unknown,
  retryable: bool,
  context: optional safe diagnostic fields
}
```

No error response includes hidden tenant/name/location material. `ExistsAck`, `NotPresent` and policy-masked response are intentionally distinct internally but can be externally coalesced by access policy.

## Model/property tests and adversarial acceptance

Each implementation slice names a model or invariant oracle in `implementation-slices.json`. Required cross-cutting tests:

- PBT: `decode(encode(address)) == address`; null/empty/zero tuple injectivity; comparator antisymmetry/transitivity; canonical encode stable; hash identity independent of location/metadata; stream chain sequence/hash linkage; namespace parse/format round-trip. Executable syntax reference: `python3 -I uri_codec_reference.py self-test`.
- MBT: registry lifecycle (`create/update/revoke`), sidecar head concurrent update and ABA prevention, stream lifecycle under append/finalize/abort/expiry races, signed world/key rotation/revocation and local override.
- Cross-edition oracle: same golden input vectors produce same typed outcome/error and same canonical signed bytes in Rust and hosted TypeScript; no platform-specific coercion.
- Adversarial vectors: malformed/overlong percent escapes, `%2F` delimiter confusion, double decoding, plus-as-space, encoded dot segments, Unicode confusables, truncated CID/hash, trailing bytes, unknown critical type, duplicate CBOR keys, noncanonical signature payload, signer substitution, tenant ID substitution, sidecar stale-head overwrite/ABA, signature replay across namespace/domain, expiration boundary, hidden-location count leak, cursor replay under another tenant, stream fork and duplicate sequence, completed-stream target permission downgrade, infinite/nested numeric exponent causing resource exhaustion, and renderer active-content/network fetch.

## Settled summit rulings and remaining external seams

1. **#9117:** internal V1 URI syntax is settled above; required lower-case registry alias is URI authority bound to caller-selected local resolver. Syntax is unregistered; no IANA claim.
2. **#9113/#9120:** initial content profile is raw exact-byte Iroh BLAKE3-256. #9113 owns profile/shape dispatch; #9120 owns immutable content core. Artifact SHA-256 commitment remains domain-specific. Atlas producer contract stays unresolved.
3. **#9114:** deterministic signed CBOR snapshots plus digest-derived immutable names and CAS generation/parent head. Snapshot digest hashes complete canonical signed envelope, not its signature preimage. This intentionally refines intake's timestamp-suffixed candidate.
4. **#9115:** ack shares Part B B19 V2 receipt contract; every valid authorized visible location in complete sidecar is indexed; explicit tombstones reconcile stale/deleted locators.
5. **#9118/#9122:** array URI distinguishes null, bytes-empty and empty array. Numeric domain uses unbounded integer/decimal/binary-float significands and exponents, canonical positive zero, rejects non-finite values in ordered profile, sorts by type tag then within-kind comparator, with no coercion.
6. **Follow-up contracts:** Part B B19 is input for receipt schema; check current #6277/#6287–6290 staged designs and #6312 status; obtain Atlas `content_hash` producer evidence; preserve legacy artifact serialization/signature bytes while resolving docs/code preimage mismatch. No SSO (#9077), tenancy (#9083), or #9084 reference-vs-copy decision is made here.
