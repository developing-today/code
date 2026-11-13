# Future infrastructure research register

**Checked:** 2026-11-12. **Parent issue:** #9166. **Companion design:** [`../designs/federation-infrastructure.md`](../designs/federation-infrastructure.md). Read with [`../architecture-review.md`](../architecture-review.md) and [`../source-precedence.md`](../source-precedence.md). Research only; no service was installed, deployed, benchmarked, or connected to real credentials.

## Research conclusion

V1 means the existing pinned behavior, including defects and absent APIs; no new V1 implementation is proposed here. Preserve BWS, 1Password, Fleet broker and existing SOPS/age practices while measuring them. OpenBao is staged research only: V2 synthetic lab → V3 a single explicitly non-HA site/failure-domain test → V4 independently sited federation after operator and security approval. The three-node local Raft cluster requested by the addendum is useful for functional/quorum experiments but emphatically not HA. Do not mirror secrets by default; static credentials, expiring leases, signing keys, deployment ciphertext and trust bundles require different lifecycle and custody.

Fleet logical IDs are stable opaque identities, not paths or provider IDs. Never use display-name/path equality as identity or infer portable CAS from one adapter's capability. Root material is outside the ordinary resolver/read/cache/prefetch/benchmark/equality-index planes; signer and escrow authority are separate. See the cross-cutting ADRs for binding details.

## OpenBao capability map and test questions

Official 2.7.x docs checked on 2026-11-12 expose the following capabilities. This is a documentation review, not a runtime verification.

| Surface | First-party documentation says | Fleet boundary / experiment |
|---|---|---|
| KV | KV1 stores latest only; KV2 configurable version history (default 10), CAS, soft delete/undelete and explicit destroy | Test CAS races, metadata/list access, retention and deleted-version recovery. Fleet remains canonical cross-provider reconciliation authority. |
| Dynamic credentials | Dynamic secret engines return lease IDs and TTL/renew/revoke semantics; expired lease is revoked; revocation may immediately invalidate downstream credential | Prototype a DB or service lease with outage while renewing, consumer shutdown, revoke and orphan cleanup. Never sync as ordinary static secret. |
| Transit | Crypto-as-a-service; no app data storage; encrypt/decrypt, sign/verify, HMAC/hash, random; docs list ML-DSA-44/65/87 signing/verify | Test payload sizes, key version/rotation, policies, audit and latency; do not infer TLS handshake/PQ KEX or OCSP capability from signature engine support. Keep Fleet Rust crypto where local/offline and appropriate. |
| PKI | Dynamic X.509 issuance, short TTL rationale, HTTP API and issuance policies | Test actual roots/intermediates, RSA/P-256/Ed25519/ML-DSA constraints, AIA/CRL/OCSP, key export/nonexport and rotation. Fleet owns canonical four-root design and artifact semantics unless a separate decision delegates a bounded issuer. |
| SSH | SSH engine offers signed SSH certificates and one-time SSH password modes | Test principals, TTL, host/user roles, verification and revocation independently from X.509. |
| Auth | Kubernetes JWT/TokenReview and AppRole are documented; Kubernetes tokens may be short-lived; JWT auth tradeoffs; AppRole SecretID should be guarded, preferably pull/wrapped | Compare workload attestation and static bootstrap. No long-lived AppRole secret embedded in image, repo or agent prompt. Test expiry, revocation and wrong namespace/service account. |
| Raft/HA | Raft replicates data among nodes and supports HA; active/standby and quorum; read scaling support varies by version | Three containers on one host are one failure domain. Build loss/partition/restore matrix and report actual tested RPO/RTO. HA availability does not imply horizontal write scale. |
| Seal and recovery | Sealed startup requires root key access; Shamir threshold per node; auto-unseal introduces dependency on seal service/key; losing it may prevent recovery even from backup; recovery mode is special expert path | Test restart, share custody simulation, auto-seal outage, seal key deletion response and clean snapshot restore. Document recovery artifacts separately. |
| Audit | Requests/responses logged; sensitive strings HMACed by default in relevant paths; API requests can be blocked if no audit sink succeeds or a sink hangs | Configure two sinks and test failures, backpressure, rotation, retention, access and tamper evidence. Avoid raw logs and secret data. |

