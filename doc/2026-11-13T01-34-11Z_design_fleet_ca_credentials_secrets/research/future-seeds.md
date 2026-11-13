# Future idea disposition and research seeds

This is a coverage register, not a release commitment. Each source idea below
is mapped to an existing concrete proposal or to a bounded seed in
[`../proposals/idea-seeds.json`](../proposals/idea-seeds.json). Existing proposal
IDs are canonical local IDs; they are not numeric GitHub issue receipts.
Baseline requirements (including ML-DSA-44, three-destination root escrow,
R2 research-only, optional MTC, and no production changes) remain binding.

## Source §15 idea register

| Source item | Disposition |
|---|---|
| §15.1 config/profile inspector (`explain`, `diff`, `validate`) | OPS-05 generates docs from frozen command descriptors, but does not inspect effective config; SEED-27 |
| §15.1 shell completions incl. PowerShell | OPS-05 |
| §15.1 structured secret/bundle search/filter | SEC-02 defines a schema, not search/query APIs; SEED-28 |
| §15.1 arbitrary bundle build/inspect/compare/extract | CERT-10/11 own CRL publication and OPS-04 atomic file/archive transactions, not bundle semantics; SEED-29 (excludes CMS/kit owners) |
| §15.1 trust-anchor quick install/export | CERT-13 owns export profiles and non-mutating per-client install plans; OPS-07 owns kits/guides; SEED-15 covers lint/doctor |
| §15.1 installation lint/security doctor | SEED-15 |
| §15.1 audited public kit export | OPS-07 |
| §15.1 static website/manpage docs | OPS-05 explicitly generates Markdown/manpages; static website output remains SEED-30 |
| §15.1 CSR acceptance/signing | CERT-04/05 |
| §15.1 mixed-profile certificate-selection examples | OPS-09 verifies client compatibility, not tested sample configurations; SEED-31 |
| §15.1 publication checksum index/archive snapshots | OPS-04 transacts files and SEED-14 covers long-horizon readability; publication index/snapshot contract is SEED-36 |
| §15.1 default renewal alerts | CERT-08/09 model lifecycle/CRLs and OPS-11 generates runbooks; monitoring/alert delivery is SEED-39 |
| §15.1 key-rotation visualizer/root-trust graph | Existing ROOT-02/04 model the graph, not a read-only impact dashboard; SEED-01 |
| §15.1 schema migration/import OpenSSL/step-ca | CERT-17 validates artifacts but does not define import/migration; SEED-13 |
| §15.1 batch reconciliation/interrupted replay | REC-07 and OPS-03 |
| §15.1 OP/BWS migration/mirrored vault audit | SEC-02/03/04/05 define schema/capabilities/adapters, not cross-provider migration/audit; SEED-50 |
| §15.2 native OpenSSH CA/user+host certificates | GEN-05/09 cover native keys/rotation, not CA issuance; SEED-12 |
| §15.2 SSH host-key discovery/known_hosts rollout | GEN-08 explicitly excludes host-key TOFU and GEN-09 requires verified host-key reference; discovery/managed rollout is SEED-33 |
| §15.2 OpenPGP certificates/subkeys | SEED-08 |
| §15.2 JWK/JWKS/JWS/JWT | SEED-09 |
| §15.2 JOSE/COSE/CWT/CBOR | JOSE is SEED-09; COSE/CWT is SEED-10; no lossy format conversion |
| §15.2 DID documents/Multikey | SEED-11 |
| §15.2 SPIFFE/SPIRE SVID adapters | FUT-03/06/08; no second workload identity program |
| §15.2 private ACME enrollment | SEED-16; keep separate from public ACME and private issuer core |
| §15.2 EST/SCEP/CMP device enrollment | SEED-17 |
| §15.2 SSH/WireGuard/mTLS/Tailscale/mesh deploy adapters | SSH CA/key paths: GEN-05/09/SEED-12; Tailscale: FUT-11; mesh choice: FUT-08. Separate WireGuard and mTLS adapter studies are SEED-34/SEED-35 |
| §15.2 Kubernetes cert-manager/operator | FUT-06 |
| §15.2 Cloudflare/Route53/other DNS providers | SEED-18 (DNS challenge API only) |
| §15.2 S3/R2/OCI/IPFS public certificate distribution | ARCH-06 is an internal authority seam; CERT-10/11 are CRL-specific; concrete public-object publisher is SEED-36 |
| §15.2 Sigstore/Fulcio/Rekor keyless signing | SEED-06 |
| §15.2 artifact/code signing policies/timestamps | SEED-05 |
| §15.2 transparency/append-only CA issuance ledger | REC-04/REC-05 cover credential journal mechanics, not a CA transparency log; SEED-07 |
| §15.2 HSM/PKCS#11 root/intermediate workflow | ROOT-09/10/12 and CERT-19 own escrow/recovery/ceremony/migration; adapter capability and custody research remains SEED-37 |
| §15.2 KMS/TPM/YubiKey/enclave adapters | ARCH-06 is a shared authority seam; ROOT-09/10 and CERT-19 own custody/migration, not provider adapters; SEED-37 |
| §15.2 multi-region signing HA | FUT-01/04/05/09 cover staged infrastructure/failure domains, but not shared serial/signing consistency; SEED-25 |
| §15.2 real multi-issuer OCSP | CERT-14 correctly keeps OCSP unsupported but defers responder research; SEED-38 investigates a complete responder/no-go boundary |
| §15.2 enterprise managed trust rollout | OPS-07 creates public kits and FUT-06 studies Kubernetes delivery; SEED-26 covers managed endpoint rollout/acknowledgement |
| §15.2 PQ/hybrid TLS deployment matrix | MTC-05 is optional MTC TLS integration only; SEED-04 studies ML-KEM/hybrid transport independently |
| §15.2 ML-KEM/hybrid confidentiality transport | SEED-04; signature algorithm support is not KEM/TLS support |
| §15.3 reusable config/file/ID/password generators | GEN-01/02/03 cover typed API, password and ID outputs; arbitrary deterministic config/file templates are SEED-58 |
| §15.3 JSON/YAML/JSONC/TOML/KDL/CSV metadata adapters | SEED-03 |
| §15.3 NATS job/event orchestration | SEED-02 |
| §15.3 Redis/Postgres/Supabase/S3 read/write stream connectors | SEED-02; R2 archive evaluation remains FUT-10, not a production connector |
| §15.3 remote HTTP/gRPC API | ARCH-06/FUT-12 establish authority/identity seams; SEED-24 scopes the missing authenticated service boundary, not a generic server build |
| §15.3 policy-as-code | SEED-19 |
| §15.3 multi-tenant organization/OU namespaces | SEED-20 |
| §15.3 live reconciliation/monitoring/scheduled root and CRL checks | REC-08 is scheduler/checkpoint contract, CERT-08/09 lifecycle state and OPS-11 runbooks; monitoring/alerts are SEED-39 |
| §15.3 read-only web/TUI explorer | SEED-01 |
| §15.3 reproducible/attested Rust builds | OPS-08 validates synthetic artifacts and OPS-09 client compatibility, not reproducible/attested builds; SEED-41 |
| §15.3 automatic benchmark/compatibility lab | PERF-01 accepts provider-performance benchmarks; OPS-08/09 accept artifact evidence/client matrix, but no automated matrix lab runner; SEED-57 |
| §15.3 algorithm-policy updater | ROOT-01/07 and GEN-10 define registries/path constraints, not policy-update rollout; SEED-42 (no automatic downgrade) |
| §15.3 encrypted backup/two-person root recovery | ROOT-09/10/12 own exact three-destination escrow and witnessed ceremony; threshold/FROST alternatives are SEED-43; do not add R2 |
| §15.3 hierarchical/offline signing workflow | ROOT-10/12, CERT-19 and OPS-01/03 cover custody/ceremony/general transactions, not air-gapped request-package lifecycle, media hygiene, signed-result import or replay-safe resume; SEED-61 |
| §15.3 external plugin SDK | ARCH-06 is an internal authority seam only; public plugin ABI/sandbox/trust is SEED-21 |
| §15.3 agent skill installers | OPS-06 |
| §15.3 tested docs/scenario simulators | OPS-08 and OPS-11 |
| §15.3 20+ year successor binary/format compatibility | SEED-14 |

