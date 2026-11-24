# Research notes — sources and evidence trail

*Gathered 2026-11-24 for the SECRETS-STACK-ANALYSIS record. Facts below were used in the main analysis; each is tied to its source.*

## Primary internal source

- **CNCF security & identity survey** (lane CNCF-SECURITY, 2026-11-23), workspace copy at `~/workspace/cncf-investigation/research/security-and-identity.md`, landed in the repo as part of `doc/2026-11-23T22-33-41Z_research_cncf-landscape/`. Carries the estate verdicts this analysis builds on rather than re-derives: OpenBao (OpenSSF sandbox, v2.7.x line, Vault-API-compatible, dynamic secrets / transit / PKI, since 2.6 its PKI can issue SPIFFE-shaped documents), Keycloak (CNCF incubating, 26.8.2, breadth vs our IdP's fit/footprint ~7 MB RSS), SPIFFE/SPIRE (CNCF graduated, SPIRE v1.15.x, "adopted the ID format already; SPIRE itself only with a real cluster, anchored to our CA"; attestation named the house stack's one genuine gap), cert-manager (graduated, adopt chained to our CA), SOPS (CNCF sandbox, v3.13.x, limits: no read-time access control, no audit, no dynamic secrets), ESO (CNCF sandbox, v1.0 2025-11 / v2.0 2026-02, maintainer-capacity watch item, "the bridge between our broker world and in-cluster apps").

## SPIRE mechanics (verified via spiffe.io documentation and secondary write-ups, 2026-11-24)

- Server configuration reference (https://spiffe.io/docs/latest/deploying/spire_server/): UpstreamAuthority plugin type "allows SPIRE server to integrate with existing PKI systems"; built-in UpstreamAuthority plugins include `disk` (CA loaded from disk signs SPIRE server intermediates), `aws_pca`, `awssecret`, `gcp_cas`, `vault` (Vault PKI secrets engine), `spire`, and `cert-manager` (requests intermediates via a referenced cert-manager Issuer). KeyManager plugins include `disk`, `aws_kms`, `hashicorp_vault`.
- Attestation flow: node attestation first (Kubernetes projected service account token `k8s_psat`, cloud instance identity documents, TPM/DevID), then workload attestation deriving selectors (namespace, service account, image digest, Unix UID, systemd unit, …) from the calling process via kubelet/runtime; the workload presents no credential to the Workload API (Unix socket). SPIRE documentation warns the workload-attestation signal is reliable only for unprivileged workloads (root on the node can circumvent it).
- Lifetimes: default X.509 SVID TTL 1 hour, JWT SVID TTL 5 minutes, agent rotation at half-life; revocation by deregistration (issuance stops; outstanding SVIDs expire).
- Registration-entry drift is the known operational pitfall; the SPIRE Controller Manager / `ClusterSPIFFEID` templating is the current answer (the older Kubernetes Workload Registrar was deprecated in SPIRE 1.5.0).

## SOPS mechanics and the public-repo evidence

- SOPS encrypts values, leaves keys/structure readable; per-file random data key (AES-256-GCM + file MAC) wrapped per recipient (age / PGP / cloud KMS); adding a recipient re-wraps the data key without re-encrypting values; `.sops.yaml` creation rules select recipients by path. (getsops README and multiple practitioner write-ups, e.g. https://www.bugsntea.dev/articles/managing-secrets-with-sops/ and https://dev.to/systhoughts/sops-age-store-secrets-safely-in-git-for-gitops-4g6i, both retrieved 2026-11-24.)
- GitOps: Flux decrypts SOPS natively; Argo CD via plugins — consistent across sources.
- Public-repo case study (https://git.ymrtech.com/ymrtech/ymrtech — a fleet operator's published post-mortem, retrieved 2026-11-24): a SOPS store lived in a **public** repository until 2026-09-23; the operator's own verdict is that the ciphertext part "is sound either way" — the actual defect was one catch-all creation rule wrapping six recipient identities' secrets to all six keys (a mail host's key could open portal tokens and VPN private keys). Fix: split to one file per recipient set, filename stating the set, and move to a private repo; per-host age keys generated on the host (`/var/lib/sops-nix/keys.txt`, root-only). The same post-mortem notes the leak that actually occurred in that fleet was plaintext in a *running system's log transcript*, not the repository — a useful calibration of where SOPS risk does and doesn't live.
- Rotation nuance (multiple sources): `sops rotate` / recipient removal re-wraps data keys for current files; it does not change the underlying secret values, and historical versions remain openable by previously-authorised keys — the basis for §2.2's revocation analysis.

## Notes on claims deliberately hedged in the main text

- "SPIRE's Vault UpstreamAuthority against OpenBao" is stated in the main analysis only as a *pattern* (API compatibility is OpenBao's design goal), not as a verified integration for our estate — it would need a spike before being relied on.
- No secret values were inspected at any point in this research; internal premises were read from issue bodies (titles, designs, counts, field names) only.
