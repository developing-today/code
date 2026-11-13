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
3. `Credential` metadata is not authority. Scope and authorization are enforced before every provider operation and are returned in the result's provenance. Unknown/unclassified records cannot cross to a new provider by default.
4. Secret bytes and protected history are not stored in SQL event/index rows, Git, logs, errors, default stdout or generic telemetry. A persistent canonical registry may contain redacted metadata and protected refs only; actual encrypted payload placement needs an explicit class/policy.
5. Resolver fallback is deterministic policy, not “try every store until one succeeds.” It must reject a divergent or stale replica unless class-specific freshness and source policy permit it, return source/revision/age status, and avoid silently falling back after authorization failure.
6. No ambient provider selection. Each adapter has an explicit configured scope and auth profile. BWS and `bw` are different enum cases with separate tokens, permissions and user-facing labels.

Suggested kind registry: `Login`, `ApiToken`, `SecureNote`, `SshKey`, `IrohIdentity`, `X509Bundle`, `KeyHandle`, `Totp`, `PasskeyReference`, `RecoveryCode`, `ConfigFile`, `OpaqueBinary`, extensible `Other`. `CredentialKind` is not a promise that every provider supports that kind. URI match rules, TOTP references/seeds, passkey handles, attachments, recovery fields, custom field ordering and visibility are first-class fields, not flattened to a single value.

### Classes, compartments, custody

Use explicit policy classes: `human-login`, `machine-static`, `deployment`, `transient`, `ca-root`, `ca-issuer`, `leaf-key`, `recovery`, `public-artifact`, plus `unclassified`. Replication policy is independent from classification and maps to explicit provider scopes and field allowlists. A `shared` compartment is a named, approved set of logical items/fields and destinations, not a rule that every item is mirrored everywhere. Provider-specific compartments remain provider-specific unless an explicit policy connects them.

**Root escrow is a separate custody workflow, not a generic Credential replication policy.** The addendum explicitly calls for approved initial copies in three distinct destinations: encrypted local laptop storage, authorized BWS project, and authorized 1Password vault. Each destination requires its own configured scope, encryption/access controls, integrity receipt, independent restore test and failure status. These destinations must remain three explicit policy entries—never collapse into “all providers,” never inherit V4 federation replication, and do not discover the root signer automatically. A root signer reference must be explicitly supplied by operation/config; no automatic signer discovery or public-cert-to-private-key inference. Current assignment authorizes design/synthetic proof only, not generation/export/upload. Cloudflare R2 is research as an additional separately approved location, not a fourth automatic destination. Active signing custody and escrow archives are different roles; the design must avoid a recovery key whose only copy is inside the archive it decrypts.

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

`ProviderDescriptor` states per operation (`read/list/create/update`, attachments, rich fields, version/history, server-side CAS, native id stability, pagination guarantees, size limits, consistency, rate limits, auth scopes), with tested-at version and evidence. A capability can be `supported`, `unsupported`, `unknown`, or `not_authorized`; unknown is not success. `RevisionGuard` is `Exact(rev)` only if provider provides a usable conditional write; `PreflightThenVerify { observed_rev }` exposes a non-atomic race; `Unconditional { reason }` is forbidden for protected-field overwrite unless explicit human choice. `WriteReceipt` reports accepted/pending/verified, exact native IDs/revisions, readback proof, idempotency key, partial peers, recovery action; it cannot claim both stores committed atomically.

Adapter order: (1) deterministic fake provider with capability fixtures and negative tests; (2) preserve and wrap current BWS resolver without changing legacy CLI; (3) authorized BWS Rust SDK or documented CLI adapter after toolchain/MSRV/version and capability verification; (4) OP approved API/CLI adapter, keep source structure and opaque field values; (5) `bw` adapter only after separate access approval and inventory scope; (6) SOPS/age artifact adapter for selected deployment values, not a general remote vault; (7) future OpenBao and others behind optional features. Do not add fnox as a Fleet writer absent proof—it may be read/cache/injection integration, not reconciliation authority. No root key/value access from general runtime path.

Errors are typed and redact values: `InvalidReference`, `NotFound`, `Unavailable`, `PermissionDenied(scope)`, `IncompleteInventory(reason)`, `Unsupported(operation/field)`, `Conflict`, `PreconditionFailed(expected/observed revision)`, `RateLimited(retry_at)`, `CorruptResponse`, `ProviderBug`, `PolicyDenied`, `StaleReplica`, `PartialCommit(receipt_ref)`. Do not collapse 403, pagination gap, timeout, malformed list, unknown container, item missing, and verified deletion into a single `NotFound`.