## §27 and §46 coverage

| Source item | Disposition |
|---|---|
| §27.1 provider mapping probes | SEC-02/03/04/05/06 |
| §27.1 secrets maintenance tool | REC-01–08, OPS-01–04; no duplicate reconciliation engine |
| §27.1 one-command keys/secrets | GEN-01–06, OPS-01/04 cover generating and file output; two-store output authority/partial-write semantics remain SEED-49 |
| §27.1 root/issuer X.509 lab | ROOT-01–08, CERT-01–07/17, MTC-02 (optional) |
| §27.1 systemd sync rollout | REC-04 journal/outbox, REC-07 guarded apply, REC-08 scheduler/checkpoint/watermark contract, and OPS-10 approval gates; no deployment in this design work |
| §27.1 trusted deployment kit | OPS-07/09 |
| §27.1 documentation registry | OPS-05/11 |
| §27.1 PKI publish auditor | CERT-10/11/14/15 and OPS-11 define publication/runbooks, not periodic mirror/DNS/MIME/expiry audit; SEED-39 |
| §27.2 first-class workload identities | FUT-03/08 accept auth/attestation experiments; SEED-45 owns the missing canonical principal/resource graph |
| §27.2 principal/resource graph | FUT-08/09/12 cover workload/site/capability portions; none accepts the typed cross-domain graph and orphan analysis; SEED-45 (SEED-20 remains tenant-isolation only) |
| §27.2 cert validity vs authorization/policy | SEED-19; ARCH-06 and CERT-03 remain signer authority owners |
| §27.2 delegated certificate/key use | ARCH-06, ROOT-05, CERT-03/19 |
| §27.2 fleet-wide rotation orchestration | REC-03/05/07 and CERT-08/16 cover per-domain state, not consumer inventory/readiness and cross-host rollout; SEED-46 |
| §27.2 passkey/WebAuthn reconciliation | SEC-06 only specifies a distinct `bw` adapter; it does not establish passkey portability or replacement enrollment; SEED-44, approval-gated |
| §27.2 TOTP/HOTP/recovery kit | SEC-02/06 schema/provider research does not accept OTP generation/display, counter/recovery workflow; SEED-44; never log seeds/codes |
| §27.2 password change at origin | SEED-23; separate from vault sync and approval-gated per service |
| §27.2 personal vault alignment | SEC-02/06 define schema/provider boundary, not authorized cross-vault migration/fidelity; SEED-44, separately approved second sweep |
| §27.3 Kubernetes/Talos identity | FUT-03/06/08 cover auth, delivery and workload identity; Talos machine API PKI boundary remains SEED-56 |
| §27.3 Cozy/ingress/reverse proxy | FUT-07 selects control-plane boundaries, not service/ingress inventory or proxy ownership; SEED-47; no deployment |
| §27.3 Tailscale/tailnet | FUT-08/11 |
| §27.3 Fleet id integration | FUT-12 and GEN-06 |
| §27.3 agents/ephemeral jobs | FUT-03/08 test workload identity and ARCH-06 signer/artifact authority, not per-agent task-bound credential issue/revoke receipts; SEED-48 |
| §27.3 NATS/Redis/SQL/cloud sinks | SEED-02 |
| §27.3 external web PKI/ACME operators | SEED-18/SEED-22; no domain challenge/deployment here |
| §27.3 private OCSP/CRL | CERT-09 covers CRL; CERT-14 defers complete responder research; SEED-38. OCSP stays unsupported absent the separate gate |
| §27.3 trust-bundle controller | FUT-06 and OPS-07/09 |
| §27.3 credential transparency/inventory | SEED-07; public CT privacy boundary retained |
| §27.4 standalone public SSO | #9077 owner; no new seed |
| §27.4 private SSO/capability graph | #9077 owns SSO; FUT-12 owns OCapN seam; SEED-19 owns policy decisions and SEED-45 typed graph |
| §27.4 OCapN | FUT-12 |
| §27.4 decentralized identities/DIDs | SEED-11; FUT-12 remains transport/capability convergence only |
| §27.4 threshold/recovery custody | ROOT-09/10/12 own exact escrow and recovery; comparative FROST/threshold signing research is SEED-43; no ad-hoc threshold crypto |
| §27.4 cross-algorithm migration | ROOT-01/07, GEN-10 and OPS-09 cover capabilities/path evidence, not policy migration/update rollout; SEED-42 |
| §27.4 secure user/device enrollment | SEED-16/17; SPIFFE/FUT-08 stays separate |
| §27.4 self-service credential portal | SEED-01, strictly read-only by default; no signing path |
| §27.4 infrastructure trust API | ARCH-06 is internal authority; FUT-12 is capability-seam research; SEED-24 owns the missing authenticated network service threat model, with no server implementation |
| §27.4 cross-store encrypted archive | FUT-10 evaluates R2 continuity and PERF-06 SOPS/age recovery; neither accepts provider-independent cross-store ledger, dual approval or migration; SEED-59, excluding root material |
| §27.4 capability-driven policy repair | SEED-19; proposal-only and human-reviewed |
| §27.5 OP/BW/BWS field-mapping example prompt | SEC-02/03/04/05/06 plus SEED-50 for full cross-provider inventory/migration; no access implied |
| §27.5 SPIFFE workload-identity example prompt | FUT-03/08 and SEED-56 for Talos/node-vs-pod trust boundary |
| §27.5 OCapN/iroh example prompt | FUT-12 current packet is the single R-SEC-11/Fleet-id/OCapN owner; GEN-06/10 remain native key/registry owners |
| §27.5 ACME lifecycle example prompt | CERT-15 runbooks plus SEED-16 private enrollment, SEED-18 DNS challenge and SEED-22 public ACME; no live challenge |
| §27.5 passkey example prompt | SEED-44; separately approved user-authorized portability study only |
| §46 R-SEC-01 backend measurement/cache | PERF-01/02/03 |
| §46 R-SEC-02 inventory/migration | SEC-02/03/04/05/06 cover schema/adapters, not comprehensive approved inventory, lossless cross-provider migration plan/report; SEED-50 |
| §46 R-SEC-03 event journal/forensics | REC-04/05/07 own append-only journal, protected history and fault/replay recovery |
| §46 R-SEC-04 offline continuity | PERF-06, ROOT-09/10; no R2 |
| §46 R-SEC-05 OpenBao | FUT-01–08 |
| §46 R-SEC-06 Cloudflare relay | FUT-10 covers R2 archive only; Worker+D1/KV/DO gateway comparison remains SEED-52, default no-go absent evidence |
| §46 R-SEC-07 SecureStore vs age/SOPS | PERF-05/06 |
| §46 R-SEC-08 fnox/mise | PERF-03/04 cover fnox daemon/cache and SOPS/age; scoped mise task injection and invalidation are not accepted there; SEED-60 |
| §46 R-SEC-09 GitOps deployment supply chain | FUT-06/07 and PERF-04/06 cover K8s/GitOps and SOPS/age/NixOS recovery boundaries |
| §46 R-SEC-10 workload identity/dynamic creds | FUT-03/08 |
| §46 R-SEC-11 Fleet id/OCapN | FUT-12 current packet owns this deliverable; rechecked after the I-3 snapshot (its former packet content was unrelated offline-continuity work) |
| §46 R-SEC-12 Cozystack GitOps/CI/virtualization | FUT-07 owns Flux/Argo/Tekton/control-plane boundaries and FUT-08 mesh/no-mesh need; neither compares Lima/NixOS VM images/KubeVirt; SEED-55 |
| §46 R-SEC-13 public/private cert automation | SEED-16/18/22/36/38, CERT-10/11/14/15; SEED-39 owns ongoing monitoring/alerts |
| §46 R-SEC-14 access planes/break-glass | FUT-11 |
| §46 R-SEC-15 personal vault/passkeys/TOTP | SEC-02/06 are adapter/schema boundaries only; SEED-44 owns the authorized portability/display gap |
| §46 R-SEC-16 crypto providers/PQ/20-year | ROOT-01/07/09/10/11, CERT-17/19, MTC-01–06 optional; SEED-04/05/14/37/42 cover separate transport, signing, archive, hardware and migration-policy gaps |
| §46 R-SEC-17 metadata provenance/privacy | SEC-02/REC-04 cover schema/journal, SEED-03 serialization; queryable privacy-preserving analytics/retention/role policy is SEED-51 |
| §46 R-SEC-18 docs/skills/discoverability | OPS-05/06/07/11 |

