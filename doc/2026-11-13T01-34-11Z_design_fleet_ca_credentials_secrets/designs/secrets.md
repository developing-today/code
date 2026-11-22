# Fleet secrets and credential model

**Status:** proposed design, not implementation or authorization to access/write a provider. **Baseline reviewed:** `34faffbff26615e313488f6d04705a444342e41e` (2026-11-12); the intake census recorded its then-current short revision `6a7843cae598` and live checks. **Scope:** #9158 (canonical model, provider adapters, custody); reconciliation execution is designed in [reconciliation.md](reconciliation.md), #9159.

## Decision in one page

Keep `fleet-secrets` as the single consumer-facing Rust seam, but deepen it from today's name-to-string read chain into a typed, policy-aware credential library in stages. Preserve the shipped `get`/`list` behavior and broker/BWS read path while introducing an optional admin module/crate only when the write path and dependency graph justify it. A canonical `Credential` represents logical identity and known typed fields; it does not become a plaintext database or grant provider permissions. Provider-native IDs/revisions and unrecognized fields remain attached as fidelity data. Provider differences are described as capabilities, not erased behind false uniformity.

Provider identities are distinct: **BWS** is Bitwarden Secrets Manager, **bw** is Bitwarden Password Manager, and **op** is 1Password. Existing code proves only broker/BWS reads; the `op` adapter and `bw` personal-vault adapter are not present, and BWS access does not authorize `bw`. Keep `bw` a separately authorized optional adapter and separately staged inventory. Do not infer `op` scope from a CLI login either.

V1 is current-state evidence, not an implementation proposal stage; all new implementation work starts in V2. Root private material is excluded from ordinary resolver hot caches, prefetch, generic fallback and equality-index machinery. Root signing authority is a separate explicit signer/escrow interface, never ambient resolver discovery.

Versioned architecture:

| Version | Meaning | Inclusion / gate |
|---|---|---|
| V1 — current, verified | `fleet secrets get|list`; broker-first plus BWS CLI fallback for `get`; name-only broker listing; redacted resolved string wrapper; no writes, canonical Credential, `op`, reconciliation, persistent journal, or Fleet SOPS adapter. Broker is a TTL memory cache with per-secret timestamps and read-through. | Existing contract remains stable. #6767 mitigation is merged as PR #8491 (`c676d1c41c07`); deployment and authorized live canary are unverified. Do not duplicate implementation or claim deployed proof. |
| V2 — typed credentials and adapters | Add versioned typed domain model, source-specific adapter interfaces, capability descriptors, explicit scopes/policy, stable logical ID↔native references, lossless sidecars, typed errors and receipts. Start with local synthetic adapter, then authorized BWS and OP; `bw` stays separately authorized. | Read-only and fixture work first. New adapters do not get discovered implicitly; no production-vault inventory/write without applicable authorization. |
| V3 — safe reconciliation | Use a last verified common baseline, typed three-way per-field planner, verified inventory completeness, journal/WAL/outbox, optimistic revisions where available, guarded preconditions where not, item-level resumable saga, `zz.delete`, immutable receipts/history and manual conflicts. | #9159 details. Never claim cross-provider atomicity. Daily timer is downstream of a reviewed initial migration and safe acceptance gates. |
| V4 — federation and dynamic secrets | Add more provider adapters (OpenBao etc.), constrained per-item replication, workload identity and leased/dynamic credentials, capability-based resolution and encrypted continuity replicas. | Research/pilot first; OpenBao is not a current dependency. Transient leases are memory/expiry-bound, never copied as ordinary static passwords. |

