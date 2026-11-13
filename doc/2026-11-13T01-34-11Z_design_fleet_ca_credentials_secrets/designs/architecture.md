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
#[derive(Clone, Debug)]
pub struct ReadPolicy {
    pub max_age: Option<Duration>,
    pub stale: StalePolicy,          // Forbid | AllowUntil(Duration)
    pub allowed_sources: Vec<ProviderId>,
    pub require_version_proof: bool,
}
#[derive(Debug)]
pub struct ResolvedSecret {
    pub value: SecretBytes,          // redacted Debug; no Serialize/Display
    pub source: ProviderId,
    pub native_revision: Option<Revision>,
    pub fetched_at: std::time::SystemTime,
    pub stale: bool,
}
#[async_trait]
pub trait SecretStore: Send + Sync {
    fn descriptor(&self) -> &ProviderDescriptor;
    async fn get(&self, id: CredentialId, field: Option<&FieldId>, cancel: CancellationToken)
      -> Result<ProviderValue, ProviderError>;
    async fn list(&self, scope: &Scope, page: Option<PageToken>, cancel: CancellationToken)
      -> Result<Page<ProviderItem>, ProviderError>;
}
pub struct Resolver { /* policy + ordered adapters + bounded cache/single-flight */ }
impl Resolver {
    pub async fn resolve(&self, r: &SecretRef, p: &ReadPolicy, cancel: CancellationToken)
      -> Result<ResolvedSecret, ResolveError>;
    pub async fn resolve_many(&self, refs: &[SecretRef], p: &ReadPolicy, cancel: CancellationToken)
      -> Result<Vec<Result<ResolvedSecret, ResolveError>>, BatchError>;
}
```

`SecretBytes` owns zeroizing bytes where supported, has a deliberately redacted `Debug` implementation, and does not implement `Display` or `Serialize`; bytes are exposed only through an explicit closure (`with_exposed`). Secret-bearing values do not implement implicit `Clone`; any exceptional copy has a named, audited operation. `ResolvedSecret` may be debugged safely but metadata serializes only through a separate redacted receipt type. `SecretAlias { scope, name }` is a mutable lookup alias; it resolves to an opaque `CredentialId`, which remains stable across rename/relocation. Alias resolution must be explicit and scoped. Ordinary `Resolver` rejects `RootPrivate` and returns no signer capability. Root signer access is a separate authority plane and excludes ordinary resolver caches, prefetch, benchmarks and equality indices. Source selection is explicit policy, not “try any provider.” `resolve_many` preserves request order and per-item errors; it groups by provider/session internally and bounds concurrency. `ReadPolicy` defaults must be fail-closed; stale permission is explicit and class-dependent.

`ProviderDescriptor` carries stable ID/version and declared capabilities; capability discovery is separate from permission testing, and a successful list must include pagination completion evidence. For writes, later `fleet-secrets-admin` provides typed `Credential` / `SecretField` / `ProviderItem` mappings with ordered typed fields and encrypted lossless sidecar; provider-specific unknown fields cannot be silently dropped. Distinguish Bitwarden Secrets Manager (`bws`), Bitwarden Password Manager (`bw`) and `op`; credentials and auth scopes are separate.

Errors are stable machine classes with redacted context, not arbitrary stderr: `InvalidReference`, `NotFound`, `Unauthenticated`, `PermissionDenied`, `Unavailable`, `RateLimited { retry_after }`, `Unsupported { operation }`, `Stale { age }`, `Conflict`, `MalformedProviderData`, `Cancelled`, `Timeout`, `Partial { completed, pending }`, `Integrity`, `PolicyDenied`, `AuthorityPlaneDenied`, and `Internal`. Secret identifiers may themselves be sensitive; callers choose log-safe IDs/hash pseudonyms. Do not preserve payload or values inside provider error strings.

### Admin/write, reconciliation and custody

```rust
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

