# Fleet v0.3 architecture — V1 evidence to staged V2/V3/V4

**Status:** proposed architecture for #9157; no code or production change authorized here. **Baseline commit:** `34faffbff26615e313488f6d04705a444342e41e` (2026-11-12). V1 facts and exact code/test evidence are in [`evidence/census.md`](../evidence/census.md). Source precedence: current task and dated addendum refinements, then v0.3 specification; see `note.md` and source sections §§1, 6, 9, 25, 26, 27, 35, 37, 43, 48.1–48.4 plus addendum §§1–10. Signatures below are design-level Rust pseudocode, not existing APIs.

## Decision summary

1. Keep the present `fleet` multicall binary and registry. Add commands/features to it rather than create a competing CLI or certificate server.
2. Preserve the existing `fleet-secrets` value resolver and compatibility behavior, but grow it into the stable runtime secret interface. Start with one cohesive secrets module/crate; split an admin/sync crate only when the write path and its dependency graph are real. Do not create a crate per noun.
3. Keep read, administrative mutation, signing, and escrow authority planes separate. Runtime retrieval and signer/provider traits are small, typed, injectable, async and memory-friendly; CLI, cache daemon, reconciliation, codecs, PKI algorithms and provider integrations are optional feature/build selections. A read reference never implies write, replication, export or signing authority.
4. Use distinct modules/crates only where consumers or dependency closure actually differ: retain `fleet-secrets`; add a cohesive `fleet-crypto` plus `fleet-pki` when needed; keep CLI integration in existing `fleet`; introduce `fleet-secrets-admin` only for mutation/reconciliation/history after a real consumer requires it. Format/archive/publication adapters remain features or private modules until separate reuse justifies crates.
5. Treat #6767 accurately: per-secret refresh/timestamps/age/source mitigation is merged as PR #8491 (`c676d1c41c07`) and source tests pass; GitHub issue #6767 is still OPEN for deployment and live canary (latest relevant comment fetched 2026-11-12, created 2026-11-11T03:27:23Z). Do not duplicate the implementation, conflate its original fail-whole refresh defect with a broader max-age policy, or claim deployed proof.
6. Keep root key custody independent from ordinary secret replication. Initial three-destination escrow (encrypted laptop, authorized BWS, authorized 1Password) is the addendum's policy goal, not execution authority in this design session. RootPrivate is excluded from ordinary hot caches, prefetch, benchmark datasets and equality indexes. R2 is research, not a fourth automatic destination. Offline/hardware signer migration is a planned custody adapter.
7. Keep federation and workload identity as hooks/interfaces plus explicit V2/V3/V4 research gates; no OpenBao/fnox/SecureStore/R2/Cloudflare service is a baseline dependency.

## Module/seam comparison and chosen crate shape

| Option | Leverage/locality | Cost/risk | Decision |
|---|---|---|---|
| One crate per concept from spec §26 (`model`, `sync`, `providers`, `audit`, `crypto`, `generate`, `pki`, `ssh`, `iroh`, `io`, `publish`, `docs`) | Potentially clear owners and independently selectable dependencies. | At this stage many are interfaces without two implementations; shallow wrappers, feature wiring, compile/build graph, public semver surfaces and cross-crate type duplication precede concrete variability. No source evidence demonstrates consumer needs for every split. | Reject as initial decomposition. Split only after real distinct dependency closure/reuse. |
| One giant `fleet` crate | Easiest short-term integration. | CLI, provider, secrets, crypto and PKI dependencies leak into library consumers; no clean API reuse; test seams mix IO, policy, parsing and cryptography. | Reject as long-term shape. |
| **Cohesive runtime secrets + optional admin; crypto/PKI only as real separable seams** | Existing resolver remains one public entry; adapter variation lives behind `SecretStore`; admin path does not contaminate runtime; PKI can be used as library without Fleet CLI. Avoids creating dead crates. | Requires explicit feature discipline and later API review; one crate may temporarily contain cohesive model and provider traits. | **Choose.** V2 proves consumers and features. Extract only if dependency closure or ownership forces it. |

`fleet-secrets` is the existing seam: retain the by-name resolver and `SecretValue` invariants; add types without breaking `fleet secrets get`. A provider trait is a real seam because BWS, OP, local age/SOPS, broker, and eventual OpenBao differ in methods, versioning and freshness. Do not force a single identical operation set; use capability descriptors and operation-specific results. The broker is an adapter/cache, not a source of authority or an admin control plane. The existing GitHub creation outbox is a pattern, not a backend to reuse for secret plaintext or secret journal events. Authority is not implied by a provider capability: capability, principal grant, and operation plan must all authorize execution.

## Proposed library interfaces

### Secrets runtime (`fleet-secrets`)

