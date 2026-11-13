# Fleet PKI issuance, publication, and trust design

**Status:** Proposed domain design for #9164. This is not a production runbook or authorization to generate/sign production CA material. V1 has no Fleet PKI implementation. Every API below is a design contract, not a claim that the command or crate exists.

## 1. Scope, authority, and source precedence

This design covers purpose-specific issuer sets, leaf issuance and lifecycle, CSRs/CMS delivery kits, certificate profiles, URI publication, CRL/OCSP status, and trust-installation boundaries. It depends on the separate root/custody design (#9163) for root graph and signer handles, and on shared Fleet APIs (#9157/#9158) for registries, secret/key references and receipts.

Precedence: current assignment and the dated addendum refine their points; the v0.3 spec governs otherwise; its §33 resolution supersedes historical alternatives. The coordinator handoff §4.2 is the acceptance contract for child issues. The complete implementation prompt is future-stage context, not permission to implement production PKI now. Intake `note.md` binds this run to design/synthetic proof only: no production CA generation/signing, root escrow/export, live vault write/reconciliation, DNS or trust-store change, or deployment.

**Source facts** below are attributed to source records/standards. **Decisions** and API names are design proposals. Where a source conflict remains, it is made an explicit gate rather than concealed.

## 2. Source-backed invariants and scope

### 2.1 Requirements and facts

* The reference inventory has four independent root keys/certificates: RSA-4096, P-256, Ed25519 and ML-DSA-44; 9 asymmetric root cross-certificates (6 classical-to-classical, 3 classical-to-ML-DSA), no ML-DSA-root signature over a classical root; four self-signed roots + 9 cross-certs = 13 root certificate variants. PQ-only validation trusts the ML-DSA root and requires all-ML-DSA path signatures. (Spec §§3.1–3.5; §24.2; Appendix A PKI-001–008.)
* Base and Internal Web are sibling purpose sets. Each has A/B independent keys for each of four algorithms: 8 keys per set, 16 issuer keys total. The spec's explicit signing matrix gives, per partition and set, 3 classical issuer keys × 3 classical signing roots plus 1 ML-DSA issuer key × 4 eligible roots = 13 certs; ×2 partitions = 26 per set, 52 issuer CA certificates total. Add 13 root variants for 65 CA certificates, excluding leaves. Same-key cross-signed certificates share SPKI/SKI but differ in issuer, AKI, serial, signature, fingerprint, path and revocation issuer. (Spec §24.2; Appendix A PKI-068; handoff §§7.5–7.6.)
* **Count clarification required:** the current task wording describes “32 issuer cert variants” plus “20 direct intermediate cross certs”; §24.2 instead specifies 52 issuer certificates directly from its signer matrix. Both sum to 52, but the split is not derivable unambiguously from that matrix (one primary cert per key plus every other eligible signature yields 16 + 36). This design treats **65 total and the §24.2 per-algorithm signer matrix as hard acceptance**, and requires a named, enumerated 52-row certificate graph in V2 before implementation. The proposed 32+20 partition is a reporting label only until its exact signers are approved; it must not reduce eligible signer edges or change total count silently. CERT-02 owns this reconciliation.
* Base and Internal Web reference CA expiry is `2032-07-07T12:00:00Z`; root reference expiry is `2050-07-07T12:00:00Z`. General defaults remain root 20 calendar years, issuer 5 years, ordinary web leaf 90 days, and Base reference leaf 1 year. No other special Internal Web date is invented (the same 2032 reference date is now explicit in §33.2). (Spec §§24.1, 24.5, 33.2; Appendix A PKI-019–023.)
* Leaf certificates have a single SPKI and single issuer signature. If several algorithms are requested, issue separate key/certificate variants with the same approved identity set; do not describe these as a multiply-signed certificate. Ed25519 uses its intrinsic signature; ML-DSA-44 follows its PKIX algorithm identifier and mode, not an injected SHA-256 digest. (Spec §§3.4, 24.6; RFC 5280; RFC 9881.)
* Base may issue a generated test identity without DNS SAN; TLS/Web issuance requires explicit DNS/IP identity or validated issuer defaults. Web default SAN shorthand for `developing.today` and `developing-today.com` creates apex and wildcard for each. Wildcard is only a complete leftmost DNS label and matches one label. CN is not DNS identity; SAN is. Private `Internal Web` or `Public Web` issuers do not become publicly trusted because their names are public DNS names. (Spec §§24.6, 33.2; RFC 9525.)
* Leaf AIA names actual issuer candidates; CDP names the CRL signed by that certificate's issuer. A cross-signed CA cert's issuer is the signing root, and it belongs on that root's CRL. URLs must be complete immutable object paths for the containing certificate's lifetime and point to real objects. HTTP is used for broad AIA/CDP compatibility; clients can ignore AIA or stop after one candidate, so composed chains are the interoperability guarantee. Triple mirrors are supported, not proof of failover. (Spec §§4.1–4.3, 24.7; RFC 5280.)
* CRL publication is part of issuance readiness whenever CDP is embedded: create a real signed empty CRL before publishing the certificate; track issuer key, CRL number, thisUpdate/nextUpdate and revoked serials; roots revoke intermediate and cross-cert variants they signed; issuer CRLs revoke leaves. OCSP is not advertised without a real authoritative responder for each represented issuer. (Spec §§4.3, 24.7; RFC 5280 §§5–6; RFC 6960.)
* Addendum 5 requests temporary explicit escrow to three approved destinations—encrypted laptop storage, approved BWS project, approved 1Password vault—with encryption, access restriction, integrity and restoration tests; SOPS/age archives and independently protected recovery recipients; possible private client-encrypted R2 is research only. This supersedes a generic no-replication default for the initial root keys but does not authorize this design session to execute it. It does not authorize all future providers or automatic root-key replication. (Addendum §5; `note.md` §26–30; spec §§33.2, 34.2.)

### 2.2 Decisions proposed here

1. Model issuer keys, issuer certificates, issuer sets and purpose profiles as distinct entities. A signer key can sign multiple certificate variants for its subject key; variants are never conflated with key identity.
2. Model direct issuer cross-signing as a versioned graph policy (`roots-only`, `issuers-direct`, `both`, `none`). Reference fixture uses `both`; new ordinary issuer sets may use `roots-only` after compatibility evidence. V2 owns the explicit graph enumeration and total check.
3. A default active issuer is an explicit selected `(issuer_set, algorithm, partition, certificate_variant)` tuple. Never infer it from file ordering, most-recent timestamp, trust store contents or a successful lookup.
4. `InternalWeb` is private trust, with `serverAuth` leaves, validated DNS constraints when supported, and explicit client trust installation. Public WebPKI remains an independent ACME/public-CA product and is outside this issuance path.
5. Bind a request to exactly one authorized signer identity per issuance operation. No provider, algorithm, parent certificate, or key-custody fallback is silent. A cross-sign graph action enumerates every planned signer in advance and requires authorization for each root key actually used.

## 3. Domain model and API contract

### 3.1 Typed objects

```rust
// Illustrative API contract; exact names/types belong to implementation.
struct IssuerSetRef { bundle_id: Uuid, generation: u32, purpose: Purpose }
struct IssuerKeyRef { key_id: Uuid, algorithm: SignatureScheme, partition: Partition }
struct IssuerCertRef { cert_id: Uuid, subject_key_id: Uuid, signer_key_id: Uuid,
    issuer_name: DistinguishedName, serial: Serial, fingerprint: Sha256,
    generation: u32, partition: Partition, revision: u32 }
struct SigningAuthorization { transaction_id: Uuid, root_key_fingerprint_or_handle: KeyRef,
    subject_spki_id: OpaqueId, signature_algorithm: SignatureScheme,
    profile_purpose: Purpose, exact_validity: Validity,
    artifact_plan_digest: Sha256, authorized_destinations: Set<DestinationRef>,
    approval: ApprovalRef, expires_at: UtcTime, single_use: bool }
struct IssuerProfile { purpose: Purpose, subject_template: SubjectTemplate,
    key_usage: Set<KeyUsage>, eku: Set<Eku>, path_len: Option<u8>,
    name_constraints: NameConstraintsPolicy, validity: ValidityPolicy,
    aia: PublicationSet, cdp: PublicationSet, leaf_defaults: Option<LeafTemplate> }
struct LeafRequest { issuer: IssuerSetRef, subject: Option<DistinguishedName>,
    spki_or_csr: SubjectKeyInput, purpose: Purpose, sans: Vec<San>,
    validity: ValidityRequest, algorithm_variants: Vec<SignatureScheme>,
    partition: Option<Partition>, profile: DeliveryProfile }
struct IssuanceReceipt { operation_id: Uuid, status: OperationStatus,
    artifact_refs: Vec<ArtifactRef>, cert_fingerprints: Vec<Sha256>,
    signer_receipts: Vec<SignerReceipt>, publication_receipts: Vec<PublishReceipt>,
    warnings: Vec<PolicyWarning>, rollback: Vec<RecoveryAction> }
```

`Purpose` is a closed typed registry (initially `Base`, `InternalWeb`, future `PublicWeb`, `ClientAuth`, `CodeSigning`, `Device` only when registered). Profile policy controls eligibility. Every logical object has a stable opaque ID independent of subject/display name, slug, filesystem path, DNS identity, provider-native ID and logical secret path; the path is a locator/attribute, not identity. This is the shared cross-domain ADR-S01 constraint, not a second implementation of Fleet's canonical secret identity. IDs and generation/partition/revision are structured, not recovered by parsing a filename. Store key handles as opaque references plus exportability/capability metadata. Root private material is outside the ordinary resolver, hot caches, prefetch, benchmark datasets and equality indices; only a bounded explicitly authorized signer operation may hold it. Keep private and public manifest projections separate where public infrastructure metadata would reveal sensitive store paths.

### 3.2 Rust API / CLI relationship

Proposed facade:

```rust
trait Signer {
    fn descriptor(&self) -> SignerDescriptor;
    async fn sign(&self, authorization: &SignerAuthorization,
                  algorithm: SignatureScheme, exact_tbs_der: &[u8])
        -> Result<Signature, SignerError>;
}
async fn create_issuer_set(req: IssuerSetRequest, ctx: &mut IssuanceContext)
    -> Result<IssuanceReceipt, PkiError>;
async fn issue_leaf(req: LeafRequest, ctx: &mut IssuanceContext)
    -> Result<IssuanceReceipt, PkiError>;
async fn renew(cert: IssuerCertRef, req: RenewRequest, ctx: &mut IssuanceContext)
    -> Result<IssuanceReceipt, PkiError>;
async fn rotate(issuer: IssuerSetRef, req: RotateRequest, ctx: &mut IssuanceContext)
    -> Result<IssuanceReceipt, PkiError>;
```

The CLI is a thin Fleet-registry entry point to the same API. Planned surface: `fleet cert issuer create|inspect|list|renew|rotate|replace|restrict`; `fleet cert leaf issue|inspect|list|renew|rotate`; `fleet cert csr inspect|verify|issue-plan`; `fleet cert chain build|select|test|graph`; `fleet cert crl build|inspect|verify|publish|status`; `fleet cert publish plan|stage|check|commit|rollback|status`; `fleet cert trust export|install-plan|test`. An alias such as `fleet pki` is optional and may not duplicate behavior. Keep existing `fleet secrets get|list` semantics unchanged.

All mutating calls accept structured requests and return typed receipts; public artifacts may be streamed to caller-provided sinks. Low-level API includes injected clock, deterministic test-only entropy, explicit signer adapter and in-memory output sink. Production API has no fake signer. `--dry-run` performs all non-secret preflight and renders planned effects without opening the signer, reserving irrevocable serials, writing stores, publishing, or changing DNS/trust.

### 3.3 Capability and signing adapter contract

`SignerDescriptor` declares key ID, public SPKI/fingerprint, custody kind, exportability, accepted signature schemes/parameters, synchronous/asynchronous status, supported object types (certificate/CRL/OCSP), authorization source and verification method. Every root signing operation consumes a transaction-bound `SigningAuthorization`: root key fingerprint/handle, subject public-key identity + signature algorithm, profile/purpose, exact validity, artifact-plan digest, authorized output/escrow destinations, expiry, and single-use by default. The signer refuses any mismatch and returns a receipt naming the transaction. Inventory lookup, escrow read access, or holding key bytes never mints authorization. An adapter signs exact input bytes and returns an auditable receipt. Verify public-key correspondence and signature before finalizing output. A signer failure is terminal for that planned signer. Never generate a software key or switch from ML-DSA to Ed/RSA, from HSM to file, or from one root signer to another as fallback. (Architecture review A-2.)

Adapters are independently feature-gated: explicit native RustCrypto components, rcgen, and OpenSSL are not interchangeable contracts. Before pinning, issue per-algorithm + per-object proof (certificate/CRL/CSR/CMS where applicable), independent verify, key custody, platform build and license/version evidence. Reject a missing primitive with `unsupported_algorithm` or `signer_unavailable`; do not silently substitute. The evaluated capability table and source links are in [`research/pki-standards.md`](../research/pki-standards.md).

## 4. Issuer, CSR, leaf, renewal and revocation workflows

### 4.1 Issuer-set creation

1. Parse/normalize arguments and schema; reject unknown flags, duplicate IDs, invalid date combinations, unsafe DNS, unsupported alg/profile combinations before any side effect.
2. Resolve public parent metadata and proposed key handles. Signing requires an explicitly supplied authorized root source / signer handle (or an authenticated offline transaction); catalog default discovery is public metadata only. Check each signer authorization is scoped to this operation, purpose, algorithm, certificate count, expiration and approval.
3. Validate all profile inputs and exact graph edges. Generate or import independent issuer keys for four algorithms × A/B × Base/InternalWeb = 16 distinct key IDs; ensure distinct SPKI/SKI for all 16.
4. Allocate collision-free positive CA serials per issuing key under a durable issuance ledger. Serial allocation must be idempotent on retry and never reused after a signature attempt; gaps are acceptable.
5. Construct CA cert TBS with CA=true, critical BasicConstraints and KeyUsage `keyCertSign,cRLSign`, SKI/AKI, issuer-specific validity, pathLen 0 for leaf-only issuer, profile/policy extensions, issuer-correct AIA and CDP. Signing algorithm and OID are validated for key type; Base and Internal Web are siblings.
6. Sign only the enumerated edge list; parse and verify each generated cert; assert SPKI, subject, issuer, serial, validity, constraints, algorithm, signature, AKI/SKI, expected edge and manifest agree.
7. Build initial empty CRLs for each actual CRL signing authority before any CDP-bearing certificate becomes publishable; validate CRL signatures/issuer/AKI/number/times.
8. Create public/private manifests, deterministic artifact index, profile and test results. Initial status is `staged`, not `active` or `published`.

No sample operation is executable against production in V1/V2. A V3 live operation requires an explicit approval token/policy for each root signing action and custody destinations. The addendum's exact three-copy root escrow applies only to root #9163 and must use separate, explicit encryption/integrity/readback/restore steps; it is not part of routine issuer key sync.

### 4.2 Count and cross-certificate matrix gate

The immutable V2 synthetic fixture must contain:

* 4 root key identities; 13 root CA certificate variants.
* 16 distinct issuer key identities (Base A/B + InternalWeb A/B × four algorithms).
* 52 total issuer CA certificate variants under the §24.2 graph; 65 CA certificates total; no leaves included in that count.
* Per-purpose set = 8 issuer keys and 26 certificates. Every certificate variant has a distinct `(issuer DN, serial)` and fingerprint; variants for same issuer key have identical SPKI/SKI and different issuer/AKI/signature.

**Unresolved decomposition:** user wording gives 32 “issuer cert variants” plus 20 “direct intermediate cross certs” while §24.2's exact matrix derives 52 entries with 36 additional signatures over one-per-key baseline. The expected grand total agrees (52). The `CERT-02` issue must make a reviewed, explicit inventory/graph with stable cert IDs and signer edges. Until then, do not invent which 20 edges are omitted, call 32 certs “all eligible variants,” or issue a misleading “65 passed” report. The oracle is the signed graph manifest compared against source matrix and exact count.

### 4.3 CSR intake and issuance

PKCS #10 CSR is untrusted input. Decode strict DER/PEM with size/depth bounds; verify CSR self-signature/proof-of-possession; validate subject and SPKI; reject unsupported algorithm OIDs, malformed/duplicate extensions, unknown critical extensions, oversized attributes and invalid SAN syntax. Treat requested SAN, subject, extensions, validity and key usage as proposals only. Recompute approved extension set from issuer policy; no caller CSR extension may weaken CA constraints or elevate purpose. Compare exact CSR key SPKI with the issued certificate SPKI. Record CSR digest and approved request in receipt; do not retain raw CSR unless policy asks.

Support submit/issue workflow through Fleet APIs and offline file transfer. An issue plan shows normalized identities, requested-vs-approved fields, issuer/key/partition, validity, output and signer action. A signer authorization is checked immediately before signing. Repeated request with same idempotency key returns existing receipt only if request hash is identical; changed CSR or SAN requires a new operation.

### 4.4 Leaf issuance and SAN rules

* `Base` test mode may auto-create a generated non-host identity and visibly state it is not suitable for hostname validation without a DNS SAN. Production Base issuance requires an explicit intended purpose.
* `InternalWeb` needs SANs or a validated profile template. `--dns developing.today --dns developing-today.com --wildcard` normalizes to four SANs: both apexes and `*.developing.today`, `*.developing-today.com`, deduped. `--wildcard-only` is distinct; no apex inferred under this flag.
* Accept DNS SAN only after IDNA A-label normalization, FQDN syntax and profile/name-constraint checks. Wildcard must be one `*` as the complete left-most label; reject `*.*.example`, `foo*.example`, wildcard IP/URI/email, public-suffix wildcard under configured PSL evidence, CN-as-fallback, invalid A-label and constrained-out name. IP SANs are octet addresses; URI SAN requires URI profile. Do not trust alternate spellings or shell/path separators.
* Leaf KU/EKU and key profile are algorithm-correct. Web sets `serverAuth`; never add keyEncipherment universally. If multiple algorithms are selected, create separately keyed leaves per variant, but the requested signature algorithm and server/client support matrix remain explicit.
* Leaf notAfter must not exceed issuer/root validity or policy, and must be later than notBefore with configured clock skew. Emit `notBefore/notAfter` in UTC and receipt exactly.
* Default output minimizes private-key exposure: public certificate, chain candidates, CSR/fingerprint/manifest and key handle only. Private key export/PFX requires explicit protected destination plus separate approval; unencrypted archive is blocked absent a specific hazard acknowledgement.

### 4.5 Renewal, rotation, replacement

`renew` reissues certificate(s) using the same key identity only under a key-reuse policy; increments cert revision and serial, does not claim key rotation. `rotate` creates new independent key identity and increments generation; certificate keys do not alias prior generation. `replace` is orchestration: create → distribute → verify consumer readiness → activate → overlap → deprecate old → revoke/retire per plan. Never revoke old issuer/root automatically as soon as new material exists. Require explicit active partition, transition window, rollback, affected-leaf inventory, CRL and trust plan.

Retry/resume reads durable operation/manifest receipts and does not re-sign or overwrite a different key/certificate silently. A manifest drift causes `needs_review`, not a “repair” that replaces keys. Signing source unavailability leaves operation resumable at a typed `waiting_for_signer` state.

### 4.6 Revoke, CRL and OCSP

Revocation is an append-only state transition keyed by `(issuer_key_id, serial)`, with reason/time/approver/evidence reference. No un-revoke operation by default; `certificateHold` semantics require a separately reviewed policy. A revocation request does not mutate signed artifacts directly: it transitions `requested → authorized → CRL_pending → CRL_signed → staged → published → verified` and creates affected dependent inventory/notification.

CRL operation is per signing CA and monotonic CRL number. A CRL build snapshots authorized revocations and explicit validity window; it signs exactly once, verifies, stores immutable CRL and advances current head only after signature success. Publication uses immutable CRL-number paths plus controlled latest alias if needed; old URIs remain available. CDN content must match manifest hash and MIME. Stale/missing CRL creates `revocation_unavailable`; policy decides block new issuance/publication and warns consumers. Never report `good` based solely on absence in an unavailable CRL.

OCSP is **unsupported/deferred** unless a real responder is separately built and authorized for each CA issuer key, with complete issuance/status database, responder certificate/KU/EKU, key custody, freshness, nonce/cache, rate limiting, request validation, audit and publication. No `id-ad-ocsp` URI is placed in AIA by default. Unsupported issuer/algorithm/status returns RFC-appropriate `unknown`/`unauthorized`/`tryLater` as applicable; do not synthesize `good`. OpenBao's documented ML-DSA OCSP limitation is a known no-go for that route, not a reason to emit a false endpoint. RFC 6960 `good` is not proof of issuance.

## 5. Immutable output, AIA/CDP, publication, and trust

### 5.1 Artifact plan and paths

Manifest schema `fleet.pki.bundle/v1` is canonical JSON; YAML is a rendering. It contains bundle/key/cert IDs, purpose, generation/partition/revision, parent refs, exact signer identity, request/profile hash, algorithms/parameters, DER hashes, serial/issuer/SKI/AKI/fingerprint, validity and extensions, key handles and exportability (no key bytes), signer receipts, CRL number/freshness, trust profile, publication refs/status, tested client matrix, tool/library versions, policy decisions/warnings and rollback/recovery state.

Private/public manifest projections avoid leaking secret-store URIs or sensitive topology. Paths are deterministic from opaque IDs/validated slugs and are never built from raw display strings; changing a path or display name does not change object identity. An output path is created with no-follow/symlink-safe semantics; writes use restrictive permissions and staged temp directory on same filesystem; atomic rename commits a local public-artifact set. Bundle checksum is over exact file bytes. PEM/DER encodings, CMS content and chain order are explicit metadata.

Suggested immutable public layout:

```text
certs/issuers/<purpose>/<algorithm>/<generation><partition>/<cert-id>.crt
certs/issuers/<purpose>/<algorithm>/<generation><partition>/ca.p7c
certs/leaves/<profile>/<identity>/<generation>/<algorithm>/<cert-id>.crt
certs/chains/<profile>/<identity>/<chain-id>.pem
crl/<issuer-key-id>/<crl-number>.crl
```

Human/stable aliases may point at current objects but are not embedded in certificates unless durable for certificate lifetime. Proposed mirror set is primary + secondary public + private-use `.internal`, all configurable. Public URLs must not require authentication or HTTPS bootstrap; `pki.developing.today` is illustrative, not asserted deployed. DNS changes are a separate manifest and explicit operator operation.

### 5.2 AIA/CDP construction

For each to-be-issued certificate, derive AIA CA Issuers from its actual issuer certificate/key candidates and CDP from that issuer key's signed CRL. For cross-certificate variants, use the signing root's issuer candidates and CRL. Root self-signed anchors normally omit AIA/CDP. Every embedded URL must resolve in the planned publication manifest to an exact immutable artifact with correct bytes/MIME. Issuer CA Issuers may expose single DER cert (`application/pkix-cert`) and optional certs-only CMS (`application/pkcs7-mime`); CRL uses `application/pkix-crl`.

Before `publishable`, independently check all referenced artifacts exist, signatures are valid, URLs are complete, three mirror manifests are consistent (if configured), and a signed empty CRL is ready for every certificate issuer's CDP. Do not make HTTPS redirects a prerequisite. AIA download/fallback is opportunistic; generated chain packages identify exact candidates for each trust anchor and precompose tested chain alternatives.

### 5.3 Publication transaction and rollback

Publication is an explicit plan with deterministic artifact rows `(source, immutable destination, content type, hash, visibility, cache control, mirror, DNS prerequisite)`. Flow: `planned → staged → verified → committed → mirror_verified`; each step is idempotent, records receipt, and checks server bytes/hash/MIME/HTTP status. Certificates/CRLs are never overwritten at immutable paths. `latest` aliases are promoted last. DNS records are emitted as BIND fragment + structured provider-neutral JSON/YAML, with unresolved values represented as non-executable placeholders; `apply-dns` is a distinct gated operation.

If any mirror fails before alias promotion, retain immutable staged objects and mark partial; retry only missing objects. For each adapter and operation record write guard as `atomic-precondition`, `serialized-by-contract`, or `unguarded`. Until primary documentation and isolated tests prove otherwise, treat BWS, op, and bw as unguarded. Conflict-sensitive updates require one of the first two guard levels; Fleet lease/readback is not serialization. On an unguarded target the plan stops in `needs_review`, unless a separately approved named weaker policy states the lost-update window in both plan and receipt. Additive operations (creating immutable artifacts/history) are not gated by this conflict rule. Use compare-and-swap only where the specific publication adapter/version documents and proves conditional update; otherwise serialize through a documented contract or stop. If a bad certificate/CRL has already been published, do not delete evidence: publish corrected new version, restore alias only with a safe guard, revoke bad issued certificates where appropriate, and retain cache/consumer remediation. Rollback cannot retract cached data; report residual exposure. Never reuse serials or rewrite an immutable URL. (Architecture review A-1/S02.)

### 5.4 Trust model and installation kits

Trust profiles are explicitly separate artifacts:

* `roots-all`: four self-signed anchors, compatibility profile; no PQ-assurance claim.
* `roots-classical`: RSA/P-256/Ed25519 anchors.
* `roots-pq`: ML-DSA root only + enforcement policy that every signature on path is ML-DSA.
* Cross-certificates and intermediates are untrusted path-building candidates, never silently installed as anchors.

`fleet cert trust export` writes public kits and fingerprints. `install-plan` prints OS/app-specific steps; `install` is later and privileged/approval-gated, with explicit scope (user vs system) and exact root fingerprint confirmation. No global import during issuance/publication. Guides distinguish CA trust installation from server leaf/private-key deployment and native SSH CAs. Every tested consumer row has install/verify/rotate/overlap/remove; unsupported algorithms are documented, not papered over. A trust package does not guarantee TLS signature negotiation, key exchange, revocation enforcement, or AIA discovery. Confirm RFC 9525 hostname checks and PKIX path validation separately.

## 6. Operation states, failure, and phase rollback

### 6.1 Issuance state machine

`draft → normalized → preflight_ok → approval_pending → authorized → key_ready → serial_reserved → signed → self_verified → independently_verified → staged → published → active → superseded/revoked/expired`.

Terminal/paused states include `rejected`, `unsupported`, `denied`, `waiting_for_signer`, `partial`, `needs_review`, `failed_recoverable`, `failed_terminal`. State transitions are append-only receipts with request hash, policy version, actor/approval, signer ID, output hashes and causation/idempotency IDs. `active` requires policy checks + verification and, when URLs are embedded, published CRL/certificate objects. Publication is separate from issuance; trust installation is separate from both.

### 6.2 Pre-side-effect argument contract

Every operator CLI/script has a declared command-line contract and strict parser before subprocess, network, file write, lock, signer, serial reservation, secret-store access, publish or deploy. Unknown/unaccounted arguments are usage errors. `--` is recognized only if explicitly declared. No shell expansion in batch plans. This applies even to wrappers that call Fleet/OpenSSL. In application code the same normalization/validation is a pure preflight stage. Test with extra args, typoed flags, duplicate/conflicting modes and malicious path values, asserting no file/network/signer/subprocess side effect.

### 6.3 Rollback boundaries

* Before `signed`: cancel operation; release non-secret lock; reserved serial remains consumed if reservation was durable.
* After signer invocation/signature exists: preserve signature and operation evidence; do not call signer again on retry without idempotency proof. Mark artifact nonpublishable until exact self/independent verification.
* After local staging: delete only operation-owned staged files after manifest/hash check; never broad recursive cleanup or overwrite user files. Otherwise quarantine staged artifacts for inspection.
* After publication: immutable objects remain; roll aliases back by CAS, publish corrected artifacts at new IDs, retain incident receipt and notify affected clients. CRL revocation state is monotonic and cannot be rolled back by deleting entries.
* After activation/trust import: use staged overlap and explicit uninstall/removal plan, preserve old trust during recovery unless incident policy says remove immediately; an incident response is not routine rollback.

## 7. Verification and test oracles

Test oracles are explicit: canonical graph counts/edge list; exact bytes/manifest hashes; independent public-key signature verification; RFC profile assertions; reference trust/path validator; HTTP endpoint bytes/MIME; CRL number/serial membership/freshness; CSR PoP; certificate key-SPKI equality; policy state transitions. Parsing success alone is never an issuance oracle.

### 7.1 Required fixture and interoperability matrix

1. Generate deterministic *synthetic* root and issuer key fixtures with test-only entropy or fixed approved test keys. Assert root 4 keys/13 certs; two issuer sets, 16 distinct keys/52 certs; total 20 CA key IDs/65 certs; no reverse ML-DSA→classical edge. Validate explicit graph certificate IDs against the reconciled CERT-02 matrix.
2. Assert same-key signer variants preserve SPKI and SKI while issuer, AKI, serial, signature, issuer CRL owner and fingerprint differ. Assert A/B keys differ in key ID, SPKI, SKI and private-key test fixture.
3. Test Base and InternalWeb expiry exactly `2032-07-07T12:00:00Z`; root exact reference `2050-07-07T12:00:00Z`; defaults and leap-day arithmetic independently.
4. CSR valid PoP accepts; invalid signature, unknown critical extension, unrequested/unauthorized SAN, mismatched SPKI, invalid algorithm parameters, malformed DER, oversized CSR and path traversal reject before signer invocation.
5. Wildcards: accept `*.developing.today`; reject multi-label, non-leftmost, multiple-star, wildcard IP, public suffix (per pinned suffix fixture), CN-only hostname. Verify SAN dedupe and exact apex+wildcard expansion for both domains.
6. Sign/verify each supported key/signature path with an independently configured implementation. OpenSSL 3.5+ plus at least one Rust X.509 path validator as source/spec demand; document unsupported ML-DSA validator/client combinations as unavailable, never pass. Verify full ML-DSA path accepted only by PQ-only profile; classical issuer/root path rejected even if signatures are otherwise valid. Include invalid AKI, wrong subject key, expired CA, bad pathLen, revoked CA/leaf, EKU mismatch, unsupported OID and forbidden reverse edge.
7. Build a leaf certificate with correct AIA/CDP and exact chain; prove cross-cert's AIA names its signer and CDP names signer's CRL. Confirm empty CRLs exist before certificate is marked publishable. Mutate URI host/path, MIME, bytes or mirror hash and require failure.
8. Test CRL monotonic number, serial uniqueness/collision, signature, `thisUpdate < nextUpdate`, revoked leaf/intermediate/cross-cert membership in the correct signer CRL, stale CRL and missing CDP target. OCSP absent means no AIA URI; unsupported CA query never returns synthetic `good`.
9. CMS kit must parse as certs-only SignedData, no signers, contain only expected certificate fingerprints and chain candidates. DER/PEM round-trip bytes, PEM labels, MIME and published hashes match manifest. Never parse a `.crt` by extension alone.
10. Publication fault injection: partial upload, stale alias, redirect to HTTPS, differing mirrors, wrong content type, network timeout, server hash mismatch, concurrent alias writer, recovery retry; assert immutable paths and prior alias retained unless CAS promotion succeeds.
11. Trust install plan does not mutate. Install needs explicit privilege + expected fingerprint + scope. Negative tests cover attempted automatic root import, cross-cert anchor inclusion, unsupported platform claims, classical trust accepted under PQ-only and PQ certificate accepted on non-ML client.
12. CLI aliases/flags have equivalence/property tests; conflicts/unknown args cause nonzero usage before any subprocess/network/file/lock. JSON stdout is stable schema, diagnostics on stderr, secrets absent in logs. API and CLI outputs share oracle fixtures.

### 7.2 Library selection gate

Use findings in `../research/pki-standards.md`, then implementation issue must re-check pinned versions and run a capability matrix before dependency adoption. Minimum proof per backend: generate and parse each target signature/cert profile; CSR PoP; CRL build/verify; CMS certs-only encode/inspect; external signer exact-TBS behavior; Rust and OpenSSL independent validation; no fallback. `rcgen`/`x509-cert`/RustCrypto/OpenSSL may be composed for separate capabilities, but no crate alone satisfies the Fleet lifecycle and trust requirements. Any unproven primitive remains `unsupported` in capabilities and blocks that profile.

## 8. Staging and approval gates

This domain follows v0.3 §48 incremental release shape and the intake's V1–V4 classification:

### V1 — Census and design (current)

No Fleet `cert`/`pki` domain exists per intake note. Deliver source-grounded design, requirement-to-test map, library capability/research record and issues. No Fleet PKI production code or production root operation. Synthetic repo/source inspection is permitted. No live signer, root escrow, vault write, DNS/trust install or external publish.

### V2 — Synthetic graph and crypto adapters

Implement typed domain/API/CLI skeleton and capability declarations; synthetic roots/issuers/leaves only; enumerated certificate graph; deterministic test fixtures; standards/profile verification; failure injection; no production signer, real vault, public DNS/network publication, or user trust change. The exact 65-count graph gate and distinct adapter evidence are blockers. Rollback: remove/deactivate only test fixtures and test feature flags; retain failure artifacts as non-secret CI evidence.

### V3 — Guarded issuance, publication, trust

Production support requires reviewed policy, explicit approved signer, approved key storage/escrow, authorized root source per signature, scoped approvals and durable audit. Initially stage artifacts and use test/private HTTP mirrors; production publication, DNS and trust install each separately approved. Root key custody must implement addendum 5's local+BWS+OP initial three destinations only with explicit authorization and encrypted handling, integrity checks and tested independent restoration; do not silently add R2 or future providers. Leaf issuance and CRL publication can be on scoped intermediates after their root signatures/CRL are valid. OCSP remains off until complete responder acceptance evidence passes. Rollback follows §6.3; production published objects are immutable and cannot be erased as a rollback mechanism.

### V4 — Offline/hardware evolution

Separate future work for offline ceremony, HSM/PKCS#11/KMS/external signers, key attestation, quorum/threshold/recovery, certificate transparency/private issuance ledger, rotation and 20-year archival recovery. Preserve the signer interface; no blocker for V2 synthetic path and no claim that HSM, OpenBao or future infrastructure is deployed. Migrate root signing by dual operation/validation, trust overlap and rehearsed restore before retiring temporary root custody.

## 9. Granular issue-ready breakdown for #9164

Every proposed issue is an independently reviewable deliverable, not a GitHub write. Native parent is #9164; dependencies below are blockers unless noted. Each ticket must retain the coordinator §4.2 fields (kind, source IDs/ranges, user outcome, current evidence, files/interfaces, API/CLI, blockedBy, safe env, tests/negative tests/oracle, rollback, priority/state/size, open decisions). Priority is proposed, not assigned. `V1/V2` issues can be designed/tested without production authority; V3 operations remain Blocked/approval-gated.

| ID / proposed title | Req/source | Deliverable and boundary | Acceptance / oracle incl. negative tests | Depends on / rollback | Stage |
|---|---|---|---|---|---|
| CERT-01 — Define issuer and leaf typed profile/API contracts | PKI-009–018, 027, 052, 057; spec §§3,6,24; Appendix A | `fleet-pki` facade types for purpose, key/cert identity, issuer profile, leaf request, signer and receipt; no CLI implementation beyond registry design. | Compile API contract tests; profile normalization stable; reject unknown purpose, identity/path confusion and ambiguous parent; JSON schema golden oracle. | #9157; revert schema only before persisted production manifests; version migration after. | V1 design → V2 |
| CERT-02 — Reconcile and freeze the 65-certificate signer graph | PKI-001–006, 009–010, PKI2-004–008, PKI-068; §§3.2,24.2,33.2,48.2 | Create explicit root+issuer graph manifest including 16 keys, 52 issuer certs, signer edge IDs, 13 roots, and resolve user 32+20 split against §24.2. | Machine oracle counts 20 CA key IDs/65 cert IDs; independent edge-set compare; no ML→classical; same-key variants same SPKI and distinct issuer/serial/fingerprint; fail if any unapproved edge/count. | #9163, CERT-01; graph version immutable once signed; rollback by new graph version. | V1 decision, V2 fixture gate |
| CERT-03 — Implement signer capability and explicit authorization contract | PKI-008, PKI2-008, PKI-071/074; spec §§10,24.3,33.3 | `Signer` adapter contract and typed authorization receipts for software test signer, external signer; no production credential integration. | Synthetic sign/verify exact TBS; denied/unavailable/expired/wrong-key/wrong-alg auth never invokes alternate signer; logs omit key bytes. | #9157/#9163; preserve receipts; revoke test handles only. | V2 |
| CERT-04 — Build strict CSR parser and proof-of-possession validator | PKI-014, 026, 045; §§10,24; RFC 2986 | CSR PEM/DER inspect, signature validation, SPKI and requested extension extraction, bounded parse. | Valid CSR accepted; malformed DER, invalid PoP, unsupported OID, unknown critical ext, oversized input, duplicate extension and malicious path rejected before signer; parser oracle + OpenSSL cross-check. | CERT-01, crypto adapters; no persistent CSR by default. | V2 |
| CERT-05 — Define purpose profiles, extension policy, and SAN/wildcard validator | PKI-015–016, 023–027; §§3.3–3.4,24.6,44; RFC 5280, RFC 9525 | Base/InternalWeb profile registry; SAN normalization; KU/EKU/pathLen/name constraints; no CN fallback. | Exact four Web SANs; profile controls and public-suffix fixture; reject malformed/wide wildcard, CN-only, constrained-out SAN, algorithm-incompatible KU/EKU. Oracle is canonical SAN/profile report plus independent hostname/path checks. | CERT-01; no rollback of immutable profile IDs, supersede with versioned profile. | V2 |
| CERT-06 — Implement issuer-set synthetic builder and graph verification | PKI-009–013, 018–022, PKI2-002–009; §§24.2–5 | Generate synthetic A/B keys/certs for Base and InternalWeb, explicit parent signer list, manifest and deterministic IDs. | 16 distinct issuer keys; 52 issuer certs per frozen CERT-02 graph; exact expiry, naming, CA extensions, unique serials; no signer calls outside enumerated graph; verify all signatures independently. | CERT-01–03; test-only entropy; delete only owned fixture output or retain as non-secret test artifact. | V2 |
| CERT-07 — Implement leaf issuance API/CLI and fast in-memory mode | PKI-023–027, 051–052, 055–057; §§6,24.6,48 | Issue Base and InternalWeb synthetic leaves, multiple separately keyed algorithm variants, no docs/network fast path. | 1y Base and 90d Web defaults; SAN and issuer constraints; private key stays in memory/test handle; API/CLI parity; expired parent, no SAN web, invalid wildcard and unsupported algorithm fail. | CERT-04–06; issuance operation idempotent; staged test artifacts only. | V2 |
| CERT-08 — Add renew/rotate/replace state and safe resume | PKI-042–044; §§5,9,24.4–5,48.2 | Same-key cert revision renew; new key generation rotate; replacement plan and resumable receipts. | Renew preserves key ID/SPKI and increments cert revision/serial; rotate changes key ID/SPKI/generation; retry cannot double-sign; interruption/fault injection resumes from receipt. | CERT-03,06–07; before activation can cancel; afterward requires explicit overlap/deprecation plan. | V2→V3 |
| CERT-09 — Build CRL state, signing, and lifecycle ledger | PKI-033,036; §§4.3,10–11,24.7; RFC 5280 | Per-issuer serial ledger, authorized revocation states, CRL number/time build and verify for synthetic roots/intermediates. | Correct issuer CRL contains only authorized revoked serials; cross-cert appears on signer-root CRL; monotonic number, valid signatures/times; reject stale state, serial collisions, unauthorized revoke and CRL number reuse. | CERT-03,06; CRL is immutable; correction via next CRL, never delete history. | V2 |
| CERT-10 — Construct AIA/CDP and immutable publication plans | PKI-033–040; §§4,24.7–9,44.1 | URL graph builder, MIME/artifact manifest, 3-mirror plan and local HTTP fixture checker; no production DNS/deploy. | AIA issuer, CDP signer CRL, complete immutable URLs; empty CRL prerequisite; all mirrors hash-equal; reject directory/bare-host URL, wrong MIME, redirect dependency, missing file or mismatched mirror. | CERT-06,09; local stage can be discarded only if operation-owned; external objects immutable. | V2 |
| CERT-11 — Add atomic staged publication and rollback receipts | PKI-037–040,055,057; §§4,6,24.7,48 | Generic object publisher interface; local test server first; immutable objects, CAS latest alias, manifest receipts. | Fault inject timeout/partial mirror/alias race/hash mismatch; retry converges; old alias preserved until all required artifacts verify; rollback CAS and incident receipt tests. | #9157 publisher seam, CERT-10; rollback cannot retract cache, report residual exposure. | V2; V3 gated |
| CERT-12 — Implement certificates-only CMS and delivery kit formats | PKI-028–035,048,055; §§4,7,24.9; RFC 5652 | PEM/DER, p7c certs-only SignedData, chains, minimal/public-only/handoff kits; private formats gated. | CMS parser says SignedData/no signers/expected fingerprints; PEM/DER equivalent cert; correct chain order/MIME; reject private key in public kit, unsupported algorithm and mislabeled SSH/JWT. | CERT-06–07, library evidence; artifact output immutable after receipt. | V2 |
| CERT-13 — Implement trust-profile export, install plan, and test matrix | PKI-006–007,035,059,061–062; §§3.5,11,44,48 | `roots-all`, classical, PQ-only bundles; untrusted cross-certs/intermediates; platform install plans, no automatic mutation. | PQ-only rejects classical chain; all-roots not labelled PQ; cross cert not anchor; OS plan requires fingerprint/scope; tests OpenSSL + supported Rust path validator and explicit unavailable cases. | CERT-02,06,12; install is separate V3 approval step; uninstall is planned, not silent. | V2; gated V3 |
| CERT-14 — Make OCSP capability explicitly absent unless complete | PKI-033, PKI2-022; §§4.3,24.7,33.2,48.2; RFC 6960 | Capabilities expose OCSP unsupported; URI omitted by default; issue a future implementation contract only after real responder readiness. | No OCSP AIA with no endpoint; query unsupported algorithm/issuer never returns fake good; distinguish unknown/unauthorized/tryLater; explicitly test OpenBao ML-DSA gap. | CERT-03,09; future dedicated issue; no rollback needed while absent. | V1 decision, V2 negative test |
| CERT-15 — Generate DNS and publication runbooks without applying them | PKI-037–040,059; §§4.4,24.7–8,44 | BIND + provider-neutral change manifest, URL/DNS prerequisites, install/readme docs; no network mutation. | Valid zone parser and schema; unresolved placeholder is not executable; correct distinction CAA/public ACME vs private CA; no claim DNS live; docs link exact relative artifacts. | CERT-10,12; generated files versioned; no DNS apply. | V2 |
| CERT-16 — Add issuance state, journaling, and failure recovery | PKI-043,047,049–050,054–057; §§6,10,24–25,48 | Durable operation states/receipts, idempotency, event IDs, batch resume, side-effect ordering. | Inject failure before/after signer, CRL, artifact write, publish; ensure no serial reuse, no duplicate signer action, no false active status, proper recovery receipt; unknown args before every side effect. | #9157 event/outbox contracts; CERT-03,06,09,11; recovery never erases signed/published evidence. | V2 |
| CERT-17 — Add library backend compatibility and independent validation evidence | PKI-061–064,070,074; §§11,35,48; research dossier | Pin Rust/OpenSSL adapter versions and produce per-alg/object compatibility matrix; no broad dependency adoption absent evidence. | Certificate, CSR, CRL, CMS and external signer tests as supported; OpenSSL + independent Rust validator; ML-DSA unavailable marked unsupported; no silent substitution. | CERT-03–04,06,09,12; version bump requires rerun corpus. | V2 |
| CERT-18 — Stage guarded production issuance and publication approvals | PKI-008,053–057,071; §§10,24.3,33.2,48; Addendum §5 | Approval policy, signer/key destination allowlists, exact object changes, audit and operator review; this is not part of V1. | Missing/expired/wrong-scope approval blocks before secret/key/network side effect; root signer explicit; three escrow targets only when separately authorized; dry-run proves plan and no mutation. | CERT-03,08–11,13,16; approval-specific recovery; addendum root custody coordinated with #9163. | V3 blocked pending explicit authority |
| CERT-19 — Prepare offline/hardware migration and rotation rehearsal | PKI-044,071,074; §§24.3,33.3,40,48; Addendum §5 | Design/test adapter migration, protected recovery and 20-year replace/overlap plan; no HSM assertion without actual proof. | Simulated signer outage/restore and migration verify; old/new trust overlap and recovery tested; no root secret export in artifacts. | CERT-03,08,13,18; rollback to prior signer only while approved and valid. | V4 research/design |

### 9.1 Suggested dependency waves

* **Wave 0 / design:** CERT-01, CERT-02, CERT-14, CERT-17 (research evidence can begin independently). Resolve graph count before implementation acceptance is finalized.
* **Wave 1 / foundational V2:** CERT-03, CERT-04, CERT-05, CERT-16. CERT-01 and #9157 API contracts are blockers.
* **Wave 2 / issuers and outputs:** CERT-06; then CERT-07 and CERT-09; CERT-12 and CERT-13 can begin from verified fixtures; CERT-10 depends on CRL/profile output.
* **Wave 3 / lifecycle and publication:** CERT-08, CERT-11, CERT-15, CERT-17. CERT-11 depends on #9157 publisher seam and CERT-10. CERT-18 remains explicitly blocked until approval; no issue can promote itself by being unblocked technically.
* **Wave 4 / future custody:** CERT-19 after signer abstraction and V3 approval evidence; parallel infrastructure studies remain nonblocking.

No issue should claim end-to-end PKI completion until CERT-02 count/graph, CERT-06/07 issuance, CERT-09/10 CRL/AIA readiness, CERT-12 formats, CERT-13 trust matrix and independent verification are jointly demonstrated.

## 10. Open decisions and defaults

1. **32+20 versus matrix-derived 16+36 decomposition:** default is preserve §24.2 exact signer matrix and 65 total; CERT-02 must publish explicit reconciled inventory before this is closed.
2. **Signer choice per algorithm/object:** no final crate selection yet. Default design is capability-driven adapter, with fail-closed behavior and test-specific support declaration; do not claim a single crate handles all four signatures/CSR/CRL/CMS/HSM.
3. **Exact CRL cadence/validity and outage policy:** choose per signer availability and consumer needs in a V3 policy; until then initial signed CRL is required, no invented schedule, and stale/unavailable state is not `good`.
4. **OCSP:** disabled/unadvertised by default. Future implementation needs full acceptance as §4.6; library API alone does not satisfy this.
5. **Name constraints and client enforcement:** use policy constraints and tests where interoperable; do not rely on OU or issuer EKU alone as authorization boundary. Client-specific enforcement evidence is required.
6. **Public URL/DNS ownership and triple mirror targets:** URLs are configurable placeholders in V2 only. Production addresses, DNS changes, public publishing and internal `.internal` resolver require a separate deployment decision.
7. **Production escrow details:** the addendum names three approved target classes; actual project/vault identifiers, encryption key custody, approvals and restore identities remain unverified/approval-gated. R2 stays research-only.
8. **Supported client matrix for ML-DSA/TLS:** no universal support assumption. Record exact verifier/library/client versions; TLS key exchange and signature negotiation are separate from certificate path verification.

## 11. Source and evidence register

### In-repo governing materials read

* `doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/sources/COORDINATOR_summit_handoff.md` — especially §§0, 4.2, 7.5–7.6, 9–10 (scope, ticket contract, PKI, safety).
* `.../sources/fleet_agent_handoff_prompt_v0_3.md` — complete future implementation assignment, especially decisions, PKI scope, commands, APIs, tests and no-silent-substitution invariants.
* `.../sources/fleet_architecture_spec_v0_3.md` — §§3–7, 10–11, 24, 30–31, 33, 44, 48 and Appendix A.
* `.../sources/ADDENDUM_secrets_architecture_performance_infrastructure.md` — §5 root custody refinement.
* `.../note.md` — source precedence, V1 census, stage scope and approval limits.
* `../research/pki-standards.md` — retrieved primary standards and crate capability evidence.

### Public primary sources retrieved on 2026-11-12

* [RFC 5280 — PKIX Certificate and CRL Profile](https://www.rfc-editor.org/rfc/rfc5280), §§3–6 and Appendix A.
* [RFC 6960 — OCSP](https://www.rfc-editor.org/rfc/rfc6960), §§2–4.
* [RFC 2986 — PKCS #10](https://www.rfc-editor.org/rfc/rfc2986), §§1, 3–4.
* [RFC 5652 — CMS](https://www.rfc-editor.org/rfc/rfc5652), §§5, 5.1.
* [RFC 9525 — Service Identity in TLS](https://www.rfc-editor.org/rfc/rfc9525), §§1–7.
* [RFC 9881 — ML-DSA in X.509](https://www.rfc-editor.org/rfc/rfc9881.html), linked by spec; algorithm support must be re-tested at implementation time.
* docs.rs retrieved `rcgen` 0.14.10, `x509-cert` 0.3.0, `der` 0.8.3, `cms` 0.2.3, `openssl` 0.10.82, `rustls-pki-types` 1.16.0. See research note for observed capability and limits.

Retrieved standards tell us format/protocol constraints, not what the checked-out Fleet binary implements, what any current provider authorizes, or what target clients support. Re-pin dependency versions and independently reproduce capability evidence in implementation issues. The only external evidence gathered here was read-only web retrieval; no online signing, provider access or live deployment was used.