The new seeds are research/design questions only. They do not alter baseline
architecture, authorize implementation/deployment, change provider grants, add
a root escrow location, or make MTC a dependency. The source dossier's dated
primary links are evidence available to follow-up workers; no new network source
was fetched for this register, so links in the dossier are not reasserted here
as independently re-verified facts.

## Draft proposal inventory and disposition count

Existing proposal packets inventoried before seed creation:

| File | Existing canonical draft IDs |
|---|---|
| `architecture.json` | ARCH-01, ARCH-06, ARCH-07, ARCH-10 |
| `secrets-reconciliation.json` | SEC-02–06; REC-01–08 |
| `performance.json` | PERF-01–06 |
| `generators-ops.json` | GEN-01–10; OPS-01–11 |
| `pki.json` | ROOT-01–12; CERT-01–19 |
| `future-infrastructure.json` | FUT-01–12 |
| `mtc.json` | MTC-01–06 (optional research only) |
| `idea-seeds.json` | 57 unique future seeds; each is a research/design packet, not publication authorization |

Coverage count uses the semantic rows above, including the five §27.5 example
prompts: **118 source-item rows** across §15, §27.1–27.5 and §46; **31** rows
map only to a specific existing proposal or external owner, **87** rows name at
least one new seed, and **0** are unmapped. There are **57 unique new seeds**
(preserving the original SEED-01–26 IDs and adding only IDs referenced by the
coverage table). A row naming a seed is classified as new even if it also names
existing prerequisites/owners; this makes the counts sum exactly to 118. The
mapping is not a claim that an older idea is baseline work or that any draft
proposal has been published.

### Packet materialization contract

Every seed body now records its current-evidence recheck, assignee-owned report
path `evidence/SEED-XX/`, disposable-fixture data owner, and intentionally
unassigned execution lane/capability. The publisher creates only an initial
`triage` queue item; lane assignment waits for triage. Its actual urgency map
derives `1000 → Standard / priority/normal / queue 40`, `1400 → Low /
priority/low / queue 70`, and `1800 → Someday / priority/low / queue 70`;
`issue_body` also materializes kind, stage, lifecycle, intrinsic urgency, source
ranges and requirement IDs. These are projections of the proposal, not extra
user-assigned priority fields. No production work is authorized by publication.