```rust
use std::{sync::Arc, time::Duration};
use async_trait::async_trait;

#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SecretRef { pub id: CredentialId, pub field: Option<FieldId> }
/// Stable opaque logical identity; it is not derived from a provider name or path.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
pub struct CredentialId(pub uuid::Uuid);
/// Mutable, explicitly scoped lookup name; never the credential's canonical identity.
#[derive(Clone, Debug, Eq, PartialEq, Hash)]
pub struct SecretAlias { pub scope: ScopeId, pub name: String }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DataClass { Public, Internal, MachineStatic, HumanSensitive, Recovery, RootPrivate, Dynamic }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SecretPurpose { RuntimeRead, RecoveryRead, RotationRead, AuditRead, RootSigning }
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OfflineIntent { OnlineOnly, MayUsePolicyAuthorizedReplica }
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd)]
pub enum AssuranceRequirement { Any, VerifiedVersion, HardwareBound }
#[derive(Clone, Debug)]
pub struct ResolutionContext {
    pub purpose: SecretPurpose,
    pub offline_intent: OfflineIntent,
    pub required_assurance: AssuranceRequirement,
    pub caller_max_age: Option<Duration>, // can only tighten stored policy
}
/// Returned only by the resolver's trusted policy source, never set per call.
pub struct AuthoritativeCredentialPolicy {
    stored_class: DataClass,
    revocation_policy: RevocationPolicy,
    maximum_age: Duration,
    offline_allowed: bool,
    minimum_assurance: AssuranceRequirement,
}
#[async_trait]
pub trait AuthoritativePolicySource: Send + Sync {
    async fn load(&self, id: CredentialId, cancel: CancellationToken)
      -> Result<AuthoritativeCredentialPolicy, PolicyError>;
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RevocationState { CurrentNotRevoked, Revoked, Unknown }
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FreshnessDecision { Fresh, Degraded { reason: DegradedReason } }
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DegradedReason { PolicyAuthorizedOfflineReplica, BoundedStaleUse }
#[derive(Debug)]
pub struct ResolvedSecret {
    pub value: SecretBytes,          // redacted Debug; no Serialize/Display
    pub source: ProviderId,
    pub native_revision: Option<Revision>,
    pub fetched_at: std::time::SystemTime,
    pub source_age: Duration,
    pub revocation: RevocationState,
    pub freshness: FreshnessDecision,
}
#[async_trait]
pub trait SecretStore: Send + Sync {
    fn descriptor(&self) -> &ProviderDescriptor;
    async fn get(&self, id: CredentialId, field: Option<&FieldId>, cancel: CancellationToken)
      -> Result<ProviderValue, ProviderError>;
    async fn list(&self, scope: &Scope, page: Option<PageToken>, cancel: CancellationToken)
      -> Result<Page<ProviderItem>, ProviderError>;
}
pub struct Resolver { /* trusted policy source + adapters + bounded cache/single-flight */ }
impl Resolver {
    pub async fn resolve(&self, r: &SecretRef, context: &ResolutionContext,
                         cancel: CancellationToken)
      -> Result<ResolvedSecret, ResolveError>;
    pub async fn resolve_many(&self, refs: &[SecretRef], context: &ResolutionContext,
                              cancel: CancellationToken)
      -> Result<Vec<Result<ResolvedSecret, ResolveError>>, BatchError>;
}
```

`SecretBytes` owns zeroizing bytes where supported, has a deliberately redacted `Debug` implementation, and does not implement `Display` or `Serialize`; bytes are exposed only through an explicit closure (`with_exposed`). Secret-bearing values do not implement implicit `Clone`; any exceptional copy has a named, audited operation. `ResolvedSecret` may be debugged safely but metadata serializes only through a separate redacted receipt type. `SecretAlias { scope, name }` is a mutable lookup alias; it resolves to an opaque `CredentialId`, which remains stable across rename/relocation. Alias resolution must be explicit and scoped. Ordinary `Resolver` rejects `RootPrivate` and returns no signer capability. Root signer access is a separate authority plane and excludes ordinary resolver caches, prefetch, benchmarks and equality indices. `ResolutionContext` carries purpose, offline intent, required assurance and an optional caller freshness ceiling. The resolver obtains class, revocation policy, freshness ceiling, offline allowance, assurance floor and provider/native scope binding from a trusted authoritative policy index; none is caller-supplied or inferred from a provider label/mutable sidecar. The caller may only tighten (shorter maximum age, higher assurance, online-only); it cannot lower stored class or policy, permit revoked/unknown data against policy, or turn RootPrivate into a readable grant. Policy is keyed to verified provider/account/project/vault and native IDs. Unknown or adopted items require classification review before replication. Revoked/policy-denied credentials are never served from stale fallback. Successful results contain explicit `FreshnessDecision`, `source_age`, revocation state, source and revision; stale/degraded success requires both authoritative policy and caller intent and carries a typed reason. Denial is an error, never silent fallback. A fake-clock/class/purpose/offline/revocation matrix tests these rules. PERF-02 owns freshness semantics and implementation; ARCH-01 owns the API shape and does not create a second freshness policy or resolver.

