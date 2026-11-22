# Fleet v0.3 — Secrets Infrastructure Landscape, Decision Record, and Research Dossier

**Research date:** 2026-11-06. **Status:** researched proposal, not a deployment or claim of measured local performance. **Scope:** storage, access, credential continuity, GitOps, Rust, future identity and cluster infrastructure. This document supplements the full Fleet design specification; the parent specification controls the acceptance criteria. External claims are grounded in the official or project-primary sources collected below. Verify versions, pricing and limits again at implementation.

## Executive decision (ADR-FLEET-SEC-001)

**Current recommendation: retain 1Password as the human-credential system and Bitwarden Secrets Manager (`bws`) as an existing machine-secret source; build a source-neutral `fleet-secrets` API, explicit authorized synchronization, an encrypted local continuity/cache tier, and SOPS/age for declarative NixOS and Kubernetes deployment secrets. Do not immediately replace either vault or create a home-grown central vault.** Treat OpenBao as the strongest future service/backend candidate and `fnox` as a potentially valuable complementary secret-resolution and application-injection tool. Preserve the ability to transition to **1Password + OpenBao**, **1Password + SOPS/age**, or **1Password + self-hosted/Cloudflare encrypted replica** after measured tests and a separate security review.

This is a **layered design, not a declaration that every value must be copied to every system**. Human logins, machine credentials, signing roots, dynamic credentials, deployment ciphertext, and backups have different custody and availability requirements. Two mirrored copies can improve continuity if failures are independent, but also expand the attack surface and propagate accidental changes. Availability and confidentiality must be evaluated separately.

**Immediate invariants:** maintain native item identifiers during password updates; keep user-requested recoverable `zz.delete` history without silently destroying items; never classify inaccessible items as deleted; never infer all existing 1Password/BW vaults are accessible from a BWS machine token; do not put offline CA root private keys into everyday sync scopes; require field-level merges with explicit conflict decisions; enforce host/service scopes; log operations without secret values; prove restorable offline access; benchmark before optimizing supposed BWS latency.

### Why these roles fit

| Need | Preferred source or tool now | Rationale and boundary |
|---|---|---|
| Human passwords, browser/mobile autofill and passkeys | 1Password | Existing client UX and native account/item IDs; avoid replacing a working human vault with a raw machine KV. |
| Scoped machine credentials, projects and automation tokens | Bitwarden Secrets Manager (BWS) | Already accessible; machine-account/project boundaries; native Rust SDK. Free tier explicitly offers unlimited secret records subject to 2 users / 3 projects / 3 machine accounts. |
| Later wider personal password vault sweep | Bitwarden Password Manager (`bw`) **separately** | BWS access does not grant `bw` vault access. Requires a separately approved identity/session, scope and first-run preview. |
| Declarative NixOS and GitOps workloads | SOPS + age; sops-nix; Flux SOPS | Ciphertext can be versioned with configuration; recipients/bootstrap keys remain outside repositories; native NixOS/Flux integration. |
| Low-latency repeated access | Existing Fleet secrets broker; optional fnox daemon; encrypted limited-staleness snapshot | Cache/daemon and batch reads before building a new always-on replicated secret service. Avoid writing plaintext caches. |
| Long-term dynamic secrets, leases, centralized auth, PKI and SSH issuance | OpenBao | Actively maintained open-source Vault lineage; supports standalone/HA/K8s, Transit, PKI and many auth schemes. Adds operations/HA/unseal/backup burden. |
| Global low-cost remote relay | Worker + signed requests + separately encrypted D1/R2 (future) | Only if independent need justifies a new security-critical system. Cloudflare's managed Secrets Store is **not** an unlimited general credential database. |
| Portable encrypted offline artifact | age/SOPS snapshot first; evaluate SecureStore as an import/export backend | Avoid re-inventing a vault format without cryptographic review; evaluate old format/crypto versus modern AEAD recipients. |

## Provider and tooling matrix

Rating is comparative architectural judgment, **not a measured benchmark**. Legend: Now = current phase; Pilot = isolated investigation; Later = research roadmap; Avoid = do not rely on for production now.

