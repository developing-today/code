Fleet v0.3 — Summit Design, Research, and Granular Issue-Decomposition Handoff

Coordinator / Summit execution contract · v1.0 · 6 November 2026  
Repository: developing-today/code · Priority: High / numeric urgency 450 · Directive: Do Now (one-shot direct dispatch)  
Primary route: the existing authorized Summit on OpenCode route; discover exact route IDs and live mechanism before invocation.  
Scope of this assignment: source intake, architectural review, research, traceability, detailed issues, and supervised handoff. Not a single-session end-to-end implementation of Fleet v0.3.

0. Core instruction and distinction between two assignments

The coordinating agent is responsible for source collection, secure laptop staging, initial high-level issue structure, direct Summit dispatch, verification, and continued supervision through a durable monitored workflow. The Summit agent is responsible for deeply reading the source package, inspecting the real repository, researching design decisions, creating a granular actionable issue tree, and reporting a justified architecture and staged work plan. Small reversible proof-of-concept tasks can be authorized only if they do not displace the design mission; no live secrets, production root keys, provider migrations or DNS/trust changes.

Two related Fleet efforts exist:

A. Fleet v0.3 Secrets / Cryptography / PKI: This assignment, governed by the complete v0.3 archive and addendum, needs architectural decisions and a highly granular issue backlog before broad implementation.

B. Fleet Priority / Agent Work-Item Orchestration / PM Stack: A separate integrated execution charter covers queue priority, work-item hydration, outbox, direct dispatch, GitHub synchronization, mobile PM views, and direct completion. Reuse its queue/issue/dispatch patterns where they fit; do not mix its unrelated implementation deliverables into the v0.3 secrets/PKI issue tree merely because both are in Fleet.

The earlier v0.3 implementing-agent handoff in the archive describes the eventual implementation outcome. It is important source context but does not authorize Summit to implement all of v0.3 in this planning session. The present instruction narrows this immediate execution to high-quality design, decomposition, research and issue creation. Retain the full implementation obligations in future executable issues.

1. Authoritative source package — retrieve every file and inspect fully

ZIP: https://drive.google.com/file/d/1cdmoblf8yQM2NuXsR2i90N45dABU1EPo/view?usp=drivesdk  
Addendum: https://docs.google.com/document/d/1QkQMPYGEHbjdyMBdrcSdo4i2GKxlJYK2S3PDrvCRfVw/edit?usp=drivesdk  
Repository: https://github.com/developing-today/code

The ZIP, verified by its internal SHA256SUMS manifest, contains:

1. fleet_v0_3/README.md — order and baseline.
2. fleet_v0_3/fleet_architecture_spec_v0_3.md — governing integrated 50-section design plus Appendix A; ~344 KB. Includes the original requirements, revised decisions, crosswalk, PKI acceptance matrix, CLI design, source reconciliation (§33), third-input crosswalk (§47), and implementation gates (§48).
3. fleet_v0_3/SECRETS_BACKEND_RESEARCH_2026-11-06.md — current provider/landscape research and source links; ~30 KB.
4. fleet_v0_3/DECISION_REVIEW_AND_NEXT_STEPS.md — architectural review and corrections; ~11 KB.
5. fleet_v0_3/fleet_agent_handoff_prompt_v0_3.md — previous full-implementation assignment; ~26 KB. Use as a source of future acceptance obligations, not this Summit session's execution plan.
6. fleet_v0_3/source_input_3_architectural_perspective.txt — original third perspective; ~54 KB; retain traceability even where §47 has reconciled it.
7. fleet_v0_3/SHA256SUMS — per-file integrity manifest.

Archive SHA-256, observed for the supplied file:

    d853a2bacf391856a863c292b21f5a224352a6681dd81fe55c422040b1fb5644

At intake, compute the checksum independently and run sha256sum -c SHA256SUMS after extraction. If the archive changes, record the new hash and re-evaluate; do not blindly require an old hash for a genuinely revised source package.