`ProviderDescriptor` carries stable ID/version and declared capabilities; capability discovery is separate from permission testing, and a successful list must include pagination completion evidence. For writes, later `fleet-secrets-admin` provides typed `Credential` / `SecretField` / `ProviderItem` mappings with ordered typed fields and encrypted lossless sidecar; provider-specific unknown fields cannot be silently dropped. Distinguish Bitwarden Secrets Manager (`bws`), Bitwarden Password Manager (`bw`) and `op`; credentials and auth scopes are separate.

Per adapter **and operation**, record `WriteGuardLevel::{AtomicPrecondition, SerializedByContract, Unguarded}`. A level is established only by primary documentation plus an isolated test. `SerializedByContract` means every writer of that scope is known to use and honor the same serializing mechanism; a Fleet-local lease is insufficient if a human, app, or other tool can write outside it. Until proved otherwise, BWS, `op` and `bw` are `Unguarded`. Automatic apply to conflict-sensitive fields (any secret value and any field both sides may edit) requires `AtomicPrecondition` or `SerializedByContract`; otherwise the step stops as `needs-review`, or uses a separately approved, named weaker policy with its lost-update window in plan and receipt. A Fleet-local lease and readback are not serialization. Additive create and immutable-history writes are not gated by this conflict-sensitive update rule. This is ADR-S02/A-1; detailed adapter behavior belongs to #9158/#9159.

Errors are stable machine classes with redacted context, not arbitrary stderr: `InvalidReference`, `NotFound`, `Unauthenticated`, `PermissionDenied`, `Unavailable`, `RateLimited { retry_after }`, `Unsupported { operation }`, `Stale { age }`, `Revoked`, `RevocationUnknown`, `AssuranceNotMet`, `Conflict`, `MalformedProviderData`, `Cancelled`, `Timeout`, `Partial { completed, pending }`, `Integrity`, `PolicyDenied`, `AuthorityPlaneDenied`, and `Internal`. Secret identifiers may themselves be sensitive; callers choose log-safe IDs/hash pseudonyms. Do not preserve payload or values inside provider error strings.

### Admin/write, reconciliation and custody

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteGuardLevel {
    AtomicPrecondition,
    SerializedByContract,
    Unguarded,
}
pub struct ProviderOperationGuard {
    pub operation: ProviderOperation,
    pub level: WriteGuardLevel,
    pub evidence: Option<GuardEvidence>, // primary docs + isolated proof
}
#[async_trait]
pub trait SecretWriter: SecretStore {
    async fn create(&self, item: &Credential, key: IdempotencyKey, c: CancellationToken)
      -> Result<WriteReceipt, ProviderError>;
    async fn update(&self, id: &NativeId, if_match: Option<&Revision>, item: &Credential,
                    key: IdempotencyKey, c: CancellationToken)
      -> Result<WriteReceipt, ProviderError>;
    async fn archive(&self, id: &NativeId, if_match: Option<&Revision>, key: IdempotencyKey,
                     c: CancellationToken) -> Result<WriteReceipt, ProviderError>;
}
pub struct ReconcileEngine { /* pure diff + injected provider ports, journal and policy */ }
impl ReconcileEngine {
    pub fn plan(&self, a: &Inventory, b: &Inventory, base: &VerifiedSnapshot,
                policy: &ReplicationPolicy) -> Result<SyncPlan, PlanError>;
    pub async fn apply(&self, p: &ValidatedPlan, approval: &Approval,
                       c: CancellationToken) -> Result<RunReceipt, ApplyError>;
    pub async fn resume(&self, id: RunId, c: CancellationToken) -> Result<RunReceipt, ApplyError>;
}
```

Diff/merge is deterministic and side-effect free; apply is a journaled saga, not an atomic distributed transaction. One mutation has a plan digest, user/scope approval, source/native revisions, idempotency key, preflight, write, read-back proof, receipt and compensating/recovery instructions. Cancellation before a remote write aborts cleanly; cancellation or process death after submission records `uncertain`, then resumes by read-back/idempotency rather than blindly repeating. Never interpret timeout, 403, failed page, or partial pagination as deletion. Preserve native IDs and update originals in place. `zz.delete` is protected verified history with retention policy, not an immediate destructive delete. Event payloads omit secret values; high-entropy keyed equality proofs only when separately approved.

A-1's model oracle is **preflight → external edit → apply**: on an unguarded conflict-sensitive update, the external value remains recoverable and the operation reports conflict/unsupported-guard, never success. Readback after an unconditional overwrite is not serialization. The concrete planner/journal/saga is delegated to #9159.

`fleet-secrets-admin` should be optional and may initially be a feature/module inside `fleet-secrets`; extract crate only when dependencies/users make its separate build valuable. It owns writer capability, reconciliation and audit journal. Runtime consumer cannot construct admin authority just by importing read resolver. Root signer handles are neither `SecretRef` nor `SecretBytes` and are issued only through a bounded, explicitly authorized signer session. Broker protocol already contains the #6767 metadata mitigation in source; deployment/canary evidence remains separate. Any future request-specific freshness contract is a new design requirement and must preserve old client compatibility; it is not a duplicate of #6767's merged per-secret refresh fix.

### Crypto, PKI, artifact IO

```rust
pub struct SigningAuthorization { // private fields; minted only by authority service
    root_key: AuthorizedKeyIdentity, // fingerprint/handle
    subject_spki: PublicKeyIdentity,
    signature_algorithm: SignatureAlgorithm,
    serial: Option<SerialNumber>,
    signed_payload_digest: Digest, // exact final TBS/message bytes
    profile: ProfileId,
    purpose: KeyPurpose,
    validity: ExactValidity,
    artifact_plan_digest: Digest,
    destinations: Vec<AuthorizedDestination>,
    expires_at: SystemTime,
    transaction_id: TransactionId,
    nonce: SingleUseNonce,
}
pub struct PreparedSigningTuple { /* private: SPKI, serial, exact TBS and plan digests */ }
pub struct PreparedIssuance { /* immutable final TBS + artifacts; private fields */ }
pub struct SignerSession { /* private, one validated authorization tuple */ }
pub struct DurableTupleClaim { /* private, atomic single-winner, non-Clone */ }
pub struct OutputAuthorization { /* private, exact plan/class/destination/expiry, non-Clone */ }