## Integrity proofs and metadata leakage

Do not compare low-entropy values with public hashes. For permitted fields where equality cannot be checked via provider revisions/readback semantics, compute `HMAC-SHA-256(K_eq_vN, domain || logical_id || field_id || canonical_type_encoding || value)` under a separately protected, versioned key. Root private material and recovery secrets are never inputs to ordinary equality indices. Use constant-time comparison; never send the key to provider/index. Domain-separate equality tokens from audit signatures and encryption keys. Rotate HMAC keys with overlap: retain key-version IDs and re-key only in a controlled metadata migration, never silently treat a key-version mismatch as a field change. Stable opaque `logical_id` is distinct from mutable path/name. Including `logical_id`/`field_id` in domain separation avoids equality correlation across unrelated items; if cross-replica matching needs same token, both replicas share the exact logical/field identity and HMAC version.

HMAC is not a zero-leakage proof: an observer can see which rows/fields changed, equality across replicas sharing the token, repeat frequency, access patterns, lengths if exposed, stable IDs and relationship graph. Even a keyed HMAC can reveal equality to an index reader with token access; if the key leaks, weak passwords can be enumerated offline. Therefore store tokens only in restricted local/index metadata, suppress field-level identifiers where not needed, avoid broad Supabase equality queries, and never log raw digests. An HMAC token must not be called a secret-safe public fingerprint. Access-key fields need the same handling; public certificate fingerprints are a separate public-data case.

## Performance and availability

No local performance conclusion is made here. Measure cold/warm auth, list/pagination, single/batch get, write/readback, p50/p95/p99, startup, memory, error/retry and rate-limit behavior per provider path using isolated synthetic records. Separate backend latency from process startup and network RTT. Existing evidence is architectural only: current fallback runs `bws secret list` then `get` for each `get` call (`fleet-secrets/src/lib.rs:517-602`); broker full refresh performs a list then per-item `get` with eight workers (`secrets_broker.rs:133-266`), cache reads are memory-based, and same-name misses are single-flighted (`:371-440`). These are measurements to run, not benchmark results.

Cache decision is a V2+ explicit policy, distinct from the already-merged #6767 mitigation. Do not duplicate repair work: #6767 remains open pending deployment and authorized canary, tracked by architecture proposal ARCH-02. Future cache responses carry source/revision/fetched age; policy selects `never`, memory TTL, encrypted local snapshot, or stale-allowed with class-specific max age. `ca-root`, `recovery`, `transient`, revoked/unknown, or security-critical classes fail closed offline by default. Root material is never in ordinary cache/prefetch. A provider outage is not permission to use arbitrarily stale data. Successful writes invalidate/refresh exact binding; provider revocation has a bounded propagation contract. Stale reads never authorize writes or overwrite a source.

## Issue-ready proposals for domain starter #9158

The following stable proposal IDs are for planning and later issue creation; this document does not create GitHub issues. Each is one deliverable and satisfies the coordinator handoff §4.2 issue content contract.

### SEC-01 — Characterize the existing V1 secrets compatibility boundary

- **Kind / urgency / priority:** test+contract; High, numeric urgency 450 (risk-reducing prerequisite), recommended label High. **Lifecycle:** Ready for synthetic repo-only work. **Estimate/stage:** S, V2 validation of V1 compatibility. **Reqs:** spec §§8.1, 8.4-8.5, 35.4; V3-S01, S02, S13, S25; intake note V1 census.
- **Outcome / current evidence:** typed expansion must not break shipped name-only resolution. Current evidence: `pkgs/fleet/crates/fleet-secrets/src/lib.rs:1-28,139-193,493-604`; CLI `pkgs/fleet/crates/fleet/src/secrets.rs:1-19,73-173`. `get` outputs the value by compatibility; `list` lists broker names only.
- **Deliverable / boundary:** write a compatibility contract and characterization tests around current `get/list`, no writes/adapters in this issue. No access to real vault needed. Keep `fleet-connect` feature isolation. Proposed files are `pkgs/fleet/crates/fleet-secrets/**`, `pkgs/fleet/crates/fleet/src/secrets.rs` and focused tests only.
- **Acceptance / oracle:** fixtures prove source order, empty value unresolved behavior, value redaction in Debug/errors, real `NotFound` vs unavailable, list broker-only, help/arity/exit semantics, and current read feature linkage. Oracle is exact behavior assertions and captured stdout/stderr in tests. **Negative tests:** BWS-list failure must not become absent; malformed response, permission error, empty legacy value do not return success; `list` never silently substitutes BWS enumeration.
- **Dependencies:** none; independent of SEC-02. **Rollback:** additive tests/contracts only; revert interface change if compatibility tests fail. **Recovery:** no state changed. **Open decision:** whether future typed API may represent valid empty string; default keep legacy behavior only in wrapper.

