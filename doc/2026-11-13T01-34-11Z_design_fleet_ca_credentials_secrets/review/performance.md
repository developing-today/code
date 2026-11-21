# Performance and formats design review

**Scope:** independent review of `designs/performance-formats.md`, `research/provider-formats.md`, and the six packets in `proposals/performance.json` against Fleet v0.3 §§9, 35, 37–42, 48, addendum §§2–6 and 9, coordinator handoff §§4.2/7.3–7.4, and parent ADR-S01–S06. Read-only review; no provider, production, build, issue, or deployment operation was performed. Checked 2026-11-12.

## Verdict

**AGREE with the proposed technical direction; DISAGREE that the packet set is ready to hand off unchanged.** The design preserves the present Fleet path, treats V1 as census-only, avoids inventing performance results, keeps the fnox adapter optional/read-only, makes freshness policy Fleet-owned, excludes root private-key bytes from ordinary caches/snapshots, prefers SOPS/age over a new format, and keeps SecureStore adoption conditional. Those choices align with the source package and ADR-S01/S03/S05/S06.

Two handoff issues need correction: packet urgency is internally contradictory, and PERF-05 makes the requested SecureStore evaluation contingent on a need that the evaluation is supposed to determine. No technical blocker was found in the fnox, provider-separation, freshness, or encryption analysis.

## Findings requiring changes

### P0 — Packet body urgency contradicts canonical urgency metadata

`proposals/performance.json` has a concrete priority-integrity error. The top-level `urgency` and `priority_band` values are correct, but duplicated text in packet bodies gives different values/bands:

| Packet | JSON metadata | Conflicting body text | Required correction |
|---|---|---|---|
| PERF-03 | 600 / High | 350 / High | Replace body value with 600; retain High. |
| PERF-04 | 600 / High | 350 / High | Replace body value with 600; retain High. |
| PERF-05 | 1400 / Low | 250 / Standard | Replace body value/band with 1400 / Low. |
| PERF-06 | 600 / High | 350 / High | Replace body value with 600; retain High. |

The shared charter reserves 0–399 for Critical/incident territory (and its companion charter labels 200–399 Critical); therefore the stale 350 text is not harmless prose. It can misroute work or be copied into an issue despite the structured field. The prose design already states the intended values: `designs/performance-formats.md` lines 100, 109, 118, 128. Also make the repeated “Dependencies / priority / lifecycle” paragraph agree with the final “Priority, lifecycle and sizing” section so each packet has one unambiguous urgency. Keep queue projection separate from intrinsic urgency.

**Exact fix:** update the body strings for `PERF-03`, `PERF-04`, `PERF-05`, and `PERF-06` in `proposals/performance.json`; validate each body's stage, lifecycle, urgency and band against its JSON metadata and the corresponding packet in `designs/performance-formats.md`. Do not change the shared scale or the structured values to match stale body text.

### P1 — PERF-05 gates the required evaluation on its own outcome

The sources request an actual focused SecureStore evaluation: addendum §6 says “Perform a focused technical evaluation”; spec §42.2 says it “warrants an explicit cryptography review”; §42.4 lists the required adapter-research dimensions. But PERF-05 is “optional V3 only if an actual format gap exists” (`designs/performance-formats.md` lines 126–132), so the project can defer the evidence needed to decide whether a gap exists. Adoption is correctly conditional; the evaluation itself should not be.

**Exact fix:** keep `PERF-05` as a bounded, synthetic-only research/security-comparison deliverable and move the evaluation to V2 (or another explicitly scheduled near-term stage). Keep prototype/import-export work conditional on the findings, and keep adoption/deployment prohibited absent an independent review. Include an explicit comparison against a maintained AEAD-envelope alternative, or explain why SOPS/age already covers the use case and why a separate envelope need not be prototyped. The current matrix compares principally against SOPS/age and does not fully answer addendum §6's “modern authenticated-encryption envelope” comparison. This is a scope/scheduling correction, not a recommendation to adopt SecureStore.

## Confirmed constraints and review notes