Source precedence: Current authorized user instructions and the dated v0.3 addendum refine the v0.3 baseline on their specific points. The full spec remains governing elsewhere. The decision review is a proposal requiring repo/current-source validation. Older conflicting options and security examples are historical context, not mandates when superseded. Live source evidence is more reliable than historical claims about what was implemented; any departure from a desired outcome must be recorded in an ADR and acceptance trace.

2. Coordinator preflight — observe, don't improvise

1. Read the source package README, all main source documents and addendum, at least enough to establish the scope and create safe high-level issue titles. Pin the live repository commit and read relevant AGENTS.md instructions.
2. Inspect pkgs/fleet/ (workspace, crates, registry, config, secrets, CLI, Nix feature/build profiles, resolver/broker/tests), pm-stack/, pm-stack/opencode-claimant/, pm-stack/dispatch/, the queue broker, related migrations and GitHub App/feedback/outbox integration. Verify actual tool names and deployment state; documentation describing a component as deployed is not proof it is live.
3. Search for existing Fleet v0.3 or overlapping secrets/PKI design epics and issues. Reuse/update matching issues rather than duplicating. Native parent/sub-issue relations, blockedBy links and GitHub Project #4 Kind=Epic must follow current conventions.
4. Determine the current official queue/outbox writer, the GitHub App issue-creation capability, allowed bulk issue creation pattern, idempotency/safe retry, queue materialization, and Summit route before writing anything. Do not bypass rate-limit protections or assume a raw gh issue create loop is safe.
5. Record a brief intake manifest: source links, hashes, repository commit, responsible agent, stage directory, source limitations and dispatch plan. Avoid copying private data into public issues.

3. Secure staging: put the ZIP on the laptop

The desired execution host is the existing laptop (amd / amd-6 in checked-in documentation), not the sandbox. The ZIP is supplied through a Google Drive link, not a durable path in a future agent's container.

Using the established authorized laptop remote filesystem/transfer tool:

1. Confirm host identity and accessible paths.
2. Create a non-repository staging directory such as ~/.local/share/fleet/intake/<run-id>/ with appropriate permissions; discover established intake paths first.
3. Retrieve/transfer the ZIP from authenticated Drive or an authorized local copy; never invent a persistent sandbox path.
4. Verify size, SHA-256 and internal SHA256SUMS after extraction.
5. Place a text copy/export of the addendum in the same private intake package, retaining its original Google Docs link.
6. Generate an INTAKE.md containing source URLs, checksum, file inventory, inspected commit, task ID, agent instructions and working directory. Ensure the Summit session can read these paths.
7. Keep the archive out of a public repository unless all contents are reviewed and explicit public storage is appropriate. Do not store credentials, private roots or decrypted secrets in source control.
8. Do not reset/force-update the laptop's primary checkout. Inspect git status; use the established clean-worktree or safe branch procedure.

If laptop transfer is unavailable, preserve the Drive links and an explicit blocker and choose an authorized alternate staging route. Do not claim the ZIP was uploaded without a verified filesystem receipt.

4. Central starter issue and high-level issue structure

Create or update a central GitHub Epic / planning issue:

Title: Fleet v0.3 — architecture review, research and granular implementation plan (Summit)  
Urgency: 450 / High, independent of Do Now direct-dispatch directive.  
Kind: Epic in Project #4, if appropriate to the established taxonomy, with epic label mirrored according to current rules.  
Lifecycle: Ready before dispatch; In Progress only when an executing session is verified.  
Execution lane: OpenCode, Summit route, explicitly bound to the issue and task.  
Linked sources: ZIP, addendum, repo baseline, integrated Fleet/PM Stack charter, existing relevant issues/PRs.  
Completion definition: reviewed designs, atomic issue tree, traceability, ADRs, first prototypes/evidence where specifically permitted, and a sequenced backlog; not production secrets or CA rollout.

Use a single canonical parent issue where possible. Do not create an interchangeable second parent per domain unless existing conventions require it. If multiple existing epics clearly cover domains, link rather than flatten them into one artificially new hierarchy.

4.1 High-level initial concepts (create or reuse 8–12 well-scoped parents)

Create high-level design/issues before Summit starts, but do not pre-generate hundreds of low-quality tickets. Summit will produce the granular child issues after actual source and repo review.

