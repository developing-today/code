# Storage program (P8)

> **DRAFT — drafted by the fleet 2026-11-12 from Drew's reprioritization order; awaiting Drew's ratification. Not Drew's own words.**

- Living document · Status: DRAFT · Program: P8 Storage (band 40) · Register entries: X6, X19, X1, X17 · Tracking epic: developing-today/code#8805 · Milestone: "Storage program" · Label: `epic/storage`

## Purpose

The fleet's storage grew store by store: a cold-store on Google Drive, R2 buckets per tenant, a persistent nix store, Turso databases, Supabase backups, a nix binary cache. Each piece has its own design; **no single document allocates data across them** — what belongs where, what may grow, what gets backed up to what, and what it costs. This is that allocation policy. (Phase-2 check, 2026-11-12: the cold-store living doc `doc/living/cold-store-backup.md` and the R2 layout design beside it cover the cold-store and its layout only; nothing on file ties R2 / nix store / Turso / backups together. Hence this draft.)

## Current state (from the register and goals)

- **X6 — Cold-store backup design + nightly run** (living docs `cold-store-backup.md`, `cold-store-r2-layout-design-2026-10-25.md`): nightly cold-store backup across Turso + Supabase + D1, content-addressed, filed to Google Drive; shipped and operating (2026-10-27 Turso failure fixed by PRs #1373/#1383; clean full-house run 2026-10-28). Standing filing debt: the upstream libsql `/dump` quirk issue was never filed.
- **The R2 layout design (2026-10-25)** already settles the two big stores' jobs: **Google Drive = cold, immutable archive vault** (5 TiB AI Pro capacity, ~732 GiB account usage of which only ~178 GiB is Drive files; 3 TiB project budget; cold data bundled into sealed 4–16 GB bundles with the index held outside Drive); **R2 = hot/operational tier** (free egress, $0.015/GB-mo planning rate; keep existing tenant buckets separate — `agent-inbox-mail`, `labops-files`, `nix-cache-nars`, `supabase-backups`, `webhook-gateway-payloads`). Metadata planes (D1/Turso/Postgres) hold indexes and pointers, never bulk bytes.
- **X1 — Nix persistence**: the sandbox nix store survives recycles via the ~/nix-persist symlinked store + taint-shim, under Drew's never-delete, grow-only rule ("if I never want to delete anything can it all work?"); every safe experimental feature on. The lab and laptop have their own stores; the nix-cache (C1) is the shared binary cache on R2 — with **no retention/LRU GC yet** (~30 GB/mo churn flagged in its BENCHMARK.md).
- **X7 — Supabase backups**: content-addressed streams to R2 `supabase-backups`, restore-tested; $30 total ceiling doctrine.
- **X17 — Fleet backups provisioning (code#8584, OPEN)**: Drew ruled the scope on #8575 (all three backup forms: Turso into D1, repos to Cloudflare plus mirror plus R2); provisioning (D1/R2, mirror host/org, Turso org access) is parked awaiting Drew.
- **X19 — Backups scoping family**: hash-tree manifest for backup change detection (scoping done); Backblaze personal backup for the Framework laptop (goal, status unverified); the register's own verdict: backups is the most fragmented domain in the fleet — cold-store nightly, Supabase DIY, fleet-backups provisioning, hash-tree scoping, Backblaze — **with no single owning epic**. This program is that epic.
- **Atlas storage shape (A1)**: canonical atlas.db local on the laptop; hot set ≤8 GB in Turso (Scaler landing tier); full corpus as portable SQLite shards in R2 + Backblaze B2; Supabase Postgres canonical relational for its platform data.

## The design — allocation policy

1. **Tier table (the policy in one place):**
   | Tier                | Home                                   | Holds                                              | Rule                                                                 |
   | ------------------- | -------------------------------------- | -------------------------------------------------- | -------------------------------------------------------------------- |
   | Hot ledger          | Turso                                  | coordination, registries, mirrors, Atlas hot set   | named DB per domain; size watched (T5)                               |
   | Relational platform | Supabase Postgres (muse-dev)           | agent/ci/ideas/ainventory schemas                  | $30 ceiling; DIY backups on                                          |
   | Edge operational    | R2 (per-tenant buckets)                | backups in flight, cache nars, snapshots, payloads | buckets stay separate; no cold bulk by default                       |
   | Binary cache        | R2 `nix-cache-nars` via nix-cache      | nix nars/narinfo                                   | retention GC required (deliverable)                                  |
   | Cold archive        | Google Drive cold-store                | sealed 4–16 GB bundles + manifests                 | 3 TiB project budget; index outside Drive                            |
   | Local canonical     | laptop / lab machines                  | atlas.db, nix stores, working trees                | never the only copy of anything canonical — a backup path must exist |
   | Offsite personal    | Backblaze (laptop) + B2 (Atlas shards) | machine backup, corpus shards                      | per X19/A1                                                           |
2. **Placement rule:** new data names its tier, its backup path, and its growth expectation before it is written. "We'll figure out backups later" is not an accepted state (X17's scope ruling is the template: every store gets all its backup forms named).
3. **Never-delete vs GC:** Drew's never-delete rule binds the nix store and the cold archive (grow-only, monotonic). It does not bind caches: nix-cache nars and edge payloads are reproducible and get retention policy — the one place deletion is designed, with the rule written here before it runs.
4. **Indexes outside bulk:** every bulk store (Drive bundles, R2 buckets, B2) has its index/manifest in a queryable ledger (Turso/Postgres/D1 projection), content-addressed (SHA-256 in the key/path, X7 pattern), so stores are interchangeable and restores are provable.
5. **One inventory:** a single storage inventory (per store: bytes, growth rate, backup path, last restore proof, monthly cost) maintained in this doc and refreshed by the cold-store/usage machinery — the register's fragmentation finding is closed by keeping it in one place.

## Deliverables

- [ ] This design ratified or amended by Drew.
- [ ] Storage inventory v1: every store/bucket/DB above with bytes, growth, cost, backup path, last restore proof.
- [ ] nix-cache retention/LRU GC designed and landed (C1 follow-up), with the deletion rule stated in this doc.
- [ ] X17 provisioning decisions (code#8584) taken and the three backup forms stood up or explicitly re-parked by Drew.
- [ ] The never-filed libsql `/dump` upstream-quirk issue filed (X6 filing debt).
- [ ] Hash-tree manifest scoping (X19) decided: build under this program or close as superseded by the cold-store manifest design.
- [ ] Restore proofs on a cadence: cold-store, Supabase (X7), Atlas shards — each with a dated, recorded proof, not an assumption.
- [ ] Drive folder-level inventory of the unattributed ~178 GiB (R2 layout design's prerequisite) before any existing Drive content is moved.

## Open questions for Drew

1. Does the never-delete rule extend to R2 tenant buckets (payloads, agent inbox), or are those cache-class with retention like nix-cache?
2. X17: provision the fleet-backups D1/R2 + mirror org + Turso org access now, or keep parked?
3. Is the 3 TiB Drive project budget still the cold-store cap, and who/what may spend it?
4. Backblaze for the laptop: proceed as the offsite tier, or is B2 + Drive coverage sufficient?
