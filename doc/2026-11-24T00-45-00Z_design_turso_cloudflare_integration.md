# Turso + Cloudflare integration: design record (P6)

- Date: 2026-11-24 (UTC) · Lane: code-f · Queue task: 21557 · Source issue: developing-today/code#8870
- Tracking: developing-today/code#13873 (umbrella #13871), epic #8805
- Living draft it completes: `doc/living/turso-cloudflare-integration.md` (still DRAFT, awaiting Drew's ratification)
- Status: **Proposed.** Nonbinding per the decision recorded on #8870: existing integrations continue under their current designs, and no build work should treat this record as an approved standard until Drew ratifies or amends it.

This record meets the nine-point sufficient-design standard for #8870. It does not change the living draft; it adds the missing sections the draft lacks (goals, alternatives, slices, rollout, verification, open questions with deciders).

## 1. Problem and context

Turso (libSQL, hosted) and Cloudflare (Workers, R2, D1, Queues, Durable Objects, Cron) already run together across the fleet, but each pairing was designed inside its own project:

- T1 coordination schema in Turso DB `muse-meta` (coord_workers, coord_tasks, coord_leases, coord_events, coord_checkpoints), verified 2026-10-20, ancestor of the fleet queue.
- T3 idea engine: a standing cron producing one candidate idea per run, with 131 ideas dated 2026-10-20 to 2026-11-11 in register Appendix A and no recorded pickups for most of them.
- T4 Supabase → Turso migration wave (#5690–#5693 via #6581) still in progress.
- T5 Turso storage remedy pending a ruling (code#5468; 2026-11-11 figures: storage 45%, rows read 2%).
- C1 nix-cache on Workers, R2, KV, D1, Queues and cron, shipped 2026-10-05, with no retention or LRU GC yet.
- C2 credential architecture: purpose-scoped tokens, with the account-wide token vaulted but not deployed.
- C4/C5 AI Gateway `opencode-gateway` exists, but its billing choice is open.
- A1.14 Ask-the-ledger already runs on both platforms with hard free-tier caps and fallback to local search.

**What breaks today:** every new pairing re-decides where the record lives, where the edge sits, what happens when a free-tier cap is hit, and which credential it uses. The result is inconsistent fallbacks and credential scopes, and no inventory of which pairings exist. Two live failure modes are already visible: T5 storage pressure is not sized against the ledger side of every pairing, and nix-cache grows without GC.

**Evidence:** the register entries above (T1–T5, C1–C9, A1.14) as summarized in the living draft's "Current state" section, and the status of #8870 itself.

## 2. Goals and non-goals

Goals:

- G1. One joint pattern for new Turso↔Cloudflare work: **the ledger lives in Turso; the edge lives on Cloudflare.**
- G2. Every edge surface declares a join contract: Turso DB, R2 bucket/prefix (if any), cap-fallback behaviour, and credential scope.
- G3. A written inventory of every live pairing, with its join-contract fields filled in.
- G4. Budget and cap rules that every integration states before it runs.

Non-goals:

- N1. No code, schema, or infrastructure change in this record. Build slices below are separate work.
- N2. No decision on T5 (Turso plan remedy), C5 (AI Gateway billing), or C6 (registrar layer). These are listed as open questions with deciders.
- N3. No migration of the existing pairings. Current designs keep running until a slice explicitly changes them.
- N4. No change to the idea engine's graduation rule (Drew's 👍 gate) beyond adding the join-contract checklist.

## 3. Alternatives considered

- **A0 — Do nothing (status quo).** Each project keeps its own design. Cheapest now. Cost: drift in fallbacks and credentials, and no single answer to "what does Turso hold?" as the fleet grows. Rejected as the default, but remains valid for pairings that already work.
- **A1 — Cloudflare-canonical (D1 or Durable Objects as the record).** Simpler edge story, but D1 and DO storage are weaker for the cross-project queries the ledger serves, and the A1.2 §12 rule already forbids D1 as canonical. Rejected.
- **A2 — Turso-only (no Cloudflare edge).** Keeps one vendor for records, but gives up the Worker front door, MCP surfaces, R2 for bulk bytes, and Cron/Queues that already exist. Rejected.
- **A3 — Ledger in Turso, edge on Cloudflare, with a join contract (chosen).** Matches what has shipped (T1, T3, C1, A1.14). Costs one checklist per new pairing and one inventory pass.
- **A4 — Central broker service between the two.** Adds a component to operate and a new single point of failure. Rejected for now; revisit only if the inventory shows more than a handful of pairings needing shared policy.

## 4. Chosen approach

### 4.1 Roles

- **Turso is the ledger.** Durable, queryable records (coordination, registries, mirrors, idea registers, per-product metadata) live in a named Turso database with a named owner. One main database per domain. Split only on measured pressure (T5).
- **Cloudflare is the edge.** Workers serve HTTP/webhook intake, query APIs, and MCP surfaces. R2 holds raw artifacts, snapshots, and bulk bytes that the ledger points at. D1 is a projection or cache only and is never canonical. Queues and Cron drive asynchronous and scheduled work. Durable Objects hold only edge-visible state machines (leases, ingestion state), never canonical data.

### 4.2 Join contract (every edge surface)

Each surface records four fields in its design or README:

| Field        | Meaning                                                                                     |
| ------------ | ------------------------------------------------------------------------------------------- |
| `db`         | The named Turso database it reads or writes, and its owner                                  |
| `bucket`     | The R2 bucket and prefix, or `none`                                                         |
| `fallback`   | What happens at a cap or outage: degrade to the local or Turso-direct path, never fail open |
| `credential` | Purpose-scoped token name (C2), vaulted by name, never a shared broad key                   |

### 4.3 Budget and cap rules

- Free tiers first. The $5 Turso and $5 Cloudflare plans are in play.
- Moving above that to the $25–30 band needs a named trigger, written down in the design.
- $75 is out of scope for this program.
- Every integration writes its cap behaviour down before it runs.

### 4.4 Data model notes

- The ledger tables are owned by the domain that uses them. This record adds no shared table.
- R2 keys are prefixed by the owning domain, and each object's ledger row stores its key and content hash.
- D1 rows, when used, carry a `source_db` and `source_key` so a projection can be rebuilt from the ledger.

### 4.5 Security and capability notes

- Credentials follow C2: one purpose-scoped token per surface. The account-wide token stays vaulted and undeployed.
- Edge surfaces never hold a Turso admin token. They use a per-DB read or write token.
- Fallback paths must not widen access. A degraded path reads the same owner's data with the same scope.

## 5. Dependency-ordered build slices

Each slice is sized for one lane or session and has its own acceptance check.

- **S0 — Ratify or amend this record (Drew).** Depends on nothing. Acceptance: decision recorded on #8870 and the living draft updated in a PR.
- **S1 — Integration inventory.** Depends on S0 (only needs the pattern named, not ratified). Lists Atlas publish/search, Ask-the-ledger, nix-cache, PM-stack mirror, CI webhook ledger, and idea engine, each with the four join-contract fields. Acceptance: a doc table with one row per pairing, and no blank cells (a missing value is written as `unknown` with an owner).
- **S2 — T5 remedy ruling applied.** Depends on S0 and the ruling in code#5468. Sizes the ledger side. Acceptance: ruling recorded and the chosen remedy written into the living draft.
- **S3 — Join-contract checklist in the idea engine.** Depends on S0 and S1. Adds the four fields to the graduation path. Acceptance: a new idea cannot graduate without the four fields filled in.
- **S4 — Retrofit join contracts on existing pairings.** Depends on S1 and S3. One PR per pairing, adding the fields to its design. Acceptance: the inventory has no `unknown` cells left.
- **S5 — T4 migration wave completed or re-scoped.** Depends on S2. Each migrated surface passes its own join-contract check. Acceptance: #5690–#5693 closed or re-scoped with a note.
- **S6 — nix-cache retention and LRU GC design.** Depends on S2. Designed under the storage program (P8), not built here. Acceptance: a design record with a schedule and a cap behaviour.
- **S7 — C6 registrar classification.** Independent. Acceptance: per-domain classification delivered for Drew's picks.

S0 is the only gate. S1, S2, and S7 can proceed in parallel once S0 is recorded. Nothing in S1–S7 is a build on the critical path of an unratified design, so the draft's nonbinding status is respected.

## 6. Rollout and rollback

- **Rollout:** documentation first (this record and the inventory), then the checklist (S3), then per-pairing retrofits (S4). Each step is a separate PR with a merge commit.
- **Rollback:** the record is nonbinding, so rollback is a revert of the doc PR. Retrofits roll back by reverting their PR. No retrofit changes a running pairing's behaviour without its own slice.
- **Gate:** no production pairing changes credential, bucket, or DB until its own S4 PR merges.

## 7. Verification plan

- **Doc check (this record):** all nine standard sections present; every slice has an acceptance line; every open question has a decider. Checked by reading back this file.
- **Inventory check (S1):** every pairing row has four non-empty fields or an `unknown` with an owner.
- **Live proof (S4+):** for each retrofitted surface, a fallback drill (cap hit or Turso unreachable) returns the degraded path with the same scope, recorded in the PR.
- **Queue read-back:** task 21557 shows `kind: feature_slice`, `payload.slice.number: 1`, and `parent_issue: 8870`.

## 8. Open questions

Each question has a named decider.

1. Is "ledger in Turso, edge on Cloudflare" the standing default for new pairings, or only a description of current practice? **Decider: Drew (S0).**
2. T5: prune and compact, scope sync, or raise the Turso plan (code#5468)? **Decider: Drew (S2).**
3. C5 AI Gateway billing for the grounded search plane: load about $10 of credits, BYOK, or hold? **Decider: Drew.**
4. Should the 131 unreviewed idea-engine ideas get a one-time pickup pass under this program, or stay parked until each gets a 👍? **Decider: Drew (engine owner).**
5. C6 per-domain registrar classification beyond Porkbun as the layer. **Decider: Drew.**

## Links

- Source issue: developing-today/code#8870 (this record should be linked from it by comment)
- Living draft: `doc/living/turso-cloudflare-integration.md`
- Decision on #8870: nonbinding; existing integrations unchanged