### SEC-02 — Define versioned typed Credential schema and compatibility parser

- **Kind / urgency / priority:** design+implementation+test; High, urgency 430, High. **Lifecycle:** Ready (synthetic fixtures only). **Estimate/stage:** M, V2 foundation. **Reqs:** spec §§19.2-19.6, 34.2-34.3, 35.3; V3-S03; `REC-01` depends on this. **Current evidence:** no canonical schema in current `fleet-secrets` (crate description and source above; `Cargo.toml:1-12`).
- **Outcome:** encode logical ID, canonical ref, schema/policy versions, security class, typed kind/fields, `Observed` state, opaque native refs/revisions, relations/attachments, lifecycle/provenance, and lossless unknown-field envelope; no raw value serialization to logs/index.
- **Boundary:** domain types/parser/validator in `fleet-secrets` or approved split crate; no provider call, database, CLI command tree, or production migration. Scope exact as declared. **Acceptance:** stable round-trip and migration tests across versions; serialization distinguishes absent/null/empty/redacted/inaccessible; native IDs and exact unknown fields survive; redaction property for `Debug`/errors; malformed unknown type is preserved opaque or fails with typed error, never dropped.
- **Negative tests:** title/path collision does not merge IDs; `bw` and BWS parse to distinct provider kinds; unknown class cannot auto-replicate; sensitive field is omitted from redacted formatter; attachments' bytes never appear in metadata encoding. Oracle: independent canonical test fixture, schema round-trip equality and redaction scanner.
- **Deps:** SEC-01; block REC-01. **Rollback:** keep V1 types and adapter untouched; version parser addition can be removed before persisted V2 data. **Recovery:** reject unsupported schema version, retain source-side bytes/fixture. **Open:** exact UUID representation and field normalization registry are implementation-time choices under ADR.

### SEC-03 — Add provider capability and scope descriptors with typed errors

- **Kind / urgency / priority:** design+implementation+test; High, urgency 420, High. **Lifecycle:** Backlog until SEC-02 boundary is agreed; synthetic implementation can parallelize after types settle. **Estimate/stage:** M, V2. **Reqs:** spec §§8.2, 8.6, 35.1, 38; V3-S01, S02, S13, S14.
- **Current evidence:** V1 hardcodes broker/BWS chain; no provider descriptors or write surface (`fleet-secrets/src/lib.rs:139-193,493-604`).
- **Deliverable:** capability descriptor with explicit operation/field/access/atomicity/support states and tested provider/version/scope; adapter seam for inventory/read/create/update; typed errors and receipt types. Stub providers only. Include distinct BWS, BitwardenPasswordManager, OnePassword identities; the type cannot infer authorization.
- **Acceptance:** fake adapters exercise supported, unsupported, unknown, not-authorized; capability JSON never reports unsupported writes as success; scoped handles validate container boundary; revision guard surfaces CAS vs preflight race vs none; errors redact raw response and auth material. **Negative tests:** BWS scope cannot instantiate `bw`; 403 is permission denied, not not-found; malformed/truncated listing is incomplete inventory; unsupported attachment update fails item, not whole-record success.
- **Deps:** SEC-02 and SEC-01. **Boundary:** no actual BWS/OP/bw auth, no provider mutation, no secret broker rollout. **Rollback/recovery:** keep legacy resolver path default; capability API is opt-in. **Open:** exact SDK/CLI binding selected only after version/toolchain compatibility research.

### SEC-04 — Implement BWS read adapter against explicit project scope

