# Secrets stack analysis — attestation-based issuance, SOPS scope, and the cluster overlap map

*Lane SECRETS-STACK-ANALYSIS, 2026-11-24. Answers Drew's questions of 2026-11-23 ~19:34 CST. Sources: the CNCF security & identity survey (2026-11-23, `doc/2026-11-23T22-33-41Z_research_cncf-landscape/`), the secrets-plane design #13948 and its parent problem #13893, the fleet CA / credentials programme #9155, and the IdP story #13658. Secrets are referenced by name only throughout; no values appear anywhere in this record.*

## Verdicts up front

| Question | Answer |
|---|---|
| What is attestation-based secretless issuance, and should we build or adopt it? | A workload proves what it is from platform evidence and is minted a short-lived credential (a SPIFFE SVID); no bootstrap secret exists anywhere. **Adopt SPIRE, chained under the fleet CA, when the first real cluster workload-identity need lands — do not build our own attestation.** Until then, no new system. |
| Should all secrets management move to SOPS? | **No.** SOPS is the right home for config-shaped, deploy-time secrets (including in the public repo, under the conditions in §2.3). It has no runtime-delivery, per-access audit, dynamic-minting, or real revocation story, so it cannot replace the broker/stores design in #13948. The two compose: disjoint sets of secrets, one system of record per secret. |
| Keycloak + cert-manager + External Secrets Operator — does OpenBao still have a job? | The three cover people-login, in-cluster certificates, and secret *delivery* respectively. None of them mints dynamic secrets, offers transit encryption, or leases/revokes credentials — that remainder is OpenBao's alone. **Posture: Keycloak skip, cert-manager adopt at cluster time, ESO adopt at cluster time, OpenBao defer until a dynamic-secrets need is real.** |

---

## 1. Attestation-based secretless issuance

### 1.1 What it is

Drew's sketch — "a cert to every container launch" — is essentially correct. The full shape:

In the stored-secret model, a workload proves its identity by *presenting* something it was given earlier: a token in an environment variable, a key file baked into an image, a Kubernetes Secret mounted into the pod. That thing can be copied, leaked, and replayed from anywhere, and it usually lives for months.

In the attestation model, the workload presents **nothing**. Instead, a trusted local agent inspects *evidence about the workload that the workload cannot forge* and vouches for it to an issuing server:

1. **Node attestation** — the agent first proves which machine it runs on. Evidence types: a Kubernetes projected service-account token (the `k8s_psat` attestor), a cloud instance identity document (AWS/GCP/Azure metadata, verified against the provider), a TPM / DevID certificate, or an SSH host identity. (A pre-shared *join token* also exists as an attestor — and using it reintroduces exactly the bootstrap secret the design exists to eliminate.)
2. **Workload attestation** — when a process asks the agent for an identity (over a local Unix socket, the SPIFFE Workload API — the caller presents no credential at all), the agent derives **selectors** from the caller's process via the kernel, kubelet, and container runtime: Kubernetes namespace and service account, container image digest, Unix UID, systemd unit name, process path, and similar.
3. **Registration entries** — the server holds policy mapping selector sets to a SPIFFE ID (e.g. `spiffe://<trust-domain>/ns/<namespace>/sa/<service-account>`). If the attested selectors match an entry, the server signs an **SVID** — an X.509 certificate (or JWT) carrying that ID.
4. **Lifetime and rotation** — SPIRE's defaults are a 1-hour X.509 SVID and a 5-minute JWT SVID, rotated automatically at half-life by the agent, with no restart and no human. There is no CRL-based revocation story; **revocation is achieved by deregistering the entry** — the workload simply stops receiving fresh SVIDs and its existing one expires within the hour.
5. **Verification** — peers validate the SVID against the trust domain's bundle and authorise on the SPIFFE ID itself.

So the bootstrap secret is not moved or better hidden; it is *deleted*, and the root of trust becomes "the platform's own account of what is running where."

### 1.2 Honest limits