`fleet-secrets-admin` should be optional and may initially be a feature/module inside `fleet-secrets`; extract crate only when dependencies/users make its separate build valuable. It owns writer capability, reconciliation and audit journal. Runtime consumer cannot construct admin authority just by importing read resolver. Root signer handles are neither `SecretRef` nor `SecretBytes` and are issued only through a bounded, explicitly authorized signer session. Broker protocol already contains the #6767 metadata mitigation in source; deployment/canary evidence remains separate. Any future request-specific freshness contract is a new design requirement and must preserve old client compatibility; it is not a duplicate of #6767's merged per-secret refresh fix.

### Crypto, PKI, artifact IO

```rust
#[async_trait]
pub trait Signer: Send + Sync {
    fn public_key(&self) -> PublicKey;
    fn algorithm(&self) -> SignatureAlgorithm;
    async fn sign(&self, message: &[u8], c: CancellationToken) -> Result<Signature, SignError>;
}
/// Not constructible from an ordinary secret reference or provider read.
pub struct SignerSession { /* principal grant, key purpose, expiry, audit id */ }
pub trait KeyResolver: Send + Sync {
    fn resolve<'a>(&'a self, session: &'a SignerSession, r: &'a KeyRef,
                   purpose: KeyPurpose, c: CancellationToken)
      -> BoxFuture<'a, Result<Arc<dyn Signer>, KeyError>>;
}
pub struct PkiEngine { /* profiles, validation, graph, format-independent issuance */ }
impl PkiEngine {
    pub async fn issue_leaf(&self, req: LeafRequest, signer: Arc<dyn Signer>,
                            clock: &dyn Clock, c: CancellationToken)
      -> Result<IssuedCertificate, PkiError>;
}
pub trait ArtifactSink: Send {
    async fn write(&mut self, artifact: Artifact, c: CancellationToken)
      -> Result<ArtifactReceipt, SinkError>;
}
```

`fleet-crypto` owns algorithm/type registry and signer contracts; providers are optional adapters. `fleet-pki` owns CA/root/issuer/leaf/chain/CRL profiles and policy; it depends on crypto interfaces, not a specific key export or provider. Keep no-disk operation possible. `ArtifactSink` receives typed confidentiality classification; private key outputs require an explicitly authorized custody operation, sink permission and write receipts. PKI engine does not own provider replication or DNS mutation. A future `fleet-publish` seam is warranted only when more than one real publishing adapter exists.

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

Root signing authority is distinct from encrypted escrow; escrow destinations don't automatically become active signers. Preserve migration from 3 named initial escrow destinations to offline/hardware custody and separate recovery identity. R2 remains researched client-side encrypted backup only unless independent adoption decision. SOPS ciphertext in Git is limited to explicitly scoped deployment values; age private key stays provisioned outside Git and Nix store. fnox is a near-term research target but not a mandatory abstraction: benchmark its public Rust API/stability and adapter capabilities before embedding or subprocess use. SecureStore must prove added value against age/SOPS rather than add a fourth redundant format.

## Issue-ready proposal packets (for child issues under #9157)

Each packet is one independently verifiable deliverable. Issue creator should use native parent/dependency edges and the coordinator §4.2 contract; these are design proposals only, not created issues. Urgency values below are proposed scheduling inputs, not permission to dispatch. Negative cases are mandatory test assertions; oracles are explicit.

### ARCH-01 — Freeze runtime secret type and compatibility contract
- **Kind / stage / proposed urgency:** design+implementation / V2 / High 400.
- **Requirement IDs:** spec §§1, 8.4–8.6, 26.1, 35.2–35.4, 43, 48.1–48.2 V3-S02/V3-S07/V3-S13; addendum §§3, 9.
- **Outcome/scope:** extend `fleet-secrets` with typed reference, source metadata, class, revision and freshness policy; preserve old `get/list` behavior. No writes/admin APIs.
- **Acceptance:** API compiles for in-memory fake; stable redacted error enum; `fleet secrets get <name>` stdout/exit behavior unchanged; only explicitly policy-approved source selected; in-memory resolution avoids CLI spawn.
- **Negative tests/oracle:** no `Debug`/serialization yields value; denied/stale/ambiguous URI, empty response, malformed provider values fail; compare CLI golden stdout/stderr/exit fixtures to V1 baseline (oracle: committed fixtures + type-level no-Serialize constraints).
- **Boundaries/rollback:** `fleet-secrets`, its tests, minimal `fleet` compatibility adapter only. Revert additive types while retaining legacy resolver; no data migration.
- **Dependencies:** none; prerequisite ARCH-02, ARCH-03.