Suggested areas, subject to deduplication:

1. Repository census & v0.3 requirements traceability — what exists now, current tests and exact gap map.
2. Reusable Rust crate/API/CLI architecture — feature profiles, multicall registry, typed capabilities, public APIs.
3. Canonical secrets, provider adapters & custody policy — BWS/op/bw separation, scoped data/IDs, security classes.
4. Secrets reconciliation, zz.delete retention, journaling & recovery — false-delete prevention, field-level merge, outbox, backup/restore.
5. Provider performance & fnox/SOPS/age integration — cold/warm benchmarks, offline caching, NixOS and safe injection.
6. Crypto generators and format adapters — RSA, Ed25519, ML-DSA, SSH, iroh, key storage, outputs and validation.
7. PKI root bundle, cross-signing and root-key escrow — key custody, reference graph, offline migration and restoration.
8. Issuer/leaf lifecycle, AIA/CDP/CRL/OCSP and distribution — Base/Internal Web A/B partitions, chains, public/private trust and publication.
9. Operations, documentation, safe automation and test evidence — manifests, CLI docs, agent skills, runbooks, synthetic reference artifacts.
10. Provider federation & infrastructure research — OpenBao pilots, SecureStore, Cloudflare R2, dynamic credentials, K8s/identity.

A domain may need separate design, implementation, validation, migration, operator documentation, and research children. Do not use a single "implement secrets system" issue for ten independent subsystems.

4.2 Issue content contract

Every generated child issue should specify:

- Title expressing one deliverable, not a broad vague theme.
- Kind: design, research, implementation, test, migration, operations, security review, or documentation.
- Requirement IDs and source ranges from spec/addendum/ADR and internal crosswalk.
- Why / user outcome and measurable acceptance criteria.
- Current repo evidence and verified prerequisite state.
- Technical boundaries: affected crates/files/interfaces, data owner, provider/scope and permissions.
- Expected API/CLI behavior if applicable, including flags, errors and examples.
- Dependencies / blockedBy as native GitHub relationships, distinct from epic hierarchy.
- Security and safe environment: synthetic fixtures vs authorized production.
- Tests / independent verification, negative tests, benchmarks and evidence outputs.
- Priority numeric and label; urgency vs importance/scheduling pressure explicitly considered.
- Lifecycle Ready/Backlog/Blocked/Parked; lane/capability where known.
- Estimated size / complexity and recommended implementation stage.
- Failure modes, rollback and recovery if relevant.
- Open decisions and research references.

Target a backlog where a competent agent can pick a single issue and complete it without reconstructing the entire 344 KB specification. Where a child still spans several independent acceptance outcomes, split it further. Avoid atomizing trivial lines of code into separate tickets; granularity should be meaningful, testable and independently reviewable.

5. Create issues safely through the existing GitHub App / queue outbox

The repository already has queue/project/orchestrator conventions. The issue-creation volume here may be high; use the established GitHub App and its feedback/issue-writing API plus the durable queueing outbox rather than dozens of uncoordinated raw GitHub calls. Inspect the actual tools, schema, worker identity and rate-limit behavior.

Required behavior:

1. Gather issue proposals into an inspectable plan, with stable proposal IDs and deduplication keys.
2. Search existing issues and native relationships before creating duplicates.
3. Batch or enqueue writes under the GitHub App's authorized quota, respecting primary and secondary GitHub limits; back off rather than hammering APIs.
4. Use the outbox for durable intent, idempotent retries, provider receipts, status reconciliation and recovery. A pending outbox record is not a created GitHub issue.
5. Store returned GitHub issue numbers/URLs; bind them to canonical work IDs when materialized.
6. Create native sub-issue hierarchy and blocking relationships through the single authorized orchestrator/Project writer. Do not patch Project fields directly from an unapproved client.
7. Verify every created record, selected labels/priority, Kind=Epic where relevant, and dependency links after writes settle.
8. Publish the central index of child issues and a coverage register. Do not claim full creation if some operations remain deferred or rate-limited.