V1 code evidence: `pkgs/fleet/crates/fleet-secrets/src/lib.rs:1-28,36-58,139-193,493-604` implements the broker/BWS chain, empty string as failure, redacted `SecretValue`, value-free error details and BWS list-then-get fallback. `pkgs/fleet/crates/fleet/src/secrets.rs:1-19,73-79,110-173` exposes only `get` and broker-name `list`; `get` intentionally prints its value to stdout for compatibility, while errors are redacted. `fleet secrets list` intentionally does not substitute a BWS vault listing. `pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs:1-39,190-266,294-346,348-440,532-577,611-624` implements TTL memory caching, per-secret timestamps/age/source, partial refresh, list-authoritative eviction, read-through and single-flight. PR #8491 (`c676d1c41c07`) merged the #6767 mitigation; deployment and authorized canary are unverified. The broker's list-authoritative rule is cache-install behavior, not reconciliation tombstone evidence.

## V2 domain model and invariants

### Cross-cutting architecture re-review

I agree with the full `architecture-review.md` ADR-S01/S02/S03/S05 and its 2026-11-12 re-review. The separate read, admin, signer and escrow authorities are compatible with a shared opaque logical Credential model. Root signing is not a `SecretRef` operation and must consume ROOT-05's transaction-bound `SigningAuthorization`; root key identity, subject SPKI+algorithm, purpose/profile, validity, artifact-plan digest, allowed destinations, expiry and single-use/replay protection are bound to that operation. The signer returns a transaction receipt; inventory or escrow read never grants signer authority. The three-root-escrow destinations are a closed set owned by ROOT-09/ROOT-10, each independently restorable with the other two unavailable. Root private values stay outside general read caches, prefetch, benchmark datasets, reconciliation and equality indices.

For ADR-S02/A-1, this design uses per-adapter, per-operation `atomic-precondition | serialized-by-contract | unguarded`; BWS, OP and BW default to `unguarded` pending primary documentation and isolated tests. Default conflict-sensitive updates stop as `needs-review`. The stricter default does not erase the mandatory §20.5 history-before/in-place-native-ID saga: a separately approved named weaker policy can execute it while disclosing the residual lost-update window in plan and receipt. Do not promise recovery of a concurrent value an unguarded provider may overwrite. I agree with the challenge re-review's oracle distinction: default preflight→external edit→apply must refuse and preserve the external value; weaker mode is tested for explicit risk disclosure, not impossible preservation. A-2/A-3 are delegated to ROOT-05/ROOT-09/ROOT-10 and are not duplicated here.

The domain object has logical identity; provider records are projections/bindings. Avoid an untyped JSON blob, but preserve unrecognized native data losslessly in an encrypted, versioned provider envelope. This is a design model; exact Rust syntax is implementation-owned.

```rust
struct Credential {
  logical_id: Uuid,                  // immutable; never derived from title/path/native ID
  canonical_ref: CanonicalRef,       // mutable display/search address, versioned grammar
  schema_version: u32,
  policy_ref: PolicyRef,
  class: SecurityClass,
  kind: CredentialKind,
  fields: Vec<Field>,                // ordered, typed, individually sensitive
  tags: Vec<TypedTag>,
  relations: Vec<Relation>,
  attachments: Vec<AttachmentRef>,  // bytes stay in approved encrypted object store
  bindings: Vec<ProviderBinding>,
  lifecycle: Lifecycle,
  provenance: Provenance,
}

struct Field {
  id: FieldId,                       // stable within logical record
  name: String,                      // exact observed spelling retained separately
  role: Option<SemanticRole>,
  value: Observed<FieldValue>,       // Present, Null, Absent, Redacted, Inaccessible, Unknown
  sensitivity: Sensitivity,
  provider_origin: Option<ProviderFieldRef>,
  source_revision: Option<Revision>,
  observed_at: Timestamp,
}

struct ProviderBinding {
  provider: ProviderKind,            // Bws | BitwardenPasswordManager | OnePassword | ...
  account_scope: ScopeId,
  container_id: NativeId,            // project / vault / file scope, opaque and exact
  item_id: NativeId,                 // retained through in-place updates
  revision: Option<Revision>,        // opaque ETag/revision/version clock if provider has one
  last_seen: Timestamp,
  last_verified: Timestamp,
  projection_version: u32,
  native_fidelity_ref: Option<EncryptedObjectRef>,
}
```

