Muse handoff: Fleet v0.3 credentials, secrets and certificate authority

Preservation comes first. Treat recent repo work as integration evidence, not a replacement for the recovered requirements. Run this as a design mill: preserve and map every requirement, break it into reviewed component designs, and turn each design into linked, testable implementation issues. Verify source access through the authorized identity or approved staging. Do not make private material public.

Muse, coordinate this pending design through the existing OpenCode Summit pool. The earlier submission was cancelled; do not assume it ran. First inspect existing issues, commits, tasks and sessions so we reuse work already underway rather than recreate it. Do not launch over a live bound session. Your immediate mission is intake, integration review, issue decomposition, a verified Summit launch, and durable supervision, followed by the staged implementation lifecycle. A completed design is not a completed implementation epic.

Authoritative inputs

1. Complete source archive: https://drive.google.com/file/d/1cdmoblf8yQM2NuXsR2i90N45dABU1EPo/view
2. Secrets architecture/performance/infrastructure addendum: https://docs.google.com/document/d/1QkQMPYGEHbjdyMBdrcSdo4i2GKxlJYK2S3PDrvCRfVw/edit
3. Existing Summit coordinator and design-decomposition handoff: https://docs.google.com/document/d/1AKDACXObRcUwmJIeCfLaGv6bxcWC00e3rYjOztk4qiY/edit
4. Companion PM workstream, for shared queue/dispatch conventions only: https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw/edit
5. Repository: https://github.com/developing-today/code

Source archive SHA-256 verified during this intake: d853a2bacf391856a863c292b21f5a224352a6681dd81fe55c422040b1fb5644. Recompute it and verify SHA256SUMS; the manifest covers five files, while the package also contains the third original architectural-perspective input. Read all seven archive entries, including the complete 50-section specification and its Appendix A, not merely summaries or early sections. Read the entire addendum and coordinator handoff. Preserve an immutable source snapshot and source-to-requirement trace.

Interpretation and scope

This is the CA/credentials workstream, not the separate PM-stack redesign. It includes unified secret records and BWS/op/bw adapters; daily three-way reconciliation and recoverable zz.delete history; audit/event journal and provider performance; fnox library evaluation; SOPS/age/NixOS integration; password, SSH and iroh generation; reusable Rust APIs and the existing Fleet multicall CLI; PKI root/issuer/leaf lifecycle, publication, packaging, client trust and verification; and a nonblocking OpenBao/federation/infrastructure research program.

The addendum refines the archive: retain BWS as the current machine-secret backend; actively evaluate fnox; benchmark op/bws and separately authorized bw; strengthen SOPS/age; design temporary explicit root escrow to encrypted laptop storage, the authorized BWS project and 1Password vault, not every future provider. R2 remains a researched additional destination, not automatic authorization. Preserve the future path to offline or hardware root signing. Three OpenBao containers on one host are a lab, not three failure domains.

For the reference PKI design, retain four independent roots (Ed25519, P-256, RSA, ML-DSA-44), nine asymmetric root cross-certificates, Base and Internal Web issuer sets with independent A/B partitions and eligible direct issuer cross-signatures: 20 CA key pairs and 65 CA certificate variants before leaves. Preserve exact reference expiries (roots 2050-07-07T12:00:00Z, both issuer sets 2032-07-07T12:00:00Z), distinct from normal 20y/5y/90d defaults. The revised spec, not older four-intermediate examples, controls. RSA-2048 remains a policy-gated compatibility option rather than the default long-lived root.

Do not revive superseded technical assumptions: AIA may have multiple entries, but clients may ignore them or stop early; distribute explicit valid chains. Private Public Web naming is not public WebPKI trust. Cross-signing does not confer algorithm support, and trusting classical roots does not provide PQ-only assurance. OCSP URLs must represent a functional supported service. Root signing requires an explicitly authorized signer source; never silently discover and exercise root keys.

Muse intake and launch

Pin the current repository commit, read governing instructions and relevant live work, including Fleet id/Atlas integration, resolver/broker, multicall registry, Nix feature profiles, current queue/outbox writers and design-mill conventions. Classify every requirement as verified-existing, partial, missing, research, unsupported, or approval-blocked with code/test/issue evidence. Preserve current architecture unless an ADR justifies an equivalent change. Historical urgency 450, Project #4 and route labels must be mapped to current contracts rather than hard-coded into a changed scheduler.

Create or reuse one canonical epic and a small set of domain starter issues. Use existing sanctioned issue/outbox/queue paths, idempotency and native dependencies. Then create and immediately launch the Summit task using the verified current route. Verify task ID, session binding, delivered prompt and actual activity. If route/model naming differs from the historical docs, report the mapping; do not invent a route. Stage the full sources where the execution session can read them and review nonsecret source text before committing it to the public repository.

Summit should produce a design-of-designs: complete source/requirements matrix, current capability census, domain architectures, ADRs/threat model, granular linked implementation/test/research issues, explicit dependencies, staged roadmap and first executable stage. Child designs may produce further bounded implementation issues; each issue needs measurable acceptance, source IDs, boundaries, tests and rollback. Reuse recent id/Atlas and PM infrastructure, do not duplicate it. Keep the CA epic separate from PM implementation while linking genuine shared prerequisites.