| Candidate | Now/Pilot/Later | Primary strengths | Most relevant limitations / blockers | Rust / integration status |
|---|---|---|---|---|
| **1Password** (CLI/service accounts, optional Connect) | **Now** | Excellent human login/autofill UX; multiple vaults; item fields, tags, read/write API; service accounts with vault scoping | Paid service/plan; service-account rate limits depend on plan; service accounts see only authorized vaults; access outage if no offline plan; API model differs from BWS | `op` CLI and 1Password Connect REST; first-party SDKs for JS/Go/Python; test before choosing a Rust HTTP adapter |
| **Bitwarden Secrets Manager (`bws`)** | **Now** | Existing access; free unlimited secret records; projects and machine identities; native Rust SDK; 2-user/3-project/3-machine free allowance | BWS has simple key/value/note/project shape, **not** all Bitwarden Password Manager vault logins; remote auth/session and list latency must be measured; provider limits | Official `bitwarden` Rust crate; currently docs call it beta; compare CLI vs SDK, pin version/MSRV |
| **Bitwarden Password Manager (`bw`)** | **Later approved sweep** | Full logins, URI/autofill, mobile, 2FA records; possibly 1Password mirror | Different credential source and permissions from BWS; personal vault access may require interactive unlock; no assumption of server-managed full reads | `bw` CLI via encrypted session; Bitwarden Password Manager's internal Rust SDK is **not public/stable** |
| **SOPS + age (+ sops-nix)** | **Now** | Encrypted structured files in Git; recipient rotation; excellent declarative NixOS and Flux integration; no always-online service needed at deployment after decryption material is present | No native browser/mobile manager; lacks online per-read ACL, leases and audit by itself; host age keys become critical; Git history retains encrypted old values | SOPS CLI, age Rust crates and Nix/Flux integrations; the canonical Fleet interface wraps, rather than forks, encryption |
| **OpenBao** | **Pilot / future service** | Self-hostable, open source (MPL-2.0), KV, leases, identity/auth, Transit, SSH/PKI, audit, HA Raft/PostgreSQL, Kubernetes Helm | Stateful HA/unseal/DR/TLS/monitoring; single laptop server is single-point-of-failure; no automatic browser/password-manager replacement | HTTP API callable from Rust; build a dedicated adapter; version 2.7 adds external-key PKI/Transit and ML-DSA; Go OCSP currently lacks ML-DSA |
| **fnox** | **Now evaluate; do not replace writer** | MIT/Rust; one logical provider config; 1Password, BWS, SOPS/age, Vault, keychain, dynamic leases, agent proxy, local encrypted cache and in-memory daemon | 1Password and Bitwarden password-manager providers are **read-only**; fnox isn't a bidirectional vault reconciliation engine. Its own cache invalidation/daemon protocols have lifecycle constraints | Rust CLI, `fnox-client` library for daemon reads; version and API compatibility to evaluate |
| **mise** | **Now optional developer UX** | Rust tool/version manager, task runner, shell/CI and bootstrap; can delegate scoped tasks to fnox | Not a canonical secret vault, VM/orchestration or NixOS replacement; fnox integration in mise currently experimental | Rust-based, useful for developer bootstrap and script environment; coexist with Nix flakes |
| **NeoSmart SecureStore / `securestore-rs`** | **Pilot format assessment** | Direct Rust crate, portable encrypted JSON file, multi-language implementations and complementary `ssclient` CLI; Git-friendly | Existing format uses AES-128-CBC, HMAC-SHA1 and PBKDF2-SHA1; not proof of immediate vulnerability, but merits independent crypto/protocol review before adoption as new long-lived security foundation; no network service/auth/audit or rich vault itself | Rust `securestore` crate, native and OpenSSL backends. Run security and maintenance review, import/export proof of concept |
| **`scrt` (loderunner)** | **Avoid production** | Go CLI, local storage ownership, simple operator interface | Repository itself warns it is still under development and **not production ready** | Not a Rust library; use only as design prior art |
| **`pass`/password-store** | **Optional local import/export** | Simple GPG + Git encrypted per-secret files; mature Unix workflow | GPG/key and Git collaboration model, no central API/leasing, limited structured schema and multi-platform human UX | CLI adapter only, no replacement for 1Password |
| **Infisical** | **Research alternative** | Full secrets platform with integrations, machine identities and SDKs | Features/pricing/self-hosting entitlement and identity count must be verified for intended plan; may overlap OpenBao rather than simplify landscape | HTTP/CLI, vendor/edition review required |
| **Cloudflare account Secrets Store** | **Not a canonical vault** | Managed, can bind values into Workers | **100 secrets/account, one store/account in current open beta**; values unreadable via API/dashboard, accessed through associated service binding; not bidirectional secret source | Worker `env.<binding>.get()` only; no equivalent offline/mobile manager |
| **Worker + D1/R2 + cryptographic envelope** | **Future research** | JS/TS edge runtime, modest free allowances, fast distributed retrieval, custom capability checks | You own correctness, key custody, access policy, backup, restore, audit and encryption. D1 replication requires session consistency; KV is eventual; Worker credential becomes high-value | Worker TS/JS plus shared Rust/WASM crypto if viable; must support independent cryptographic audits |
| **HashiCorp Vault** | **Not preferred** | Mature reference architecture, widest ecosystem | User's BSL/license preference; OpenBao provides an MPL-2.0 independent alternative | Vault-compatible API patterns can inform adapters, but license and lifecycle require care |