### ARCH-02 — Verify deployed #6767 mitigation with authorized live canary
- **Kind / stage / proposed urgency:** operations+test / V2 / High 450; **Blocked** pending deployment/canary authorization.
- **Requirement IDs:** spec §§8.4, 35.4, 37.2, 48.2 V3-S07/S11/S13; addendum §§2–3, 9; existing defect #6767.
- **Outcome/scope:** verify rollout of the already-merged #6767 fix and execute the authorized live canary after its separate approval. This is not an implementation ticket. The design phase does not deploy, read live values, rotate credentials, or run a canary.
- **Acceptance:** record deployed binary/source revision; verify deployed age/source metadata and per-secret refresh behavior with synthetic checks first; after explicit approval, canary a designated nonproduction/synthetic credential and record before/after value fingerprints, timestamps, observed refresh and rollback result without recording values. Confirm successful canary is independently read back from authoritative provider and broker.
- **Negative tests/oracle:** if source revision is not #8491-or-later, deployment is not actually observed, canary permission is absent, provider read fails, or observed value does not match authoritative post-rotation fingerprint, stop; do not label mitigation deployed/verified and do not retry mutation blindly. Oracle: deployed binary provenance + broker metadata + authoritative-source fingerprint comparison + canary receipt.
- **Boundaries/rollback:** deployment and canary are production-adjacent operations and are **Blocked** pending explicit approval and reviewed rollback; no edits in this design run. Rollback follows the deployed service's documented prior-binary rollback and preserves cache age/evidence. Do not remove last known-good behavior without owner signoff.
- **Dependencies:** ARCH-01 may define the separate typed freshness policy but is not a prerequisite for observing #8491. Requires deployment owner, live-canary approval, and a documented nonproduction credential/scope.

### ARCH-03 — Add capability-described read adapters and benchmark harness
- **Kind / stage / proposed urgency:** implementation+benchmark / V2 / High 400.
- **Requirement IDs:** spec §§34–35, 37–38, 42, 48.1–48.2 V3-S02/S11/S13/S14; addendum §§1–3, 6, 9.
- **Outcome/scope:** provider descriptor/read adapter for BWS and OP; `bw` remains a separate disabled adapter behind separately verified auth. Evaluate fnox public Rust API before choosing direct integration; harness supports resolver/broker, direct CLI/API where authorized, synthetic fixtures.
- **Acceptance:** capability/version/permission outputs distinguish unsupported, inaccessible and empty; reproducible cold/warm/list/batch p50/p95/p99, startup/RSS/error-rate output, redacted fixtures; no subprocess in embedded path when native client selected.
- **Negative tests/oracle:** 403/partial pages/rate limits never mean deleted/not-found; no bw credential inherited from bws config; no secret fixture content in output. Oracle: adapter conformance suite + known synthetic provider state.
- **Boundaries/rollback:** existing `fleet-secrets` modules/features and isolated harness; adapters off by default; preserve BWS CLI fallback.
- **Dependencies:** ARCH-01. No dependency on blocked deployment canary for synthetic benchmarks; production resolver/broker measurements must identify deployment revision and remain pending until ARCH-02 is authorized/completed. External provider docs/API/version research is a blocking subtask before adoption, not before BWS maintenance.

### ARCH-04 — Define canonical credential mapping and replication policy
- **Kind / stage / proposed urgency:** design+implementation / V2→V3 / High 400.
- **Requirement IDs:** spec §§19, 34.2–34.3, 35.3, 36, 38, 48.2 V3-S03/S04/S05/S08; addendum §§1, 5, 8.
- **Outcome/scope:** ordered typed credential fields, stable Fleet/native IDs, revision, provenance, classes, compartment + per-item replica policy; encrypted lossless sidecar for unmodeled provider fields. No provider writes.
- **Acceptance:** round-trip OP/BWS-compatible synthetic records, stable native IDs, per-field diff, policy explain includes read/write/replicate/export separately; root-private cannot enter default replication.
- **Negative tests/oracle:** unknown field not discarded; duplicate aliases and unsupported fields conflict instead of last-write-wins; roots excluded absent explicit high-assurance policy. Oracle: canonical JSON golden corpus and round-trip property tests.
- **Boundaries/rollback:** canonical types in existing `fleet-secrets` initially; no new crate until consumer/build evidence. Schema version parser and migration fixtures included.
- **Dependencies:** ARCH-01/03. No personal-vault sweep; `bw` access separately blocked pending authorization.