- **Attestation proves environment, not intent or code quality.** A compromised workload running in the right namespace with the right service account attests perfectly. Selectors are the security policy: an entry keyed only on namespace is a much weaker claim than one keyed on namespace + service account + image digest, and writing entries by hand drifts — in Kubernetes the entries should be generated (SPIRE Controller Manager / `ClusterSPIFFEID` templates), not hand-maintained.
- **Node trust is load-bearing.** The SPIRE documentation itself warns that workload attestation is only a reliable signal for *unprivileged* workloads: root on the node can manufacture the evidence. Attestation raises the bar from "stole a file" to "owns the node" — a real improvement, not a proof against node compromise.
- **The bootstrap problem moves, it does not vanish.** It concentrates in node attestation: if node evidence is weak (join tokens, spoofable metadata), the whole chain inherits the weakness.
- **An SVID is identity, not a secret vault.** It gets a workload *authenticated*; turning that into data-plane secrets still requires an exchange step — a vault/broker auth method that accepts the SVID and returns the actual secret. Attestation replaces the credential a workload *starts* with, not the secrets it ultimately *uses*.
- **It is infrastructure.** A SPIRE server (HA if it matters), an agent on every node, entry lifecycle, bundle distribution. Small, but not free, and idle weight until workloads actually consume SVIDs.

### 1.3 Our fit

The CNCF survey's framing stands: **the fleet CA enrols machines; workloads are a different subject, and attestation is the one genuine gap in the house stack.** Our estate PKI is ahead of the alternatives for what it aims at — multi-algorithm roots, tested escrow/restoration, `fleet cert` issuance for hosts and services (#9155) — but a host certificate answers "which machine is this," not "which process, in which pod, running which image, is calling right now."

Two ways to close the gap:

**Option A — SPIRE chained under the fleet CA (recommended).** SPIRE is explicitly designed to anchor under an existing CA: its UpstreamAuthority plugin obtains the SPIRE server's intermediate signing certificate from an upstream PKI (stock plugins include a CA loaded from disk, AWS PCA, GCP CAS, Vault PKI, and cert-manager — so our CA can serve directly via a disk-loaded intermediate, or indirectly through a cert-manager Issuer that chains to it). Root custody, escrow, and algorithm policy stay exactly where #9155 put them; SPIRE adds only the attestation and per-workload granularity it is good at. SPIRE also is not Kubernetes-only — agents run on bare hosts/VMs with systemd and Unix selectors, which suits our NixOS fleet shape.

**Option B — build attestation into our own issuance.** Extend `fleet cert` / the broker with selector collection and entry policy. Pros: no new system, reuses fleet enrolment, NixOS/systemd evidence is natural on our hosts. Cons: attestation is security-critical code where the failure mode is *subtle forgery acceptance*, the selector ecosystem (kubelet, runtimes, cloud metadata) is a moving target that SPIRE's maintainers track full-time, and every hour spent here is an hour not spent on the fleet's actual differentiators. We would own the risk forever to save running one well-understood daemon set later.

**Recommendation: Option A, on a trigger, not today.** There is no production cluster and no workload today whose identity problem a broker token doesn't solve; deploying SPIRE now would be idle infrastructure. The day in-cluster (or multi-process host) workloads need identity that isn't a stored secret, SPIRE is the pre-chosen answer, chained under the fleet CA. One cheap interim exists if a single bridge is ever needed before that day: OpenBao's PKI engine (since 2.6) can issue SPIFFE-shaped documents — noted in the CNCF survey — but that is a stopgap, not the plan.

---

## 2. Should all secrets management move to SOPS?

### 2.1 What SOPS actually is, mechanically

SOPS encrypts the *values* in structured files (YAML/JSON/ENV/INI/binary), leaving keys and structure readable so diffs stay reviewable. Each file gets a random data key; values are encrypted with it (AES-256-GCM, with a MAC over the file); the data key is wrapped once per **recipient** — an age public key, a PGP key, or a cloud KMS key. Anyone holding a matching private key (or KMS grant) decrypts. `.sops.yaml` creation rules choose recipient sets by path, so "who can decrypt what" is itself versioned policy.

### 2.2 What it covers well — and what it structurally does not

**Covers well (config-shaped secrets at rest):**

