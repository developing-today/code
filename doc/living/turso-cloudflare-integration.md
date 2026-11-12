# Turso + Cloudflare integration (P6)

> **DRAFT — drafted by the fleet 2026-11-12 from Drew's reprioritization order; awaiting Drew's ratification. Not Drew's own words.**

- Living document · Status: DRAFT · Program: P6 Turso + Cloudflare (band 30) · Register entries: T1–T5, C1–C9 · Tracking epic: developing-today/code#8805 · Milestone: "Turso + Cloudflare" · Label: `epic/turso-cloudflare`

## Purpose

Turso and Cloudflare already run together all over the fleet — but each pairing was designed inside its own project (Atlas, nix-cache, the PM stack, the idea engine). This document states the **one joint pattern** so new work composes instead of re-inventing it: **the ledger lives in Turso; the edge lives on Cloudflare.**

## Current state (from the register)

**Turso side (T1–T5):**

- T1 — coordination schema v1 in the Turso DB `muse-meta` (coord_workers / coord_tasks / coord_leases / coord_events / coord_checkpoints), shipped and verified 2026-10-20; ancestor of the fleet queue.
- T2 — temporary-surplus designs (drafted; the surplus-month frame has passed; Part A patterns read as standing practice).
- T3 — the Turso + Cloudflare **idea engine**: a standing cron producing one buildable idea per run within the budget rules (free tiers first; Turso $5 + Cloudflare $5 in play; $25–30 maybe later; $75 out); 131 dated ideas 2026-10-20 → 2026-11-11, preserved in register Appendix A. Several early ideas visibly exist as live Workers (agent-inbox, webhook-gateway) though the log itself records no pickups.
- T4 — Supabase → Turso migration wave (code#6581 executing #5690–#5693: agent append streams, mirror_* analytics, sheet-derived staging, ci.events webhook ledger → Turso).
- T5 — Turso remedy ruling pending Drew (code#5468: prune/compact, scope sync, or raise the plan; live figure 2026-11-11: storage 45%, rows-read 2%).

**Cloudflare side (C1–C9):**

- C1 — nix-cache: a Nix binary cache using the full Workers set (Worker front door, R2 `nix-cache-nars`, KV INDEX, D1, Queues, daily cron), shipped and round-trip-verified 2026-10-05; zstd push path (C1.2) and presigned R2 uploads (C1.3) followed. Open follow-up: no retention/LRU GC yet.
- C2 — credential architecture: purpose-scoped tokens, never one broad key; account-wide token vaulted, never deployed.
- C4/C5 — AI Gateway `opencode-gateway` exists; AI Gateway is transport-only for the grounded search plane by Drew's narrow ruling (C5), blocked on a billing choice.
- C6 — registrar/DNS ruling: Porkbun stays the deliberate registrar layer; per-domain classification still owed.
- C7 — the lab cluster's first Cloudflare Tunnel is live (code#6455).
- A1.14 — Ask-the-ledger: the pattern already running across both platforms (Atlas snapshots in R2, Cloudflare AI Search surface, a dedicated Turso DB, hard free-tier caps with fallback to local search).

**Gap check (the reason for this draft):** no existing doc ties the two halves into one integration pattern. A1.2 (magic environment) came closest — Turso as system-of-record, Workers + MCP front door, Durable Objects for edge-visible state machines, R2 for raw artifacts — but it was never green-lit as a single build and its pieces were absorbed piecemeal. This draft extracts the pattern from what actually shipped.

## The design — "ledger in Turso, edge on Cloudflare"

1. **Turso is the ledger.** Anything that is a durable, queryable record — coordination (tasks/leases/events), registries, mirrors, idea registers, per-product metadata — lives in a named Turso database with a named owner. One main DB per domain (the T1 "one main DB for now" rule), split only on measured pressure (cf. T5).
2. **Cloudflare is the edge.** Workers are the front door (HTTP/webhook intake, query APIs, MCP surfaces); R2 holds raw artifacts, snapshots and bulk bytes the ledger points at; D1 is a projection/cache only, never the canonical record (A1.2 §12 rule); Queues + Cron drive asynchronous and scheduled edge work; Durable Objects hold only edge-visible state machines (leases, ingestion state), never canonical data.
3. **The join contract:** every edge surface names (a) its Turso DB, (b) its R2 bucket/prefix if any, (c) its failure fallback when a cap is hit (A1.14 pattern: degrade to the local/Turso-direct path, never fail open), (d) its credential scope (C2: purpose-scoped token, vaulted by name).
4. **Budget rules (from T3, standing):** free tiers first; the $5 Turso and $5 Cloudflare plans are in play; $25–30 needs a named trigger; $75 is out. Every integration ships with its cap behaviour written down before it runs.
5. **Idea flow:** the T3 engine keeps generating candidates; a candidate becomes work only on Drew's 👍, then lands as a child design under this program with the join contract filled in.

## Deliverables

- [ ] This design ratified or amended by Drew.
- [ ] Integration inventory: every live Turso↔Cloudflare pairing (Atlas publish/search, Ask-the-ledger, nix-cache, PM-stack mirror, CI webhook ledger, idea engine) listed with its join-contract fields (DB, bucket, fallback, credential scope).
- [ ] T5 remedy ruling (code#5468) recorded and applied — it sizes the ledger side of every pairing.
- [ ] T4 migration wave (#5690–#5693 via #6581) completed or explicitly re-scoped under this pattern.
- [ ] nix-cache retention/LRU GC (C1's standing follow-up) designed under the storage program (P8) and scheduled.
- [ ] The per-domain DNS/registrar classification owed under C6 delivered for Drew's picks.
- [ ] New pairings checklist (the join contract, §3) added to the idea engine's graduation path.

## Open questions for Drew

1. Is "ledger in Turso, edge on Cloudflare" the standing default for new pairings, with exceptions needing a note — or just a description of current practice?
2. T5: prune/compact, scope sync, or raise the Turso plan (code#5468)?
3. C5's billing choice for the grounded search plane: load ~$10 AI Gateway credits, BYOK, or hold?
4. Should the idea engine's 131 unreviewed ideas get a one-time pickup pass under this program, or stay parked until individually 👍'd (the engine's current design)?