Register an actual durable supervision mechanism and verify it. While active, inspect meaningful progress, questions, failures, quotas, source coverage, session existence and task binding. Answer from evidence and existing authority, restart or continue only through authorized idempotent recovery, and never double-launch. Escalate only decisions that require the user. Do not claim monitoring exists merely because this prompt asks for it.

Acceptance and safety gates

- Commit reviewed nonsecret source artifacts, provenance and source-precedence resolution; preserve every requirement in a traceability matrix or document an equivalent approved ADR.
- Preserve the existing Rust CLI/library architecture, binary-size/feature contracts and current functioning integrations.
- Require independent cryptographic path validation, negative PQ-only tests, real client capability reporting, provider failure/reconciliation tests, performance evidence and generated reference artifacts using Fleet itself when implementation reaches that stage.
- Keep design, synthetic proof, code implementation and production rollout separately statused. No production CA generation or signing, root escrow/export, live vault writes or reconciliation, credential rotation, DNS/trust changes or live service deployment is authorized merely by this design handoff. Respect applicable explicit approval and custody requirements; synthetic fixtures must never be confused with production credentials.
- Complete with verified issue/task/session/monitor receipts, committed design evidence, issue coverage audit, unresolved decisions and the next implementation stage. Keep implementation children open until their actual acceptance gates pass.

Discussion provenance

- Detailed CA build requirements: https://chatgpt.com/c/6aed3b42-f600-83ea-9e52-ddff679fc43d?messageId=c5dfe3cd-aa2a-4e0f-9365-84d6295e735d
- Revised provider/root-custody follow-up: https://chatgpt.com/c/6aed4e36-8454-83ea-88c6-d42b54fdbbad?messageId=bbb21d76-8416-49cf-b4dc-5df93539766e
- Earlier submission statement, now superseded by the user's cancellation clarification: https://chatgpt.com/c/6aed4e36-8454-83ea-88c6-d42b54fdbbad?messageId=bbb215c4-9023-4782-84e8-5619793004cb

The archive and native Docs preserve the exact recoverable original deliverables. Some historical assistant responses are omitted by the conversation reader; do not pretend those missing replies were recovered verbatim.

Optional research note: Cloudflare Merkle Tree Certificates

Keep ML-DSA-44 in the CA baseline. Treat MTC as a separate, non-blocking controlled-stack experiment. MTC authenticates a batch checkpoint and uses an inclusion proof per certificate; it can remove repeated CA signatures from the handshake when the client has fresh authenticated landmarks. It does not remove the leaf key or the TLS handshake signature. ML-DSA-44 public key: 1,312 B; signature: 2,420 B. The draft's 736 B example is an inclusion proof, not a whole cert, and standalone fallback carries checkpoint signatures too.

Cloudflare's reported 9% median improvement was against classical chains and chiefly reflects intermediate elision, not a head-to-head benchmark versus a short ML-DSA-44 chain. The Nov 2 IETF draft is not a final RFC. Cloudflare's Sep 29 announcement targeted early 2027 production issuance. Azul is open-source Rust log/mirroring infrastructure (BSD-3-Clause-style main license plus notices), not a turnkey current MTC CA; the BSD-3-Clause Go reference is for an older draft.

Only prototype where we control issuers and verifier rollout. Compare actual bytes, verification CPU, handshake latency and failure under cold, fresh, stale and offline clients against our shortest conventional ML-DSA chain. Include landmark/cosigner provisioning and rotation, logs, revocation, freshness, compatibility and downgrade policy. Keep PQ-only trust and PQ key exchange separate and test that classical fallback does not silently claim PQ authentication.

IETF draft: https://datatracker.ietf.org/doc/html/draft-ietf-plants-merkle-tree-certs
Cloudflare MTC performance/rollout: https://blog.cloudflare.com/pq-ca-with-mtcs/
Cloudflare controlled-origin ML-DSA: https://blog.cloudflare.com/post-quantum-authentication-to-origins/
Open components: https://github.com/cloudflare/azul and https://github.com/bwesterb/mtc


Original v0.3 delivery and short implementation prompt: https://chatgpt.com/c/6aed4e36-8454-83ea-88c6-d42b54fdbbad?messageId=7b458918-f70f-4e1e-850d-ff7f4b7c6f6a
Recovered Muse transition notes: https://docs.google.com/document/d/1zKK_gHj0iN9FIk1qS2v5L2mvVlGriXCVrzAv4L1lxj0/edit
Raw CA/secrets precursor (historical only): https://docs.google.com/document/d/1IYGT8XSAWrgaMxTLjivw_kQOtjePjg0G2rpORp94Eu8/edit
Older PKI matrix (historical only): https://docs.google.com/document/d/1em30lY72gnM6MqJ-8bQ0d8E3M-RXterqu25HvY1kL8s/edit