- **Kind / urgency / priority:** implementation+test; Standard, urgency 350, Standard. **Lifecycle:** Backlog pending SEC-03 and authorized synthetic BWS test scope. **Estimate/stage:** M, V2. **Reqs:** spec §§8.2, 19.1, 19.6, 35.1, 36.1, 38; V3-S01-S03.
- **Current evidence:** CLI fallback `bws secret list` + `get` exists (`fleet-secrets/src/lib.rs:517-604`); broker does full scoped list and `get` (`fleet-queue/src/secrets_broker.rs:190-266`). BWS value/note/project model is described by spec §19.1 lines 1031-1040; verify current official interface at implementation time.
- **Deliverable / boundaries:** implement explicit project/machine identity BWS adapter, preserve value semantics and native ID, expose note/metadata/observed revision capability. Prefer Rust SDK only after MSRV/API lockfile validation; subprocess adapter must use token file/FD/env discipline, never secret argv. Keep existing fallback until parity and regression verification. Synthetic isolated project only; no existing vault sweep.
- **Acceptance:** paginated or complete inventory reports scope/completeness; list/get agree; value and native ID round trip; errors redact stdout/stderr/tokens; auth and project scoping verified. **Negative tests:** empty/incomplete list, 401/403, limit/rate-limit, malformed JSON, missing item, timeout, stale revision never marked as deletion or successful write; secret value absent from logs.
- **Deps:** SEC-03. **Rollback:** feature-gate and fall back to V1 path; do not replace current `get` absent performance and parity evidence. **Recovery:** on unknown mutation receipt, read back by native ID/idempotency marker before retry. **Open:** whether official SDK is suitable remains unverified; benchmark in separate #9160.

### SEC-05 — Implement 1Password adapter for an explicit authorized vault set

- **Kind / urgency / priority:** implementation+test; Standard, urgency 340, Standard. **Lifecycle:** Backlog; requires an approved test vault/service-account grant. **Estimate/stage:** L, V2. **Reqs:** spec §§19.1-19.5, 35.1, 36.1; V3-S02-S03.
- **Current evidence:** no Fleet `op` adapter exists per intake note lines 32-39; rich OP field expectations are specified in spec §19.1. This is not proof of accessible vaults or permissions.
- **Deliverable:** read/list then guarded create/update adapter for one configured vault; preserve item UUID, section/field order/type/visibility, tags, URLs, attachment metadata and exact unknown fields. Report any unsupported item/field. Select CLI/API only after current official docs, service account scope, rate limits and readback verified.
- **Acceptance:** fixture or authorized dedicated test vault round-trips all supported fields; scope report names only granted vault IDs; no broader session discovery; no API/session token in process arguments/logs. **Negative tests:** insufficient scope, inaccessible attachment, unknown custom field, rate limit and title collision produce typed error/conflict; do not write another vault or drop field silently.
- **Deps:** SEC-03, SEC-02. **Boundary:** authorized test vault only; no personal-vault census/migration. **Rollback:** optional feature and no default resolver switch. **Recovery:** operation receipt plus native ID readback before any retry. **Open:** CLI vs supported SDK/Connect chosen by separately evidenced capability/entitlement.

### SEC-06 — Specify separately authorized Bitwarden Password Manager adapter

- **Kind / urgency / priority:** design+research; Standard, urgency 260, Standard. **Lifecycle:** Parked / approval-blocked for actual access; design may proceed from public docs. **Estimate/stage:** M, V2 later. **Reqs:** spec §§19.1, 36.1, 38; V3-S01, S25.
- **Outcome:** establish a separate adapter/auth/scope boundary for `bw` (personal logins, URI match, custom fields, TOTP/passkeys) and an independent Sweep B decision packet. BWS is not its credential source.
- **Current evidence:** no Fleet `bw` adapter; note.md lines 34-50 classifies access as not authorized. Product distinction in spec §19.1 lines 1031-1040. Do not claim full-vault access from `BWS_ACCESS_TOKEN`.
- **Acceptance:** architecture proves distinct provider enum, token/config type and grant path; public-doc capability matrix with versions; synthetic serialization fixture for rich login. **Negative tests:** BWS credential cannot instantiate password-manager provider, no adapter auto-login, no inventory before scope grant, unsupported passkey export remains unverified rather than synthesized.
- **Deps:** SEC-02, SEC-03. **Boundary:** no access, export or personal vault operations. **Rollback:** no running code enabled; research may be superseded by explicit authorization. **Open:** future authorized user/vault set, API/export restrictions and exact native fidelity.

### SEC-07 — Add root escrow policy objects without generic replication inheritance

