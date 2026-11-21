# Fleet PKI issuance, publication, and trust design

**Status:** Proposed domain design for #9164. This is not a production runbook or authorization to generate/sign production CA material. V1 has no Fleet PKI implementation. Every API below is a design contract, not a claim that the command or crate exists.

## 1. Scope, authority, and source precedence

This design covers purpose-specific issuer sets, leaf issuance and lifecycle, CSRs/CMS delivery kits, certificate profiles, URI publication, CRL/OCSP status, and trust-installation boundaries. Per-packet hard dependencies are the exact proposal IDs in `proposals/pki.json`: ROOT packets own root graph/custody, ARCH-01/06/07 own shared identity/signer/artifact/registry seams, and SEC packets own provider adapters/policies. Parent issue #9163 is context, not a `depends_on` edge.

Precedence: current assignment and the dated addendum refine their points; the v0.3 spec governs otherwise; its §33 resolution supersedes historical alternatives. The coordinator handoff §4.2 is the acceptance contract for child issues. The complete implementation prompt is future-stage context, not permission to implement production PKI now. Intake `note.md` binds this run to design/synthetic proof only: no production CA generation/signing, root escrow/export, live vault write/reconciliation, DNS or trust-store change, or deployment.

**Source facts** below are attributed to source records/standards. **Decisions** and API names are design proposals. Where a source conflict remains, it is made an explicit gate rather than concealed.

## 2. Source-backed invariants and scope

### 2.1 Requirements and facts

* The reference inventory has four independent root keys/certificates: RSA-4096, P-256, Ed25519 and ML-DSA-44; 9 asymmetric root cross-certificates (6 classical-to-classical, 3 classical-to-ML-DSA), no ML-DSA-root signature over a classical root; four self-signed roots + 9 cross-certs = 13 root certificate variants. PQ-only validation trusts the ML-DSA root and requires all-ML-DSA path signatures. (Spec §§3.1–3.5; §24.2; Appendix A PKI-001–008.)
* Base and Internal Web are sibling purpose sets. Each has A/B independent keys for each of four algorithms: 8 keys per set, 16 issuer keys total. The spec's explicit signing matrix gives, per partition and set, 3 classical issuer keys × 3 classical signing roots plus 1 ML-DSA issuer key × 4 eligible roots = 13 certs; ×2 partitions = 26 per set, 52 issuer CA certificates total. Add 13 root variants for 65 CA certificates, excluding leaves. Same-key cross-signed certificates share SPKI/SKI but differ in issuer, AKI, serial, signature, fingerprint, path and revocation issuer. (Spec §24.2; Appendix A PKI-068; handoff §§7.5–7.6.)
* **Graph count is source-defined, not an open decision:** spec §24.2 explicitly yields 13 issuer certificates per purpose/partition. The two purposes × two partitions therefore produce 52 issuer certificate variants; with 13 root variants this is 65 CA certs over 20 CA keys before leaves. A one-certificate-per-issuer-key baseline is 16, with 36 additional signer variants under the matrix; this arithmetic is explanatory only, not a separate graph policy. CERT-02 serializes and tests the literal §24.2 signer matrix; no user approval or source reconciliation is needed to choose its edges.
* All 52 Base and Internal Web reference issuer certificate variants use exact `notAfter=2032-07-07T12:00:00Z`; all 13 root variants use exact `notAfter=2050-07-07T12:00:00Z`. A shorter cap does not pass as reference output: return `ReferenceValidityMismatch` or emit an explicitly named non-reference profile with a new authorized plan. General defaults remain root 20 calendar years, issuer 5 years, ordinary web leaf 90 days, and Base reference leaf 1 year. (Spec §§24.1, 24.5, 33.2; Appendix A PKI-019–023.)
* Leaf certificates have a single SPKI and single issuer signature. If several algorithms are requested, issue separate key/certificate variants with the same approved identity set; do not describe these as a multiply-signed certificate. Ed25519 uses its intrinsic signature; ML-DSA-44 follows its PKIX algorithm identifier and mode, not an injected SHA-256 digest. (Spec §§3.4, 24.6; RFC 5280; RFC 9881.)
* Base may issue a generated test identity without DNS SAN; TLS/Web issuance requires explicit DNS/IP identity or validated issuer defaults. Web default SAN shorthand for `developing.today` and `developing-today.com` creates apex and wildcard for each. Wildcard is only a complete leftmost DNS label and matches one label. CN is not DNS identity; SAN is. Private `Internal Web` or `Public Web` issuers do not become publicly trusted because their names are public DNS names. (Spec §§24.6, 33.2; RFC 9525.)
* Leaf AIA names actual issuer candidates; CDP names the CRL signed by that certificate's issuer. A cross-signed CA cert's issuer is the signing root, and it belongs on that root's CRL. AIA certificate variant objects are immutable. CRL-number archive objects are immutable, but the CDP embedded for one signing-key/generation must be a durable URL whose served current CRL advances monotonically for that same authority; do not freeze a certificate to the initial empty CRL or repoint it to an unrelated issuer/generation. HTTP is used for broad compatibility; clients can ignore AIA or cache CRLs, so composed chains and freshness-aware publication are needed. Triple mirrors are supported, not proof of failover or universal revocation enforcement. (Spec §§4.1–4.3, 24.7; RFC 5280.)
* CRL publication is part of issuance readiness whenever CDP is embedded: create a real signed empty CRL before publishing the certificate and keep its stable same-signer/generation CDP URL serving successive verified higher-numbered CRLs; track issuer key, CRL number, thisUpdate/nextUpdate and revoked serials; roots revoke intermediate and cross-cert variants they signed; issuer CRLs revoke leaves. OCSP is not advertised without a real authoritative responder for each represented issuer. (Spec §§4.1, 4.3, 24.7; RFC 5280 §§5–6; RFC 6960.)
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
struct PkiSignaturePlanEntry { tuple_id: OpaqueId, signer_fingerprint: Sha256,
    explicit_signer_source: ExplicitSignerSource, subject_spki_id: OpaqueId,
    signature_algorithm: SignatureScheme, profile_purpose: Purpose,
    not_before: UtcTime, not_after: UtcTime, tbs_sha256: Sha256,
    authorized_destinations: Set<DestinationRef> }
