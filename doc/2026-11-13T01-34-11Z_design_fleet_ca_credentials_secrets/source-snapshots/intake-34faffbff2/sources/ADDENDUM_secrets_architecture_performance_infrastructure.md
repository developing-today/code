Fleet v0.3 — Secrets Architecture, Performance, and Infrastructure Follow-up
This message supplements the previously supplied Fleet v0.3 specification and research archive. Treat it as an architectural refinement and additional implementation direction, not a replacement for the existing requirements.
The overall implementation plan and ordering are good. The main objective is to retain our working infrastructure while developing a faster, more capable, highly composable secrets system that can eventually support self-hosted services, geographically distributed infrastructure, Kubernetes, and dynamically issued credentials.
You have discretion to improve the following design, select appropriate libraries, split crates, and implement additional useful functionality. Preserve the intended outcomes, evaluate the options rather than assuming the choices below are final, and record significant decisions and evidence.
1. Immediate provider strategy: retain BWS
For now, continue using Bitwarden Secrets Manager (BWS) as the principal existing machine-secrets backend. We already have working access and have invested effort in integrating it.
Continue supporting 1Password (op) and the bidirectional reconciliation described in v0.3. Maintain the approved synchronized credential set and preserve the ability to assign credentials to provider-specific compartments that are not automatically replicated everywhere.
I am not committed to BWS permanently. It has reportedly been slow, sometimes taking several seconds to retrieve one secret and longer to list secrets. We need to measure this rather than rely on anecdotal reports.
1Password may eventually be preferable for greater capacity or performance, but it remains a hosted dependency outside our control. Neither should be selected as the universal backend without evidence.
Bitwarden Password Manager (bw) is separate from BWS. We can configure the CLI with appropriate access for comparative benchmarking, but a full personal-vault migration, cleanup, and deduplication sweep should remain separately scoped until the necessary authorization and access are available.
Preserve the previously specified reconciliation behavior, field mapping, audit history, zz.delete records, in-place updates, and safe handling of conflicts and provider outages.
2. Benchmark the existing providers
Implement a small, reproducible benchmark suite for BWS, 1Password, and Bitwarden Password Manager.
Measure cold and warm individual reads, sequential and batched retrieval, list operations, writes and updates, authentication overhead, concurrent requests, rate limits, failures, and recovery.
Report p50/p95/p99 latency, throughput, process startup overhead, memory usage, and the difference between direct SDK/API access and CLI subprocesses where supported.
Compare performance from the laptop, the existing agent environment, and other practical execution locations. Include measurements of the current Fleet resolver and broker.
Use synthetic secrets or safely isolated test credentials. Never expose secret values in benchmark output.
Establish actual performance baselines and identify the causes of unnecessary latency before changing providers.
3. Actively evaluate fnox for immediate integration
fnox is a near-term priority, not merely a future research topic.
Investigate its Rust crates, reusable public APIs, provider adapters, encrypted offline cache, memory-only daemon, authentication model, secret injection, and caching behavior.
Determine which components can be embedded directly into our existing Fleet Rust architecture, versus which would require invoking the fnox executable or adapting its internal code.
We should avoid rebuilding well-tested functionality unnecessarily.
Evaluate whether fnox can provide:
Fast local secret resolution and reusable cached sessions.
Provider-neutral access to BWS, 1Password, age-encrypted files, and eventually OpenBao.
Offline continuity for authorized secrets.
Scoped, configurable caching with clear freshness and invalidation policies.
Efficient in-memory retrieval for other Rust applications.
Secure process injection without needlessly placing secrets in shell histories, logs, or arguments.
Benchmark direct SDK access against fnox and our existing Fleet broker. Confirm whether fnox's Rust APIs are sufficiently stable and suitable for library embedding.
We do not need to adopt fnox wholesale. Fleet remains our main interface and orchestration layer.
If fnox provides most of the right internals, favor integrating them behind Fleet's own versioned APIs rather than constructing another complete secret resolver.
4. SOPS/age and NixOS integration
Expand our use of SOPS, age, and sops-nix, building on the patterns already present in our NixOS configuration and flakes.
Investigate storing selected encrypted secrets alongside their deployment configuration in the relevant repository, so that the configuration and its encrypted values can be versioned together.
The laptop should be able to decrypt its authorized secrets automatically through the existing NixOS/sops-nix provisioning model.
Keep private age decryption identities out of Git and the Nix store. The flake should reference securely provisioned key files or an equivalent supported decryption source.
Evaluate encrypting copies of selected secrets to a master or recovery age recipient, in addition to the copies held in BWS and 1Password.
Develop common operations for generating, encrypting, rekeying, syncing, inspecting, restoring, and deploying these files.
Consider whether SOPS/age and fnox can share a unified provider and caching interface, and how existing flake-based secrets can be adopted without disruptive migration.
5. Root CA key custody — temporary explicit replication
Clarification to the original specification: I do want the root CA private keys securely replicated initially.
We are not yet operating a mature offline root-signing environment, and I do not want the first generation of root keys to exist in only one place.
Fleet's initial root-bundle generation and custody workflow should automatically provision and verify copies in these three approved destinations:
Encrypted local storage on the laptop.
The authorized BWS vault/project.
The authorized 1Password vault.
This is an intentional, temporary custody policy for our own infrastructure.
Use strong access restrictions, encryption, appropriate metadata, integrity checks, and tested restoration. Preserve independent recoverability and distinguish active signing material from escrow copies.
Support SOPS/age-encrypted root archives, ideally with independently protected recovery recipients. Evaluate keeping both a complete encrypted root bundle and individually recoverable encrypted archives for each root.
The tooling should perform the approved initial synchronization and manage these known copies consistently. However, do not automatically extend root-key replication to every future secrets provider, vault, cache, replica, or backup destination. Root material requires its own explicit replication policy.
Investigate a dedicated Cloudflare R2 bucket as an additional temporary backup location. It should be private, tightly permissioned, versioned where appropriate, and inaccessible to ordinary application credentials. Prefer client-side encryption before upload; document the additional exposure of any plaintext-storage alternative rather than enabling it by default. Zstandard compression is useful for packaging but is not encryption.
Eventually, we will harden this arrangement by moving root signing operations offline or into appropriately protected signing hardware, with separate recovery procedures.
Design for that migration now, without preventing us from operating and backing up the initial roots today.
6. Evaluate NeoSmart SecureStore as an actual library component
Perform a focused technical evaluation of neosmart/securestore-rs.
It is interesting because of its native Rust interface, portable encrypted files, and ability to distribute secrets without requiring a continuously available remote service.
Determine whether it provides a useful building block for:
Encrypted local backups and durable offline caches.
Standalone applications.
Small, self-contained secret stores.
Versioned encrypted bundles.
Integration with fnox, SOPS/age, and Fleet's provider interfaces.
Review its cryptographic design, maintenance, compatibility, key management, concurrency behavior, format evolution, and suitability for long-term use.
Evaluate whether it is preferable to using existing age-based encryption or a modern authenticated-encryption envelope built on well-maintained crates.
Do not introduce redundant storage formats merely because they are available. If SecureStore offers a meaningful advantage, integrate it as an optional adapter or crate feature.
7. Design OpenBao as the likely future infrastructure backend
I am provisionally persuaded that OpenBao is the strongest long-term machine-secrets candidate, assuming its performance, security, operational reliability, and ecosystem integration hold up under testing.
Keep it in the active research and prototype roadmap, but don't make it a dependency of the current Fleet implementation.
Consider a staged deployment:
Initial laboratory: Run a three-node OpenBao cluster in local containers, likely on the laptop or another available machine. This is for testing cluster behavior and integrations; three containers on one physical host are not three independent failure domains.
Additional infrastructure: Evaluate deployment on Oracle Cloud's free tier once available, including practical resource requirements, persistent storage, backups, networking, sealing/unsealing, latency, failure recovery, and what real availability guarantees can be achieved within the free limits.
Long-term: Deploy or integrate OpenBao with our eventual local Kubernetes infrastructure, likely Talos/Cozystack-based, potentially spanning additional independent sites and hosts.
Investigate other cost-effective hosting options, including cloud instances, the basement cluster, and distributed or hybrid architectures. Evaluate resource requirements, quorum, availability, operator support, recovery procedures, and the actual consequences of losing a node or whole site.
Do not claim a highly available system when all nodes share one underlying failure domain.
Explore OpenBao's KV, dynamic secrets, PKI, SSH issuance, Transit signing/encryption, authentication methods, policy model, leases, and audit facilities.
We will likely continue generating some keys and certificates ourselves using Fleet's own Rust libraries, while delegating selected signing, storage, leasing, and issuance operations to OpenBao or appropriate Kubernetes services.
Research integration boundaries with cert-manager, External Secrets Operator, Secrets Store CSI drivers, NGINX, Apache APISIX, Istio, Linkerd, other service meshes, and relevant Kubernetes identity and security mechanisms.
Determine which responsibilities belong in Fleet, which belong in OpenBao, and which should be delegated to existing operators or infrastructure software.
Avoid implementing redundant PKI or secrets services simply because Fleet has the capability to generate the same artifacts.
8. Eventual multi-provider federation
My likely long-term preference is to preserve multiple independent storage systems rather than consolidate everything into one universal vault.
Evaluate a model where Fleet manages:
A shared, explicitly authorized credential compartment synchronized among 1Password, BWS, and eventually OpenBao.
Separate provider-specific compartments containing credentials not automatically copied elsewhere.
Per-credential replication policies, preferred sources, consistency rules, and conflict resolution.
Selective encrypted local replicas and caches for resilience.
Dynamic or short-lived credentials that must not be naively synchronized as ordinary static passwords.
Bidirectional synchronization with OpenBao should be possible through Fleet's reconciliation framework, but we must recognize that replication expands the number of systems whose compromise can expose the shared secrets.
Preserve recoverability without collapsing all security boundaries. Make it possible to limit a particular machine, agent, service, or workload to only the credentials it needs.
Applications should be able to obtain secrets through the Fleet library, Fleet CLI, Fleet broker, or a suitable direct provider SDK, with common policy and configuration conventions.
9. Optimize Rust crate architecture and performance
Reconsider the crate boundaries and feature architecture in light of fnox, SecureStore, SOPS/age, and eventual OpenBao integration.
Favor narrowly scoped reusable crates with optional Cargo features, stable interfaces, and clear dependencies.
We need both an extremely fast, lightweight runtime secret-access path and a full administration environment supporting synchronization, PKI, credential generation, audits, and packaging.
Evaluate separate build profiles for minimal secret consumers, small agent binaries, normal Fleet clients, full administrative builds, and service/broker deployments.
Where practical, applications should be able to use only the required provider or algorithm feature without pulling every integration and its dependencies into the final binary.
Design for excellent latency and throughput: minimize subprocess creation, repeated authentication, redundant network round trips, reparsing, allocation, and unnecessary disk access.
Evaluate encrypted persistent caches, memory-only caches, single-flight retrieval, connection pooling, prefetch, batched retrieval, provider-health routing, and per-secret freshness policies.
Caching must account for rotation, revocation, stale credentials, security classifications, and offline operation. Do not silently use stale secrets where freshness is a security requirement.
Retain strong redaction, typed secret handling, structured observability, and reliable recovery from partial provider failures.
Keep high-level CLI behavior and Rust library behavior aligned. The same capabilities should work for humans, agents, scripts, services, and other Rust applications.
10. Execution priorities and deliverables
Keep the previous implementation order, with these refinements:
Immediate: Preserve BWS/1Password operation; benchmark both and the bw CLI; inspect fnox for reusable library functionality; extend Fleet's provider abstraction and caching; improve SOPS/age integration; implement and verify the approved root-key escrow policy.
Next: Integrate useful SecureStore functionality if warranted; improve provider federation, selective replication, performance and operational resilience; establish a synthetic OpenBao pilot and document the results.
Longer-term: Evaluate production OpenBao topology, Oracle and other hosting locations, Kubernetes integration, dynamic identities, PKI/SSH signing services, policy-driven secret distribution, and multi-site high availability.
Deliver source-backed research, architectural decisions, performance measurements, implementation changes, tests, examples, operational instructions, and a prioritized backlog of the work not implemented immediately.
Update the appropriate design documents, ADRs, AGENTS.md guidance, developer documentation, agent skills, and operational runbooks in the repository.
For each significant choice, distinguish what was measured, what was verified in source code or documentation, what remains speculative, and what requires an additional operational decision.
The ultimate objective is a very fast, secure, highly composable credential infrastructure that Fleet can administer centrally but applications can consume efficiently, even when individual hosted providers or network paths are unavailable.
The existing v0.3 scope remains in force. Do not let this research and infrastructure work displace the core PKI, credential-generation, synchronization, and documentation deliverables. Improve those systems through the shared abstractions wherever possible.