### ARCH-05 — Build plan/apply/recover reconciliation and nonsecret journal
- **Kind / stage / proposed urgency:** implementation+security tests / V3 / High 450.
- **Requirement IDs:** spec §§20–21, 36, 48.1–48.2 V3-S04–S10/S25; addendum §§1, 5, 8.
- **Outcome/scope:** deterministic three-way plan from verified baseline, per-field conflict resolution, staged writes, in-place updates, journaled saga, immutable receipts, guarded `zz.delete`, resume/recovery. Synthetic adapters only until separate first-sweep authorization.
- **Acceptance:** each plan pins source revisions, idempotency and approval scope; partial writes recover, successful mutation read-backs before completion; retention and protected backup restore test; no timestamps as conflict winner.
- **Negative tests/oracle:** timeout/403/partial pagination never becomes deletion; crash after write resolves uncertainty through readback; same-field double change flags conflict and keeps both values; journal/metadata contain no plaintext. Oracle: generated state-machine runs against reference model + failure injection.
- **Boundaries/rollback:** optional `fleet-secrets-admin` module/crate; nonsecret local journal and its storage adapter; existing GitHub creation outbox is not reused. Rollback by pausing apply, preserving journal and resuming/compensating—not deleting history.
- **Dependencies:** ARCH-04; ARCH-03 provider write capability tests. First live write is separately approval-blocked; reconciliation tests do not wait on the deployment canary.

### ARCH-06 — Add crypto signer interfaces and PKI engine library
- **Kind / stage / proposed urgency:** design+implementation+test / V3 / High 450.
- **Requirement IDs:** spec §§3–7, 9, 24, 26, 28, 44, 48.1–48.2 V3-S15–S22; addendum §9.
- **Outcome/scope:** typed algorithms/signature path, signer handle abstraction, policies and in-memory artifact flow; four independent roots, nine asymmetric cross-certifications and A/B purpose partitions verified by synthetic keys. Implement requested roots-only alternative and direct eligible cross-signs; ML-DSA-44 baseline, RSA-4096 production, RSA-2048 gated.
- **Acceptance:** library API equivalent core semantics to CLI plan; verifier matrix with independent implementation; no-disk memory sink; explicit unsupported-client outcomes; generated Fleet artifact fixtures pass independent checks.
- **Negative tests/oracle:** classical path rejected by PQ-only; missing ML-DSA client capability is not “fixed” by cross-sign; invalid OID/extension/expiry and RSA-2048 long-root policy reject; unsupported OCSP never embedded. Oracle: reference chain model + OpenSSL/independent Rust verifier test corpus, with version pins.
- **Boundaries/rollback:** proposed `fleet-crypto` and `fleet-pki` only if dependency graph supports isolation; CLI remains `fleet`; no new CA daemon. Synthetic keys only during planning.
- **Dependencies:** ARCH-01 for typed key/secret refs, plus separate crypto library research/decision. No dependency on production custody or OpenBao.

### ARCH-07 — Integrate profile features into existing Fleet command registry/builds
- **Kind / stage / proposed urgency:** implementation+packaging tests / V2→V3 / High 400.
- **Requirement IDs:** spec §§1, 6, 25–26, 29, 43, 48.1 V3-S24; addendum §9.
- **Outcome/scope:** add `secrets`, `generate`, `cert`/optional `pki` alias to existing registry and one binary; use selected features/profiles `consumer-min`, `agent`, `fleet`, `admin`, `serve`; generated help from single source; keep old aliases/exit contracts.
- **Acceptance:** each binary's compiled/profile/effective capability matrix is tested; unavailable command responds in established registry format; JSON help schema; all existing variants build; connect size gate unchanged.
- **Negative tests/oracle:** no cert/generate in connect artifact; no profile name widens compile capabilities; aliases normalize to one typed request; config-disabled command cannot run. Oracle: capability golden table, smoke invocations and Nix final-size gate.
- **Boundaries/rollback:** existing `fleet-registry`, `fleet` manifests, `pkgs/fleet/default.nix`, docs. Revert feature composition without altering the old `secrets get/list` interface.
- **Dependencies:** ARCH-01/06; coordinate with the separate `fleet` binary/profile owner. Admin-profile packaging requires ARCH-05.