### Currency and limits, evidence, and caveats

- BWS official Free plan: **unlimited secrets**, **2 users**, **3 projects**, **3 machine accounts**. A second user does not magically include personal-password-vault access. Source: https://bitwarden.com/en-gb/help/secrets-manager-faqs/ and https://bitwarden.com/help/machine-accounts/ .
- Bitwarden official Secrets Manager SDK is Rust; `bitwarden` crate docs currently call release **beta** and say MSRV **1.88**. Fleet's checked repository has `Cargo.toml` `rust-version = 1.85` and pinned `rust-toolchain.toml` channel **1.99.0**; reconcile the advertised MSRV and lockfile before changing crates. Sources: https://docs.rs/bitwarden/latest/bitwarden/ and https://contributing.bitwarden.com/architecture/sdk/secrets-manager/ .
- OpenBao 2.7.2 released **2026-10-26**. v2.7 adds external PKI signing keys and ML-DSA/Transit. Release notes explicitly warn the Go OCSP implementation does **not** support ML-DSA issuer types. Sources: https://openbao.org/community/release-notes/2-7-0/ and https://openbao.org/docs/platform/k8s/ . Do not promise working ML-DSA OCSP via OpenBao without a separate implementation.
- Cloudflare Secrets Store account beta: **100 secrets/account**, **one store/account**, maximum string **65,536 bytes**, and secret values cannot be directly retrieved via dashboard or API after creation. Sources: https://developers.cloudflare.com/secrets-store/manage-secrets/ and https://developers.cloudflare.com/secrets-store/integrations/workers/ .
- Cloudflare Workers Free: **100,000 Worker requests/day**; KV Free **100,000 reads/day**, **1,000 writes/day**, **1 GiB storage**; D1 Free approximately **5 GiB storage**, **5 million rows read/day**, **100,000 rows written/day**; R2 Free **10 GB-month storage** and monthly operation allowances. Workers Paid starts at **$5/month** with included usage plus metered excess. These are **platform**, not security engineering, limits. Sources: https://developers.cloudflare.com/workers/platform/pricing/ , https://developers.cloudflare.com/kv/platform/limits/ , https://www.cloudflare.com/plans/ . Recheck all before production.
- Cloudflare KV is **eventually consistent** and caches absent keys; changes can take ~60 seconds or more across locations. Do **not** use it as the immediate-consistency source of truth for password rotation, tombstone reconciliation or compare-and-swap. D1 read replication also introduces replica lag; the D1 Sessions API provides appropriate sequential consistency where used. Sources: https://developers.cloudflare.com/kv/concepts/how-kv-works/ and https://developers.cloudflare.com/d1/best-practices/read-replication/ .
- 1Password service-account API rate limits vary by plan; do not invent throughput or assume its Connect server is free under the user's subscription. Inspect entitlement and benchmark. Source: https://1password.com/developer-security ; https://www.1password.dev/sdks/manage-items/ ; https://www.1password.dev/connect/server-configuration/ .
- SecureStore documentation specifies AES-128-CBC + HMAC-SHA1 + PBKDF2-SHA1; this **does not establish a practical cryptographic break** but differs from the modern age/SOPS AEAD-oriented path. Demand an independent assessment. Source: https://github.com/neosmart/SecureStore and https://github.com/neosmart/securestore-rs .
- `scrt` README says *not production ready*. Source: https://github.com/loderunner/scrt .

