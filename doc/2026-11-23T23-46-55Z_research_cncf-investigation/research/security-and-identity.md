# CNCF landscape — security, identity, and secrets

*Lane CNCF-SECURITY, 2026-11-23. Scope: the security/identity/secrets slice of the CNCF landscape, judged against our own estate. Every project gets a CNCF status label, a maturity verdict, and a separation between "what it is" and "should we use it." Statuses and versions were checked against CNCF project pages and current release reporting on the day of writing; where a status has changed recently, that is flagged, because several of these projects have moved up since older notes were written.*

## Status corrections first (things that moved)

- **cert-manager is graduated**, not incubating — it graduated 2024-09-29 ([CNCF project list](https://en.wikipedia.org/wiki/Cloud_Native_Computing_Foundation)).
- **Kyverno is graduated**, not incubating — it graduated 2026-03-16 ([CNCF project page](https://www.cncf.io/projects/kyverno/)).
- **OpenFGA is incubating**, not sandbox — promoted 2025-11-11 ([CNCF announcement coverage](https://www.mrvitej.in/devops/)).
- **Cartography is a CNCF sandbox project now** — it started at Lyft, applied to the CNCF sandbox in 2023 ([application](https://github.com/cncf/sandbox/issues/58)), and now lives under the `cartography-cncf` org ([overview](https://www.solancer.com/blog/your-cloud-is-a-graph)). The "Lyft, not CNCF" assumption is out of date.
- **OpenBao is not a CNCF project at all** — it sits in the OpenSSF (Linux Foundation) as a sandbox-stage project, having started at LF Edge and moved to OpenSSF in June 2025 ([project profile](https://www.builddaily.io/resources/openbao/)). It belongs in this survey anyway: it is the Vault replacement.

## Scoreboard

| Project | Status | One line | For our estate |
|---|---|---|---|
| OpenBao | OpenSSF sandbox (not CNCF) | Vault's MPL fork; same API, open governance | Not now; the broker covers us. Revisit for dynamic secrets at cluster scale |
| Consul (post-BSL) | Not CNCF; BSL | The discovery/mesh piece of the old Hashi stack | No fork emerged; in Kubernetes, discovery is native |
| Keycloak | CNCF incubating | The standard self-hosted identity provider | Competes with our IdP; beats it on breadth, loses on footprint and fit |
| SPIFFE / SPIRE | CNCF graduated | Workload identity: attested, short-lived SVIDs | Adopted the ID format already; SPIRE itself only with a real cluster, anchored to our CA |
| cert-manager | CNCF graduated | In-cluster certificate issuance and renewal | Yes, when Kubernetes lands — composing with, not replacing, fleet cert |
| OPA (+ Gatekeeper) | CNCF graduated | General policy engine (Rego) | Management plane only; Kyverno is the likelier in-cluster pick |
| Kyverno | CNCF graduated | Kubernetes-native policy in YAML | The default admission-policy candidate for our clusters |
| OpenFGA | CNCF incubating | Zanzibar-style relationship authorization | Nothing we run needs ReBAC yet |
| Notary Project / Notation (+ Ratify) | CNCF incubating (Ratify: sandbox) | X.509-based OCI artifact signing | The PKI-rooted signing option; cosign is the ecosystem default |
| Sigstore / cosign | OpenSSF (not CNCF) | Keyless, OIDC-identity artifact signing | The likely primary for our GitHub Actions pipelines |
| SOPS | CNCF sandbox | Encrypted files you can commit | Already ours; wider uses below, real limits stated |
| External Secrets Operator | CNCF sandbox | Syncs external vaults into Kubernetes Secrets | Yes — the bridge between our broker world and in-cluster apps |
| Cartography | CNCF sandbox | Read-only infrastructure asset graph (Neo4j) | Does not manage secrets; skip at our scale |
| ContainerSSH | CNCF sandbox | A fresh container per SSH connection | Exactly what Drew guessed; niche, watch only |
| Falco | CNCF graduated | Runtime threat detection from kernel events | Adopt when clusters are real; nothing of ours competes |
| Teleport | Not CNCF | Commercial access plane (SSH/K8s/database) | Overlaps our IdP + access story; no pull to adopt |
| in-toto / SLSA | in-toto: CNCF incubating | Provenance attestations for build pipelines | The vocabulary our signing story will speak |

---

## 1. After the Hashi stack

In August 2023 HashiCorp moved its products from MPL to the Business Source License; the community forks that matter are OpenTofu (Terraform — covered by the platform lane) and OpenBao (Vault). Vault's roadmap is now IBM's, following the HashiCorp acquisition ([comparison](https://techloghub.com/compare/openbao-vs-vault)).

### Consul — the piece with no fork

**What it is.** Consul did two jobs: service discovery (a registry + DNS/HTTP interface + health checks) and, later, a service mesh (Consul Connect). Unlike Vault and Terraform, **no community fork of Consul gained traction** — there is no "OpenConsul" with a ecosystem behind it.

**Should we use it.** The discovery job mostly evaporates in the world we're moving to: Kubernetes has native service discovery (Services + CoreDNS), and both candidate meshes consume it directly — Istio and Linkerd do their own discovery from the Kubernetes API rather than from a Consul registry. Outside Kubernetes, Consul-the-BSL-product still exists and people still run it, but building new on it means building on a BSL codebase whose open fork never happened. For our estate — NixOS hosts plus a coming Kubernetes — discovery is a solved-by-the-platform problem, and the mesh question (Istio vs Linkerd vs none) belongs to the network lane. Consul is the one Hashi product where the honest answer is "the replacement is not a fork; it's the platform absorbing the job."

### OpenBao — the Vault replacement

**Status: OpenSSF sandbox project under the Linux Foundation (not CNCF). Maturity verdict: real, shipping, and the credible open Vault.** Forked from Vault's last MPL-licensed code (the 1.14 line), started by IBM engineers at LF Edge in December 2023, first GA (2.0) in July 2024, moved to OpenSSF in June 2025. Current line is **v2.7.x** (v2.7.0 on 2026-09-23; v2.7.2 on 2026-10-26), MPL 2.0 ([release history](https://cloudparts.dev/technologies/openbao/), [comparison](https://www.openbao.ch/comparison/)).

**What it is.** A secrets manager server: encrypted storage, seal/unseal, auth methods (Kubernetes service-account tokens, OIDC, and more), path-based policies, audit log, and secrets engines — KV (v1/v2), dynamic database credentials, a PKI engine, transit encryption-as-a-service, SSH signing — with leases and revocation. The API is deliberately Vault-compatible: "existing Vault clients don't notice the difference," and most Kubernetes integrations work unchanged ([profile](https://www.builddaily.io/resources/openbao/)). Since the fork it has shipped features Vault reserves for Enterprise — namespaces (2.3, with per-namespace seals in 2.6), standby read-serving, and in 2.7 external HSM keys plus ML-DSA/ML-KEM post-quantum algorithms. Two honest limits: in-place migration is only tested from Vault Community 1.14.1, and running it is the same operational work as Vault — unsealing, storage cluster, backups, upgrades.

**Should we use it.** Not today. Our secrets path is a Bitwarden-backed broker (`fleet secrets get`, broker-first) plus SOPS for files — no server to unseal, no second storage cluster to babysit, and it already serves the fleet's shape: agents and CLIs fetching named secrets. OpenBao earns its keep where the broker model runs out: **dynamic secrets** (database credentials minted per-request with a TTL and revoked on lease expiry), **transit encryption** for applications, and **in-cluster auth at scale** (hundreds of pods authenticating by service account rather than a broker token). If the Kubernetes estate grows into needing any of those, OpenBao is the open-licence way to get them without returning to BSL Vault. One more composition worth noting: since 2.6 its PKI engine can issue certificates that are valid SPIFFE documents ([project talk summary](https://www.udienza.com/episodes/tony-openbao)) — a possible lightweight bridge to SPIFFE-shaped workload identity without deploying SPIRE (below).

---

## 2. Identity: people vs workloads

### Keycloak vs OpenBao — the one-liner

**Keycloak logs people in; OpenBao guards secrets. They are different layers, they are often run together, and neither substitutes for the other.** Keycloak is an identity provider: users, realms, login flows, MFA, federation to LDAP/AD and external IdPs, issuing OIDC/SAML tokens to applications. OpenBao is a vault: it stores and leases secrets to clients (human or workload) that authenticate to *it*. A common real deployment uses Keycloak as the OIDC auth method *into* OpenBao — Keycloak proves who the human is, OpenBao decides what secrets that identity may read. Asking "Keycloak or OpenBao?" is asking "door lock or safe?"

### Keycloak

**Status: CNCF incubating** (accepted 2023-04-10; originated at Red Hat, still Red Hat/IBM-stewarded) ([CNCF project page](https://www.cncf.io/projects/keycloak/)). **Maturity verdict: the default self-hosted IdP of the industry.** Current release 26.8.2 (2026-11-05), Apache 2.0, quarterly minors ([release check](https://startwithidentity.com/vendors/open-source/keycloak/)). OIDC, OAuth 2.0, and SAML 2.0; user federation (LDAP/AD), identity brokering, an admin console, a Kubernetes operator. It is a Java/Quarkus server backed by a relational database.

**Should we use it — honestly, against our own IdP.** This is the one place in this survey where a CNCF project **plainly beats something we built, on breadth**: our `pkgs/idp-provider` is a deliberately minimal provider (authorization code + PKCE, static client registry, RS256, append-only audit; federation doors to GitHub/Google/Microsoft built this month; SAML chosen as the samael library path). Keycloak has a decade of production hardening, every protocol corner (including full SAML IdP *and* SP), LDAP/AD federation we will never build, MFA machinery, and an operator. If we ever need to federate *in* arbitrary enterprise identity providers, manage many tenants' users, or hand a non-us admin console, Keycloak wins outright and it isn't close.

Where ours wins is fit and footprint: our provider idles at ~7 MB RSS with a ~5 MB binary (measured at its proof run), issues tokens carrying *our* account identity and role set, keeps its signing keys in *our* PKI's custody, and integrates with the id project's key model — none of which Keycloak can do without becoming a different product. The standing design decision (minimal provider first, fold path later) still holds. Verdict: **keep ours for our own sign-in; reach for Keycloak the day a workload needs a general-purpose, multi-tenant IdP** — and note the two compose, since our provider can federate to a Keycloak realm as one more door if that day comes.

### SPIFFE / SPIRE — workload identity

**Status: CNCF graduated** (SPIFFE and SPIRE both graduated August 2022) ([SPIFFE](https://www.cncf.io/projects/spiffe/), [SPIRE](https://www.cncf.io/projects/spire/)). **Maturity verdict: the established standard for workload identity; SPIRE is actively maintained (v1.15.x line through 2026).**

**What it is.** SPIFFE defines the identity format — a URI like `spiffe://trust-domain/workload` — and the SVID credential (an X.509 certificate or JWT) that carries it. SPIRE is the implementation: a server plus per-node agents that **attest** workloads (proving what a process is from platform evidence — Kubernetes service account, cloud instance metadata, process properties) and mint short-lived SVIDs against registration entries mapping selectors to identities. The point of the design is *secretless issuance*: a workload starts with nothing — no stored credential — and receives its identity by proving what it is, with certificates that live minutes-to-hours and rotate automatically ([architecture overview](https://www.mrvitej.in/devops/cncf-spire/)).

**Should we use it.** Drew already has the headline from our own design work, and it stands: **we adopted SPIFFE's ID format and its short-TTL/rotation discipline into the fleet design without deploying SPIRE**, and the earlier P2P/auth survey named attestation-based secretless issuance as the one thing the house stack genuinely lacks. Against our fleet CA specifically: our CA already won the layer it aims at — multi-algorithm roots (Ed25519, P-256, RSA-4096, ML-DSA-44, plus the P-384 IdP CA as interim primary), tested escrow and restoration, `fleet cert` issuance for hosts and services. SPIRE's server CA is deliberately a narrower thing — a per-trust-domain signing CA for workload SVIDs — and SPIRE is explicitly designed to anchor under an existing CA via its upstream-authority plugins (Vault and cert-manager are the stock examples). So the composition, if we ever need it, is **SPIRE in-cluster for pod identity, chained to our fleet CA as upstream root** — SPIRE adds attestation and per-workload granularity; our CA keeps root custody. Verdict: **no SPIRE now** (no production cluster, no workload-identity problem yet that broker tokens don't solve); **SPIRE is the pre-chosen answer the day in-cluster workloads need identity that isn't a stored secret**, and it slots under our CA rather than competing with it.

---

## 3. Certificates in the cluster

### cert-manager

**Status: CNCF graduated** (2024-09-29 — see the correction note at the top). **Maturity verdict: the unchallenged standard for certificate issuance inside Kubernetes**, with hundreds of contributors and a long release history ([graduation coverage](https://www.nhigovernance.com/frameworks/open-source-nhi-projects.html)).

**What it is.** A Kubernetes controller that watches `Certificate` resources and keeps them issued and renewed, talking to issuers: ACME (Let's Encrypt and friends, including DNS-01), a simple CA issuer, Vault/OpenBao, Venafi, and external issuers. It puts the resulting key pairs into Kubernetes Secrets where ingresses and workloads consume them.

**Should we use it.** Yes, when Kubernetes lands — and it **composes with fleet cert rather than competing with it**, because the scopes differ. Our fleet cert system is the estate PKI: host identity, the IdP hierarchy, cross-algorithm roots, escrow — issuance *to machines and services we operate*, driven by `fleet cert`. cert-manager's scope is *in-cluster* issuance ergonomics: per-namespace certificates, automatic renewal, ACME for anything public-facing. The natural wiring is cert-manager in the cluster with an issuer that chains to our CA (directly via a CA issuer holding an intermediate, or via an external issuer against our PKI), so cluster workloads get frictionless certs whose trust anchors are still ours. Running cert-manager with public ACME for internal services, or trying to make `fleet cert` do per-pod renewal, would each be using a tool outside its scope.

---

## 4. Policy and authorization

### Open Policy Agent (OPA) + Gatekeeper

**Status: CNCF graduated** (2021-02-04) ([graduation announcement](https://www.cncf.io/announcements/2021/02/04/cloud-native-computing-foundation-announces-open-policy-agent-graduation/)). **Maturity verdict: the general-purpose policy engine of the ecosystem.** (Governance note: OPA's creators and much of the Styra team joined Apple in August 2025; the project states governance and licensing are unchanged.)

**What it is.** A policy engine decoupled from the services it governs: you send JSON in, it evaluates rules written in Rego, you get a decision out. In Kubernetes its face is **Gatekeeper**, an admission webhook that expresses policies as `ConstraintTemplate`/`Constraint` custom resources and can also audit existing resources. Drew already has my standing take, which this survey confirms rather than revises: **OPA is a management-plane tool — it decides what an operator will admit, issue, or accept; it is not a capability system** and doesn't represent delegatable authority (that's the Meadowcap/Keyline/UCAN layer in our design work).

**Should we use it.** The real in-cluster question is OPA-vs-Kyverno, and the honest summary of the field's experience: choose Gatekeeper if you want one policy language (Rego) reused across Kubernetes, Terraform plans, CI, and API authorization; choose Kyverno if your scope is Kubernetes only ([comparison](https://www.qovery.com/blog/opa-gatekeeper-kyverno-platform-layer-deployment-policy)). For our clusters — small, Nix/GitOps-driven, no existing Rego investment — **Kyverno is the likelier fit** (below). OPA-the-library may still show up inside our own tooling someday for operator-side admission decisions (the management-plane role from the Cedar/OPA analysis), but that's an embedding decision, not a cluster dependency.

### Kyverno

**Status: CNCF graduated** (2026-03-16 — see the correction note at the top) ([CNCF project page](https://www.cncf.io/projects/kyverno/)). **Maturity verdict: graduated, widely deployed, and the Kubernetes-native policy default.**

**What it is.** An admission controller whose policies are themselves Kubernetes resources written in YAML — no new language. It validates, mutates, generates, and cleans up resources: require image signatures, inject defaults, forbid privileged pods, generate NetworkPolicies per namespace. Where Gatekeeper's power is Rego's generality, Kyverno's is that a platform engineer who can read a manifest can read the policy.

**Should we use it.** It is the front-runner for our clusters' admission policy, including being the enforcement point for image-signature verification (it verifies cosign/Notation signatures natively at admission). One graduated project, YAML all the way down, GitOps-friendly — it matches how we already work.

### OpenFGA

**Status: CNCF incubating** (promoted 2025-11-11; created at Okta/Auth0). **Maturity verdict: the reference open implementation of Zanzibar-style authorization, with dozens of production adopters.**

**What it is.** A relationship-based access control (ReBAC) server. You model authorization as relationships — `user:anne` is `editor` of `document:spec`, `document:spec` is in `folder:designs`, editors of a folder inherit its documents — and applications ask the server "can this subject do this relation on this object?" over HTTP/gRPC, backed by Postgres/MySQL/SQLite. This is Google Drive-style sharing semantics: permissions that flow through *relationships between objects*, which neither RBAC roles nor OPA-style attribute rules express comfortably.

**How it differs from its neighbours.** OPA answers "does this request satisfy the operator's rules?" (policy evaluation over attributes/context). Capability systems answer "does this holder present an unforgeable, attenuated grant?" (authority as a cryptographic object, verifiable offline). OpenFGA answers "what does the relationship graph say this subject may do to this object?" — a *centralized lookup* over a stored graph, which means it needs the server (or a replica) in the request path and has no offline/delegation story at all.

**Should we use it.** Not now: nothing we run has Drive-style object-sharing semantics. The plausible future home is the id project's worlds/apps layer — if worlds ever grow shared objects with inherited, per-object permissions granted user-to-user, ReBAC is the right model and OpenFGA is the obvious implementation. Until that requirement exists, adding a stateful authorization server would be buying a layer we don't need; our roles-plus-capabilities design covers what we actually have.

---

## 5. Secrets delivery: files, clusters, and the broker

### SOPS — wider than "a few secrets in a repo"

**Status: CNCF sandbox** (donated by Mozilla in 2023 to the `getsops` maintainers; MPL 2.0; current releases in the v3.13.x line) ([README](https://raw.githubusercontent.com/getsops/sops/main/README.rst), [profile](https://www.mrvitej.in/devops/cncf-sops/)).

**What it is.** An editor for encrypted files: it encrypts the *values* in YAML/JSON/ENV/INI/binary files while leaving keys readable, so the ciphertext can live in git and diffs stay reviewable. The per-file data key is wrapped for one or more recipients — age keys, PGP keys, or cloud KMS (AWS/GCP/Azure/Huawei) — and anyone holding a matching private key (or KMS grant) can decrypt.

**Drew's real question — what else is it for, at scale?** Quite a lot, and we already lean on some of it:

- **Whole encrypted configuration, not just secrets.** Anything in a config file can be encrypted — the standard pattern is encrypting the sensitive values and leaving structure visible, but SOPS happily encrypts entire files, so "config that is 90% non-secret" stops being a reason to keep it out of git.
- **GitOps decryption.** Flux decrypts SOPS files natively in-cluster; Argo CD does it via plugins (KSOPS and friends). This is the standard way GitOps pipelines carry secrets: the repo is the distribution mechanism, the cluster's KMS/age key is the only decryption point.
- **Per-environment, per-team key topology.** Different files wrapped to different recipient sets (prod KMS key for prod files, a team age key for dev), and `.sops.yaml` creation rules that pick recipients by path — so "who can decrypt what" is itself versioned policy in the repo.
- **Rotation as re-encryption.** `sops rotate` re-wraps data keys to a new recipient set; removing a departed member's key is a concrete, auditable operation rather than a hope.
- **CI as a decryptor.** Pipelines hold one age key or KMS grant and materialize plaintext only in-memory/at deploy time.

**Its limits, plainly.** SOPS is encryption at rest for files — it has **no access control at read time** (anyone with a recipient key decrypts everything wrapped to it, forever, including old git history), **no audit log** of who decrypted what, **no dynamic secrets** (nothing is minted per-use or revoked on a lease), and ciphertext metadata (keys, structure, file paths) is visible to anyone with repo read access. Revoking a person means rotating every file they could decrypt *and* the underlying secrets in the files that matter.

**Where the broker/Vault still wins.** Runtime delivery to agents and services (our broker: named fetch, cache, no plaintext ever in a repo), anything needing per-access audit, short-lived minted credentials, and secrets whose compromise must be revocable without a repo-wide rotation. The settled division of labour: **SOPS for config-shaped secrets that travel through git; the broker for runtime-shaped secrets fetched by machines; OpenBao only if dynamic secrets enter the picture.** That division is already how we operate — the wider SOPS uses above (whole-file encryption, per-path recipient rules, GitOps decryption when clusters land) are the growth path, not a change of direction.

### External Secrets Operator (ESO)

**Status: CNCF sandbox** (accepted 2022-07-26; incubation applied for in 2024 and still pending; v1.0 shipped 2025-11, v2.0 in 2026-02 — with a maintainer-capacity pause in 2025 that is worth knowing about) ([CNCF project page](https://www.cncf.io/projects/external-secrets/), [profile](https://www.mrvitej.in/devops/cncf-external-secrets/)).

**What it is.** A Kubernetes operator that reads secrets from an external store (Vault/OpenBao, cloud secret managers, and dozens of other providers, including generic webhooks) and materializes them as ordinary Kubernetes Secrets on a refresh interval, driven by `ExternalSecret`/`SecretStore` resources.

**Should we use it.** Yes — this is the designated bridge between our two worlds. When cluster workloads need secrets whose home is our broker/Bitwarden side, the alternatives are (a) ESO with a provider/webhook pointing at our source of truth, or (b) a homegrown sync script we'd have to secure and maintain. ESO is the ecosystem's standard answer, its APIs are stable, and its known weakness (a thin maintainer bench, sandbox status) is a watch item rather than a blocker. It also composes with policy: ESO resources are exactly the kind of thing Kyverno/Gatekeeper should guard, since a malicious `ExternalSecret` is an exfiltration path.

---

## 6. Artifact trust: signing what we ship

### Notary Project / Notation (+ Ratify)

**Status: Notary Project is CNCF incubating; its verifier companion Ratify is CNCF sandbox** (accepted 2024-08-30; Ratify v1.4.x current, v2 in development). Notation, the signing CLI, is at v1.3.x ([project profile](https://www.mrvitej.in/devops/cncf-notary-ratify/)).

**What it is.** The rebuilt successor to Docker's Notary: specifications and tools for signing OCI artifacts with **X.509 certificates from a CA you control**, storing signatures in the registry as OCI artifacts (via the Referrers API), and verifying against an explicit trust policy — which CAs and identities are trusted for which registries. Signing keys typically live in a KMS/HSM behind plugins. Ratify is the in-cluster half: a verification engine that admission (via Gatekeeper, or Kyverno) consults before a pod runs.

### Sigstore / cosign

**Status: not CNCF — Sigstore is an OpenSSF project** under the Linux Foundation. **Maturity verdict: the ecosystem's default for open-source artifact signing.**

**What it is.** cosign signs OCI artifacts (and blobs, and attestations) with the headline trick of **keyless signing**: the signer authenticates with an OIDC identity (a GitHub Actions workflow, say), Sigstore's Fulcio CA issues a minutes-lived certificate binding a throwaway key to that identity, cosign signs, and the event is recorded in Rekor, a public transparency log. No long-lived signing key exists to steal or rotate. Key-based and KMS-backed modes also exist.

**Which did the ecosystem actually adopt?** Both survived, along trust-model lines, and they now store signatures the same way (OCI Referrers), so the choice is about where trust is rooted ([comparison](https://www.encryptionconsulting.com/signing-containers-and-oci-artifacts/), [trust-model analysis](https://www.devopstraininginstitute.com/blog/what-makes-artifact-signing-crucial-in-supply-chain-security)). cosign won the open-source and CI-native world — GitHub's own artifact attestations are Sigstore-based, and keyless removed the reason most projects never signed at all. Notation/Notary holds the enterprise-PKI world — organizations that already run a CA, need signing metadata kept private (no public transparency log), or sit in the Microsoft/AWS orbit, where Ratify is the pushed verifier. They are not mutually exclusive; some pipelines sign with both.

**For our estate.** Our CI is GitHub Actions and our IdP now exists — the natural primary is **cosign keyless**, with signature verification enforced at admission by Kyverno. The interesting second option is Notation anchored to *our own fleet CA*: we are one of the few estates that actually operates a proper PKI, so the PKI-rooted model costs us less than it costs most. Decide when the first cluster enforces admission; the registry-side mechanics are identical either way. One warning both camps share: a signature nothing verifies is decoration — enforcement at admission, in enforce (not audit) mode, is the actual control.

### in-toto / SLSA, in one paragraph

in-toto (CNCF incubating) is the framework for **provenance attestations**: signed statements, in a standard predicate format, recording what a build step did — which sources, which builder, which outputs. SLSA is the supply-chain framework (levels, requirements) that consumes such attestations. cosign signs and verifies in-toto attestations natively, which is how "this image was built by this workflow from this commit" becomes a checkable admission input rather than a claim. For us this is vocabulary to adopt with the signing work, not a separate system to run.

---

## 7. Visibility and access

### Cartography — first, the correction

**Drew asked whether Cartography manages his secrets. It does not — it manages no secrets and stores none.** (And the framing "Lyft, not CNCF" is itself now out of date: it is a **CNCF sandbox** project, moved from Lyft to the `cartography-cncf` org.)

**What it is.** A read-only **infrastructure asset graph**: sync jobs pull metadata from cloud accounts, Kubernetes, identity providers, GitHub, and dozens of other sources into a Neo4j graph — assets as nodes, relationships as edges — so security questions become graph queries: "which internet-facing instances assume a role that can reach this database?", "show me the attack path from this exposed key to production data" ([project description](https://github.com/cncf/sandbox/issues/58), [walkthrough](https://www.solancer.com/blog/your-cloud-is-a-graph)). It is a *snapshot* tool (batch sync, stale between runs), it writes nothing back, and it exists to expose hidden dependency and privilege relationships — the security/compliance value is *visibility*, not control.

**Should we use it.** No, at our scale. Cartography pays off when an estate is too large and multi-cloud for anyone to hold in their head — hundreds of accounts, inherited IAM spaghetti. Our estate is a handful of hosts, one operator, and infrastructure-as-code that already *is* the map (the Nix config and the fleet's own records say what exists and why). The honest overlap: our Atlas project is the same instinct — the estate as a queryable graph — applied to our own knowledge rather than cloud APIs. If we ever inherit a sprawling multi-account cloud footprint, Cartography is the tool; until then it would be a Neo4j instance maintained to tell us what our Nix files already say.

### Falco

**Status: CNCF graduated** (2024-02-29) ([graduation announcement](https://www.cncf.io/announcements/2024/02/29/cloud-native-computing-foundation-announces-falco-graduation/)). **Maturity verdict: the de facto Kubernetes runtime threat-detection engine** (created by Sysdig; the commercial Sysdig platform is built on it).

**What it is.** A per-node sensor that consumes kernel events (syscalls, via eBPF or a kernel module), enriches them with container/Kubernetes metadata, and evaluates rules in real time: a shell spawned in a production container, a read of `/etc/shadow`, a crypto miner starting, a sensitive mount appearing. A plugin system extends the same rule engine to cloud audit logs (AWS CloudTrail, GitHub, Okta event sources). Falco *detects and alerts*; it does not block (enforcement is a different layer — admission policy before the fact, or LSM-based tools during).

**Should we use it.** When we run a real cluster, yes — it is the standard answer to "how would we know a workload is misbehaving at runtime," and nothing we have built competes with it (our monitoring is quota/queue-shaped, not syscall-shaped). It earns its place on the day one of our clusters runs workloads we'd want watched; before that day it's a DaemonSet with nothing to watch.

### ContainerSSH

**Status: CNCF sandbox. Maturity verdict: small, focused, still shipping (v0.6.0, 2026-03), with a narrow but real niche.**

**What it is — and Drew's guess is exactly right.** ContainerSSH is an SSH server that **launches a fresh container for each incoming SSH connection** (Kubernetes, Podman, or Docker backends), drops the user transparently into it, and removes the container when they disconnect ([CNCF project page](https://www.cncf.io/projects/containerssh/)). There are no system users: on each connection it calls an **authentication webhook** (your service decides whether this password/key is valid, and for whom) and a **configuration webhook** (which can return a per-user container spec — image, limits, mounts), so identity and environment are decided dynamically, per connection ([design walkthrough](https://www.systemshardening.com/articles/linux/containerssh-bastion-replacement/)). Audit logging of sessions is built in.

**Should we use it.** Only if a specific shape appears: giving humans or students shell access to *ephemeral* environments (labs, debugging sandboxes, honeypots), or a bastion replacement where nobody should ever land on a real host. Our access story is different — fleet-managed hosts, our own IdP, agent-driven operations — and the webhook *is* the security boundary in ContainerSSH, meaning we'd be building the critical piece ourselves anyway. File under "know it exists"; the dynamic-launch trick is the takeaway, and it answers Drew's second question directly: yes, dynamically launching an image per SSH login is precisely its design.

### Teleport, briefly

Not a CNCF project (commercial open-core, by Teleport the company). It is an access plane — SSH, Kubernetes, database, and web-app access behind one identity-aware proxy with session recording. It overlaps the *human access* side of our IdP story and the bastion side of ContainerSSH, with a much larger operational footprint than either. It surfaces in every survey of this space, so it is named here; nothing in our estate pulls toward adopting it while our own IdP + fleet access paths exist.

---

## 8. The bottom line: who beats whom

**Where a CNCF-ecosystem project plainly beats what we built**

- **Keycloak vs our IdP, on breadth**: protocols, federation-in, admin UX, hardening years. Ours wins only on fit and footprint — which is the reason it exists, but the gap should be stated, not smoothed.
- **cert-manager vs any in-cluster use of fleet cert**: per-namespace issuance and renewal ergonomics inside Kubernetes are a solved problem; we should consume the solution.
- **ESO vs a homegrown secret-sync**: if the thought "we'll write a small syncer" ever occurs, ESO is the correct answer instead.
- **Falco vs our monitoring, for runtime detection**: different sense organs entirely; theirs exist and ours don't.

**Where ours already won**

- **Fleet CA vs SPIRE-as-CA and vs OpenBao's PKI engine**: for estate/host PKI — multi-algorithm roots, escrow with tested restoration, cross-signing design — our build is ahead of what either would give us, and both are designed to anchor *under* a CA like ours rather than replace it.
- **Secrets broker vs OpenBao, at our current shape**: no server to unseal or cluster to operate, Bitwarden as the backing store, and a fetch model matched to agents and CLIs. OpenBao's advantages (dynamic secrets, transit, in-cluster auth at scale) are real but are answers to problems we don't yet have.
- **Our IdP vs Keycloak, on the one axis that motivated it**: identity bound to our own keys, roles, and PKI custody, at ~7 MB RSS instead of a JVM estate.

**Adoption order when Kubernetes actually lands**

1. cert-manager (issuance), chained to our CA.
2. Kyverno (admission policy + signature enforcement).
3. cosign keyless signing in CI (Notation-under-our-CA as the considered alternative), with in-toto/SLSA provenance as the vocabulary.
4. ESO for broker→cluster secret delivery.
5. Falco once workloads are worth watching.
6. SPIRE only when stored-secret identity for workloads becomes the actual problem — anchored to fleet CA.
7. OpenBao only when dynamic secrets become the actual problem.
8. OpenFGA only if worlds/apps grow relationship-shaped sharing. Cartography and ContainerSSH: no trigger in sight.

*Projects covered: 17 (OpenBao, Consul, Keycloak, SPIFFE/SPIRE, cert-manager, OPA/Gatekeeper, Kyverno, OpenFGA, Notary/Notation/Ratify, Sigstore/cosign, SOPS, External Secrets Operator, Cartography, Falco, ContainerSSH, Teleport, in-toto/SLSA).*