- Secrets that travel *with configuration through git*: NixOS host configuration (the sops-nix pattern — decrypt at activation, per-host age keys generated on the host), GitOps payloads (Flux decrypts SOPS natively in-cluster), CI materialisation (one age key or KMS grant in the pipeline, plaintext only in memory/at deploy).
- Whole-file encryption when a config is mostly sensitive; per-environment/per-team recipient topology; rotation of the *recipient set* as a concrete re-wrap operation (`sops rotate`).

**Does not cover (and no file format can):**

- **Runtime delivery.** A running process that needs a secret *by name, now, possibly on a machine with no repo checkout* gets nothing from SOPS. The broker's whole shape — named fetch, tiered cache, no plaintext in any repo — exists for this case.
- **Rotation without redeploy.** Changing a SOPS-held value means edit, commit, push, redeploy/re-activate. Changing a broker-held value is a `put` (#13948 §5.5) and the next fetch sees it.
- **Revocation.** Git history is permanent and replicated to every clone. Removing a recipient re-wraps *current* files; every historical version remains decryptable by the old key, and the *underlying secret values* in those versions are unchanged — real revocation still means rotating the secrets themselves, file by file. A broker/store revokes centrally and immediately.
- **Per-access audit.** SOPS has no record of who decrypted what, ever. The broker and stores log access by name.
- **Dynamic secrets.** Nothing is minted per-use with a TTL and a lease; a SOPS value is a static string until a human changes it.
- **Per-workload identity.** A recipient key decrypts *everything wrapped to it* — granularity is per-file recipient sets, never per-process. (This is exactly the gap §1's attestation addresses, from the other side.)

**Where the key-custody problem moves:** from store accounts to age private keys — one per host/person, each needing escrow and a tested restore. Custody doesn't shrink; it changes shape. Losing an age key loses everything wrapped to it; leaking one opens everything ever wrapped to it, history included.

### 2.3 The repo question, head-on

**Is SOPS ciphertext safe in a public repo?** The cryptography, yes — AES-256-GCM values under age/KMS wrapping is not the weak point, and public SOPS stores are a real, working pattern in the wild (one documented fleet ran its whole SOPS store in a public repository; the failure that eventually moved it private was a *recipient-set* defect — one catch-all rule wrapping six identities' secrets to all six keys — not a cryptanalytic one; see supplements). But "the ciphertext is strong" is not the whole answer:

- **Metadata leaks by design.** Key names, file paths, structure, which values changed and when, ciphertext sizes, and the recipient public keys (which identify hosts and people) are all public. That is an inventory of what secrets exist and how the estate is shaped.
- **History is forever, publicly archived.** Public ciphertext is copied by mirrors, forks, and crawlers beyond any later deletion. This creates a *harvest-now, decrypt-later* exposure: any future compromise of a recipient private key retroactively opens every historical version wrapped to it. In a private repo, a leaked key opens the current set; in a public repo, it opens the *archaeology*.
- **Blast radius concentrates.** One leaked host age key + a public repo = every value ever wrapped to that host, no further access needed. The same leak against the broker model yields a token that can be revoked and an access log to inspect.
- **Operational asymmetry.** A single plaintext mistake in a public repo is scraped within minutes and is un-recallable. (True in private repos too — but the audience is whoever holds repo access, not the internet.)

**Vs a private secrets repo:** Drew's stated costs are real and we weigh them as stated — a second repo to keep in step, and private flake inputs make Nix builds harder (builders and CI need credentials to fetch the input, which is itself a secret-distribution problem, and public reproducibility suffers). Mitigations exist (a private repo consumed as a non-flake path input; sops-nix reading from outside the flake), but each is friction purchased to protect *metadata and history*, not values.

**The hybrid shape (recommended):** the question "public or private repo?" only has to be answered for the class of secrets SOPS is actually suited to, and for that class the public repo is acceptable *under discipline*:

- **Tier A — crown jewels and runtime secrets: never SOPS-only, in any repo.** CA private keys, broker/store tokens, and anything needing revocation, audit, or rotation-without-redeploy stay in the #13948 world: Bitwarden Secrets primary, 1Password mirror, broker delivery, name-only references in git. (This is already how CA material is held under #9155.)
- **Tier B — config-shaped, deploy-time secrets: SOPS, in the same (public) repo.** Conditions: one file per recipient set (a file's name effectively states who can open it — the documented lesson from the public-store case above); per-host age keys generated on the host; values individually low-to-medium sensitivity and individually rotatable; no single catch-all creation rule.
- **Tier C — cluster-bound secrets:** delivered by ESO from the Tier A stores (§3), or SOPS-via-Flux for Tier B material — chosen per cluster, not globally.

### 2.4 Composition with #13948 — the actual recommendation

**Do not move all secrets management to SOPS. The two systems govern disjoint sets, with one system of record per secret:**

- #13948 (delta refresh, peer sync, dual-store `put`) is the design for **runtime-shaped** secrets — fetched by machines, by name, with audit and revocation. SOPS changes nothing in it; it neither conflicts with nor supersedes any of its six slices.
- SOPS is the design for **config-shaped** secrets — travelling with the configuration that consumes them, decrypted at activation/apply time. The broker changes nothing in it.
- The one rule needed at the seam: a given secret has exactly **one** home (store/broker *or* a named SOPS file), recorded where #9158's canonical secrets model / #10358's scoping work already decides which store and namespace a name resolves to. If a Tier B value later needs runtime fetch or fast revocation, it is *promoted* — rotated into the store and removed from the file (accepting that history retains the old, now-dead value). The reverse demotion should not happen: once a secret has needed audit/revocation, it doesn't go back to a file.

Arguing the replacement case honestly: moving everything to SOPS would delete the broker's operating cost — and with it, per-access audit, central revocation, rotation without redeploy, dynamic credentials if they ever arrive, and the entire #13948 sync design's reason to exist, in exchange for making git the distribution mechanism for secrets git was never able to un-distribute. That trade is wrong for Tier A at any repo visibility, and unnecessary for Tier B, which SOPS already serves.

---

## 3. The cluster stack overlap map: Keycloak, cert-manager, ESO, OpenBao

The four are often named in one breath because all four "do security in Kubernetes." They sit at four different layers, and the overlaps are narrower than the naming suggests.

| Component | Layer / what it actually does | Overlaps with | Unique remainder |
|---|---|---|---|
| **Keycloak** | **People identity.** Users, realms, login flows, MFA, federation to LDAP/AD and external IdPs; issues OIDC/SAML tokens to applications. | **Our own IdP** (#13658 / #9077 programme) — near-total overlap *for our use*. (Not with any vault: it stores no workload secrets.) | Federation-*in* of arbitrary enterprise IdPs, multi-tenant user administration, a decade of protocol corners (full SAML IdP *and* SP, LDAP/AD). |
| **cert-manager** | **In-cluster certificate issuance.** Watches `Certificate` resources; issues/renews from ACME, CA, Vault/OpenBao, and external issuers; lands key pairs in Kubernetes Secrets. | **Fleet CA** at the single point "issues certificates" — scopes differ: estate/host PKI vs per-namespace in-cluster ergonomics. | Nothing conceptual — its value is being the standard, maintained implementation of in-cluster issuance/renewal. |
| **External Secrets Operator** | **Secret delivery shim.** Reads from an external store (Vault/OpenBao, cloud secret managers, generic webhooks) and materialises ordinary Kubernetes Secrets on a refresh interval. | Nothing above it: it *consumes* stores, it is not one. Its backend can be our broker (webhook provider), Bitwarden/1Password-side stores, or OpenBao. | The standard, maintained bridge from "secrets live outside the cluster" to "apps consume Kubernetes Secrets," with templating and refresh. |
| **OpenBao** | **Vault server.** KV store, **dynamic secrets** (database credentials minted per-request, TTL'd, revoked on lease expiry), **transit encryption-as-a-service**, its own PKI engine, auth methods (Kubernetes SA, OIDC, …), leases and audit. | KV overlaps the broker/stores *functionally* (a place secrets live); PKI overlaps the fleet CA (and can sit under it); its OIDC auth method *consumes* an IdP (ours or Keycloak) — it does not replace one. | **Dynamic secrets, transit encryption, lease-based revocation.** None of the other three mints a credential that did not exist before the request, or encrypts application data as a service. |

### 3.1 Posture per component

- **Keycloak — SKIP (keep our IdP).** What it would do for us, stated plainly: nothing our provider doesn't already do for our own sign-in, at the price of a JVM estate and a relational database, and at the cost of the properties our IdP exists for (identity bound to our own PKI custody and role model, ~7 MB footprint). Its genuine wins are breadth we don't currently need. **Trigger to revisit:** federating *in* arbitrary external/enterprise identity providers, multi-tenant user administration for other people's users, or handing administration to non-us operators. The two also compose — our provider can federate to a Keycloak realm as one more door — so skipping now forecloses nothing.
- **cert-manager — ADOPT when Kubernetes lands, chained to our CA.** Via a CA Issuer holding a fleet-CA intermediate (or an external issuer against our PKI), cluster workloads get frictionless issuance whose trust anchors remain ours. It composes with, and does not replace, `fleet cert` — exactly as the CNCF survey concluded.
- **ESO — ADOPT when cluster workloads need Tier A secrets.** The alternative is a homegrown sync script we would have to secure ourselves; ESO is the ecosystem's answer and its backend being *our broker* keeps #13948 as the single runtime source of truth. One caution carried from the survey: guard `ExternalSecret` resources with admission policy (Kyverno) — a malicious ExternalSecret is an exfiltration path — and note ESO's thin maintainer bench as a watch item, not a blocker.
- **OpenBao — DEFER; it keeps a job, but the job hasn't arrived.** Adopting Keycloak + cert-manager + ESO (or, in our case, skipping Keycloak and adopting the other two) still leaves OpenBao's remainder untouched: if cluster applications ever need per-request database credentials, application-level transit encryption, or lease-revocable credentials at in-cluster scale, OpenBao is the open-licence way to get them and nothing else on this list substitutes. Until one of those needs is real, running it means operating a seal/unseal vault cluster to duplicate what the broker already does. **Trigger:** any one of dynamic secrets / transit / in-cluster auth at scale becoming an actual requirement. Note the composition is friendly when that day comes: ESO's backend can switch to OpenBao by configuration, cert-manager can use it as an issuer, and SPIRE's stock Vault UpstreamAuthority pattern is the shape our §1 chain would follow — deferring OpenBao blocks none of the other three.

### 3.2 The one-sentence version

Keycloak is a door, cert-manager is a mint for certificates, ESO is a conveyor belt, OpenBao is a safe that can also mint credentials on demand — adopting the door, the mint, and the belt still leaves the safe's unique work undone, and we will rent the safe when we have something that needs it.

---

## Recommendation set (consolidated)

1. **Attestation:** SPIRE pre-chosen, chained under the fleet CA via UpstreamAuthority; deploy on the first real workload-identity need, not before. Do not build our own attestation.
2. **SOPS:** expand Tier B use (config-shaped secrets, same public repo, one file per recipient set, per-host age keys) — and explicitly do **not** migrate Tier A/runtime secrets into SOPS. One system of record per secret; promotion is one-way (file → store, by rotation).
3. **Repo shape:** no second secrets repo. The public repo carries Tier B ciphertext under §2.3's discipline; Tier A never enters git in any form, ciphertext or otherwise.
4. **Cluster stack:** cert-manager (adopt, chained to fleet CA) and ESO (adopt, broker-backed, policy-guarded) at cluster time; Keycloak skipped in favour of our IdP; OpenBao deferred behind a named trigger (dynamic secrets / transit / in-cluster auth at scale).
5. **Composition with #13948:** no change to its slices; this record adds the SOPS tier beside it and the one-home-per-secret rule at the seam, recorded on #13948 itself.

*Supplements: `supplements/research-notes.md` (sources and evidence trail), `supplements/issue-premises.md` (the #13948 / #13893 / #9155 / #13658 premises this analysis composes with, name-only).*