## Native credential schema and synchronization implications

### Three distinct provider types, never silently conflated

- `op://` human-oriented 1Password item (may be Login, Secure Note, API Credential, SSH Key, passkey, document/file, custom). Supports rich sections, tags and fields; service account needs actual permissions to the relevant vault.
- `bws://` Bitwarden **Secrets Manager** item (machine account/token scopes; primarily simple secrets). No ambient access to the password-manager vault. Store complex logical records as a versioned canonical, encrypted payload plus searchable non-secret metadata, with integrity proof and provider-specific mapping, *only for authorized records*.
- `bw://` Bitwarden **Password Manager** login, TOTP or other vault item (separate service/session and authorization). This is a later sweep; do not present it as accessible via `BWS_ACCESS_TOKEN`.

A stored logical object can have one or more provider handles, but **provider copies must not automatically inherit every other's privileges**. Represent `replica-policy` and `class-policy` independently. All values, including custom fields, attachments, URI/autofill metadata, OTP seeds and passkeys, are confidential by default. A browser autofill item is not interchangeable with a generic BWS key/value pair. Preserve original native record IDs through in-place updates and reconcile custom-field types carefully.

### Failure and recovery model

1. Keep an encrypted last-verified baseline for three-way comparisons; a provider timeout is `unavailable`, **never** `deleted`.
2. A deletion event requires a verified native prior ID, successful refreshed listing, and stable confirmation. Revisions and tombstones have immutable ledger IDs.
3. Under the user's retention rule, create dated `zz.delete` history records on **approved authorized stores** before updating a stale password in place. Preserve the native IDs of autofill records; the history archive must not itself turn into a normal login/autofill suggestion.
4. Store provider-native exact IDs and object hashes separately from canonical logical IDs; don't key equality on display name alone.
5. If both sides change independently since a verified baseline, hold for field-level conflict review. Timestamp can rank candidates, not prove truth. “Not found” due to insufficient scope is not deletion.
6. Never mirror a root private key, recovery code, hardware-backed key or restricted item without a separate explicit class and destination policy. A live copy of every credential everywhere is both an availability tactic **and** an increased blast radius.
7. Service can return an authenticated, encrypted **stale-but-approved read snapshot** only where profile permits and where revocation/rotation windows and max age are satisfied. Do not use offline stale values for writes or as authority to overwrite a current provider.
8. Verify reads and writes and tamper-evident state. Retained `zz.delete` values are sensitive and should live under tight authorization with separate incident/recovery documentation. Never silently purge; expose long-term retention tradeoffs to the user.
9. Keep one canonical append-only run/action audit record, plus provider write receipts and Supabase indexes. Do not log secret values, password diffs, TOTP seeds or unencrypted content hashes susceptible to guessing; use keyed/HMAC-based comparison tokens with a separately protected key if equality proofs are needed.
10. First run is an inventory/reconciliation project: dry-run maps, pair confidence, metadata-preservation proof, read-back of all important fields, review of unexpected deletions, and simulation with fixtures. Later once-daily reconciliation can use change windows + periodic full re-scans to catch API omissions.

## Architecture alternatives and decisions

### Path A — Near-term current system (preferred)