Where the "feedback GitHub App" tool supports high-volume issue writing, prefer that established path for this issue volume; verify the actual installed interface rather than asserting an invented command. If the existing outbox cannot support the workload, document a bounded engineering fix or safe fallback rather than bypassing safeguards.

6. Direct Summit dispatch — not ordinary queue selection

The central issue must be owned by the Summit OpenCode task launched for it, not simultaneously available for unrelated queue claimants.

Required sequence:

    sources staged and verified
      → central issue created/reused
      → high-level domain issues linked
      → queue/outbox write acknowledged
      → direct authorized Summit dispatch requested
      → task/lease bound to central issue
      → OpenCode root session created and prompted
      → session ID and route verified
      → issue/project visible as In Progress
      → supervision state registered

Inspect the existing pm-stack/opencode-claimant/ contracts. Checked-in documentation describes claimant.py and poll.py, task bindings, an OpenCode server and shadow/live modes. Do not change the claimant's mode from shadow to live as an implicit side effect of this assignment, and do not bypass its invariants or cap. The current direct dispatch path may differ from the ordinary claimant; select the authorized direct route and verify it.

Route selection must explicitly ask for Summit, not silently fall back to Workhorse/build. Use the provider/route values available in the live registry. Do not invent a providerID/modelID from memory.

The returned evidence should include:

    repo_issue_number
    canonical_queue_work_id
    task_id
    agent_route
    session_id
    assigned_host
    accepted_at
    started_at (when observed)
    lease/ownership state
    dispatch_receipt_id
    initial prompt acknowledgment

Do not consider the task launched simply because an issue was marked do-now. A raw session-created result with no prompt delivered is not a functioning agent; use the existing recovery/zero-message checks rather than double-launching.

6.1 Prompt to Summit (deliver intact with source links)

> You are the Summit architecture and research agent assigned to the central Fleet v0.3 design epic. Read the full source ZIP, its README and all governing v0.3 documents, and the separate dated addendum. Verify all source hashes. Inspect current developing-today/code and its AGENTS.md, existing Fleet crate boundaries, secrets resolver, queue, Outbox, PM Stack source-of-truth, build profiles, tests and previous design records. Pin the commit and state which behaviors are verified versus proposed.
>
> Your primary deliverable is a deeply researched, technically justified architecture and highly granular, linked GitHub issue backlog for implementing the complete v0.3 specification over stages—not a single giant implementation attempt. Reconcile every source requirement with actual code and planned tests. Produce stable requirement IDs, source-to-implementation-to-test mapping, meaningful ADRs, a design-risk register, proof-of-concept plans, migration/rollback gates and issue-ready implementation packets.
>
> Create/reuse high-level domain issues and break them into actionable children with independently verifiable acceptance conditions. Use the existing GitHub App feedback/issue-writing integration and queue outbox for high-volume issue creation; preserve rate limits, deduplication, native sub-issue/blockedBy relationships, labels, priority, project conventions and canonical queue identities. Give the parent issue an inspectable index of all children and link every requirement to the appropriate ticket(s). Create research issues for future technologies without prematurely making them blockers of the current Fleet implementation.
>
> Preserve BWS as the current machine-secret backend and authorized 1Password access, evaluate fnox urgently, benchmark providers, plan SOPS/age and root-key escrow per the addendum, and research SecureStore/OpenBao and later infrastructure as separate stages. Preserve the exact reference PKI matrix and test requirements. Do not infer bw personal-vault access from BWS, expose secrets in public issues, sync production vaults, generate live roots, alter trust stores or DNS, or declare implementation complete on the basis of a plan. Distinguish measured evidence, source-verified constraints, educated projections, unsupported features and explicit approval gates.
>
> Make justified architectural improvements, including alternatives not named in the prompt where they materially improve security, performance, recovery or maintainability. Document differences and tradeoffs instead of uncritically agreeing with every mechanism. Leave clear checkpoints and a final issue comment containing the architecture, issue index, stage sequencing, critical dependencies, implemented/tested evidence (if any), and remaining decisions. The coordinator will supervise your session and answer genuine blockers through the existing channel.

7. Mandatory design areas for Summit — do not compress away hard requirements

7.1 Reusable Fleet architecture and performance