- **fnox API vs daemon:** Confirmed by docs.rs `fnox-client` 1.48.0 API docs and the fnox daemon guide. The client only reads from an already-running daemon; it does not start it, store values, or trigger provider calls. The daemon explicitly does not learn about vault rotation and requires refresh/clear; its same-user IPC is not an authorization boundary. `PERF-03` accurately proposes a public-API-only, optional read-cache adapter, with misses/failures kept distinct and Fleet freshness policy authoritative. Keep the CLI/`exec` comparator separate from the client hit path.
- **BWS vs BW and OP:** The evidence correctly distinguishes BWS machine-account/project access from Bitwarden Password Manager (`bw`) user/session access and from OP vault/service-account access. Bitwarden's machine-account/CLI docs scope BWS operations to assigned projects and explicitly describe read vs read/write permissions. The fnox provider docs distinguish BWS from read-only OP and BW providers. No permission is inferred across these surfaces.
- **No CAS claim:** Provider research correctly labels conditional writes unproven; revision timestamps are not CAS. The benchmark packet's synthetic mutation cases do not imply live writes, and the design does not promote a provider to guarded without primary evidence and an isolated test.
- **Freshness and revocation:** `PERF-02` correctly layers policy max-age/revocation decisions over the merged per-secret broker metadata, and explicitly distinguishes that code from #6767's still-open deployment/live-canary obligation. Source inspection confirms broker cache entries hold per-secret fetch timestamps and failed refreshes retain old values; thus fetched age is not evidence that upstream is current. The policy tests cover age-preservation, revocation precedence, scope isolation and inaccessible-vs-absent distinctions.
- **Root material:** The ordinary hot cache and offline snapshot exclusions are categorical in the performance design and PERF-02 boundary. Root signer use and root escrow remain distinct authority/custody flows owned by the root/security domain; this packet set does not accidentally turn the temporary three-destination escrow exception into a general cache tier.
- **SOPS/age and Nix:** PERF-04 and PERF-06 appropriately separate a Fleet artifact adapter from NixOS provisioning/recovery. The designs address recipient rekeying without implying Git-history erasure, underlying-secret rotation where revocation is required, store/derivation exclusion, and independently protected offline recovery. Synthetic fixtures and disposable VM proof are appropriate; no production action is implied.
- **SecureStore comparison:** The findings are carefully bounded: upstream documentation states AES-128-CBC/HMAC-SHA1/PBKDF2-SHA1, but this is not presented as proof of a practical break or as a cryptographic audit. Comparing key custody, multi-recipient recovery, rekeying, concurrency, format evolution, compatibility and Nix integration against SOPS/age is sound. The only gap is completing and scheduling the evaluation unconditionally and explicitly considering the modern AEAD alternative named by the addendum.
- **Benchmarks and scope:** No timing or performance conclusion is fabricated. PERF-01 uses an independently recomputable quantile oracle, redaction checks and synthetic-first execution; live adapters and mutations remain separately authorized. The six packet scopes are generally independently reviewable, with PERF-03 depending on the benchmark/freshness contracts and PERF-06 coordinating with PERF-04's manifest interface. PERF-06's dependency is reasonable if limited to interface compatibility, but should not prevent independent VM/recovery testing against the existing SOPS configuration.
- **V1 and duplicate issue handling:** The reviewed performance design labels V1 as current-state census, starts new implementation at V2-foundation, and does not propose duplicating #6767's merged code fix. It distinguishes PR #8491 source-level evidence from unverified deployment/canary evidence. I found no stale claim of measured performance or an instruction to run the canary.

## Source checks