`1Password (people)` + `BWS (machines)` + `SOPS/age (declarative deployments)` + `Fleet broker/cache/SDK & sync engine`. Leverages paid/existing tools; scope narrowly, keep offline read continuity for selected non-root secrets. It does **not** require spinning up another stateful network service.

**Commitment now:** create backend capability matrix; provider contracts; authorized first/second sweep plans; read/write reconciliation; benchmark BWS/op; caching; local encrypted continuity snapshot; export-import; SOPS provider; event logs. Preserve compatibility with future OpenBao and Cloudflare adapters.

### Path B — 1Password + SOPS/age only

Smaller online blast radius and fewer foreign vendor failure modes, if OP service account/access works for all needed vaults and machines. Tradeoff: one commercial online authority remains; SOPS protects deployment recoverability but cannot replace native autofill, dynamically leased credentials or broad audit. Benchmark and compare before retiring BWS.

### Path C — 1Password + OpenBao

Human and machine/PKI separation with a strong standalone machine service, later HA. Excellent long-term candidate for many K8s sites if the project accepts operating a secret management control plane. Do not treat `OpenBao on one laptop` as highly available. Start pilot with synthetic data, auth policies, audit, unseal/restore testing, explicit full-backup proof and production hardening checklist.

### Path D — 1Password + custom Cloudflare encrypted service

Could be inexpensive and globally reachable with configurable policies; future only. Build a thin Worker gateway over **application-level envelope-encrypted values** in D1 (revision/index) and R2 (encrypted immutable blob/archive), optionally a Durable Object to serialize updates. Keep keys independent of database; use Cloudflare Access or mutual auth and narrow machine identities; consider Tailscale as a private connectivity layer, not an automatically supplied identity proof at a public Worker URL. Replication must have version/CAS, audit, per-principal policy, anti-replay, revocation, negative tests, backup/restore, and external review. Worker `env`-bound master key is a significant single compromise boundary. Do not place root CA signing keys here. **Do not confuse the 100-secret Secrets Store with D1/R2 storage.**

### Path E — OpenBao + SOPS/age with reduced commercial dependence

Viable after cluster maturity. Adds operational responsibility, may sacrifice human password manager and passkey UX if 1Password retired. Treat personal credential app as a different product category, not automatically replaced by OpenBao KV.

## The deeper design choice: an identity fabric instead of perpetual static copies

The end state should be: machine/service/agent has a **workload identity** (SPIFFE/SPIRE, Kubernetes service account, Tailscale identity or trusted host certificate); policy maps identity to capability/role and short-lived authorization; a secret store issues only narrowly scoped credentials, ideally dynamic and revocable; applications consume these without durable global vault tokens. This reduces duplication and per-item reconciliation. The next-stage research must compare OpenBao AppRole/Kubernetes/JWT/OIDC auth, OpenBao dynamic engines, SPIRE SVIDs, cert-manager, trust-manager, Tailscale workload restrictions, and eventual Fleet `id` integration. A full public SSO/OIDC/SAML service is a distinct engineering project and **not** a prerequisite for secrets sync.

## Upcoming infrastructure investigation — boundaries, not premature commitments