Preserve the existing Rust multicall binary and library-first approach. Analyze the current registry, fleet-config, fleet-secrets, queue/proxy domains, Nix build profiles, binary-size budgets, configuration and help/agent documentation. Decide crate interfaces for canonical secret records, provider adapters, reconciliation/events/outbox, crypto/signers, PKI, SSH and iroh, artifact IO, publication and generated documentation. Support in-memory hot paths, scripted/noninteractive CLI, machine-readable receipts, optional Cargo features, low startup overhead and safe secret types/redaction. High-level CLI behavior and reusable Rust APIs must remain aligned.

7.2 Credential types, scopes and reconciliation

Distinguish BWS = Bitwarden Secrets Manager, bw = Bitwarden Password Manager, and op = 1Password; they have different authorization, APIs and record models. Preserve canonical logical IDs, native IDs and revisions, rich typed fields, account/host metadata when known, sidecars for fidelity, per-provider compartments and approved shared scopes. Design the v0.3 two-sided/three-way reconciliation against a last verified baseline, per-field conflict handling, native ID stability, in-place password updates, recoverable zz.delete history, false-deletion prevention, immutable audit receipts, provider outage handling and scoped data protection. Never treat a timeout or permission denial as a deleted credential. Distinguish future personal-vault access/permission from currently authorized BWS access. Supabase/indices may store metadata and blinded equality proofs where safe, not plaintext secrets.

7.3 Real provider benchmarking and near-term fnox

Bench BWS, 1Password and, when separately authorized, Bitwarden Password Manager: CLI vs native SDK/API, cold/warm reads, lists, pagination, writes, auth overhead, concurrent and batched calls, p50/p95/p99, throughput, error/retry cost, process overhead and memory. Compare laptop, current agent host and other practical locations. Use synthetic isolated vaults, not real credentials in logs. Investigate fnox immediately as Rust libraries/provider adapters, memory-only daemon, offline encrypted cache and injection; confirm actual APIs, maintenance and writable provider capabilities. Measure against existing Fleet resolver/broker, not anecdotes. Do not adopt fnox wholesale or create a competing CLI by default.

7.4 SOPS/age, NixOS bootstrap and local formats

Evaluate SOPS/age/sops-nix and existing NixOS flakes, Flux-style GitOps for later K8s, secure provisioned key locations outside Git and Nix store, recipient rotation/rekey, encrypted copies with recovery recipients, and versioned deployment ciphertext. Compare NeoSmart SecureStore as a real library candidate against age/SOPS or maintained authenticated-encryption envelope crates; test cryptographic design, file format evolution, concurrency, long-term recovery and compatibility. Avoid gratuitous new portable formats. Define an explicit cache TTL, revocation, outage/fallback and trust policy; never let stale-cache use silently defeat rotation.

7.5 Root PKI architecture, reference matrix and custody

The reference run requires four independent root key identities, nine asymmetric root cross-certificates, purpose-specific Base and Internal Web issuer sets, A/B independent issuer partitions, and direct eligible issuer cross-signatures: 20 CA key pairs / 65 CA certificates, excluding leaves. Retain roots-only as an alternative for future issuer sets; reference acceptance explicitly uses both. Ed25519/ML-DSA-44 and RSA algorithm adapters, production RSA-4096 policy and policy-gated RSA-2048 compatibility, strict PQ-only trust paths versus classical-compatibility trust must be specified and independently validated. Reference root expiry 2050-07-07T12:00:00Z, Base and Internal Web issuer expiry 2032-07-07T12:00:00Z; general policy defaults differ (roots ~20 years, issuers ~5 years, ordinary leaves ~90 days). If standards or exact library/client capabilities make a requested path unsafe or unsupported, explain and test the boundary rather than silently substitute.

Addendum refinement: Initial root private keys are intentionally escrowed in three approved custody destinations: encrypted local laptop storage, approved BWS project, and approved 1Password vault, with integrity checks and restoration tests. Plan SOPS/age-encrypted archives with separately protected recovery recipients and consider private client-side-encrypted R2 backup as a researched additional location—not automatic replication into every future backend. Preserve a path to offline/hardware-protected signing later. This requirement is a design/custody policy, not authorization to generate or synchronize production roots during Summit planning. Separate active signing custody from escrow and prevent circular recovery dependencies.