#[async_trait]
pub trait SigningAuthorizationPort: Send + Sync {
    async fn authorize_prepared(&self, tuple: &PreparedSigningTuple,
                                cancel: CancellationToken)
        -> Result<SignerSession, AuthorizationError>;
}
#[async_trait]
pub trait DurableAuthorizationLedger: Send + Sync {
    async fn consume_once(&self, session: &mut SignerSession, tuple_digest: Digest,
                          cancel: CancellationToken) -> Result<DurableTupleClaim, ClaimError>;
    async fn record_result(&self, claim: DurableTupleClaim,
                           result: SigningReceiptOrUncertain) -> Result<(), LedgerError>;
}
#[async_trait]
pub trait Signer: Send + Sync {
    async fn sign(&self, claim: &DurableTupleClaim, exact_tbs: &[u8],
                  cancel: CancellationToken) -> Result<SigningReceipt, SignError>;
}
#[async_trait]
pub trait OutputAuthorizationPort: Send + Sync {
    async fn authorize_output(&self, plan: &ArtifactPlan, artifact: &ClassifiedArtifact,
                              cancel: CancellationToken)
        -> Result<OutputAuthorization, OutputAuthorizationError>;
}
#[async_trait]
pub trait ArtifactSink: Send {
    async fn write(&mut self, authorization: &OutputAuthorization,
                   artifact: ClassifiedArtifact, cancel: CancellationToken)
        -> Result<ArtifactReceipt, SinkError>;
}