- **Cozystack/Talos/Kubernetes:** Cozystack's own technical review describes Flux, Helm, cert-manager, External Secrets Operator, OpenBao, KubeVirt, NATS, and other infrastructure layers. Flux + Helm is a natural baseline; compare Argo CD as an alternate reconciliation UX. **Tekton is primarily pipelines/CI**, not the same architectural substitute as a GitOps reconciler. Investigate how Cozystack's management cluster uses Flux before customizing it. Source: https://github.com/cozystack/cozystack/blob/main/GENERAL_TECHNICAL_REVIEW.md .
- **SOPS/age:** sops-nix decrypts NixOS secrets at activation with permissioned files; Flux's Kustomization decryption supports SOPS. For secrets directly injected from live OpenBao/1Password/BWS, evaluate External Secrets Operator and Secrets Store CSI/Agent; some currently advertised ESO providers (including OpenBao/BWS/1Password) are marked **alpha**, so treat version-specific provider functionality as an integration test, not a guarantee. Sources: https://github.com/Mic92/sops-nix ; https://fluxcd.io/flux/components/kustomize/kustomizations/ ; https://external-secrets.io/v2.13.0/introduction/stability-support/ .
- **NixOS vs Lima vs mise:** prefer NixOS declarative hosts/VM images if Nix already owns fleet deployment; use Lima for portable local developer VMs or atypical guest workflows, not as a replacement for KubeVirt in the cluster. mise complements Nix for versioned project tools/tasks and can be installed from Nix; avoid letting it silently introduce a second host configuration authority. Sources: https://lima-vm.io/docs/ ; https://mise.jdx.dev/ .
- **Argo CD / Tekton / service mesh:** compare UX, RBAC, drift handling and source of truth (Flux vs Argo), CI/pipeline execution (Tekton vs existing CI), and data-plane security/operations (Linkerd vs Istio). Do not add a service mesh solely because it seems standard; measure which services need mTLS, identity and policy. Teleport belongs to access/bastion/identity research, distinct from source-of-truth secrets.
- **OpenTofu:** optional narrowly scoped IaC with encrypted state/plan and a proper remote locking and backup model. It is not needed to make PKI or sync work. Source: https://opentofu.org/docs/language/state/encryption/ .

## Research backlog: issue-ready investigations

Each should become a separate design issue with an explicit evidence matrix, prototype, failure cases and go/no-go decision. **Difficulty** is relative complexity, not a time estimate.

| ID | Research epic | Difficulty | Leading decision or artifact |
|---|---|---|---|
| R-SEC-01 | Provider benchmarks: BWS CLI vs Rust SDK, OP CLI vs SDK/Connect, cold/warm, 1/10/100 items, list, RTT, auth, cache | Medium | Reproducible latency and reliability report; choose caching vs new backend. |
| R-SEC-02 | Full first-run 1Password/BWS/BW mappings and granted access audit | High | Redacted discovery inventory, attribute-equivalence map, safe migration plan, explicit second-sweep authorization. |
| R-SEC-03 | Offline encrypted continuity snapshot and break-glass recovery | High | Threat model, tested restore on disconnected host, max-staleness policy, independent decryption key custody. |
| R-SEC-04 | OpenBao service design: standalone → HA K8s multi-site | Very high | Proof with Raft snapshots, unseal, auth, PKI/Transit, failover, ML-DSA, unsupported OCSP path, CRLs, operability. |
| R-SEC-05 | Cloudflare Worker encrypted gateway | Very high | Threat model and architecture RFC; D1 consistency, crypto, Access/Tailscale, escrow, offsite restore, audit, cost model. |
| R-SEC-06 | SecureStore v3 security and interoperability audit | Medium | AEAD/MAC/KDF analysis, sample cross-language decryption, compatibility with age/SOPS, recommendation. |
| R-SEC-07 | fnox + Fleet broker interop or reuse | Medium | Demonstrate bounded provider reads, TTL, provenance and invalidation, CLI vs Rust integration, no writers conflated. |
| R-SEC-08 | `sops-nix` + Flux + External Secrets multi-site delivery | High | Working Talos/NixOS examples, age recipients, GitOps rotation, offline recovery, ESO alpha-provider tests. |
| R-SEC-09 | Identity-first secretless services (SPIFFE/SPIRE/OpenBao/K8s/Tailscale) | Very high | Workload identity graph and progressive migration from static tokens to short-lived credentials. |
| R-SEC-10 | Cloud-first/public ACME + private PKI + cert-manager/ trust-manager | High | Domain/ACME/DCV/CT/CAA design, internal trust bundles, cert profiles and renewal acceptance. |
| R-SEC-11 | Cozystack Flux/Helm vs Argo CD; Tekton/CI; service mesh | High | Decision matrix for management cluster and tenant clusters, blast radius, tenancy, operations. |
| R-SEC-12 | NixOS image generation, Lima dev VMs, KubeVirt, mise bootstrap | Medium-high | One-source-of-truth design, sample fleet host lifecycle, portability criteria. |
| R-SEC-13 | OpenTofu for bounded infrastructure and secret-free state | Medium | Pilot one Cloudflare/K8s resource with encrypted remote state and no exposed secrets. |
| R-SEC-14 | Teleport, tailnet policy, Cloudflare Access and service identity | High | Access plane threat model, machine identity attestation, emergency access design. |
| R-SEC-15 | User vault migration/passkeys/TOTP/OTP UX | Very high | Explicitly authorized second sweep; field-complete export/import proof; autofill and passkey portability study. |
| R-SEC-16 | Fleet `id`, OCapN, capabilities, service-to-service identity | Research-intensive | Scope and identity/capability model; integration contracts and delegated authority design. |
| R-SEC-17 | Distributed revocation, signer isolation and cryptographic agility | Very high | 20-year PKI lifecycle, key provider abstraction, ML-DSA interoperability, client matrix. |
| R-SEC-18 | Secret provenance, supply chain signing, metrics and forensics | High | Tamper-evident event history without secret plaintext, Merkle/signature or equivalent evaluation, provenance schema. |