7.6 Issuers, leaves, interoperability and publication

Design declarative generation of root/issuer/leaf bundles, CSR, PEM, DER (.crt/.cer), certs-only CMS (.p7c), chains, CRLs, metadata manifests, DNS records and deployment guides. Correctly construct AIA caIssuers from the issuer, CDPs from signer CRLs, optional real served OCSP only when supported, configurable HTTP mirrors and immutable paths. Assume clients may not fetch alternate AIA chains: proactively provide known-good bundles and trust installers. Distinguish private CA web certificates for managed clients from public WebPKI/ACME trust, CT/CAA/DCV needs and public domains. Preserve Base test-leaf and Web SAN/wildcard reference kits for developing.today and developing-today.com, and include strict PQ-only rejection tests and real client validation where supported.

7.7 Documentation, operational automation and real proof

Future implementation must provide CLI help, manpages/completions, agent skills/AGENTS.md updates, examples, batch YAML/JSON, policy/manifest schemas, readable packaging, source-linked operational instructions, synthetic test fixtures, unit/property/fuzz/integration tests, independent OpenSSL/Rust/client checks, crash/reconciliation tests and rollback. The *eventual implementing* agent should use the finished Fleet binary itself to produce reference artifacts and independently verify them. Summit should create precise issues and tests for those obligations; it must not claim a prototype or mock proves production readiness.

7.8 Future infrastructure research — explicit, nonblocking program

Investigate OpenBao KV/dynamic credentials/Transit/PKI/SSH/audit/auth, an isolated synthetic lab (possibly three containers, explicitly one failure domain), Oracle free-tier resource/uptime/backup constraints, eventual local Talos/Cozystack/Kubernetes and other independent sites. Evaluate operators, cert-manager, External Secrets Operator, Secrets Store CSI, NGINX/APISIX/Istio/Linkerd, workload identity/SPIFFE/SPIRE, Tailscale/Teleport, SSO/OIDC, passkeys/TOTP, Cloudflare R2/Workers encrypted continuity, and related dependencies. Propose separate, falsifiable research issues with go/no-go criteria and evidence sources, not immediate production dependencies.

8. Traceability and issue decomposition quality bar

Produce a machine-readable and human-readable matrix, ideally under repository docs/ or doc/ according to conventions, with columns:

    requirement_id
    source_file_and_section
    source_precedence_resolution
    user_outcome
    severity_or_security_class
    verified_current_behavior
    proposed_architecture_or_API
    primary_issue_number
    dependent_issue_numbers
    provider_capabilities_and_permissions
    acceptance_test_or_benchmark
    migration_or_rollback
    evidence_status
    current_status

Coverage must include the original v0.3 sections, source-input-three crosswalk and all addendum refinements, not merely the major themes. Use explicit statuses: already-implemented, verified, partially-implemented, not-implemented, research, unsupported, blocked-approval, future.

Every requirement should map to ≥1 actionable implementation/test/research issue or an explicit verified-existing evidence link. Every issue should map back to requirement IDs. Do not create 300 trivial issues to appear thorough, but do not hide 30 independent deliverables in one parent ticket.

Use a coverage audit at the end: number of requirements, covered by existing code, linked to newly created issues, linked to existing issues, explicitly deferred, blocked on approval, unsupported, and unresolved. Fail the assignment if unexplained requirements remain unmapped.

9. ADRs, evidence, research and required architectural challenge

For consequential choices, compare alternatives and record an ADR with:

    problem / source requirement
    current system constraints
    considered options
    measured or externally validated facts
    security / performance / maintenance tradeoffs
    recommended decision
    implications and migration
    alternatives rejected and why
    test / reversal triggers

Explicitly challenge assumptions such as "BWS is slow", "OpenBao is automatically more reliable", "fnox can write every provider", "three same-host containers are HA", "all root copies should sync to every provider", "every private CA path is browser-compatible", and "GitHub App rate limits vanish with batching". Use current authoritative documentation and real benchmarks where practical. Do not manufacture results or dated citations. Keep source-backed findings separate from speculative plans.

