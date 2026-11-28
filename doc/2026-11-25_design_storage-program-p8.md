# Storage program (P8) — sufficient design

- Date: 2026-11-25
- Source issue: developing-today/code#8872 (design(P8): Storage program — DRAFT for Drew's ratification)
- Tracking epic: developing-today/code#8805 · Program: P8 Storage (band 40) · Label: `epic/storage`
- Queue task: 21558 (repo-health-code-pilot, slice 1 for #8872)
- Prior living DRAFT: `doc/living/storage-program.md` (2026-11-12, still DRAFT-headed)
- Related: #8584 (X17 fleet backups provisioning), #8811 (libsql `/dump` quirk filing), #8805 (tracking epic), #13873 / #13871 (AUDIT-7 tracking)
- Status: design only — no implementation in this slice

## 1. Problem + context

The fleet's storage grew store by store with no single allocation policy: cold-store on Google Drive (nightly Turso + Supabase + D1 exports, content-addressed, operating since 2026-10-28 after the #1373/#1383 Turso fix chain), five separate R2 tenant buckets (`agent-inbox-mail`, `labops-files`, `nix-cache-nars`, `supabase-backups`, `webhook-gateway-payloads`), persistent nix stores plus the shared R2 nix-cache with no retention GC (~30 GB/mo churn), Turso hot ledgers and Supabase Postgres under a $30 ceiling, Atlas shards across R2 + B2, and Backblaze personal backup (status unverified). The register's own verdict (X19) is that backups is the most fragmented domain with no single owning epic.

What breaks today, with evidence:

- New data can be written with no named tier, backup path, or growth expectation; "we'll figure out backups later" is an accepted state (`doc/living/storage-program.md` § placement rule names this as the gap).
- The 2026-10-25 R2 layout design settled Drive = cold immutable vault (5 TiB AI Pro, ~178 GiB Drive-resident, 3 TiB project budget, sealed 4–16 GB bundles, index outside Drive) and R2 = hot tier — but nothing ties R2 / nix store / Turso / backups allocation together; the living P8 doc states this outright.
- Standing debts with no owner: nix-cache retention/LRU GC unbuilt; X17 provisioning (D1/R2, mirror host/org, Turso org access, code#8584) parked; ~178 GiB unattributed Drive content un-inventoried (explicit prerequisite in the R2 layout design before moving anything); hash-tree manifest scoping undecided (build vs. superseded).
- The living P8 doc is a fleet-drafted DRAFT (header: "Not Drew's own words"). Triage (tasks 18762/20230, 2026-11-17) confirmed the doc exists and is current. The 2026-11-22 issue decision adopts it as the fleet's working design under FDR-0013; that is an agent decision, not a claim that Drew personally authored or ratified it. The living doc still has a stale DRAFT header, which S7 aligns. This dated record upgrades the design to the sufficient standard; work proceeds on the adopted working design and can be amended by the next PR.

## 2. Goals and non-goals

Goals:

- G1: One allocation policy: every data class names its tier, backup path, growth expectation, and cost home before it is written.
- G2: A single storage inventory (per store: bytes, growth, backup path, last restore proof, monthly cost), refreshed by machinery, closing the X19 fragmentation finding.
- G3: Deletion designed exactly once: never-delete binds nix store + cold archive; caches and edge payloads get explicit retention. No silent accumulation, no silent loss.
- G4: Restores provable per class: every bulk store has its index outside the bulk, content-addressed, with dated restore proofs on a cadence.
- G5: Clear the named debts with an owner and an order: inventory v1, nix-cache GC, X17 provisioning decision, libsql filing (done, #8811 — verify and close the loop), hash-tree decision, Drive inventory prerequisite.

Non-goals:

- NG1: No new stores or migrations in this program slice. No bucket consolidation "for neatness"; tenant boundaries stay.
- NG2: No per-object migration of existing Drive content before the folder-level inventory lands (R2 layout design prerequisite stands).
- NG3: This design does not ratify itself. Ratification is Drew's act; this record is the sufficient-design artifact he ratifies or amends.
- NG4: No backup-transport redesign (cold-store nightly, Supabase DIY streams stay); this program allocates and verifies, it does not rebuild transports.

## 3. Alternatives considered

- A1 — Do nothing (keep per-store docs, no program). Tradeoff: zero cost now; fragmentation compounds, debts stay ownerless, the next store failure is a loss not a restore. Rejected: the register already found this state deficient.
- A2 — Single-store consolidation (one bucket/vault for everything). Tradeoff: superficially tidy; violates the settled Drive-cold / R2-hot economics (egress, upload caps, browsability), forces migration risk onto working tenants (avatar, supabase-backups credential scope). Rejected.
- A3 — Policy-only doc with no inventory machinery (write the tier table, stop). Tradeoff: cheap; inventory rots, restore proofs stay assumptions. Rejected: G2/G4 require machinery.
- A4 — Rebuild transports first (unify backup pipelines, then allocate). Tradeoff: technically cleaner long-term; blocks allocation on a multi-month rebuild while unprotected data keeps landing. Rejected: allocate and verify first (this design), rebuild only where a slice proves it necessary.
- A5 — Chosen: allocation policy + inventory machinery + ordered debt clearance (below). Accepts living with five R2 buckets and two cold tiers in exchange for every byte having a named home, backup, and proof.

## 4. Chosen approach

### 4.1 Interfaces / contracts

- Placement contract (admission rule): a new data class lands only with `(tier, backup_path, growth_expectation, cost_home)` recorded in the inventory. Cold-store nightly, Supabase DIY, and nix-cache push paths are the first three callers to conform.
- Tier table (unchanged policy from the living DRAFT, carried forward):
  | Tier                | Home                                   | Holds                                              | Rule                                           |
  | ------------------- | -------------------------------------- | -------------------------------------------------- | ---------------------------------------------- |
  | Hot ledger          | Turso                                  | coordination, registries, mirrors, Atlas hot set   | named DB per domain; size watched              |
  | Relational platform | Supabase Postgres (muse-dev)           | agent/ci/ideas/ainventory schemas                  | $30 ceiling; DIY backups on                    |
  | Edge operational    | R2 per-tenant buckets                  | backups in flight, cache nars, snapshots, payloads | buckets stay separate; no cold bulk by default |
  | Binary cache        | R2 `nix-cache-nars` via nix-cache      | nix nars/narinfo                                   | retention GC required (slice S3)               |
  | Cold archive        | Google Drive cold-store                | sealed 4–16 GB bundles + manifests                 | 3 TiB project budget; index outside Drive      |
  | Local canonical     | laptop / lab machines                  | atlas.db, nix stores, working trees                | never the sole copy — a backup path must exist |
  | Offsite personal    | Backblaze (laptop) + B2 (Atlas shards) | machine backup, corpus shards                      | per X19/A1                                     |
- Never-delete vs GC contract: never-delete binds the nix store and the cold archive (grow-only). Caches (nix-cache nars) and edge payloads (mail/webhook staging) are cache-class with written retention. R2 tenant payload retention is decided in S1 (open question O1 below), not by accident.

### 4.2 Data model

- Inventory record per store: `{ store, tier, bytes, growth_per_month, backup_path, last_restore_proof_date, monthly_cost_usd, owner }`. Kept as a dated snapshot under `doc/living/storage-inventory/` plus the canonical table in the living program doc; refresh owned by the cold-store/usage machinery (slice S2).
- Bulk/index separation: every bulk store (Drive bundles, R2 buckets, B2) has its index in a queryable ledger (Turso/Postgres/D1 projection), content-addressed (SHA-256 in key/path, X7 pattern). Bundle model stays: `b_<first-16-hex>.tar.zst`, 4–16 GB sealed, member offsets in the manifest. No new formats in this program.

### 4.3 Security and capability analysis

- No new credential scopes in this design. Existing scopes stand: bucket-scoped R2 token for nix-cache (`nix-cache-nars` only), bucket-scoped `supabase-backups` credential, owner-authenticated Drive restore traffic, no public Drive links.
- Payload caution carries over from the R2 layout design: mail/webhook payloads can be sensitive or low-value — indefinite retention or bundling "just in case" is explicitly not the default; only items flagged for legal/financial/project-history reasons get a Drive copy.
- Manifest integrity: the manifest (DB + immutable Drive snapshots + R2 read replica) is easier to restore than the data it describes; "never delete the last verified manifest snapshot for a live bundle" is a hard rule.

## 5. Build slices (dependency-ordered, one lane/session each)

- S1 — Drive inventory prerequisite + retention answers. Owner: one lane. In: R2 layout design §1. Out: folder-level inventory of the unattributed ~178 GiB published; O1/O3 answered or explicitly deferred by the decider. Acceptance: inventory file in `doc/living/storage-inventory/`; no existing Drive content moved. Depends on: nothing. Blocks: S5.
- S2 — Storage inventory v1 + refresh machinery. Owner: one lane. In: tier table §4.1. Out: every store/bucket/DB row filled (bytes, growth, cost, backup path, last restore proof); refresh job or runbook owned by cold-store/usage machinery. Acceptance: inventory snapshot committed; refresh cadence stated. Depends on: S1 (Drive row needs its numbers). Blocks: S5/S6.
- S3 — nix-cache retention/LRU GC design + land. Owner: one lane. In: C1 follow-up, BENCHMARK churn data. Out: deletion rule stated in the living doc first, then the GC itself. Acceptance: rule text merged; GC verified against cache hit-rate; no archive-class data deleted. Depends on: S2 (cache row baselined). Blocks: none (parallel-safe after S2).
- S4 — X17 provisioning follow-up. Owner: one lane. Out: use already-authorized fleet credentials to provision the scoped D1/R2 + mirror + Turso access and stand up the three backup forms; if a capability is unavailable, record the concrete technical blocker and a runnable fallback (not an owner-answer wait). Acceptance: #8584 updated with endpoints or the exact capability gap and fallback. Depends on: S2 inventory. This remains separate from the three first-pass restore proofs in S6.
- S5 — Hash-tree manifest decision (build vs. superseded). Owner: one lane. Out: decision recorded; if build, a follow-up slice; if superseded, X19 scope closed with pointer to cold-store manifest. Acceptance: X19 register note updated. Depends on: S1/S2. Blocks: none.
- S6 — Restore-proof cadence, first pass. Owner: one lane. Out: dated restore proofs for cold-store, Supabase (X7), Atlas shards — recorded, not assumed. Acceptance: three proof records with dates linked from the living doc. Depends on: S2 (and S4 for fleet-backups scope). Blocks: program close-out.
- S7 — Align living-doc status and cross-links. Owner: one lane. Out: living header and issue link reflect the current working design; if amended, ordinary PR records the replacement. Acceptance: header/status consistent with the adopted working design and #8872 links this dated record. Depends on: S2. Blocks: program close-out.

## 6. Rollout + rollback

- Rollout: land S1→S2 first (read-only inventory work, no production writes). S3's deletion rule text lands before its code; GC rolls out with the 30-day trash grace from the R2 layout design. S4 provisions alongside, never by migrating live tenants. Decisions use the reversible defaults in §8; no task waits for a person-answer. Each slice merges to its own branch/PR with the base merged in; merge commits only.
- Rollback: S1/S2 are additive docs — rollback is revert of the snapshot commit. S3 rollback is disable-GC + restore-from-trash inside the grace window (trash is grace, not strategy; only post-grace trash is permanently removed). S4 rollback is DNS/config cutback to pre-provisioning paths; no live tenant is renamed, so names remain valid. Manifest rule from §4.3 is the backstop: no bundle loses its last verified manifest snapshot.

## 7. Verification plan

- Design-level (this slice): read back this doc against the 9-point checklist — problem with evidence (§1), goals + non-goals (§2), alternatives incl. do-nothing (§3), approach with interfaces/data model/security (§4), ordered slices with acceptance criteria (§5), rollout + rollback (§6), verification (§7), open questions with deciders (§8), dated record + issue + queue linkage (header).
- Build-level (per slice): S1 — inventory file exists, byte math reconciles with the 2026-10-25 `about.get` baseline. S2 — every tier-table row has all seven fields; refresh run demonstrated. S3 — GC run log + post-run cache substitution proof (push → delete locally → substitute from cache). S4 — endpoint + credential-scope check (bucket-scoped where applicable). S5 — decision record posted. S6 — three dated restore proofs (cold-store full-house rerun or equivalent, Supabase restore, Atlas shard restore), each with log/output linked.
- Live proof already held (regression baseline): 2026-10-28 cold-store full-house clean run; Supabase DIY restore-tested; nix-cache CA round-trip (push → delete → substitute in 1.8s). New work must not regress these.

## 8. Questions examined and decisions (deciders recorded)

- O0 — Is an additional personal ratification required before implementation? Decider: storage-program lane under FDR-0013. Decision: no; #8872 was commissioned by the reprioritization order, and the 2026-11-22 issue decision adopts it as the working design. Drew may amend it by ordinary PR/comment; no work waits for that.
- O1 — R2 payload retention. Decider: storage-program lane. Decision: treat mail/webhook payloads as cache-class by default (180 days per the R2 layout); archive only explicitly flagged items. This is reversible through a later policy PR.
- O2 — X17 provisioning. Decider: storage-program lane. Decision: keep #8584 as a separately scoped follow-up and do not provision infrastructure in this design slice; S4 records the exact existing capability and provisions only the scoped resources using already-authorized fleet credentials. This is sequencing, not a Drew-answer blocker.
- O3 — Drive project budget. Decider: storage-program lane. Decision: retain the documented 3 TiB cap as a planning constraint; inventory actual bytes before any move.
- O4 — Backblaze for the laptop. Decider: storage-program lane. Decision: keep Backblaze as the target offsite tier and make verification an S2/S6 inventory/proof item; B2 + Drive cover Atlas/cold archives, not personal laptop data.
- O5 — Hash-tree manifest. Decider: storage-program lane in S5. Decision: reuse the cold-store manifest unless evidence identifies a distinct change-detection requirement; close X19 as superseded if no such requirement is found.

## 9. Record linkage

- Dated record: this file (`doc/2026-11-25_design_storage-program-p8.md`).
- Tracking issue: developing-today/code#8872 (this design linked from the issue on completion).
- Queue item: task 21558 (repo-health-code-pilot slice 1 for #8872; complete on merge + issue link).
- Living doc disposition: `doc/living/storage-program.md` stays DRAFT-headed until O0 is answered in Drew's own words; S7 performs the mechanical flip. The stale "never filed" line about the libsql quirk is superseded by #8811 (verify-and-close in S2).
- No implementation in this slice: no transports changed, no buckets created, no lifecycle rules touched, nothing spent.