pub struct PkiEngine { /* profiles, validation, graph, format-independent issuance */ }
impl PkiEngine {
    /// Apply-stage preparation reserves a serial and freezes exact SPKI/TBS.
    /// Dry-run/preflight must not consume serials or mint signature authority.
    pub async fn prepare_leaf(&self, req: LeafRequest, serials: &dyn SerialLedger,
                              clock: &dyn Clock, cancel: CancellationToken)
        -> Result<PreparedIssuance, PkiError>;
    pub async fn sign_prepared(&self, prepared: PreparedIssuance,
                               session: &mut SignerSession, claim: DurableTupleClaim,
                               signer: &dyn Signer, cancel: CancellationToken)
        -> Result<SignedIssuance, PkiError>;
    /// Convenience helper prepares first and asks the injected port to authorize
    /// the now-known tuple; it never accepts a prefilled grant for unknown bytes.
    pub async fn issue_leaf(&self, req: LeafRequest,
                            authorization: &dyn SigningAuthorizationPort,
                            serials: &dyn SerialLedger, ledger: &dyn DurableAuthorizationLedger,
                            signer: &dyn Signer, clock: &dyn Clock, cancel: CancellationToken)
        -> Result<IssuedCertificate, PkiError>;
    /// Root request always names the root-key identity and separate ceremony approval.
    pub async fn issue_root(&self, req: RootRequest, root_key: ExplicitRootKeyRef,
                            ceremony: &RootKeyCreationAuthorization,
                            authorization: &dyn SigningAuthorizationPort,
                            signer: &dyn Signer, ledger: &dyn DurableAuthorizationLedger,
                            cancel: CancellationToken) -> Result<RootArtifact, PkiError>;
}
```

`fleet-crypto` owns algorithm/type registry and signer contracts; providers are optional adapters. `fleet-pki` owns CA/root/issuer/leaf/chain/CRL profiles and policy; it depends on crypto interfaces, not a specific key export or provider. Keep no-disk operation possible. Key-generation/storage ceremony approval is separate from signature authorization. The signer grant binds root key fingerprint/handle; subject SPKI identity and signature algorithm; profile/purpose; exact validity; reserved serial where relevant; hash of exact final TBS/message bytes; artifact-plan digest; authorized output/escrow destinations; expiry; transaction ID; single-use by default. Grant issuance occurs only after the signing subject/key is known, required serial is durably reserved (gaps are acceptable), and final bytes/plan are frozen. A dry run may render estimates/placeholders but cannot authorize a final signature or burn a serial. A multi-signature bounded grant must enumerate every final tuple; unlisted signatures are never allowed. Before invocation, the ledger atomically and durably consumes the exact tuple, single-winner across concurrent consumers. State is `authorized → consumed/claimed → submitted → receipt-recorded` or `uncertain`; a crash after claim without authoritative final receipt burns the tuple. A lost result with no authoritative signer status remains uncertain and blocked, never treated as unused or retried. Receipt names the authorized transaction. `SigningAuthorization`, `SignerSession`, `DurableTupleClaim` and `OutputAuthorization` have private constructors and no implicit `Clone`; `PreparedIssuance` is immutable and privately constructed by `PkiEngine`. Only the trusted authorization, ledger and output-authorization ports construct grants/claims. Inventory lookup, escrow read access, possession of key bytes, and a valid path never mint authorization. Tests refuse ambient discovery, signer substitution, changed SPKI/serial/TBS/plan digest, wrong purpose, expiry, replay, signer handle used as ordinary secret reference, concurrent double-consume, duplicate invocation, and retries after uncertain outcome. Inject crashes before claim, after durable claim/before signer call, after signer call/before receipt, and after receipt. ROOT-03/ROOT-04 synthetic signing must use this seam and must depend on the shared authorization contract (or explicitly assert identical enforcement), never sign directly around it. This is ADR-S01/A-2; PKI-specific implementation is delegated to ROOT-05/CERT-03. `ArtifactSink::write` requires an unforgeable `OutputAuthorization` bound to exact artifact-plan digest, confidentiality class, destination and expiry; there is no private-output method that accepts only an artifact or ordinary sink. Denied/expired/wrong-plan grants cause zero sink calls. The high-level `issue_leaf` helper receives an injected `SigningAuthorizationPort`/session factory and obtains authorization only after `prepare_leaf` has fixed subject SPKI, reserved serial and exact TBS. It never accepts a prefilled grant for unknown bytes. The preparation/reservation operation is side-effect explicit: a dry run may show estimates/placeholders but cannot reserve/burn a serial or mint final-TBS authorization. Root operations always use an explicit root key identity and grant. PKI engine does not own provider replication or DNS mutation. A future `fleet-publish` seam is warranted only when more than one real publishing adapter exists.

The reference baseline keeps ML-DSA-44 required, with RSA-4096 production policy and explicit RSA-2048 compatibility gate; no silent algorithm substitution. Test-only deterministic entropy/clock are injected, never enabled in production. Existing `pkgs/id` Iroh endpoint key may be wrapped by an `IrohIdentityAdapter`, retaining identity-specific file semantics; it must not become a CA signer implicitly.

## Feature / dependency DAG and profiles

```text
fleet-registry (std-only)
  └─ fleet CLI
      ├─ secrets-read -> fleet-secrets-core -> selected provider adapters
      │                                  ├─ broker protocol (existing)
      │                                  ├─ bws CLI (compat, existing)
      │                                  ├─ op | bw (optional, distinct auth)
      │                                  ├─ local-age/sops (optional)
      │                                  └─ fnox/OpenBao (future optional)
      ├─ secrets-admin -> separate mutation grant + typed writer adapters + journal + reconciliation
      ├─ crypto-core -> selected algorithm implementation(s)
      │   └─ signer-session -> explicit signing grant + selected key provider
      ├─ pki -> crypto-core + X.509 + profiles/validation
      ├─ generate -> crypto-core + selected protocol adapters (ssh / iroh)
      ├─ artifact-formats -> pki or generate (only selected codecs)
      ├─ publish -> public artifact interfaces + selected HTTP/DNS adapters (admin only)
      └─ docs/skills -> registry/schema metadata (no runtime crypto deps)