`FieldValue` covers string, bytes/attachment ref, bool, number, URI/login component, ordered list, structured value, and an opaque native type. `Observed<T>` is not just `Option<T>`: `Absent` is a verified fact about the source representation, `Inaccessible`/`Redacted`/`Unknown` mean no claim, and `Null`/empty are distinct values. The current legacy `get` deliberately rejects empty strings (#6275); new typed adapter contracts must preserve provider truth, while the legacy `get` compatibility wrapper can continue treating empty as unresolved until a separately reviewed behavior change.

Core invariants:

1. `logical_id` survives rename, path change, provider migration, and value rotation. Native identity is `(provider, account_scope, container_id, item_id)` and is never replaced by a display-name match. Native revisions are opaque and provider-scoped.
2. Every field has typed semantics and confidentiality classification. Unknown provider fields are carried unchanged or the projection is explicitly `lossless_projection_unavailable`; never report fully synced based on password-only equality.
3. `Credential` metadata is not authority. Scope and authorization are enforced before every provider operation and are returned in the result's provenance. `DataClass` must come from trusted policy bound to provider/account/container/native IDs, never a caller-supplied label or mutable provider sidecar. Unknown/adopted records require classification review before replication.
4. Secret bytes and protected history are not stored in SQL event/index rows, Git, logs, errors or generic telemetry. New typed/admin commands return redacted status by default and require explicit reveal/output authority to emit values. The existing `fleet secrets get` stdout behavior is a documented V1 compatibility exception, not a new-code output precedent. A persistent canonical registry may contain redacted metadata and protected refs only; encrypted payload placement needs explicit class/policy.
5. Resolver fallback is deterministic policy, not “try every store until one succeeds.” It must reject a divergent or stale replica unless class-specific freshness and source policy permit it, return source/revision/age status, and avoid silently falling back after authorization failure.
6. No ambient provider selection. Each adapter has an explicit configured scope and auth profile. BWS and `bw` are different enum cases with separate tokens, permissions and user-facing labels.

Suggested kind registry: `Login`, `ApiToken`, `SecureNote`, `SshKey`, `IrohIdentity`, `X509Bundle`, `KeyHandle`, `Totp`, `PasskeyReference`, `RecoveryCode`, `ConfigFile`, `OpaqueBinary`, extensible `Other`. `CredentialKind` is not a promise that every provider supports that kind. URI match rules, TOTP references/seeds, passkey handles, attachments, recovery fields, custom field ordering and visibility are first-class fields, not flattened to a single value.

### Classes, compartments, custody

Use explicit policy classes: `human-login`, `machine-static`, `deployment`, `transient`, `ca-root`, `ca-issuer`, `leaf-key`, `recovery`, `public-artifact`, plus `unclassified`. Replication policy is independent from classification and maps to explicit provider scopes and field allowlists. A `shared` compartment is a named, approved set of logical items/fields and destinations, not a rule that every item is mirrored everywhere. Provider-specific compartments remain provider-specific unless an explicit policy connects them.

**Root escrow is a separate custody workflow, not a generic Credential replication policy.** The addendum explicitly calls for approved initial copies in three distinct destinations: encrypted local laptop storage, authorized BWS project, and authorized 1Password vault. The closed destination set, adapter and independent-restore acceptance are owned by ROOT-09/ROOT-10 under #9163; operation-level approval and legacy activation gates are owned by OPS-10. These are not generic replication destinations and must never collapse into “all providers” or inherit V4 federation. **Activation precondition:** V1 `secrets_broker::do_refresh` lists all BWS records visible to its token, fetches each value, and installs them in the ordinary cache (`fleet-queue/src/secrets_broker.rs:190-260`); the legacy `fleet-secrets` fallback resolves accessible names (`fleet-secrets/src/lib.rs:517-604`), and `fleet secrets get` prints the value to stdout. Before the first root escrow copy, the trusted root policy must bind the exact escrow project/container; root native IDs are registered from each escrow write receipt before any ordinary principal can gain scope. Ordinary broker/runtime principals must not list/get/cache that scope (prefer a separate least-privilege machine grant, excluded from their `BWS_ACCESS_TOKEN`). Defense-in-depth exclusions in legacy broker refresh and name-only fallback must reject/exclude root refs and encrypted root archives before cache ingestion, backed by an original-path synthetic denial test. If legacy scope cannot be proven safe, activation remains blocked. Do not create a BWS project or change grants here; production escrow remains blocked until scope identities and this boundary are verified. Encryption does not downgrade a root archive's `ca-root` classification.

A root key handle is not signing authority; root signing consumes ROOT-05's transaction-bound `SigningAuthorization` for exact signer, subject public-key identity/algorithm, profile/purpose, validity, artifact-plan digest, authorized output/escrow destinations, expiry and anti-replay. Inventory discovery, escrow read, or possession of key bytes cannot mint it. Current assignment authorizes design/synthetic proof only, not generation/export/upload. Cloudflare R2 is research only, not a fourth automatic destination. Active signing custody and escrow archives are different roles; recovery material must not depend only on the archive it decrypts.

Security class policy should default: human-login stays in its assigned OP vault; machine-static remains at its configured source unless approved replicas; deployment ciphertext may be SOPS/age managed with decryption keys outside Git/Nix store; transient is source-issued and memory-only; `ca-root` uses explicit escrow policy only, not generic sync/cache; `ca-issuer` is restricted signer storage; leaf keys are host/service-scoped; recovery is separately escrowed; public artifacts may be published but not trusted just because fetched. This preserves the baseline classes in spec §34.2 (lines 2153-2167).

### Adapter seam and capability model

The interface should be small at the consumer seam and deep in the adapter/coordinator implementation. Separate `CredentialStore` operations from provider-specific protocol details, and expose unsupported atomicity honestly:

```rust
trait CredentialProvider {
  fn descriptor(&self) -> ProviderDescriptor; // version, configured scope, auth scope, tested capabilities
  fn inventory(&self, request: InventoryRequest) -> Result<InventoryPage, ProviderError>;
  fn read(&self, reference: &NativeRef) -> Result<ProviderItem, ProviderError>;
  fn create(&self, item: ProviderWrite, idempotency: IdempotencyKey) -> Result<WriteReceipt, ProviderError>;
  fn update(&self, reference: &NativeRef, guard: RevisionGuard, item: ProviderWrite,
            idempotency: IdempotencyKey) -> Result<WriteReceipt, ProviderError>;
  // No generic delete. If a provider can physically delete, it is never called by normal reconcile.
}
```

`ProviderDescriptor` states per operation (`read/list/create/update`, attachments, rich fields, version/history, native id stability, pagination guarantees, size limits, consistency, rate limits, auth scopes, and whether deletion inventory is authoritative or only read-visible), with tested-at version and evidence. A capability can be `supported`, `unsupported`, `unknown`, or `not_authorized`; unknown is not success. For each mutating operation, separately declare write guard `atomic-precondition | serialized-by-contract | unguarded`; revision existence alone does not prove atomicity. `atomic-precondition` means provider enforcement for all writers; `serialized-by-contract` means every writer is forced through the same serializer; a Fleet-local lock is insufficient if an external editor exists. Pending primary evidence plus isolated tests, BWS/OP/BW remain unguarded. `WriteReceipt` reports accepted/pending/verified, guard level, exact native IDs/revisions, readback proof, idempotency key, partial peers, recovery action and any approved weaker-mode lost-update warning; it cannot claim both stores committed atomically.

Adapter order: (1) deterministic fake provider with capability fixtures and negative tests; (2) preserve and wrap current BWS resolver without changing legacy CLI; (3) authorized BWS Rust SDK or documented CLI adapter after toolchain/MSRV/version and capability verification; (4) OP approved API/CLI adapter, keep source structure and opaque field values; (5) `bw` adapter only after separate access approval and inventory scope; (6) SOPS/age artifact adapter for selected deployment values, not a general remote vault; (7) future OpenBao and others behind optional features. Do not add fnox as a Fleet writer absent proof—it may be read/cache/injection integration, not reconciliation authority. No root key/value access from general runtime path.

Errors are typed and redact values: `InvalidReference`, `NotFound`, `Unavailable`, `PermissionDenied(scope)`, `IncompleteInventory(reason)`, `Unsupported(operation/field)`, `Conflict`, `PreconditionFailed(expected/observed revision)`, `RateLimited(retry_at)`, `CorruptResponse`, `ProviderBug`, `PolicyDenied`, `StaleReplica`, `PartialCommit(receipt_ref)`. Do not collapse 403, pagination gap, timeout, malformed list, unknown container, item missing, and verified deletion into a single `NotFound`.

## Integrity proofs and metadata leakage

Do not compare low-entropy values with public hashes. For permitted fields where equality cannot be checked via provider revisions/readback semantics, compute `HMAC-SHA-256(K_eq_vN, domain || logical_id || field_id || canonical_type_encoding || value)` under a separately protected, versioned key. Root private material and recovery secrets are never inputs to ordinary equality indices. Use constant-time comparison; never send the key to provider/index. Domain-separate equality tokens from audit signatures and encryption keys. Rotate HMAC keys with overlap: retain key-version IDs and re-key only in a controlled metadata migration, never silently treat a key-version mismatch as a field change. Stable opaque `logical_id` is distinct from mutable path/name. Including `logical_id`/`field_id` in domain separation avoids equality correlation across unrelated items; if cross-replica matching needs same token, both replicas share the exact logical/field identity and HMAC version.

HMAC is not a zero-leakage proof: an observer can see which rows/fields changed, equality across replicas sharing the token, repeat frequency, access patterns, lengths if exposed, stable IDs and relationship graph. Even a keyed HMAC can reveal equality to an index reader with token access; if the key leaks, weak passwords can be enumerated offline. Therefore store tokens only in restricted local/index metadata, suppress field-level identifiers where not needed, avoid broad Supabase equality queries, and never log raw digests. An HMAC token must not be called a secret-safe public fingerprint. Access-key fields need the same handling; public certificate fingerprints are a separate public-data case.

## Performance and availability

No local performance conclusion is made here. Measure cold/warm auth, list/pagination, single/batch get, write/readback, p50/p95/p99, startup, memory, error/retry and rate-limit behavior per provider path using isolated synthetic records. Separate backend latency from process startup and network RTT. Existing evidence is architectural only: current fallback runs `bws secret list` then `get` for each `get` call (`fleet-secrets/src/lib.rs:517-602`); broker full refresh performs a list then per-item `get` with eight workers (`secrets_broker.rs:133-266`), cache reads are memory-based, and same-name misses are single-flighted (`:371-440`). These are measurements to run, not benchmark results.

Cache decision is a V2+ explicit policy, distinct from the already-merged #6767 mitigation. Do not duplicate repair work: #6767 remains open pending deployment and authorized canary; PERF-02 under #9160 is the canonical issue. Future cache responses carry source/revision/fetched age; policy selects `never`, memory TTL, encrypted local snapshot, or stale-allowed with class-specific max age. `ca-root`, `recovery`, `transient`, revoked/unknown, or security-critical classes fail closed offline by default. Root material is never in ordinary cache/prefetch/equality indexes. A provider outage is not permission to use arbitrarily stale data. Successful writes invalidate/refresh exact binding; provider revocation has a bounded propagation contract. Stale reads never authorize writes or overwrite a source.

## Issue-ready proposals for domain starter #9158

The following stable proposal IDs are for planning and later issue creation; this document does not create GitHub issues. Each is one deliverable and satisfies the coordinator handoff §4.2 issue content contract. Intrinsic urgency follows the companion charter's bands (lower is sooner) and is not importance, blocking status, dispatch directive or the existing queue priority value. The publisher preserves both numeric score and band in issue metadata/body, then maps to the existing queue scale without changing the PM schema.

### SEC-02 — Define versioned typed Credential schema and compatibility parser

- **Kind / urgency / priority:** design+implementation+test; High, intrinsic urgency 600. **Lifecycle:** Ready (synthetic fixtures only). **Estimate/stage:** M, V2 foundation. **Reqs:** spec §§19.2-19.6, 34.2-34.3, 35.3; V3-S03; `REC-01` depends on this. **Current evidence:** no canonical schema in current `fleet-secrets` (crate description and source above; `Cargo.toml:1-12`). Shared runtime compatibility/type contract is owned by ARCH-01.
- **Outcome:** encode opaque stable logical ID, canonical ref, schema/policy versions, security class, typed kind/fields, `Observed` state, opaque native refs/revisions, relations/attachments, lifecycle/provenance, and lossless unknown-field envelope; no raw value serialization to logs/index. `DataClass` is derived from trusted policy bound to provider/account/container/native IDs, never caller authority or solely mutable provider sidecar metadata.
- **Boundary:** domain types/parser/validator in `fleet-secrets` or approved split crate; no provider call, database, CLI command tree, or production migration. Scope exact as declared. **Acceptance:** stable round-trip and migration tests across versions; serialization distinguishes absent/null/empty/redacted/inaccessible; native IDs and exact unknown fields survive; redaction property for `Debug`/errors; malformed unknown type is preserved opaque or fails with typed error, never dropped.
- **Negative tests:** title/path collision does not merge IDs; `bw` and BWS parse to distinct provider kinds; a caller-supplied class or mutable sidecar cannot downgrade a root-scoped object to ordinary `machine-static`; unknown/adopted class cannot auto-replicate before review; sensitive field is omitted from redacted formatter; attachment bytes never appear in metadata encoding. Oracle: trusted registry fixture plus canonical round-trip and redaction scanner.
- **Deps:** ARCH-01 (runtime secret type/compatibility contract); block REC-01. **Rollback:** keep V1 types and adapter untouched; version parser addition can be removed before persisted V2 data. **Recovery:** reject unsupported schema version, retain source-side bytes/fixture. **Open:** exact opaque ID representation and field normalization registry are implementation-time choices under ADR.

### SEC-03 — Add provider capability and scope descriptors with typed errors

- **Kind / urgency / priority:** design+implementation+test; High, intrinsic urgency 550. **Lifecycle:** Backlog until SEC-02 boundary is agreed; synthetic implementation can parallelize after types settle. **Estimate/stage:** M, V2. **Reqs:** spec §§8.2, 8.6, 35.1, 38; V3-S01, S02, S13, S14.
- **Current evidence:** V1 hardcodes broker/BWS chain; no provider descriptors or write surface (`fleet-secrets/src/lib.rs:139-193,493-604`).
- **Deliverable:** capability descriptor with explicit operation/field/access/deletion-authority/write-guard/support states and tested provider/version/scope; adapter seam for inventory/read/create/update; typed errors and receipt types. Include distinct BWS, BitwardenPasswordManager, OnePassword identities; capability never implies authorization. Consume trusted scope/native-ID classification/exclusion policy and legacy-path denial contract owned by OPS-10/ROOT-09; do not implement competing root custody.
- **Acceptance:** fake adapters exercise supported, unsupported, unknown, not-authorized, `read-visible-only` vs `deletion-authoritative`, and each write-guard level; capability JSON never reports unsupported writes as success; scoped handles validate container boundary; errors redact raw response/auth material. **Negative tests:** BWS scope cannot instantiate `bw`; caller/sidecar cannot downgrade protected root scope; a synthetic root-archive item on the original broker-compatible refresh/name-fallback path is rejected before cache/value return (activation implementation owned by OPS-10/ROOT-09); 403 is permission denied, not not-found; malformed/truncated listing is incomplete; unsupported attachment update fails item, not whole-record success.
- **Deps:** SEC-02 and ARCH-01. **Boundary:** no actual BWS/OP/bw auth, no provider mutation, no secret broker rollout. **Rollback/recovery:** keep legacy resolver path default; capability API is opt-in. **Open:** exact SDK/CLI binding selected only after version/toolchain compatibility research.

### SEC-04 — Implement BWS adapter against explicit project scope

- **Kind / urgency / priority:** implementation+test; Standard, intrinsic urgency 1000. **Lifecycle:** Backlog pending SEC-03; synthetic adapter tests first, with any live isolated project separately authorized. **Estimate/stage:** M, V2. **Reqs:** spec §§8.2, 19.1, 19.6, 35.1, 36.1, 38; V3-S01-S03.
- **Current evidence:** CLI fallback `bws secret list` + `get` exists (`fleet-secrets/src/lib.rs:517-604`); broker does full scoped list and `get` (`fleet-queue/src/secrets_broker.rs:190-266`). BWS value/note/project model is described by spec §19.1 lines 1031-1040; verify current official interface at implementation time.
- **Deliverable / boundaries:** implement explicit project/machine identity BWS adapter with scoped list/read/create/update; preserve value semantics/native ID and expose note/metadata/revision and per-operation guard level. Prefer Rust SDK only after MSRV/API lockfile validation; subprocess adapter uses token file/FD/env, never secret argv. Existing fallback stays until parity/performance review. Implement with fake providers first; any disposable BWS test project is separately authorized. No existing vault sweep or production mutation.
- **Acceptance:** pagination and read-visibility/deletion-authority evidence explicit; list/get agree; synthetic create/update readback and native ID stable; receipt distinguishes accepted from verified; BWS remains `unguarded` for conflict-sensitive update until primary docs plus isolated test prove a stronger guard. Default conflict-sensitive update stops for review; immutable history/create may proceed. Root-escrow project/container and native IDs are denied by trusted policy before the first copy; ordinary broker/runtime principals cannot read that scope; encrypted archive remains `ca-root`.
- **Negative tests:** synthetic root-archive fixture sent through original broker-compatible refresh and `fleet secrets get` path is denied before cache/value return (activation gate owned by OPS-10/ROOT-09); empty/incomplete listing, 401/403, rate-limit, malformed JSON, ambiguous missing item and timeout never count as deletion; unguarded stale update does not auto-apply; no secret values in logs; BWS authorization never selects `bw`.
- **Deps:** SEC-03. **Rollback:** feature-gate and retain V1 read path; do not replace current `get` absent parity and performance evidence. **Recovery:** unknown mutation receipt is reconciled by native ID/idempotency marker before retry. **Open:** SDK suitability, conditional update semantics and idempotency require primary source plus isolated test; actual write target requires separate authorization.

### SEC-05 — Implement 1Password adapter for an explicit authorized vault set

- **Kind / urgency / priority:** implementation+test; Standard, intrinsic urgency 1000. **Lifecycle:** Backlog; synthetic adapter contract first, any live test vault/service-account grant is separate authorization. **Estimate/stage:** L, V2. **Reqs:** spec §§19.1-19.5, 35.1, 36.1; V3-S02-S03.
- **Current evidence:** no Fleet `op` adapter exists per intake note lines 32-39; rich OP field expectations are specified in spec §19.1. This is not proof of accessible vaults or permissions.
- **Deliverable:** read/list then guarded create/update adapter for one configured vault; preserve item UUID, section/field order/type/visibility, tags, URLs, attachment metadata and exact unknown fields. Report any unsupported item/field. Select CLI/API only after current official docs, service account scope, rate limits and readback verified.
- **Acceptance:** fixture or authorized dedicated test vault round-trips all supported fields; scope report names only granted vault IDs; no broader session discovery; no API/session token in process arguments/logs. **Negative tests:** insufficient scope, inaccessible attachment, unknown custom field, rate limit and title collision produce typed error/conflict; do not write another vault or drop field silently.
- **Deps:** SEC-03, SEC-02. **Boundary:** authorized test vault only; no personal-vault census/migration. **Rollback:** optional feature and no default resolver switch. **Recovery:** operation receipt plus native ID readback before any retry. **Open:** CLI vs supported SDK/Connect chosen by separately evidenced capability/entitlement.

### SEC-06 — Specify separately authorized Bitwarden Password Manager adapter

- **Kind / urgency / priority:** design+research; Someday, intrinsic urgency 1800. **Lifecycle:** Backlog; actual access and migration are separately approval-blocked, while public-source design may proceed. **Estimate/stage:** M, V2 later. **Reqs:** spec §§19.1, 36.1, 38; V3-S01, S25.
- **Outcome:** establish a separate adapter/auth/scope boundary for `bw` (personal logins, URI match, custom fields, TOTP/passkeys) and an independent Sweep B decision packet. BWS is not its credential source.
- **Current evidence:** no Fleet `bw` adapter; note.md lines 34-50 classifies access as not authorized. Product distinction in spec §19.1 lines 1031-1040. Do not claim full-vault access from `BWS_ACCESS_TOKEN`.
- **Acceptance:** architecture proves distinct provider enum, token/config type and grant path; public-doc capability matrix with versions; synthetic serialization fixture for rich login. **Negative tests:** BWS credential cannot instantiate password-manager provider, no adapter auto-login, no inventory before scope grant, unsupported passkey export remains unverified rather than synthesized.
- **Deps:** SEC-02, SEC-03. **Boundary:** no access, export or personal vault operations. **Rollback:** no running code enabled; research may be superseded by explicit authorization. **Open:** future authorized user/vault set, API/export restrictions and exact native fidelity.


### Cross-domain prerequisites and canonical issues

- **ARCH-01** owns the shared runtime secret type and existing `get/list` compatibility boundary; SEC-02 must extend that contract rather than open a duplicate issue.
- **PERF-02** is the single canonical owner for freshness, invalidation, singleflight and the #6767 implementation/deployment/canary evidence. Secrets adapters and reconciliation consume its policy; do not open SEC-08 or REC-09 duplicates.
- **ROOT-09/ROOT-10** own the closed three-destination root escrow adapter and isolated recovery proof. This domain models `ca-root` as excluded from ordinary Credential replication/cache/equality index, but does not duplicate root-specific custody implementation or signer authorization.

## Deferred and explicit non-goals

- No live `bws`, `op`, or `bw` inventory/write; no secrets-broker credential read in this design session; no production root key generation, export, sync or signing; no schedule enablement.
- No generic remote credential SQL database; no automatic secret replication to all configured providers; no automatic root signer discovery.
- No assumption that `fnox` is a writable reconciliation engine, that OpenBao is available or that a provider API offers CAS. These must be individually evidenced.
- PKI shape/cryptographic policy and CA issuer/root generation remain other domain issues. Secrets design provides protected references and custody policy, not production CA authorization.

## Source index

- Governing source: `sources/fleet_architecture_spec_v0_3.md` §§8.1-8.6 (lines 592-645), §§19.1-19.6 (1027-1085), §§34-35 (2124-2219), §38 (2269-2278), §48 (2637-2695).
- Refinement: `sources/ADDENDUM_secrets_architecture_performance_infrastructure.md` §§1, 5-6, 8-9 (lines 5-10, 42-67, 82-103). Root escrow's three destinations are explicit; BWS retained; `bw` remains distinct.
- Execution/approval: `sources/COORDINATOR_summit_handoff.md` §§4.2, 7.2, 10 (lines 102-122, 196-199, 267-273); `note.md` lines 24-50.
- Retrieved prior art and provider/API limitations: [`../research/secrets-prior-art.md`](../research/secrets-prior-art.md) (primary Bitwarden BWS/Password Manager CLI and 1Password Connect docs, AWS outbox source; retrieved 2026-11-12; no live account or benchmark access).
- Current code references are given inline above; all cited code references are checkout-local, not claims of external API behavior.