- **Kind / urgency / priority:** design+security review+test; High, urgency 440, High but **implementation action is approval-blocked**. **Lifecycle:** Blocked for any production root operation; Ready only for policy/schema and synthetic tests. **Estimate/stage:** M, V2/V3 custody seam. **Reqs:** addendum §5 lines 42-55; spec §§8.2, 34.2, 38, 48.2 V3-S08/S18; intake note lines 26-30.
- **Deliverable:** dedicated `RootEscrowPolicy` with a closed, enumerated set of exactly three independently addressed destinations (encrypted laptop path, approved BWS project, approved 1Password vault); explicit recovery material and scope, integrity manifest, per-destination restore evidence, active-signer vs escrow role, and no auto-discovery. R2 is researched optional future destination, not part of the closed set. Do not let generic credentials replication alter this list.
- **Acceptance:** schema/model test rejects wildcard/future provider, missing destination or extra destination; custody is complete only when all three are present and each restores in isolation while the other two are unavailable; exact restored synthetic key matches expected public identity; signer is never inferred. **Negative tests:** no production key generation/read/export/provider write; general `sync all` denies `ca-root`; one destination unavailable or one isolated restore failing yields incomplete custody (not success); recovery key stored only in archive fails circularity validation.
- **Deps:** SEC-02, SEC-03; coordinate with #9163 custody design and #9165 evidence without absorbing PKI implementation. Root signer design must consume transaction-bound `SigningAuthorization` bound to signer fingerprint/handle, subject public-key identity and algorithm, purpose/profile, validity, artifact-plan digest, authorized output/escrow destinations, expiry and anti-replay semantics. Escrow-read authority never mints signing authorization. **Rollback:** policy feature disabled; never auto-delete existing escrow copies. **Recovery:** no live mutation in this stage; isolated restore proof before enablement. **Open:** separate approval/security review defines crypto format, recovery-recipient custody, active root workflow and permanent-vs-temporary policy.

### SEC-08 — Specify class-aware resolution and cache freshness policy

- **Kind / urgency / priority:** design+test; High, urgency 410, High. **Lifecycle:** Backlog pending capability semantics and ARCH-02 deployment/canary evidence. **Estimate/stage:** M, V2. **Reqs:** addendum §§9, spec §§35.4, 37.2, 48.2 V3-S07/S09; architecture proposal ARCH-02.
- **Current evidence:** #6767 mitigation is merged as PR #8491 (`c676d1c41c07`); deployment and authorized live canary remain unverified. Per-secret cache timestamps/age are implemented in `secrets_broker.rs:244-266,280-346,532-545`. This proposal does not duplicate the implementation fix.
- **Deliverable:** policy-only class-aware resolution contract for source, revision, age, freshness, revocation and offline mode; exclude root private material from ordinary hot cache/prefetch/equality indices. Any cache implementation change is separately scoped after evidence.
- **Acceptance:** fake-clock/provider tests reject stale freshness-required and revoked values; permitted stale read identifies source/age/revision/reason; offline read never authorizes write; replica divergence fails closed. **Negative tests:** old token never returned for freshness-required class; root/recovery/transient denied offline; unknown age is not fresh.
- **Deps:** SEC-03 and ARCH-02 as evidence input (not duplicate implementation). **Rollback:** default to live-only/fail-closed. **Recovery:** authoritative read only; never write provider from cache. **Open:** TTL/max-age and revocation targets require benchmark and security policy, not invented values.

## Deferred and explicit non-goals

- No live `bws`, `op`, or `bw` inventory/write; no secrets-broker credential read in this design session; no production root key generation, export, sync or signing; no schedule enablement.
- No generic remote credential SQL database; no automatic secret replication to all configured providers; no automatic root signer discovery.
- No assumption that `fnox` is a writable reconciliation engine, that OpenBao is available or that a provider API offers CAS. These must be individually evidenced.
- PKI shape/cryptographic policy and CA issuer/root generation remain other domain issues. Secrets design provides protected references and custody policy, not production CA authorization.

## Source index

- Governing source: `sources/fleet_architecture_spec_v0_3.md` §§8.1-8.6 (lines 592-645), §§19.1-19.6 (1027-1085), §§34-35 (2124-2219), §38 (2269-2278), §48 (2637-2695).
- Refinement: `sources/ADDENDUM_secrets_architecture_performance_infrastructure.md` §§1, 5-6, 8-9 (lines 5-10, 42-67, 82-103). Root escrow's three destinations are explicit; BWS retained; `bw` remains distinct.
- Execution/approval: `sources/COORDINATOR_summit_handoff.md` §§4.2, 7.2, 10 (lines 102-122, 196-199, 267-273); `note.md` lines 24-50.
- Current code references are given inline above; all cited code references are checkout-local, not claims of external API behavior.