```

No reverse dependency from secrets core to CLI, PKI, disk archive, OpenBao, Nix, or admin. Feature names should remain additive and default-off for optional provider/algorithm implementations. Build profiles proposed: `consumer-min` (resolver + one selected source, no CLI admin, crypto absent); `agent` (runtime resolver + structured receipts); `fleet` (existing full multicall); `admin` (write/reconciliation/PKI/codecs); `serve` (broker roles with chosen providers, not all CLI tooling). Existing variant names stay compatible; aliases/profile surface in registry marks unavailable capabilities rather than hiding help.

### Proposed performance and packaging acceptance budgets (not measured)

- Preserve `fleet-connect` target ≤1.5 MiB and hard ≤2 MiB exactly; no secrets/crypto dependency in connect.
- Proposed `consumer-min` release binary ≤3 MiB stripped with one selected provider, and ≤8 MiB for `agent` without PQ/OpenSSL; these are design targets requiring baseline measurement before ratification.
- Runtime startup (process start to ready, warm local resolver, no network) p95 ≤25 ms on reference laptop; in-memory `resolve` overhead p95 ≤1 ms excluding adapter/network. Record hardware, toolchain, cache state.
- Warm in-process cache lookup p95 ≤100 μs for up to 10k refs; single-flight produces exactly one provider request per simultaneous same-key miss; bounded default concurrency ≤8 until per-provider measurements justify separate caps.
- Do not specify “fast” network retrieval without measurement. For broker stale-age, budget is policy-defined: zero stale for `RootPrivate`, `Recovery`, `Dynamic` unless explicit nonproduction exception; ordinary secret classes must specify max age and offline permission.
- Compare build/install binary size, startup, allocations/peak RSS, p50/p95/p99 per provider, cache behavior and concurrency across laptop and agent host. Benchmarks use synthetic vaults; no secret values in results.

## Ownership, concurrency and cancellation rules

- Provider adapter owns native-ID mapping, pagination, API auth/session and capability truth; canonical model owns logical identity/classification; policy owns allowed source, replication and freshness; reconcile engine owns the plan/run state machine; journal owns durable nonsecret operation receipts; CLI owns argument/config parsing and rendering only.
- Each `Resolver` owns immutable policy and shared adapter handles (`Arc<dyn SecretStore>`); adapters are `Send + Sync` only if their clients are actually safe. In-flight requests use keyed single-flight; lock only to inspect/update maps, never across network I/O. Cache entry stores source, revision, fetched time, class and invalidation generation; no “refresh succeeded” timestamp may freshen an item fetched earlier. RootPrivate has no ordinary cache/prefetch path; a signer session may hold key material only for its bounded authorized operation.
- Cancellation token is passed through every async adapter and signing operation. A dropped Rust future may not cancel a remote request; adapter must classify unknown submission outcome as `uncertain` and journal before retry. Deadline wraps each provider call and total request separately; retries are bounded, jittered, respect Retry-After, and only retry idempotent operations.
- Secret bytes are owned, redacted, not cloneable where practical; controlled duplication uses explicit method and zeroizing allocator where supported. No secrets in tracing attributes, errors, panic strings, metrics, process args, or ordinary output. Avoid lock poisoning unwraps in new security-sensitive code.

## V1 → V2 → V3 → V4

| Stage | Scope / exit gate | Explicitly not a prerequisite |
|---|---|---|
| **V1 (current)** | Existing name-only resolver, broker+CLI cache, multicall variants and queue/outbox. See evidence census. | Any claimed canonical credential/PKI API. |
| **V2 (foundation)** | Typed secret/ref/class and capability model; separate authority planes; synthetic BWS/OP adapter tests; read-only provider benchmark; SOPS/age design/integration; admin and journal plan/apply with synthetic providers; crypto/signer seam and PKI synthetic graph. Retain BWS and OP, keep `bw` separate and authorization-blocked. #6767 source implementation is complete; deployment/canary remains a separately approval-blocked operational obligation, not a duplicate code task. | Production vault sweep, live credential rotation, generating/escrowing production roots, OpenBao service. fnox adoption requires library/API, maintenance and benchmark proof. |
| **V3 (admin / PKI separation)** | Fully versioned admin API, typed provider writes, two-sided/three-way reconcile, protected `zz.delete`, audit/recovery; `fleet-crypto` + `fleet-pki` with ML-DSA-44 baseline and RSA policies; four-root / nine asymmetric cross-cert / A/B issuer reference tests; operator CLI remains wrapper; root custody workflow with explicit approval and tested restoration. | Root generation/upload during this design; production changes before separate gates and authorization. |
| **V4 (federation hooks)** | Per-secret replication and provider compartments, policy-authorized BWS/OP/OpenBao/age replicas, dynamic-credential types, workload identity/attestation hooks, independent failure/compromise graphs, SPIFFE/SPIRE and infrastructure research; evaluate remote RPC/API separately. | Treating federation as universal mirroring, promoting R2 or OpenBao as baseline, or claiming HA from same-host replicas. |

Root signing authority is distinct from encrypted escrow; escrow destinations do not automatically become active signers. The closed custody type is:

```rust
pub enum RootEscrowDestination {
    EncryptedLaptop,
    BwsProject,
    OnePasswordVault,
}
pub struct RootEscrowPolicy {
    destinations: BTreeMap<RootEscrowDestination, DestinationPolicy>,
} // dedicated policy; never generic replication
```

It is not a generic provider list; no wildcard/future provider is permitted. Custody is complete only when all three are present and each independently restores with the other two unavailable. A missing destination or failed restore is incomplete custody, not success-with-warning. This is ADR-S05/A-3; the detailed implementation and synthetic restore proof belong to ROOT-09/ROOT-10. **Activation safety precondition:** before the first escrow copy, register the exact BWS project / OP vault and native escrow object IDs in the trusted policy registry, bound to verified provider/account scope; runtime principals (including the broker and legacy `fleet secrets get` BWS fallback) must not be able to read that root scope. Prefer a separately least-privileged runtime grant. If the provider grant cannot separate it, both the all-access broker refresh/cache and name-only fallback must reject the exact root scope before list/get/cache/print; a self-asserted `DataClass::RootPrivate` or mutable sidecar is not sufficient. Unknown/adopted records require classification review before replication. ROOT-09/OPS-10 must prove this with the original broker-compatible synthetic path; production escrow remains blocked until actual scope/native IDs and permissions are separately approved and verified. No BWS project is created automatically by this design. R2 remains researched client-side encrypted backup only unless independent adoption decision. SOPS ciphertext in Git is limited to explicitly scoped deployment values; age private key stays provisioned outside Git and Nix store. fnox is a near-term research target but not a mandatory abstraction: benchmark its public Rust API/stability and adapter capabilities before embedding or subprocess use. SecureStore must prove added value against age/SOPS rather than add a fourth redundant format.

## Architecture-owned proposals and cross-domain ownership

These are the only proposals owned by #9157 architecture. Domain work remains with the existing domain proposals listed below; the former broad ARCH-02–ARCH-05 and ARCH-08–ARCH-09 packets are retired as proposals, not silently dropped. Their requirement coverage is explicitly routed to domain owners. All proposed packets below are design/development planning only; no issues are created here.

The architecture proposal urgency scores retain the shared charter's intrinsic scale (lower is sooner): 400–799 High, 800–1199 Standard, 1200–1599 Low, 1600–1999 Someday, 2000+ Cold; 0–399 is reserved for incident/critical. These four foundational proposals use numeric scores 450–500, all High. Preserve the intrinsic score and band separately from importance, hard-blocking status, implementation stage and dispatch directive. If a publisher projects to the existing queue scale, use High→20, Standard→40, Low/Someday/Cold→70; do not alter the PM schema or equate queue priority with intrinsic urgency.

## Architecture challenge re-review

The architecture challenge is read in full. Its two blockers are now reflected as interface contracts, not claimed implementation evidence: A-1 requires per-adapter/per-operation guard level and denies automatic conflict-sensitive updates on unguarded targets unless an explicitly approved weaker policy is named; A-2 requires transaction-bound, single-use signing authorization and mismatch refusal. A-3 is constrained to the closed `RootEscrowPolicy` destination set with each copy restored in isolation before reporting complete custody. A-4 is owned by the ROOT/CERT verifier/client capability matrix; A-5 is owned by PERF measurement and REC completeness/cancellation testing. None authorizes production work or blocks synthetic contracts.

The JSON `urgency` is the shared charter's intrinsic numeric scale (lower is sooner); these four proposals use the High band 400–799. Preserve both numeric score and band. Urgency is separate from importance, blocking, stage and dispatch, and is not the Project/queue priority scale. No PM schema or queue mapping is changed here.

### Cross-domain requirement routing (not new architecture tickets)

| Prior packet ID / subject | Existing owner proposal(s) | Boundary |
|---|---|---|
| ARCH-02 #6767 deployment/canary | Existing issue #6767 (open; PR #8491 code fix merged); no duplicate issue. Freshness contract belongs PERF-02. | This design does not deploy or canary. #6767's original fail-whole refresh defect is not the separate general caller max-age policy. |
| ARCH-03 adapters, capabilities, benchmarks, fnox/SOPS | PERF-01–PERF-05; SEC-03–SEC-06 | Architecture defines seams only; domain owns adapters, upstream research and measurement. |
| ARCH-04 canonical credential fields and replication policy | SEC-02/SEC-03; future replication invariants FUT-09 | Opaque logical IDs/alias relationship is coordinated with ARCH-01; no duplicate schema project. |
| ARCH-05 plan/apply, journal, `zz.delete`, recovery | REC-01–REC-08; OPS-01/OPS-03 | A-1 guard semantics in this document are binding input; REC owns planner, guard enforcement, journal and state machine. |
| ARCH-08 root escrow and restore | ROOT-09/ROOT-10; OPS-10 | Closed three-destination policy and restore-complete condition are binding; no architecture-owned custody implementation. |
| ARCH-09 federation and workload identity | FUT-01–FUT-12 | Nonblocking research, no baseline service or deployment. |
| PKI graph, profiles, issuance, client support | ROOT-01–ROOT-12; CERT-01–CERT-18; GEN-01/GEN-04; OPS-07–OPS-09 | ARCH-06 owns only generic shared signer/artifact authority interface, not PKI graph implementation or test matrix. |

### Proposal ID migration note

`ARCH-01` remains the runtime identity/type/compatibility seam and depends on TRACE-02 for the V1 golden oracle. `ARCH-06` is narrowed from the former “crypto signer interfaces and PKI engine” umbrella to generic signer/artifact authority interfaces. `ARCH-07` remains registry/feature wiring only; OPS-05 generated-help coordination is soft, not a blockedBy dependency. `ARCH-10` is added for independently testable profile/minimal dependency-closure enforcement. Former `ARCH-02` routes to #6767/PERF-02; `ARCH-03` to PERF/SEC; `ARCH-04` to SEC-02/SEC-03/FUT-09; `ARCH-05` to REC-01–REC-08/OPS; `ARCH-08` to ROOT-09/ROOT-10/OPS-10; `ARCH-09` to FUT. Old SEC-07, SEC-08 and REC-09 labels from a prior draft are retired historical aliases only, with no publishable body; current owners are ROOT-09/ROOT-10, PERF-02 and REC-01–REC-08 respectively. Those old ARCH IDs are migration aliases only and must not be filed as duplicate issues.

Preserve the old numeric scores here as migration data; do not treat them as the new project priority or hide score changes:

| Former ID | Former urgency | Disposition / current owner | Current architecture urgency |
|---|---:|---|---:|
| ARCH-01 runtime API | 400 | Retained as ARCH-01; depends on #9156 TRACE-02 for CLI/profile golden oracle; SEC-02 consumes the opaque ID seam | 450 High |
| ARCH-02 broker rollout | 450 | No duplicate; existing #6767 owns deploy/canary, PERF-02 owns freshness policy; SEC-08 is retired historical-only alias | none |
| ARCH-03 adapters/benchmarks | 400 | PERF-01–05 and SEC-03–06 | none |
| ARCH-04 schema/replication | 400 | SEC-02/03, FUT-09 | none |
| ARCH-05 reconciliation | 450 | REC-01–08 and OPS-01/03; REC-09 is retired historical-only alias | none |
| ARCH-06 signer/PKI umbrella | 450 | Narrowed to generic signer/artifact authority seam | 500 High |
| ARCH-07 registry | 400 | Retained as ARCH-07, only registry/feature wiring | 450 High |
| ARCH-08 custody | 450 | ROOT-09/10 and OPS-10; SEC-07 is retired historical-only alias | none |
| ARCH-09 federation research | 200 | FUT-01–12; old score not carried because it conflicts with current scale (0–399 incident/critical reserved) | none |
| ARCH-10 minimal closure | new | Independent profile/dependency-closure proof | 450 High |

## Decisions still requiring evidence/review

- Which supported official Rust BWS/OP SDKs are stable, authorized and maintained versus CLI use; do not assert before code/docs retrieval and a synthetic benchmark.
- Exact provider consistency, pagination, revision/CAS, attachment and capability semantics; do not treat declared capability as granted permission. Until primary docs plus isolated tests show otherwise, BWS/OP/BW writes are unguarded for conflict-sensitive fields.
- Rust async runtime/library choice for secrets crate and whether a synchronous wrapper is needed; current broker is thread-based and must not dictate library consumers.
- Binary-size budget beyond existing `fleet-connect`; proposed consumer profile targets need measured baselines and owner approval.
- Typed private-key zeroization guarantees and algorithm/library support matrix, especially ML-DSA X.509/client support; validate against current primary sources before selecting dependencies.
- Whether `fleet-secrets-admin` becomes a separate crate after first implementation; decide from measured feature closure/consumer reuse, not naming aesthetics.


## Additive architecture packets (append-only update)

### ARCH-12 — Unified input-reference/output-target resolver

One reusable library surface normalizes typed file/directory/archive/stdin/structured bytes, PKI fingerprints, opaque IDs, qualified secret refs, non-root inventory, explicit signer handles and in-memory objects into `InputRef`; it separately plans file/archive/stdout/provider/memory/no-store targets. Ambiguity is a typed error, never first-match; signer handles are not authority; default inventory cannot authorize root signing; private output requires ARCH-06 `OutputAuthorization`. It delegates secret resolution to ARCH-01 and safe file/archive transactions to OPS-04. One cohesive `fleet-io` seam is justified by multiple callers, not split by format/input noun. V2, Backlog, L, High/600; dependencies ARCH-01/06 and OPS-04. No standalone CLI or new V1 behavior.

### ARCH-11 — FleetContext composition facade and CLI/library parity

One optional `FleetContext` facade composes ARCH-01 reads, ARCH-12 input/output plans, ARCH-06 authorization ports and domain services. It normalizes/dispatches requests but owns no domain policy and cannot mint signer/output authority. Existing CLI adapters and embedded Rust calls use the same typed operation path; synthetic in-memory stream tests exercise a two-sink pipeline without real services. V2, Backlog, L, High/700; dependencies ARCH-01/06/07/12. A god-object is the reversal trigger.

Addendum to the initial architecture packet set: ARCH-11/12 extend the architecture proposal list; ARCH-01/06/07/10 packet bodies remain unchanged.