## Official and primary references (research index)

1. Bitwarden Secrets Manager FAQ and machine accounts: https://bitwarden.com/en-gb/help/secrets-manager-faqs/ ; https://bitwarden.com/help/machine-accounts/ ; Rust SDK: https://contributing.bitwarden.com/architecture/sdk/secrets-manager/ ; crate: https://docs.rs/bitwarden/latest/bitwarden/ .
2. 1Password CLI/SDK/Connect: https://www.1password.dev/sdks/manage-items/ ; https://www.1password.dev/connect/server-configuration/ ; https://developer.1password.com/docs/cli/secrets-scripts .
3. OpenBao release notes: https://openbao.org/community/release-notes/2-7-0/ ; Kubernetes: https://openbao.org/docs/platform/k8s/ ; Transit: https://openbao.org/docs/secrets/transit/ ; Helm production checklist: https://openbao.org/docs/platform/k8s/helm/run/ .
4. Cloudflare Secrets Store: https://developers.cloudflare.com/secrets-store/manage-secrets/ ; Workers: https://developers.cloudflare.com/workers/platform/pricing/ ; KV consistency: https://developers.cloudflare.com/kv/concepts/how-kv-works/ ; D1 consistency: https://developers.cloudflare.com/d1/best-practices/read-replication/ .
5. fnox: https://fnox.jdx.dev/ ; provider catalog: https://fnox.jdx.dev/providers/overview ; cache: https://fnox.jdx.dev/guide/sync ; daemon: https://fnox.jdx.dev/guide/daemon .
6. SOPS: https://getsops.io/docs/ ; sops-nix: https://github.com/Mic92/sops-nix ; Flux SOPS: https://fluxcd.io/flux/components/kustomize/kustomizations/ .
7. SecureStore: https://github.com/neosmart/securestore-rs ; protocol/crypto: https://github.com/neosmart/SecureStore .
8. scrt: https://github.com/loderunner/scrt ; pass: https://www.passwordstore.org/ ; Infisical: https://infisical.com/pricing .
9. Cozystack: https://github.com/cozystack/cozystack/blob/main/GENERAL_TECHNICAL_REVIEW.md ; ESO providers: https://external-secrets.io/v2.13.0/introduction/stability-support/ ; cert-manager trust: https://cert-manager.io/docs/trust/ .
10. mise: https://mise.jdx.dev/ ; Lima: https://lima-vm.io/docs/ ; OpenTofu encryption: https://opentofu.org/docs/language/state/encryption/ .
11. HashiCorp licensing text: https://github.com/hashicorp/vault/blob/main/LICENSE .

**Research limitations:** No access to the user's actual 1Password plan, service account grants, BW Password Manager session, BWS latency measurements, live Cloudflare billing, laptop runtime, Vault readiness, or future cluster topology was exercised. The user-provided third input is conceptual/architectural guidance, not evidence that implementation has occurred. All suggested CLI examples in the specification are proposed, not claims of existing commands.