10. Production safety and authority boundaries

Allowed without additional secrets authorization: reading repository/public research; creating design docs/issues; isolated synthetic crypto test vectors; isolated test vaults with explicitly confirmed access; local mock pilots; reversible documentation and test changes that comply with repo rules.

Requires explicit applicable approval and reviewed rollback/custody controls: editing live vault contents; full personal password-manager inventory; wide production reconciliation; zz.delete shadow history creation in real vaults; rotating real credentials; generating/distributing production root private keys; changing trust anchors, DNS, public deployment, root signing operations; enabling a new live OpenBao/Cloudflare secret service; provider-wide replication expansion.

Never commit secret values, production private keys, unredacted credentials, password diffs, TOTP seeds, or decryptable provider tokens to the public repository, issue comments, PM board, logs, or telemetry. Do not infer authorized bw access from bws access. Do not use a failed provider list response as evidence of deletion. Preserve separate roots, signed path policy, no silent cryptographic substitution and independent restoration proof.

11. Issue priority, status and staging conventions

- Central Summit design issue: 450 / High / Do Now, direct assigned Summit, verified In Progress on actual session start.
- High-level domain architecture issues: High or Standard depending on actual critical path; do not stamp them all Critical.
- Bounded immediate foundations: High when they unblock implementation or reveal material security risk; otherwise Standard.
- Future investigations: normally Standard, Low or Someday according to opportunity, cost and dependency; retain important subjects visibly and periodically review.
- Root-key operations blocked on approval: mark Blocked with required decision and safe next step, not silently Ready.
- Completed work: requires verified evidence; do not close issues just because they were enumerated or designed.

Numeric urgency can be refined according to the companion PM/queue charter. Distinguish intrinsic urgency, importance, effective scheduling pressure, status and dispatch directive. Use priority labels derived from canonical scores, the approved Project #4 Kind=Epic hierarchy and native relationships, not a custom second Project writer.

12. Summit's recommended output tree

The exact repository paths must follow local conventions. A reasonable deliverable set is:

    docs/design/fleet-v03/architecture-review.md
    docs/design/fleet-v03/requirements-matrix.csv
    docs/design/fleet-v03/source-precedence.md
    docs/design/fleet-v03/implementation-roadmap.md
    docs/design/fleet-v03/provider-capability-matrix.md
    docs/design/fleet-v03/secrets-and-custody-threat-model.md
    docs/design/fleet-v03/pki-reference-matrix.md
    docs/design/fleet-v03/benchmarks-and-prototypes.md
    docs/design/fleet-v03/issue-index.md
    docs/adr/ADR-...md

Do not generate empty scaffolding documents and claim them as complete. The matrix can be split into thematic files if one file becomes unwieldy; preserve stable IDs and an index. Generate diagrams and runnable experiments where they illuminate design, then link their actual source and results.

13. Coordinator supervision — continued attention without fictional background work

The user requests checks every 5–15 minutes while Summit is active, with corrective intervention on stalls, unanswered questions, rate limits or design drift. A one-shot coordinating agent/chat cannot promise to remain executing after it returns, so use the existing durable scheduler, Tend/watchdog, session-monitoring system or authorized automation if monitoring must outlast the current execution.

Required control design:

1. After direct launch, perform an immediate readback of the task, bound session, first prompt/message and current activity.
2. Register a durable monitor/watch record keyed to the canonical issue and session ID, with a 5–15 minute inspection cadence while active (suggest 10 minutes, subject to actual service capacity). Reuse an existing Tend or OpenCode supervision mechanism if equivalent; do not create redundant polling systems.
3. On each check, inspect session existence, meaningful new log/messages, current objective, blocker/questions, last activity, tool failures, issue/outbox progress and terminal state.
4. If a question is answerable from sources or repository authority, send a precise answer through the established session channel and record the response. Respect approval gates for live secret/production operations; do not fabricate authorization.
5. If the agent veers toward premature end-to-end implementation, remind it of the planning scope and granular issue deliverables. If it stalls or hits quotas, record the precise failure, use authorized recovery with idempotency and avoid double-launch over a live bound session.
6. Record monitor checkpoints and public-safe issue updates without flooding the issue every 10 minutes. Detect lost sessions, tasks completed early, insufficient issue coverage, and unmet source-read requirements.
7. End the monitor when a verified Summit terminal state and deliverables are inspected, or on explicit authorized cancellation. On session completion, run a final coverage/issue-index verification and deliver a comprehensive report.