- Governing spec §§9.1–9.3, 35.1–35.4, 37.1–37.3, 38–42.4, 48.1–48.4.
- Addendum §§2–6 and 9; coordinator §§4.2, 7.3–7.4.
- Parent `architecture-review.md` ADR-S01–S06, especially ADR-S01 (authority/cache exclusions), ADR-S03 (freshness), ADR-S05 (SOPS/age and independent recovery), and `designs/architecture.md`'s V1/V2 staging and #6767 correction.
- Repository source: `pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs` lines 49–68, 190–265; `pkgs/fleet/crates/fleet-secrets/src/lib.rs` lines 1–28.
- Primary sources rechecked: [fnox-client 1.48.0 Client API](https://docs.rs/fnox-client/1.48.0/fnox_client/struct.Client.html), [fnox daemon guide](https://fnox.jdx.dev/guide/daemon), [Bitwarden machine accounts](https://bitwarden.com/help/machine-accounts/), [BWS CLI](https://bitwarden.com/help/secrets-manager-cli/), and [1Password Service Accounts](https://www.1password.dev/service-accounts.md). The SecureStore verdict is limited to the repository's cited upstream docs; no source audit or cryptographic audit was performed here.

## Disposition

**AGREE** with the performance/cache/format architecture, subject to the P0 urgency consistency fix and P1 PERF-05 scheduling/scope correction above. These fixes are required before the six proposals are treated as internally consistent coordinator packets. No issue publication or external action is asserted.

## Re-review — updated artifacts, 2026-11-12

I re-read the three requested artifacts after the author reported applying the findings.

### Previous findings

- **P0 urgency mismatch — fixed in scope.** The structured values and duplicated body summaries now agree for PERF-03/04/06 (600/High) and PERF-05 (1000/Standard). The PERF-05 stage/lifecycle/queue projection are also aligned between the design and JSON.
- **P1 SecureStore evaluation gating — fixed in scope.** PERF-05 is now a required V2 evaluation; only prototyping, import/export and adoption remain conditional. Both the design and research note call for an explicit comparison with a maintained AEAD-envelope alternative or a reasoned explanation that SOPS/age subsumes it.
- **PERF-01 measurement rigor — improvement verified.** The design and packet cite spec §46 R-SEC-01 and now require 1/10/100/1000 plus 500 curve points, distinguish full-census listing, and correctly label 100-sample p99 as exploratory. A p99 used for comparison/gating needs at least 10,000 measured operations per cell and a 95% quantile confidence interval; otherwise it cannot support a superiority claim. No benchmark is claimed to have run. This is a conservative, defensible statistical gate, not a fabricated performance target.

### Remaining finding — P1 stale ownership IDs in PERF-02

PERF-02 still directs coordination to `designs/architecture.json` **ARCH-02** and `proposals/secrets-reconciliation.json` **SEC-08/REC-09**. These identifiers do not describe current packet ownership: architecture proposal IDs were retired/migrated, and the current secrets/reconciliation design explicitly says not to create SEC-08 or REC-09 duplicates (`designs/secrets.md` lines 175–179; `designs/reconciliation.md` line 201; `review/integration-draft.md` lines 62–65). The reference is particularly misleading because PERF-02 itself is the canonical freshness-policy owner. It risks sending issue creation or coordination back toward retired duplicate work.

**Exact fix:** in both `designs/performance-formats.md` PERF-02 “Dependencies” and the PERF-02 body in `proposals/performance.json`, remove the `ARCH-02`, `SEC-08`, and `REC-09` references. Point to the current consumer contracts by their current design paths (for example `designs/secrets.md` and `designs/reconciliation.md`) and state that they consume PERF-02; do not name retired issue IDs as active coordination targets. If architecture context is needed, refer to `designs/architecture.md`'s migration note, not a nonexistent `proposals/architecture.json` ARCH-02 packet.

### Updated disposition

**AGREE with the revised content, with one remaining P1 handoff correction:** retire the stale PERF-02 coordination IDs in both performance artifacts before packet handoff. I revalidated JSON parsing (six packets), stage/urgency/body consistency, the corrected SecureStore scope, PERF-01's new measurement gate, and the previously reviewed source-backed security boundaries. The original two findings are closed. No builds, Git operations, provider access, or external writes were performed in this re-review.

## Final re-review — packet handoff readiness, 2026-11-12

### Closed

- **CLOSED — urgency alignment.** Re-parsed all six objects in `proposals/performance.json` and compared structured `urgency`/`priority_band`/`stage` with each body and `designs/performance-formats.md`: PERF-01/02 450 High; PERF-03/04/06 600 High; PERF-05 1000 Standard. The repeated priority summaries and queue projections agree.
- **CLOSED — SecureStore scope.** PERF-05 (`designs/performance-formats.md` lines 126–132; JSON PERF-05) makes the bounded V2 evaluation required. Prototype/import/export/adoption remain conditional on evidence; no user decision is a prerequisite. The research doc says the same (`research/provider-formats.md` lines 13, 91), and both design and packet compare a maintained AEAD-envelope alternative or justify why SOPS/age subsumes it.
- **CLOSED — no measured-performance overclaim / soak blocking.** `designs/performance-formats.md` lines 3, 52, 85 and JSON PERF-01 state no benchmark was run, label 100 samples exploratory for p99, require at least 10,000 operations per cell plus an interval for any p99 used in comparison/gating, and disallow a superiority claim when that evidence is unavailable. The 24-hour soak is required only before claiming systemic provider slowness; it does not gate creation of the synthetic baseline or the other packets. No measurements or fabricated target are reported.
- **CLOSED — no hidden user-approval dependency for synthetic work.** PERF-01, PERF-02 and PERF-05 can proceed with their synthetic/source-only deliverables; live provider experiments and adoption/production operations remain separately approval-gated. PERF-06's configuration-owner and disposable-VM availability are explicit environment prerequisites, not an implied user authorization request.

### Open

- **OPEN — P1 stale PERF-02 ownership references.** `designs/performance-formats.md` PERF-02 “Dependencies” (line 96) and JSON `PERF-02.body` still say to coordinate with `designs/architecture.json` ARCH-02 and `proposals/secrets-reconciliation.json` SEC-08/REC-09. These are not current active packet owners: `designs/secrets.md` lines 175–179 and `designs/reconciliation.md` line 201 state PERF-02 is the canonical freshness owner and that SEC-08/REC-09 are not to be duplicated; `review/integration-draft.md` lines 62–65 documents their retirement. Also, the actual architecture packet file is `proposals/architecture.json`, and its ARCH-02 is a retired/migration alias, not an active packet. **Required:** remove those active coordination targets in both performance artifacts and point to `designs/secrets.md` / `designs/reconciliation.md` as consumers of PERF-02's contract. This is a packet-integrity risk for duplicate or impossible work routing.

- **OPEN — P2 p99 confidence procedure is not yet reproducible enough.** The 10,000-operation floor is a useful minimum and the text correctly avoids claiming it guarantees a reliable p99. However, neither `designs/performance-formats.md` line 52 / PERF-01 line 85 nor the JSON body specifies how to compute the “95% quantile confidence interval” (quantile convention, interval construction, and handling of dependence/ties). “Independently recomputed quantiles/confidence” is not a fully decidable oracle until the method is named. **Required:** specify a reproducible method (for example, a nonparametric order-statistic interval with its quantile definition and assumptions; define a dependence-aware method if operations are correlated) and retain the rule that a wide/unavailable interval cannot support a p99 superiority claim. Do not state that 10,000 alone makes a p99 reliable. This can be implemented in the benchmark issue; it does not require a user-supplied latency threshold.

### Final verdict

**AGREE with the technical design; DISAGREE that all six packets are publication-ready today.** The prior P0 and SecureStore P1 are closed, and the updated benchmark requirements do not turn the soak into a design gate or assert measured results. Close the remaining P1 stale-owner references and make the p99 confidence procedure reproducible before handoff. No benchmark, provider operation, Git action, or external write was performed in this final pass.

## Final closure re-review — 2026-11-12

I re-read the full current `designs/performance-formats.md` and `research/provider-formats.md`, parsed all six packets and checked all of the PERF-01/PERF-02 body text in `proposals/performance.json` against the design and structured metadata.

- **CLOSED — retired coordination references:** PERF-02 now names `designs/secrets.md` and `designs/reconciliation.md` as consumers and active proposals ARCH-01/SEC-03/REC-07 as implementation coordination. The stale ARCH-02/SEC-08/REC-09 identifiers are absent from the normative performance design and packet JSON.
- **CLOSED — p99 oracle:** both the design (line 52) and PERF-01 packet acceptance (design line 85 and JSON PERF-01 body) specify nearest-rank `ceil(.99n)`, one-based sorted observations with ties retained, the IID/stationary assumption, exact equal-tail binomial order-statistic CI rank conditions/endpoints, and a preregistered block-bootstrap fallback when dependence invalidates IID. They separately retain all attempts/failures, describe the 10,000-operation floor as insufficient by itself to establish interval precision, and forbid superiority claims for wide/unavailable intervals. No benchmark result is asserted.
- **CLOSED — previous findings:** urgency/body/Markdown consistency across PERF-01–06, required bounded V2 SecureStore evaluation with conditional adoption, and nonblocking treatment of the 24-hour soak were rechecked and remain resolved. Synthetic research and implementation have no hidden user-approval prerequisite; live access/deployment and adoption gates remain explicit.

### Final disposition

**AGREE — all six packets are publication-ready as proposals.** No open findings remain in the three reviewed artifacts. This is a content review only: no benchmark ran, no provider/production action occurred, no issue publication is asserted, and no Git/queue operation was performed.

## Final author-fix verification — 2026-11-12

Re-read the complete final performance design and provider research documents, parsed all six final packet objects, and reviewed each packet body against its structured fields and the design summaries. The prior findings remain **CLOSED**:

- PERF-02 references only current consumer designs (`designs/secrets.md`, `designs/reconciliation.md`) and active implementation proposals ARCH-01/SEC-03/REC-07; retired IDs ARCH-02/SEC-08/REC-09 are absent from these normative performance artifacts.
- PERF-01 now defines nearest-rank p99, exact equal-tail binomial order-statistic interval ranks/endpoints, ties, IID/stationary assumptions, and preregistered block-bootstrap fallback. The 10,000-operation floor is expressly not sufficient by itself to establish interval precision; wide/unavailable intervals cannot support superiority claims. The method appears consistently in the design and JSON packet oracle.
- Original priority and SecureStore findings remain closed: six packets’ structured urgency/band/stage and body summaries agree; the bounded V2 SecureStore evaluation is required while prototypes/adoption remain conditional.
- The 24-hour soak is only a prerequisite to claiming systemic provider slowness, not a gate for the baseline or other work. No synthetic or production benchmark result is claimed; no hidden user approval blocks the synthetic/source-only tasks. Separate publisher metadata/normalization review is outside this content review.

### Explicit final verdict

**AGREE — all six packets are publication-ready as proposals. No remaining security or semantic gap was found in the reviewed performance/cache/format scope.** This verdict does not claim that issues were published, benchmarks were run, or production operations were authorized.
