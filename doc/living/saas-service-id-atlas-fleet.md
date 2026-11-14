# SaaS service for ID / Atlas / fleet (P4)

> **DRAFT — drafted by the fleet 2026-11-12 from Drew's reprioritization order; awaiting Drew's ratification. Not Drew's own words.**

- Living document · Status: DRAFT · Program: P4 SaaS service (band 30) · Register entries: X8, X7 · Tracking epic: developing-today/code#8805 · Milestone: "SaaS service" · Label: `epic/saas-service`

## Purpose

P1 (ID), P2 (Atlas) and P3 (fleet) are binaries and local systems. None of them yet has a single design describing its **hosted service counterpart**: the always-on, multi-client, account-bearing service a user reaches when they are not running the binary themselves. This document is that design. It names the service, what it hosts for each product, and what already exists to build it from.

## Current state (from the register and goals)

- **X8 — Supabase agent schema + coordination platform** (register §6, built 2026-10-20): one Free Supabase project (`muse-dev`, developing.today org) with separable schemas — `agent` (workers/tasks/leases/events/checkpoints; `claim_task` = SELECT FOR UPDATE SKIP LOCKED + lease + event, proven live), `ci` (GitHub CI events + `recent_failures` view, fed by a real webhook on developing-today/code), `ideas` (mirror of the idea registers), `ainventory` (benchmark workbook import). Design artifacts: `~/workspace/supabase/muse-dev/migrations/0001–0005`, `~/hatch-personal/workspace/supabase/ARCHITECTURE.md`.
- **X7 — Supabase DIY backups + doctrine + $30 ceiling** (register §6, 2026-10-21): spend capped at $0 Free / $25 Pro / $30 total in practice; DIY backups stream Postgres → zstd → SHA-256 content-addressed keys in the R2 bucket `supabase-backups`, restore-tested (4,362 INSERTs regenerated exactly); doctrine: scripted, declarative, idempotent; no Terraform unless absolutely necessary.
- **Goal `supabase-muse-dev-platform-backups`** owns the muse-dev project today: schemas agent/ci/ideas/ainventory, the DIY backup system, the Turso boundary, and the phased tooling cutover.
- **The open pick (T1/X8):** the Phase 1 coordination cutover still awaits a ruling — Turso `muse-meta` coord schema vs the Supabase `agent` schema as the one coordination home. The fleet queue in practice grew from the Turso line.
- **Adjacent service pieces already live:** the Atlas multi-client search service design (A1.12, code#5350 — one Rust binary, CLI + serve modes, tailnet serve, per-client bearer tokens with a max sensitivity ceiling, read-only Turso token, port 18760 proposed); ID Phase 14 (F1.1 — hosted worlds, discovery/federation, local-first with a hosted backstop: "we can be the backstop"); Ask-the-ledger (A1.14, code#6266 — an edge Q&A surface over non-secret Atlas snapshots).

**Gap this draft fills:** the register and REPRIORITIZATION.md (P4) both record that no single design frames these pieces as one SaaS service — the service counterpart of the ID/Atlas/fleet binaries. X8 is the ancestor platform, not that framing.

## The design

1. **One service, three product faces.** The SaaS service is a single hosted platform with three faces sharing accounts, auth, quotas and operations:
   - **ID face** — hosted worlds + discovery/federation backstop per F1.1 Phase 14: local-first always; the service is the always-on rendezvous, relay and hosted-world home for people who run no server. Anything hosted must remain fully self-hostable by others, small or large.
   - **Atlas face** — the hosted query/search service per A1.12: the Rust binary's serve mode, per-client tokens with sensitivity ceilings, read-only data tokens held server-side only. The service is distribution/hot copy; the canonical corpus stays local-first (local atlas.db), never service-only.
   - **Fleet face** — coordination and control: agent workers/tasks/leases/events (the X8 `agent` schema pattern), CI event intake (the X8 `ci` schema + webhook pattern), and the PM-stack-facing status surfaces.
2. **Platform home (v1): the muse-dev Supabase project**, per X8/X7 — it already holds the agent/ci/ideas/ainventory schemas, the webhook, and the backup pipeline, inside the $30/mo ceiling. Turso remains the fleet coordination/ledger store where it already won (queue, Atlas hot copy); the T1-vs-X8 coordination-home pick is settled explicitly as a deliverable below, not left ambiguous.
3. **Data rules:** canonical data lives with the product's local-first store; the service holds hosted copies, coordination state, and non-secret snapshots. Secret-bearing rows never cross into edge/public surfaces (the A1.14 allowlist pattern). Every hosted dataset has a DIY backup path from day one (X7 pattern: content-addressed, restore-tested).
4. **Identity & access:** per-client bearer tokens with per-product sensitivity ceilings (A1.12 Q1–Q3 pattern); per-worker credentials (already on X8's in-flight list); no shared broad keys in service secrets (Cloudflare credential architecture, C2, is the model).
5. **Cost doctrine:** Free tier first; the $30/mo total Supabase ceiling (X7) binds until Drew raises it; any new paid surface names its trigger before it is enabled.

## Deliverables

- [ ] This design ratified or amended by Drew.
- [ ] Coordination-home ruling recorded (Turso coord schema vs Supabase agent schema — the T1/X8 open pick) and the loser demoted to mirror/retired in writing.
- [ ] Service face inventory: for each of ID / Atlas / fleet, the hosted endpoints that exist, their auth, and their data home — one page, kept current in this doc.
- [ ] Atlas serve mode (A1.12 steps #5352–#5356) recognised as the Atlas face's v1; its cutover criteria referenced here.
- [ ] ID hosted-worlds backstop scoped as the ID face's v1 (from the F1.1 ladder), with the self-hosting story written down.
- [ ] Backup/restore proof for every service-held dataset, X7-style, re-run on a schedule.
- [ ] Cost ledger line for the service (Supabase tier, R2, any edge spend) against the $30 ceiling, reviewed monthly.

## Open questions for Drew

1. Is Supabase muse-dev the right v1 platform home, or should the service be designed platform-neutral from the start (with muse-dev as first deployment)?
2. The coordination-home pick (T1 vs X8) — rule now, or let the fleet face's first real load decide it?
3. Does the SaaS service ever take outside users in v1, or is it Drew/fleet-only until the three faces are proven? (This decides auth, quotas and abuse posture.)
4. Is the $30/mo ceiling still the service's total budget, or does the service get its own ceiling separate from the muse-dev platform's?