## Infrastructure alternatives and boundaries

* **Talos/Cozystack/Kubernetes:** evaluate eventual management/tenant topology and Flux/Helm deployment patterns. Do not presume cluster exists/ready. K8s is a workload/orchestration substrate, not a replacement credential manager or offsite backup. NixOS remains host declarative authority where already adopted; mise can manage project tools but not become a second host authority.
* **Flux or Argo CD:** choose one reconciler/source-of-truth per object. Tekton is a CI/pipeline runner, not a GitOps reconciler. Compare RBAC, drift, bootstrap, rollback and operations in a disposable cluster before adding another controller.
* **cert-manager/trust-manager:** delegate Kubernetes certificate object lifecycle and trust-bundle rollout where compatible. Do not put offline root key onto cluster nodes; no double writers for same TLS Secret. Non-Kubernetes/public PKI and Fleet root hierarchy remain separate.
* **ESO or CSI:** ESO syncs external provider values into Kubernetes Secret resources; CSI mounts external values via volumes. Both introduce provider-to-cluster/node boundaries. ESO source matrix currently labels OpenBao provider alpha; CSI's core feature status is not a provider assurance. Test rotation, deletion, outage, RBAC/etcd exposure and pod reload against pinned versions.
* **SPIFFE/SPIRE:** use if attested workload identity and SVID rotation are justified. Bundle endpoint/trust-domain/profile are explicit and relationship is directional. It answers workload identity/federated trust; not static secret storage or human SSO.
* **Tailscale/Teleport/Cloudflare Access:** evaluate as connectivity and access plane only. Validate principal binding and resource authorization separately. #9077 is owner of SSO/OIDC, passkeys and relying-party lifecycle; this research only describes potential integration contracts.
* **Cloudflare:** Workers free tier limits/CPU and D1/KV limits are finite and evolve. R2 offers encrypted archive storage potential but no secret policy engine, consistency/CAS or key custody. Cloudflare Secrets Store is limited to one store and 100 production strings/account (max 65,536 bytes), unreadable through API/dashboard after creation, and only appropriate for Worker-bound secrets. Custom Worker+D1+R2+DO is a new security-critical service with a large Fleet ownership burden; no current need established.
* **OCI free tier:** current docs reviewed state $300 trial credits for up to 30 days, after which Always Free resources remain; A1 total must stay within 2 OCPU and 12 GB memory across instances to avoid disable/delete after trial. Home region matters; live quotas, storage, public networking, availability and actual resource fit remain unverified. Even if three instances fit, one cloud region/account/control plane is not multi-site resilience.

## OpenBao staged experiment requirements

### V2: synthetic functional lab

Pin image digest and version; three local nodes with distinct data volumes; synthetic values only. Cover install and config, TLS listeners and cluster traffic, policy isolation, token TTL, KV v2 CAS/version/delete/undelete, selected dynamic engine lease/revoke, Transit sign/verify including ML-DSA-44, PKI/SSH issuance, dual audit sinks, monitoring, Raft snapshot and clean restore. Inject node restart/kill, host poweroff simulation, partition, disk full, audit unavailable/hung, stale token, expired lease and malformed request. Record hardware/CPU/RAM/disk, startup, request latency distributions, recovery steps, failed actions and operator time. The design's security oracle is least-privilege policy plus modeled outcome; cryptographic operation oracle is independent verify; recovery oracle is successful read-back from restored isolated cluster.

### V3: one actual failure domain (nonproduction)

Only after V2 review, evaluate an approved lab host/location with synthetic or expressly approved low-risk test data. Document whether nodes share host, rack, power, network, region, storage/account and admins. Exercise host loss, disk loss, region/site loss, restore to clean host, seal and audit recovery, certificate expiry and operator absence. Set measured RPO/RTO and service objective; establish budget, alerts, patch/upgrade owner, pager, backup custodian and annual recovery drill. If there is only one machine, label availability accordingly and reject “HA”.

### V4: independent-site federation