### ARCH-08 — Specify root custody, escrow and recovery adapters
- **Kind / stage / proposed urgency:** security design + synthetic recovery test / V3 / High 450, then Blocked for production.
- **Requirement IDs:** spec §§3, 10, 24, 30, 38, 48.2 V3-S08/S09/S18/S25; addendum §§5, 7.
- **Outcome/scope:** model active signer separate from encrypted escrow; initial destinations exactly encrypted laptop + authorized BWS project + authorized OP vault; checksums, independent restore proof, SOPS/age recipient separation; R2 evaluated only. Offline/HSM migration plan.
- **Acceptance:** threat/failure-domain and compromise graph; synthetic archive can restore on isolated environment with one independent recovery identity; custody manifest lists per-root identities, revision, fingerprint, authorized destinations and verification receipts.
- **Negative tests/oracle:** no root key enters general sync/cache/provider; no same-host three-container “HA”; R2 not auto-enabled; recovery secret absent from archive/repo. Oracle: custody state model and synthetic restore test.
- **Boundaries/rollback:** design and synthetic fixture only until explicit production approval. Rollback means stop replication and preserve verified escrow copies; do not silently remove the last recoverable copy.
- **Dependencies:** ARCH-04/05/06. Production root generation/export/upload blocked by separate authorization and go/no-go.

### ARCH-09 — Research V4 federation and workload identity contracts
- **Kind / stage / proposed urgency:** research / V4 nonblocking / Standard 200.
- **Requirement IDs:** spec §§27, 35, 37–42, 46 R-SEC-05–18; addendum §§4, 7–9.
- **Outcome/scope:** evidence-backed evaluation of OpenBao (lab then independent failure domains), SPIFFE/SPIRE, fnox, SOPS/age, SecureStore, R2/Cloudflare, Kubernetes operators and identity; define stable provider/policy hooks, ownership and go/no-go criteria. No runtime dependency or deploy.
- **Acceptance:** each candidate has primary-source links retrieved/date, actual maintenance/API/capability findings, threat and failure graph, cost/availability limits, pilot fixture and explicit no-go. fnox API stability and provider write coverage measured/reviewed; SecureStore compared to age, not presumed additive.
- **Negative tests/oracle:** three containers on one host cannot pass independence test; hosted outage fallback disallowed without freshness/version proof; secret replicas cannot broaden scopes; dynamic credentials not represented as static sync. Oracle: explicit failure/compromise matrix and reproducible local-lab evidence.
- **Boundaries/rollback:** docs and isolated synthetic labs only. Research can be parked without blocking V2/V3.
- **Dependencies:** ARCH-01/02/04 for meaningful interface assessment; does not block PKI or secrets core.

## Decisions still requiring evidence/review

- Which supported official Rust BWS/OP SDKs are stable, authorized and maintained versus CLI use; do not assert before code/docs retrieval and a synthetic benchmark.
- Exact provider consistency, pagination, revision/CAS, attachment and capability semantics; do not treat declared capability as granted permission.
- Rust async runtime/library choice for secrets crate and whether a synchronous wrapper is needed; current broker is thread-based and must not dictate library consumers.
- Binary-size budget beyond existing `fleet-connect`; proposed `consumer-min`/`agent` targets need measured baselines and owner approval.
- Typed private-key zeroization guarantees and algorithm/library support matrix, especially ML-DSA X.509/client support; validate against current primary sources before selecting dependencies.
- Whether `fleet-secrets-admin` becomes a separate crate after first implementation; decide from measured feature closure/consumer reuse, not naming aesthetics.
