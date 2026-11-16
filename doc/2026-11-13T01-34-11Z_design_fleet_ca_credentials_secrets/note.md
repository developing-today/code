# Fleet CA, credentials & secrets — v0.3 design mill: intake note

**Program epic:** #9155 · **Domain starters:** #9156–#9167 (native sub-issues of #9155) · **Run:** design mill, ordered by Drew 2026-11-12 19:28 CST · **Summit route:** `opencode-route/summit` (bound session; launch receipt is recorded on #9155)

This directory is the canonical in-repo home of the Fleet v0.3 credentials/secrets/CA intake. `sources/` holds the authoritative inputs **verbatim**; this note is the orchestrator's intake record: provenance, precedence, the V1 census, and the classification the Summit's build-out starts from. The Summit reads sources from here, not from chat or Drive links.

## Sources (one intake) and provenance

| File in `sources/`                                            | Origin                                                                          | Notes                                                                                                                             |
| ------------------------------------------------------------- | ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------- |
| `fleet_architecture_spec_v0_3.md`                             | v0.3 package (zip)                                                              | Governing spec: §§0–49 + Appendix A. 343,701 B.                                                                                   |
| `SECRETS_BACKEND_RESEARCH_2026-11-06.md`                      | v0.3 package                                                                    | Provider matrix, ADR-FLEET-SEC-001, R-SEC-01…18 research backlog.                                                                 |
| `DECISION_REVIEW_AND_NEXT_STEPS.md`                           | v0.3 package                                                                    | Decision review; proposals until validated against live sources.                                                                  |
| `fleet_agent_handoff_prompt_v0_3.md`                          | v0.3 package                                                                    | The eventual implementation assignment (future acceptance obligations; not this design phase's execution plan).                   |
| `source_input_3_architectural_perspective.txt`                | v0.3 package                                                                    | Third original input; §47 of the spec reconciles it — retained for traceability.                                                  |
| `README.md`, `SHA256SUMS`                                     | v0.3 package                                                                    | Package order/baseline + per-file integrity manifest.                                                                             |
| `HANDOFF_claude_fleet_ca_credentials_secrets.md`              | Google Doc `1yeSWQvMIer92zQ4lj99TsTH2oXZem0WfH-WMYKYJpJ0` (modified 2026-11-12) | The run's handoff: scope, interpretation, safety gates, MTC research note. Text export (formatting flattened).                    |
| `ADDENDUM_secrets_architecture_performance_infrastructure.md` | Google Doc `1QkQMPYGEHbjdyMBdrcSdo4i2GKxlJYK2S3PDrvCRfVw`                       | Dated refinement of the package: BWS retained, fnox near-term, root escrow to 3 named destinations, OpenBao staging. Text export. |
| `COORDINATOR_summit_handoff.md`                               | Google Doc `1AKDACXObRcUwmJIeCfLaGv6bxcWC00e3rYjOztk4qiY`                       | Execution contract for this run (issue content contract, supervision design, completion criteria). Text export.                   |
| `COMPANION_pm_charter_shared_conventions.md`                  | Google Doc `1_DQwNbuRmJnITDKKJJ7A3Q0FJa-q2rkzWlz-4PyA_iw`                       | Shared queue/dispatch conventions ONLY; the PM workstream is a separate program. Text export.                                     |

**Integrity:** the package zip (`fleet_architecture_v0_3_complete.zip`, Drive file `1cdmoblf8yQM2NuXsR2i90N45dABU1EPo`) has SHA-256 `d853a2bacf391856a863c292b21f5a224352a6681dd81fe55c422040b1fb5644` — recomputed at intake in the sandbox AND after transfer to the laptop; `sha256sum -c SHA256SUMS` passes on all five covered files in both places. Immutable snapshot staged on the laptop at `~/.local/share/fleet/intake/fleet-ca-v03-20261112/` (with an `INTAKE.md`).

**Precedence:** Drew's current instructions → the dated addendum (on its specific points) → the v0.3 spec baseline (incl. its §33.2 conflict table) → the decision review (proposal, requires validation) → historical inputs (v0.2 ledger §30, Appendix A, source input 3 where §47 supersedes). Departures from a desired outcome require an ADR + acceptance trace, never a silent drop.

## Binding constraints (Drew, 2026-11-12)

- **ML-DSA-44 is the baseline** signature scheme.
- **Merkle Tree Certificates are an OPTIONAL experiment** (#9167) — non-blocking, never a v1 dependency.
- Safety gates (from the handoff doc, restated on #9155): design + synthetic proof only in this phase. No production CA generation/signing, root escrow/export, live vault writes or reconciliation, credential rotation, DNS/trust changes, or live deployment is authorized by this handoff.

## V1 — what exists today (intake census, pinned `6a7843cae598` + live checks 2026-11-12)

- **Fleet multicall** (laptop build, source rev `743966553e`): domains queue, serve, proxy, dispatch, secrets, gh, search, laptop-agent. **No `cert`/`pki` or `generate` domain exists.** Crates: fleet, fleet-config, fleet-dispatch(-test), fleet-proxy(-proto), fleet-queue, fleet-registry, fleet-search, fleet-secrets (`pkgs/fleet/crates/`).
- **`fleet secrets get|list`**: broker-first (laptop secrets-broker), BWS fallback inside the verb (TOOLS.d/secrets.md route); name-only references; values never printed/logged; unexpectedly-empty resolution is a failure. No `put`, no sync/reconcile, no `op` adapter in the CLI.
- **Secrets broker**: in production. Known defect: #6767 (broker serves stale values after BWS rotation — read-through cache never invalidates) — directly relevant to the caching/freshness design in #9160.
- **SOPS/age**: in use in the fleet's NixOS configuration via sops-nix patterns (laptop provisioning); no Fleet-level SOPS provider interface yet.
- **GitHub/queue machinery the mill uses**: `fleet gh` (merged today, PR #9014) for issue writes with token rotation; fleet-queue (Turso `fleet-queue`) for the task item; project-reconcile boards Project #4.
- **No prior program**: searches for v0.3/PKI/ML-DSA/fnox/zz.delete/four-root issues at intake returned none — the earlier submission was cancelled and never ran (per the handoff doc). Adjacent, not duplicate: #9077 (SSO/OIDC — a relying-party/identity program; this CA program links it, does not absorb it), #6767 (broker staleness), the id program (#2056) for iroh identity points.

## Requirement classification (intake level — the Summit verifies per-requirement)

| Class                            | What falls in it (examples)                                                                                                                                                                                                                      |
| -------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| Verified-existing                | secrets `get`/`list` broker→BWS resolution; name-only secret references; fleet multicall registry/profiles; SOPS/age in NixOS provisioning (outside Fleet).                                                                                      |
| Partial                          | provider access (BWS yes; `op` only via external CLI habits, no Fleet adapter; `bw` not authorized); caching (broker cache exists, invalidation defective per #6767); event/audit (fleet has queue/outbox patterns, no secrets journal).         |
| Missing                          | canonical Credential model + field mapping; reconciliation engine + `zz.delete`; benchmark suite; fnox integration; generators (password/SSH/iroh); the entire PKI (`cert` domain): roots, issuers, leaves, CRL/publication; trust-install kits. |
| Research                         | OpenBao staging; SecureStore; Cloudflare R2 root-backup destination; federation model; workload identity (SPIFFE/SPIRE); MTC experiment (#9167).                                                                                                 |
| Unsupported (record, don't fake) | ML-DSA OCSP via OpenBao (Go OCSP lacks ML-DSA per the research dossier); universal AIA chain discovery (clients may ignore/stop early); PQ assurance from classical trust (verifier policy, not issuance).                                       |
| Approval-blocked                 | any production vault write/reconciliation; production root generation/escrow/signing; credential rotation; DNS/trust-store changes; `bw` personal-vault access (separate authorization).                                                         |

## Domain starters (Summit files granular children under each)

#9156 census & traceability · #9157 Rust crate/API/CLI architecture · #9158 canonical secrets model, adapters & custody · #9159 reconciliation, zz.delete, journal & recovery · #9160 benchmarks & fnox/SOPS-age · #9162 generators (password/SSH/iroh) · #9163 PKI roots, cross-signing & custody · #9164 issuers, leaves, AIA/CDP/CRL/OCSP & publication · #9165 ops, docs, automation & test evidence · #9166 federation & infrastructure research (nonblocking) · #9167 MTC experiment (optional, nonblocking)

## Instructions to the Summit (binding, in addition to the launch packet)

- Build the **numbered statement inventory** from ALL sources (spec §§0–49 + App A + §31 register; addendum §§1–10; handoff gates; coordinator-handoff §7 mandatory design areas) and map every statement to a domain starter, a granular child, an existing issue, or an explicit class (research / unsupported / approval-blocked). **Zero unmapped** — the coverage audit lands on #9155.
- The traceability matrix follows the coordinator handoff §8 column set and lives in this design directory (follow-ups may split it thematically; keep stable IDs and an index).
- Questions: record on #9155 as comments prefixed `QUESTION:` with the default you chose; never stall, never use the `question` tool.
- File granular issues through `fleet gh` (the sanctioned rotation path), native-parented under the domain starters, with the coordinator handoff §4.2 content contract (one deliverable per issue; requirement IDs; measurable acceptance; boundaries; tests incl. negative tests; rollback; lifecycle state).
- Keep the CA epic (#9155) separate from PM implementation; link genuine shared prerequisites only.