Separate site control planes and authority. Test read-only trust/bundle exchange first; then only explicitly approved replication classes. Model directional trust, site partition, stale snapshot, concurrent edits, tombstone, credential revocation, lease outage, stale clients, whole-site compromise and removal of trust relationship. Require non-secret signed metadata for provenance, separate site operators, per-record replication scope and observable drift. Distinguish data replication from identity/trust federation. No active-active writes unless conflicts and consistency protocol are proven; default remains local authoritative source plus narrow one-way distribution.

## Priority sequence / go-no-go

1. FUT-01 and FUT-02: test OpenBao behavior and operational baseline using synthetic data.
2. FUT-03 and FUT-04: bootstrap/auth plus complete backup/seal/audit restoration.
3. FUT-05 and FUT-06: OCI feasibility and K8s operator integration, only if useful topology exists.
4. FUT-07 and FUT-08: GitOps and workload identity/mesh selection; avoid adding services by convention.
5. FUT-09 and FUT-12: formal replication model and SOPS/fnox/local continuity comparison.
6. FUT-10 and FUT-11: R2 relay and access plane only if an unmet user requirement remains.
7. V4 federation is blocked on explicit human review of V3 evidence, cost/operations, compromise model and revocation behavior.

Go criteria common to all: reproducible pinned environment; explicit system-of-record and security boundary; positive and negative tests; failure and restoration evidence; quantified cost/resources and operator burden; limitations and unknowns listed; interface does not require production dependency. No-go: hidden root copy, reliance on one host claimed as HA, unbounded stale reads, uncontrolled duplication, unresolved backup/seal key, missing audit sink, K8s controller conflict, unsupported algorithm assumption, or new ongoing operator burden without accepted user outcome.

## Evidence and source register

Primary sources fetched on 2026-11-12:

1. OpenBao official 2.7.x: [HA](https://openbao.org/docs/concepts/ha/), [Raft](https://openbao.org/docs/configuration/storage/raft/), [seal](https://openbao.org/docs/concepts/seal/), [recovery](https://openbao.org/docs/concepts/recovery-mode/), [audit](https://openbao.org/docs/audit/), [KV](https://openbao.org/docs/secrets/kv/), [Transit](https://openbao.org/docs/secrets/transit/), [lease lifecycle](https://openbao.org/docs/concepts/lease/), [PKI](https://openbao.org/docs/secrets/pki/), [SSH](https://openbao.org/docs/secrets/ssh/), [Kubernetes auth](https://openbao.org/docs/auth/kubernetes/), [AppRole](https://openbao.org/docs/auth/approle/).
2. Oracle [OCI Free Tier](https://docs.oracle.com/en-us/iaas/Content/FreeTier/freetier.htm); checked current free-trial and A1 limits, but tenancy-specific quota/capacity not verified.
3. Kubernetes ecosystem: [cert-manager](https://cert-manager.io/docs/), [trust-manager](https://cert-manager.io/docs/trust/trust-manager/), [ESO support levels](https://external-secrets.io/latest/introduction/stability-support/), [CSI driver](https://secrets-store-csi-driver.sigs.k8s.io/).
4. [SPIFFE Federation standard](https://spiffe.io/docs/latest/spiffe-specs/spiffe_federation/) and [SPIRE overview](https://spiffe.io/docs/latest/spire-about/).
5. Cloudflare [Secrets Store](https://developers.cloudflare.com/secrets-store/manage-secrets/), [Workers pricing](https://developers.cloudflare.com/workers/platform/pricing/), [R2 pricing](https://developers.cloudflare.com/r2/pricing/), [D1 replication](https://developers.cloudflare.com/d1/best-practices/read-replication/).
6. In-repository: backend research dossier (2026-11-06); specification §§27, 34–42, 45–49; addendum §§5, 7–8; source input 3; coordinator §4.2. These govern Fleet intent; live vendor docs govern current technical claims.

## Limitations / not verified

No local containers, Kubernetes, OCI account, Cloudflare account, Tailscale/Teleport plan, provider token, Fleet workload or live credentials were accessed. Nothing was benchmarked or deployed. Resource minimums, end-user costs, account-specific free allowances, cluster topology, RPO/RTO, operator coverage, exact OCI quota, and actual security guarantees remain open. Product documentation changes; each issue must recheck sources at execution. Full issue packets are in `../designs/proposals/future-infrastructure.json`; they are research candidates, not authorization to create issues or deploy.