Coordinator termination rule: The initial interactive coordinator may end only after the documents, central issue, staged sources, initial issue set, accepted/started Summit task and durable monitoring handoff are verified. Ending the coordinator does not mean the Summit work is complete. If no durable monitoring mechanism is available, explicitly report that limitation and do not promise periodic check-ins or continuing supervision.

14. Completion criteria for the Summit planning mission

Summit is done when all of the following are verified:

- [ ] Full source archive/addendum and current repo were read, with checksum and pinned commit recorded.
- [ ] Requirement/source precedence and conflicts are documented.
- [ ] Repository capability census is evidence-backed.
- [ ] Provider/crypto/PKI choices have ADRs, alternatives and gates.
- [ ] A comprehensive requirement-to-code-to-test matrix exists.
- [ ] One central parent issue indexes the major domain issues and granular child issues.
- [ ] Issue creation used the actual GitHub App/feedback and durable outbox path with rate-limit safety, or an equivalent verified sanctioned path.
- [ ] Ticket counts, links, status and native relationships were verified, not merely enqueued.
- [ ] Critical-path prerequisites and Queue Table Improvements/PM Stack dependencies are identified without mixing the two efforts.
- [ ] Immediate synthetic research/prototypes versus blocked production changes versus later investigation are separated.
- [ ] Future research includes evidence plans and go/no-go criteria, not just tool names.
- [ ] Final issue comment and committed design artifacts contain an actionable roadmap and evidence status.
- [ ] The coordinator received a verified terminal result and performed a final issue/coverage audit.

Do not close the full implementation epic just because the design planning mission finished. Complete the *Summit design task/issue* according to its own verified definition of done while keeping implementation children in the appropriate queue states.

15. Final reports

Coordinator launch receipt

    Parent issue URL:
    High-level issues created/reused:
    Source ZIP Drive URL:
    ZIP staging path and SHA256:
    Addendum link/export path:
    Repo pinned commit:
    GitHub App/outbox method:
    Queue/work ID:
    Direct dispatch action:
    Summit route:
    OpenCode task ID:
    OpenCode session ID:
    Prompt delivered / first message verified:
    Active monitor or scheduled check ID:
    Monitoring cadence and owner:
    Status (accepted / started / failed):
    Blockers and recovery path:

Summit final design report

    Requirements inventoried:
    Verified existing capabilities:
    Requirement gaps:
    ADRs and open decisions:
    High-level domain issues:
    Granular child issues:
    Reused issues / deduplicated:
    Deferred / research issues:
    Explicit approval gates:
    Source/benchmark evidence:
    Test and acceptance matrix:
    Implementation critical path:
    First independently executable stage:
    Unverified / unsupported / incomplete:
    Repo commit / PR and final issue URL:

16. Short operational summary

Coordinator: inspect → stage ZIP/addendum on laptop → create/reuse central issue and initial concepts → write through established GitHub App/outbox → bind and directly start Summit → verify prompt/session → set up durable monitoring → report launch receipt.

Summit: read everything → inspect live code → investigate source-backed alternatives → document AD​​Rs and security boundaries → map every requirement → create a very granular, deduplicated and dependency-linked issue tree through the sanctioned outbox → classify now/next/later/approval-gated work → link and summarize on the parent → deliver verified design status.

Never equate a queued task with a started session, a posted message with an acknowledged instruction, an outbox intent with a created GitHub issue, a design with implementation, or a successful test fixture with authorization to change production secrets.

Companion Google Doc — Fleet / PM Stack priority and work-item orchestration execution charter:
https://docs.google.com/document/d/1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw/edit