struct PkiSigningRequest { transaction_id: Uuid, tuple_id: OpaqueId,
    authorization: SharedSigningAuthorizationRef, exact_tbs_der: Vec<u8> }
struct KeyCeremonyApprovalRef { approval_id: Uuid, operation_id: Uuid,
    permitted_algorithms: Set<SignatureScheme>, key_count: u32,
    authorized_key_destinations: Set<DestinationRef>, expires_at: UtcTime }
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

`Purpose` is a closed typed registry (initially `Base`, `InternalWeb`, future `PublicWeb`, `ClientAuth`, `CodeSigning`, `Device` only when registered). Profile policy controls eligibility. Every logical object has a stable opaque ID independent of subject/display name, slug, filesystem path, DNS identity, provider-native ID and logical secret path; the path is a locator/attribute, not identity. This is the shared cross-domain ADR-S01 constraint, not a second implementation of Fleet's canonical secret identity. IDs and generation/partition/revision are structured, not recovered by parsing a filename. Store key handles as opaque references plus exportability/capability metadata. Root private material is outside ordinary resolver hot caches, prefetch, benchmark datasets, and equality indices; only a bounded explicitly authorized signer operation may hold it. Enforce this before the first escrow copy using trusted policy bound to provider scope/native IDs and preferably provider-side least-privilege grants: legacy broker refresh/list and name-only fallback must neither enumerate/cache nor resolve root archives. A caller-supplied `DataClass` or mutable sidecar alone is not authority; unknown/adopted records require classification review before replication. Test mock broker-compatible list/cache/get paths. Actual grants/identities remain later approval work. Keep private and public manifest projections separate where public infrastructure metadata would reveal sensitive store paths.

### 3.2 Rust API / CLI relationship

Proposed facade:

```rust
// Generic Signer / KeyResolver / artifact-authority contract belongs to ARCH-06.
// PKI defines only this protocol-specific adapter and finite signed-plan input.
trait PkiSigningAdapter {
    fn capabilities(&self) -> PkiSignerCapabilities;
    async fn sign_certificate(&self, request: PkiSigningRequest)
        -> Result<PkiSignatureReceipt, PkiSignerError>;
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

All mutating calls accept structured requests and return typed receipts; public artifacts may be streamed to caller-provided sinks. Low-level API includes injected clock, deterministic test-only entropy, ARCH-06 signer adapter and in-memory output sink. Production API has no fake signer. `--dry-run` performs non-secret preflight and renders non-executable placeholders/estimates when final public SPKI, serial, or exact TBS bytes do not yet exist. It cannot mint a signature authorization, call a signer, or present an estimate as an authorized final artifact plan; it also does not reserve serials, write stores, publish, or change DNS/trust.

### 3.3 Capability and signing adapter contract

ARCH-06 owns the generic `Signer`, key-resolver and artifact-authority contract; PKI must consume that seam, not define a competing general signer API. PKI submits a finite canonical tuple set before authorization. Each tuple binds `(signer key fingerprint/handle and explicit source, subject SPKI identity, signature algorithm and parameters, profile/purpose, exact notBefore/notAfter, SHA-256 of the exact TBS/artifact-plan entry, authorized output/escrow destinations)`. A canonical digest binds the complete tuple set and transaction; tuples cannot be added or altered after authorization. Single tuple use is default; bounded batch authorization is allowed only for the finite enumerated tuple set. ARCH-06's signer durably consumes each tuple before signing, with anti-replay state surviving process restart. An uncertain/lost result burns that tuple until reconciled from durable signer receipt/audit; it is never blindly retried. PKI's adapter checks explicit source/tuple matching and verifies the returned signature and public-key correspondence before finalizing output. Inventory lookup, escrow read access, or holding key bytes never mints authorization. Signer failure is terminal for that tuple; never generate a software key or switch algorithm, parent, provider, or signer as fallback. Receipt names transaction and tuple without key material. (Architecture review A-2; review/architecture-challenge.md re-review precision note.)

Adapters are independently feature-gated: explicit native RustCrypto components, rcgen, and OpenSSL are not interchangeable contracts. Before pinning, issue per-algorithm + per-object proof (certificate/CRL/CSR/CMS where applicable), independent verify, key custody, platform build and license/version evidence. Reject a missing primitive with `unsupported_algorithm` or `signer_unavailable`; do not silently substitute. The evaluated capability table and source links are in [`research/pki-standards.md`](../research/pki-standards.md).

## 4. Issuer, CSR, leaf, renewal and revocation workflows

### 4.1 Issuer-set creation

1. Parse/normalize arguments and schema; reject unknown flags, duplicate IDs, invalid date combinations, unsafe DNS, unsupported alg/profile combinations before any side effect.
2. Resolve public parent metadata and require an explicit signer source to be named for any planned root signature; source presence is not signature authorization. Public inventory lookup remains distinct from active signer authority.
3. Validate all profile inputs and exact graph edges. Before key generation/storage, check the separate shared ceremony approval (OPS-10/operation policy) for permitted algorithms, count and key destinations; this approval does not authorize any certificate signature.
4. Generate or import independent issuer keys for four algorithms × A/B × Base/InternalWeb = 16 distinct key IDs; verify public SPKI/SKI and the selected signer key identity. Root and issuer key generation/storage approvals are separate from signing authorization.
5. Allocate collision-free positive CA serials per issuing key under a durable issuance ledger. Reservation is durable before TBS construction; gaps are acceptable and a serial is never reused after a signer attempt or uncertain outcome.
6. Construct and freeze exact certificate TBS bytes, including subject SPKI, serial, issuer, exact notBefore/notAfter, extensions, algorithm parameters, and destinations. In reference mode require root cert variants exactly `2050-07-07T12:00:00Z` and all Base/Internal Web issuer variants exactly `2032-07-07T12:00:00Z`. If a signer/parent/policy cap would shorten a reference artifact, return `ReferenceValidityMismatch` or require an explicitly named non-reference profile and a new authorized plan; never pass a shortened artifact as reference output. Ordinary profiles still obey their own caps. Compute each TBS SHA-256 and the digest of the finite canonical per-signature tuple set.
7. Only now request ARCH-06 `SigningAuthorization` bound to the final tuple set (signer source/key, subject SPKI, algorithm, purpose, dates, TBS hash, approved outputs/escrow destinations). Signer atomically and durably consumes each tuple before invocation; uncertain outcomes burn a tuple until authoritative reconciliation. Sign only enumerated tuples.
8. Parse and verify each generated cert; assert SPKI, subject, issuer, serial, validity, constraints, algorithm, signature, AKI/SKI, expected edge and manifest agree.
9. Build initial signed empty CRLs for each actual CRL signing authority before any CDP-bearing certificate becomes publishable; validate issuer/AKI/number/times. Subsequent higher-number updates must continue at the certificate's original durable CDP URL.
10. Create public/private manifests, deterministic artifact index, profile and test results. Initial status is `staged`, not `active` or `published`.

No sample operation is executable against production in V1/V2. A V3 live operation requires separate ceremony/key-generation/storage approval and a final-TBS-bound signature authorization for each finite signer tuple. The addendum's exact three-copy root escrow is handled by ROOT-09/10 using explicit encryption/integrity/readback/restore; it is not routine issuer key sync.

### 4.2 Count and cross-certificate matrix gate

The immutable V2 synthetic fixture must contain:

* 4 root key identities; 13 root CA certificate variants.
* 16 distinct issuer key identities (Base A/B + InternalWeb A/B × four algorithms).
* 52 total issuer CA certificate variants under the §24.2 graph; 65 CA certificates total; no leaves included in that count.
* Per-purpose set = 8 issuer keys and 26 certificates. Every certificate variant has a distinct `(issuer DN, serial)` and fingerprint; variants for same issuer key have identical SPKI/SKI and different issuer/AKI/signature.

The exact graph is source-defined by §24.2: 13 issuer certificates per purpose/partition, four groups = 52 issuer certs, plus 13 root variants = 65 CA certificates over 20 CA keys before leaves. CERT-02 serializes this literal table with stable certificate and signer-edge IDs; it is not a user approval gate or an open graph choice. The one-per-issuer-key baseline of 16 plus 36 additional variants is explanatory arithmetic, not an alternative requirement. The oracle is exact manifest tuple equality against §24.2.

### 4.3 CSR intake and issuance

PKCS #10 CSR is untrusted input. Decode strict DER/PEM with size/depth bounds; verify CSR self-signature/proof-of-possession; validate subject and SPKI; reject unsupported algorithm OIDs, malformed/duplicate extensions, unknown critical extensions, oversized attributes and invalid SAN syntax. Treat requested SAN, subject, extensions, validity and key usage as proposals only. Recompute approved extension set from issuer policy; no caller CSR extension may weaken CA constraints or elevate purpose. Compare exact CSR key SPKI with the issued certificate SPKI. Record CSR digest and approved request in receipt; do not retain raw CSR unless policy asks.

Support submit/issue workflow through Fleet APIs and offline file transfer. An issue plan shows normalized identities, requested-vs-approved fields, issuer/key/partition, validity, output and ceremony action. Key-generation/storage ceremony approval is separate from each signature grant. Once CSR SPKI, serial, validity, final extensions and exact TBS are fixed, compute tuple/hash and request signature authorization; recheck/consume immediately before signing. Repeated request with same idempotency key returns existing receipt only if request and TBS hashes are identical; changed CSR/SAN/serial/profile requires a new operation and authorization.

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

CRL operation is per signing CA and monotonic CRL number. A CRL build snapshots authorized revocations and explicit validity window; it signs exactly once, verifies, stores an immutable numbered CRL and advances the stable per-signing-key/generation CDP route only after signature and mirror verification. Certificates keep their original CDP URL; that URL must serve current higher-numbered bytes, not remain pinned to the initial empty CRL. Old numbered archive objects remain available. CDN content must match manifest hash/MIME and have bounded cache lifetime (`max-age` no later than `nextUpdate`, validators/revalidation where supported); never roll the CDP route back to a lower CRL number. Test: issue a synthetic cert embedding CDP, publish empty CRL, revoke its serial, publish the next CRL to all configured mirrors, then fetch the **original CDP URL** and verify the higher CRL contains the revoked serial. This is a service/oracle contract, not a claim every client refreshes or enforces revocation. Stale/missing CRL creates `revocation_unavailable`; policy decides block new issuance/publication and warns consumers. Never report `good` based solely on absence in an unavailable CRL.

OCSP is **unsupported/deferred** unless a real responder is separately built and authorized for each CA issuer key, with complete issuance/status database, responder certificate/KU/EKU, key custody, freshness, nonce/cache, rate limiting, request validation, audit and publication. No `id-ad-ocsp` URI is placed in AIA by default. Unsupported issuer/algorithm/status returns RFC-appropriate `unknown`/`unauthorized`/`tryLater` as applicable; do not synthesize `good`. OpenBao's documented ML-DSA OCSP limitation is a known no-go for that route, not a reason to emit a false endpoint. RFC 6960 `good` is not proof of issuance.

## 5. Immutable output, AIA/CDP, publication, and trust

### 5.1 Artifact plan and paths

Manifest schema `fleet.pki.bundle/v1` is canonical JSON; YAML is a rendering. It contains bundle/key/cert IDs, purpose, generation/partition/revision, parent refs, exact signer identity, request/profile hash, algorithms/parameters, DER hashes, serial/issuer/SKI/AKI/fingerprint, validity and extensions, key handles and exportability (no key bytes), signer receipts, CRL number/freshness, trust profile, publication refs/status, tested client matrix, tool/library versions, policy decisions/warnings and rollback/recovery state. Distinguish immutable CRL archive objects from the stable per-issuer/generation CDP URL advertised in certificates; the latter serves the latest verified monotonic CRL and must have bounded freshness headers.

Private/public manifest projections avoid leaking secret-store URIs or sensitive topology. Paths are deterministic from opaque IDs/validated slugs and are never built from raw display strings; changing a path or display name does not change object identity. An output path is created with no-follow/symlink-safe semantics; writes use restrictive permissions and staged temp directory on same filesystem; atomic rename commits a local public-artifact set. Bundle checksum is over exact file bytes. PEM/DER encodings, CMS content and chain order are explicit metadata.

Suggested immutable public layout:

```text
certs/issuers/<purpose>/<algorithm>/<generation><partition>/<cert-id>.crt
certs/issuers/<purpose>/<algorithm>/<generation><partition>/ca.p7c
certs/leaves/<profile>/<identity>/<generation>/<algorithm>/<cert-id>.crt
certs/chains/<profile>/<identity>/<chain-id>.pem
crl/<issuer-key-id>/<crl-number>.crl
crl/<issuer-key-id>/<generation>/current.crl  # durable URL used in CDP; serves latest verified numbered CRL
```

Human/stable aliases may point at current objects but are not embedded in certificates unless durable for certificate lifetime. Proposed mirror set is primary + secondary public + private-use `.internal`, all configurable. Public URLs must not require authentication or HTTPS bootstrap; `pki.developing.today` is illustrative, not asserted deployed. DNS changes are a separate manifest and explicit operator operation.

### 5.2 AIA/CDP construction

For each to-be-issued certificate, derive AIA CA Issuers from its actual issuer certificate/key candidates and CDP from that issuer key's signed CRL. For cross-certificate variants, use the signing root's issuer candidates and CRL. Root self-signed anchors normally omit AIA/CDP. AIA certificate-object URLs resolve to immutable versioned artifacts. A CDP resolves to a durable per-signing-key/generation current-CRL URL: it initially serves the required signed empty CRL, then serves later verified CRL numbers for that same signer without changing the certificate's CDP value. Each numbered CRL archive URL remains immutable. The current URL must never resolve to an unrelated signer or generation. Issuer CA Issuers may expose single DER cert (`application/pkix-cert`) and optional certs-only CMS (`application/pkcs7-mime`); CRL uses `application/pkix-crl`.

Before `publishable`, independently check all referenced artifacts exist, signatures are valid, URLs are complete, three mirror manifests are consistent (if configured), and a signed empty CRL is ready at each certificate issuer's CDP URL. Do not make HTTPS redirects a prerequisite. AIA download/fallback is opportunistic; generated chain packages identify exact candidates for each trust anchor and precompose tested chain alternatives. For every later CRL update, publish the immutable numbered object to all configured mirrors, verify matching bytes/signature/issuer/CRL number/freshness on all mirrors, then advance each stable current-CDP URL. Bound `Cache-Control: max-age` by the CRL's `nextUpdate`, emit validator metadata (`ETag`/`Last-Modified` and `Expires` where supported), and require conditional revalidation/accurate served bytes; never set an unbounded cache lifetime. Clients may still fail to refresh or enforce revocation, so do not claim universal client behavior.

### 5.3 Publication transaction and rollback

Publication is an explicit plan with deterministic artifact rows `(source, immutable destination, content type, hash, visibility, cache control, mirror, DNS prerequisite)`. Flow: `planned → staged → verified → committed → mirror_verified`; each step is idempotent, records receipt, and checks server bytes/hash/MIME/HTTP status. Certificates and numbered CRL archives are never overwritten at immutable paths. The stable `current.crl` URL is a per-signing-key/generation pointer that advances only after every configured mirror serves the same fresh, valid, monotonic CRL; it is not a mutable alias across issuer identities. CRL pointer promotion is a separate guarded operation and obeys A-1's `atomic-precondition | serialized-by-contract | unguarded` rule; if the actual storage/CDN target is unguarded, stop for review unless a separately approved named weaker policy records the race. `latest` non-CDP aliases are promoted last. DNS records are emitted as BIND fragment + structured provider-neutral JSON/YAML, with unresolved values represented as non-executable placeholders; `apply-dns` is a distinct gated operation.

If any mirror fails before alias promotion, retain immutable staged objects and mark partial; retry only missing objects. For each adapter and operation record write guard as `atomic-precondition`, `serialized-by-contract`, or `unguarded`. Until primary documentation and isolated tests prove otherwise, treat BWS, op, and bw as unguarded. Conflict-sensitive updates require one of the first two guard levels; Fleet lease/readback is not serialization. On an unguarded target the plan stops in `needs_review`, unless a separately approved named weaker policy states the lost-update window in both plan and receipt. Additive operations (creating immutable artifacts/history) are not gated by this conflict rule. Use compare-and-swap only where the specific publication adapter/version documents and proves conditional update; otherwise serialize through a documented contract or stop. The default `preflight → external edit → apply` oracle requires the external value to remain recoverable and apply to report conflict/unsupported-guard, not success. A separately approved weaker-mode test asserts its named risk status; it must not claim preservation when the provider cannot guarantee that. If a bad certificate/CRL has already been published, do not delete evidence: publish corrected new version, restore alias only with a safe guard, revoke bad issued certificates where appropriate, and retain cache/consumer remediation. Rollback cannot retract cached data; report residual exposure. Never reuse serials or rewrite an immutable URL. (Architecture review A-1/S02; review/architecture-challenge.md re-review.)

### 5.4 Trust model and installation kits

Trust profiles are explicitly separate artifacts:

* `roots-all`: four self-signed anchors, compatibility profile; no PQ-assurance claim.
* `roots-classical`: RSA/P-256/Ed25519 anchors.
* `roots-pq`: ML-DSA root only + enforcement policy that every signature on path is ML-DSA.
* Cross-certificates and intermediates are untrusted path-building candidates, never silently installed as anchors.

`fleet cert trust export` writes public kits and fingerprints. `install-plan` prints OS/app-specific steps; `install` is later and privileged/approval-gated, with explicit scope (user vs system) and exact root fingerprint confirmation. No global import during issuance/publication. Guides distinguish CA trust installation from server leaf/private-key deployment and native SSH CAs. Every tested consumer row has install/verify/rotate/overlap/remove; unsupported algorithms are documented, not papered over. A trust package does not guarantee TLS signature negotiation, key exchange, revocation enforcement, or AIA discovery. Confirm RFC 9525 hostname checks and PKIX path validation separately.

## 6. Operation states, failure, and phase rollback

### 6.1 Issuance state machine

`draft → normalized → preflight_ok → ceremony_approval_pending → ceremony_authorized → key_ready → serial_reserved → tbs_frozen → signature_authorization_pending → signature_authorized → tuple_durably_consumed → signed → self_verified → independently_verified → staged → published → active → superseded/revoked/expired`.

Terminal/paused states include `rejected`, `unsupported`, `denied`, `waiting_for_signer`, `signature_outcome_uncertain`, `partial`, `needs_review`, `failed_recoverable`, `failed_terminal`. State transitions are append-only receipts with request, final TBS and tuple-set hashes; separate ceremony approval and signature authorization; tuple consumption state; signer ID; output hashes and causation/idempotency IDs. A grant created before final SPKI, serial, validity and exact TBS exists cannot authorize signing. `signature_outcome_uncertain` burns the tuple until an authoritative signer receipt/audit reconciliation; absence of a local receipt never proves it was unused. `active` requires policy checks + verification and, when URLs are embedded, published certificate and currently valid CDP resources. Publication is separate from issuance; trust installation is separate from both.

### 6.2 Pre-side-effect argument contract

Every operator CLI/script has a declared command-line contract and strict parser before subprocess, network, file write, lock, signer, serial reservation, secret-store access, publish or deploy. Unknown/unaccounted arguments are usage errors. `--` is recognized only if explicitly declared. No shell expansion in batch plans. This applies even to wrappers that call Fleet/OpenSSL. In application code the same normalization/validation is a pure preflight stage. Test with extra args, typoed flags, duplicate/conflicting modes and malicious path values, asserting no file/network/signer/subprocess side effect.

### 6.3 Rollback boundaries

* Before `signed`: cancel operation; release non-secret lock; reserved serial remains consumed if reservation was durable.
* After signer invocation/signature exists: preserve signature and operation evidence; do not call signer again on retry without idempotency proof. Mark artifact nonpublishable until exact self/independent verification.
* After local staging: delete only operation-owned staged files after manifest/hash check; never broad recursive cleanup or overwrite user files. Otherwise quarantine staged artifacts for inspection.
* After publication: immutable objects remain; roll non-CDP aliases back only by a documented atomic precondition or serialized-by-contract guard; otherwise stop for review. A per-key/generation current-CDP URL must never move to a lower CRL number; recover by publishing a corrected higher-number CRL after all mirror/signature checks. Retain incident receipt and notify affected clients. CRL revocation state is monotonic and cannot be rolled back by deleting entries.
* After activation/trust import: use staged overlap and explicit uninstall/removal plan, preserve old trust during recovery unless incident policy says remove immediately; an incident response is not routine rollback.

## 7. Verification and test oracles

Test oracles are explicit: canonical graph counts/edge list; exact bytes/manifest hashes; independent public-key signature verification; RFC profile assertions; reference trust/path validator; HTTP endpoint bytes/MIME; CRL number/serial membership/freshness; CSR PoP; certificate key-SPKI equality; policy state transitions. Parsing success alone is never an issuance oracle.

### 7.1 Required fixture and interoperability matrix

1. Generate deterministic *synthetic* root and issuer key fixtures with test-only entropy or fixed approved test keys. Assert root 4 keys/13 certs; two issuer sets, 16 distinct keys/52 certs; total 20 CA key IDs/65 certs; no reverse ML-DSA→classical edge. Validate explicit graph certificate IDs against the frozen §24.2 CERT-02 manifest.
2. Assert same-key signer variants preserve SPKI and SKI while issuer, AKI, serial, signature, issuer CRL owner and fingerprint differ. Assert A/B keys differ in key ID, SPKI, SKI and private-key test fixture.
3. Assert each of all 52 Base/InternalWeb issuer cert variants uses exact `2032-07-07T12:00:00Z`; each of all 13 root variants exact `2050-07-07T12:00:00Z`; shorter parent/policy cap fails reference mode. Test general defaults and leap-day arithmetic separately.
4. CSR valid PoP accepts; invalid signature, unknown critical extension, unrequested/unauthorized SAN, mismatched SPKI, invalid algorithm parameters, malformed DER, oversized CSR and path traversal reject before signer invocation.
5. Wildcards: accept `*.developing.today`; reject multi-label, non-leftmost, multiple-star, wildcard IP, public suffix (per pinned suffix fixture), CN-only hostname. Verify SAN dedupe and exact apex+wildcard expansion for both domains.
6. Sign/verify each supported key/signature path with an independently configured implementation. OpenSSL 3.5+ plus at least one Rust X.509 path validator as source/spec demand; document unsupported ML-DSA validator/client combinations as unavailable, never pass. Verify full ML-DSA path accepted only by PQ-only profile; classical issuer/root path rejected even if signatures are otherwise valid. Include invalid AKI, wrong subject key, expired CA, bad pathLen, revoked CA/leaf, EKU mismatch, unsupported OID and forbidden reverse edge.
7. Build a leaf certificate with correct AIA/CDP and exact chain; prove cross-cert's AIA names its signer and CDP names signer's CRL. Confirm empty CRLs exist before certificate is marked publishable. Mutate URI host/path, MIME, bytes or mirror hash and require failure.
8. Test CRL monotonic number, serial uniqueness/collision, signature, `thisUpdate < nextUpdate`, revoked leaf/intermediate/cross-cert membership in the correct signer CRL, stale CRL and missing CDP target. OCSP absent means no AIA URI; unsupported CA query never returns synthetic `good`.
9. CMS kit must parse as certs-only SignedData, no signers, contain only expected certificate fingerprints and chain candidates. DER/PEM round-trip bytes, PEM labels, MIME and published hashes match manifest. Never parse a `.crt` by extension alone.
10. Publication fault injection: partial upload, stale alias, redirect to HTTPS, differing mirrors, wrong content type, network timeout, server hash mismatch, concurrent alias writer, recovery retry. Under the default guarded contract, preflight → external edit → apply leaves the external value recoverable and reports conflict. A separately approved weaker mode states its named residual risk and does not promise preservation. Assert immutable paths and prior alias retained unless a proven adapter guard succeeds.
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

The standalone packets are in `../proposals/pki.json`; `size` is S/M/L relative scope, not a calendar estimate, and `kind`+`stage` provide the design/research vs synthetic code/test vs production-operations lane rationale. No dispatch or queue-lane binding is asserted. Their `urgency` and `urgency_band` use the shared intrinsic lower-is-sooner scale (High 400–799, Standard 800–1199, Low 1200–1599, Someday 1600–1999, Cold 2000+). This is distinct from importance, dependency blocking, lifecycle and dispatch. A future publisher must preserve both values and project to the existing queue scale only via High→20, Standard→40, Low/Someday/Cold→70; do not alter the PM schema or infer intrinsic urgency from queue priority.

**Urgency migration record (draft values → re-estimated current values):** CERT-01 320→500; CERT-02 400→450; CERT-03 400→550; CERT-04 300→1000; CERT-05 330→650; CERT-06 350→650; CERT-07 340→650; CERT-08 320→1000; CERT-09 330→1000; CERT-10 320→1000; CERT-11 310→1000; CERT-12 300→1000; CERT-13 320→1000; CERT-14 320→1000; CERT-15 250→1400; CERT-16 330→700; CERT-17 320→1000; CERT-18 450→1000; CERT-19 220→1800. The prior values were outside the current charter; these are fresh prioritizations, not linear scale conversions.

| ID / proposed title | Req/source | Deliverable and boundary | Acceptance / oracle incl. negative tests | Depends on / rollback | Stage |
|---|---|---|---|---|---|
| CERT-01 — Define issuer and leaf typed profile/API contracts | PKI-009–018, 027, 052, 057; spec §§3,6,24; Appendix A | `fleet-pki` facade types for purpose, key/cert identity, issuer profile, leaf request, shared signer ref and receipt; no CLI implementation beyond registry design. | Compile API contract tests; profile normalization stable; reject unknown purpose, identity/path confusion and ambiguous parent; JSON schema golden oracle. | ARCH-01, ARCH-06; revert schema only before persisted production manifests; version migration after. | V1 design → V2 |
| CERT-02 — Freeze the explicit 65-certificate signer graph | PKI-001–006, 009–010, PKI2-004–008, PKI-068; §§3.2,24.2,33.2,48.2 | Serialize the literal §24.2 graph: 16 issuer keys, 52 issuer certs, 13 root cert variants and exact signer edge IDs; one canonical edge manifest owned here. | Machine oracle counts 20 CA key IDs/65 CA cert IDs; exact edge-set comparison; all 13 roots exact `2050-07-07T12:00:00Z`, all 52 issuer certs exact `2032-07-07T12:00:00Z` in reference mode. Earlier caps fail reference mode or require an explicit non-reference profile/new authorization. | ROOT-01, ROOT-02, ROOT-03, ROOT-04, ROOT-05, CERT-01, ARCH-06; graph version immutable once signed; rollback by a new graph version. | V1 design, V2 fixture gate |
| CERT-03 — Implement algorithm/object-specific PKI signer adapters | PKI-008, PKI2-008, PKI-071/074; spec §§10,24.3,33.3 | PKI-specific capability reports and adapter to ARCH-06's shared signer/artifact authority interface; no duplicate generic signer contract. Consume exact tuples only after final public SPKI, serial, dates, and TBS hash are frozen; key ceremony/storage approval is separate. | Synthetic exact-TBS sign/verify; per-algorithm/object capability proof. Reject authorization built before final fields. Race two consumers and inject crashes around durable consume, signer call and receipt; at most one invocation, uncertain tuple stays burned until authoritative reconciliation. Oracle: durable consume state + signer invocation count + independently verified signature. | ARCH-06, ROOT-05; preserve receipts; revoke only test handles. | V2 |
| CERT-04 — Build strict CSR parser and proof-of-possession validator | PKI-014, 026, 045; §§10,24; RFC 2986 | CSR PEM/DER inspect, signature validation, SPKI and requested extension extraction, bounded parse. | Valid CSR accepted; malformed DER, invalid PoP, unsupported OID, unknown critical ext, oversized input, duplicate extension and malicious path rejected before signer; parser oracle + OpenSSL cross-check. | CERT-01, crypto adapters; no persistent CSR by default. | V2 |
| CERT-05 — Define purpose profiles, extension policy, and SAN/wildcard validator | PKI-015–016, 023–027; §§3.3–3.4,24.6,44; RFC 5280, RFC 9525 | Base/InternalWeb profile registry; SAN normalization; KU/EKU/pathLen/name constraints; no CN fallback. | Exact four Web SANs; profile controls and public-suffix fixture; reject malformed/wide wildcard, CN-only, constrained-out SAN, algorithm-incompatible KU/EKU. Oracle is canonical SAN/profile report plus independent hostname/path checks. | CERT-01; no rollback of immutable profile IDs, supersede with versioned profile. | V2 |
| CERT-06 — Implement issuer-set synthetic builder and graph verification | PKI-009–013, 018–022, PKI2-002–009; §§24.2–5 | Separate ceremony/key/storage approval from signature grants. Generate synthetic A/B public keys, reserve serials, freeze exact TBS and request finite ARCH-06 tuples; manifest uses stable opaque IDs. | 16 distinct issuer keys; all 52 issuer certs follow frozen CERT-02 matrix and exact `2032-07-07T12:00:00Z` reference expiry. Earlier caps fail reference mode or require a named non-reference profile/new authorization. Reject grant not bound to final SPKI/serial/dates/TBS. | CERT-01, CERT-02, CERT-03, CERT-05, ROOT-03, ROOT-04, ROOT-05, ARCH-06; test-only entropy; fixture-only rollback. | V2 |
| CERT-07 — Implement leaf issuance API/CLI and fast in-memory mode | PKI-023–027, 051–052, 055–057; §§6,24.6,48 | Issue Base and InternalWeb synthetic leaves, multiple separately keyed algorithm variants, no docs/network fast path. | 1y Base and 90d Web defaults; SAN and issuer constraints; private key stays in memory/test handle; API/CLI parity; expired parent, no SAN web, invalid wildcard and unsupported algorithm fail. | CERT-04, CERT-05, CERT-06, ARCH-07; issuance operation idempotent; staged test artifacts only. | V2 |
| CERT-08 — Add renew/rotate/replace state and safe resume | PKI-042–044; §§5,9,24.4–5,48.2 | Same-key cert revision renew; new key generation rotate; replacement plan and resumable receipts. | Renew preserves key ID/SPKI and increments cert revision/serial; rotate changes key ID/SPKI/generation; retry cannot double-sign; interruption/fault injection resumes from receipt. | CERT-03, CERT-06, CERT-07, OPS-03; before activation can cancel; afterward requires explicit overlap/deprecation plan. | V2→V3 |
| CERT-09 — Build CRL state, signing, and lifecycle ledger | PKI-033,036; §§4.3,10–11,24.7; RFC 5280 | Per-issuer serial ledger and issuer-signed leaf CRLs; ROOT-06 owns root-signed CA CRLs. | Initial empty CRL → revoke test serial → higher CRL fetched at original unchanged CDP contains serial; monotonic number, valid signatures/times, no phantom good. | CERT-03, CERT-06, ROOT-06, ARCH-06; CERT-10 publication; CRL archives immutable; correction via higher number. | V2 |
| CERT-10 — Construct durable CDP and immutable CRL publication plans | PKI-033–040; §§4,24.7–9,44.1 | URL graph builder, per-key/generation stable CDP URL, numbered CRL archives, MIME/cache headers, three-mirror plan and local HTTP fixture; no production DNS/deploy. | Initial empty CRL → revoke serial → publish higher CRL → fetch original unchanged CDP and observe serial. Verify mirrors, signature, issuer, freshness, ETag/Expires and max-age≤nextUpdate before promotion. | CERT-06, CERT-09, ROOT-06, ARCH-06; local stage only; never point current URL at unrelated issuer or lower CRL number. | V2 |
| CERT-11 — Add staged publication and guard-aware alias receipts | PKI-037–040,055,057; §§4,6,24.7,48; ADR-S02/A-1 | PKI adapter to ARCH-06 shared artifact-authority seam; immutable objects and controlled routes; guard per adapter/operation, no portable CAS assumption. BWS/op/bw stay unguarded absent evidence. | Default preflight→external edit→apply preserves external value and reports conflict. Separately approved weaker mode discloses lost-update window and does not claim preservation. CRL route advances only after verified mirrors. | ARCH-06, CERT-10; rollback cannot retract cache, report residual exposure. | V2; V3 gated |
| CERT-12 — Implement certificates-only CMS and delivery kit formats | PKI-028–035,048,055; §§4,7,24.9; RFC 5652 | PEM/DER, p7c certs-only SignedData, chains, minimal/public-only/handoff kits; private formats gated. | CMS parser says SignedData/no signers/expected fingerprints; PEM/DER equivalent cert; correct chain order/MIME; reject private key in public kit, unsupported algorithm and mislabeled SSH/JWT. | CERT-06, CERT-07, CERT-17; artifact output immutable after receipt. | V2 |
| CERT-13 — Implement trust-profile export, install plan, and test matrix | PKI-006–007,035,059,061–062; §§3.5,11,44,48 | `roots-all`, classical, PQ-only bundles; untrusted cross-certs/intermediates; platform install plans, no automatic mutation. | PQ-only rejects classical chain; all-roots not labelled PQ; cross cert not anchor; OS plan requires fingerprint/scope; tests OpenSSL + supported Rust path validator and explicit unavailable cases. | CERT-02, CERT-06, CERT-12, ROOT-07; install is separate V3 approval step; uninstall is planned, not silent. | V2; gated V3 |
| CERT-14 — Make OCSP capability explicitly absent unless complete | PKI-033, PKI2-022; §§4.3,24.7,33.2,48.2; RFC 6960 | Capabilities expose OCSP unsupported; URI omitted by default; issue a future implementation contract only after real responder readiness. | No OCSP AIA with no endpoint; query unsupported algorithm/issuer never returns fake good; distinguish unknown/unauthorized/tryLater; explicitly test OpenBao ML-DSA gap. | ROOT-01, CERT-03, CERT-09; future dedicated issue; no rollback needed while absent. | V1 decision, V2 negative test |
| CERT-15 — Generate DNS and publication runbooks without applying them | PKI-037–040,059; §§4.4,24.7–8,44 | BIND + provider-neutral change manifest, URL/DNS prerequisites, install/readme docs; no network mutation. | Valid zone parser and schema; unresolved placeholder is not executable; correct distinction CAA/public ACME vs private CA; no claim DNS live; docs link exact relative artifacts. | CERT-10, CERT-12; generated files versioned; no DNS apply. | V2 |
| CERT-16 — Add issuance state, journaling, and failure recovery | PKI-043,047,049–050,054–057; §§6,10,24–25,48; ADR-S02/A-1 | PKI operation receipts on OPS-01/OPS-03 shared operation engine; no duplicate journal. | Inject failure before/after signer, CRL, artifact write, publish. Default unguarded write stops `needs_review` and external value remains recoverable; weaker approved mode states risk and does not claim preservation. Unknown args rejected before side effects. | OPS-01, OPS-03, ARCH-06, CERT-03, CERT-06, CERT-09, CERT-11; preserve signed/published evidence. | V2 |
| CERT-17 — Add library backend compatibility and independent validation evidence | PKI-061–064,070,074; §§11,35,48; research dossier | Pin Rust/OpenSSL adapter versions and produce per-alg/object compatibility matrix; no broad dependency adoption absent evidence. | Certificate, CSR, CRL, CMS and external signer tests as supported; OpenSSL + independent Rust validator; ML-DSA unavailable marked unsupported; no silent substitution. | ROOT-01, CERT-01, CERT-03, CERT-04, CERT-06, CERT-09, CERT-12, ARCH-06; version bump requires rerun corpus. | V2 |
| CERT-18 — Stage guarded production issuance and publication approvals | PKI-008,053–057,071; §§10,24.3,33.2,48; Addendum §5 | Approval policy, signer/key destination allowlists, exact object changes, audit and operator review; this is not part of V1. | Missing/expired/wrong-scope approval blocks before secret/key/network side effect; root signer explicit; three escrow targets only when separately authorized; dry-run proves plan and no mutation. | ARCH-06, OPS-10, CERT-03, CERT-08, CERT-09, CERT-10, CERT-11, CERT-13, CERT-16, ROOT-05, ROOT-09, ROOT-10. | V3 blocked pending explicit authority |
| CERT-19 — Prepare offline/hardware migration and rotation rehearsal | PKI-044,071,074; §§24.3,33.3,40,48; Addendum §5 | Design/test adapter migration, protected recovery and 20-year replace/overlap plan; no HSM assertion without actual proof. | Simulated signer outage/restore and migration verify; old/new trust overlap and recovery tested; no root secret export in artifacts. | CERT-03, CERT-08, CERT-13, CERT-18, ROOT-11; rollback to prior signer only while approved and valid. | V4 research/design |

### 9.1 Suggested dependency waves

* **Wave 0 / design:** CERT-01, CERT-02, CERT-14, CERT-17 (research evidence can begin independently). Serialize the explicit §24.2 edge manifest before the V2 fixture is frozen; no source ambiguity or human graph approval is pending.
* **Wave 1 / foundational V2:** CERT-03, CERT-04, CERT-05, CERT-16. CERT-01 and ARCH-01/ARCH-06 shared API contracts are blockers.
* **Wave 2 / issuers and outputs:** CERT-06; then CERT-07 and CERT-09; CERT-12 and CERT-13 can begin from verified fixtures; CERT-10 depends on CRL/profile output.
* **Wave 3 / lifecycle and publication:** CERT-08, CERT-11, CERT-15, CERT-17. CERT-11 depends on ARCH-06's shared artifact-authority seam and CERT-10. CERT-18 remains explicitly blocked until approval; no issue can promote itself by being unblocked technically.
* **Wave 4 / future custody:** CERT-19 after signer abstraction and V3 approval evidence; parallel infrastructure studies remain nonblocking.

No issue should claim end-to-end PKI completion until CERT-02 count/graph, CERT-06/07 issuance, CERT-09/10 CRL/AIA readiness, CERT-12 formats, CERT-13 trust matrix and independent verification are jointly demonstrated.

## 10. Open decisions and defaults

1. **Exact reference graph:** §24.2 directly specifies 13 issuer certs per purpose/partition, 52 issuer certs total, plus 13 root certs = 65. CERT-02 emits that literal graph as a machine-readable manifest; there is no pending source interpretation or approval.
2. **Signer choice per algorithm/object:** no final crate selection yet. Default design is capability-driven adapter, with fail-closed behavior and test-specific support declaration; do not claim a single crate handles all four signatures/CSR/CRL/CMS/HSM.
3. **Exact CRL cadence/validity and outage policy:** choose per signer availability and consumer needs in a V3 policy; until then initial signed CRL is required, no invented schedule, and stale/unavailable state is not `good`.
4. **OCSP:** disabled/unadvertised by default. Future implementation needs full acceptance as §4.6; library API alone does not satisfy this.
5. **Name constraints and client enforcement:** use policy constraints and tests where interoperable; do not rely on OU or issuer EKU alone as authorization boundary. Client-specific enforcement evidence is required.
6. **Public URL/DNS ownership and triple mirror targets:** URLs are configurable placeholders in V2 only. Production addresses, DNS changes, public publishing and internal `.internal` resolver require a separate deployment decision.
7. **Production escrow details:** the addendum names three approved target classes; actual project/vault identifiers, encryption key custody, approvals and restore identities remain unverified/approval-gated. R2 stays research-only.
8. **Supported client matrix for ML-DSA/TLS:** no universal support assumption. Record exact verifier/library/client versions; TLS key exchange and signature negotiation are separate from certificate path verification.

## 11. Source and evidence register

### In-repo governing materials read

**Source oracle:** Archive sections and source line ranges refer to the immutable original intake at `doc/2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets/source-snapshots/intake-34faffbff2/sources/`, commit `34faffbff26615e313488f6d04705a444342e41e`; `../source-integrity.md` records provenance. The formatted `sources/` view is not the byte/line oracle.

* `../source-snapshots/intake-34faffbff2/sources/COORDINATOR_summit_handoff.md` — especially §§0, 4.2, 7.5–7.6, 9–10 (scope, ticket contract, PKI, safety).
* `../source-snapshots/intake-34faffbff2/sources/fleet_agent_handoff_prompt_v0_3.md` — complete future implementation assignment, especially decisions, PKI scope, commands, APIs, tests and no-silent-substitution invariants.
* `../source-snapshots/intake-34faffbff2/sources/fleet_architecture_spec_v0_3.md` — §§3–7, 10–11, 24, 30–31, 33, 44, 48 and Appendix A.
* `../source-snapshots/intake-34faffbff2/sources/ADDENDUM_secrets_architecture_performance_infrastructure.md` — §5 root custody refinement.
* `../source-snapshots/intake-34faffbff2/note.md` — source precedence, V1 census, stage scope and approval limits.
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
