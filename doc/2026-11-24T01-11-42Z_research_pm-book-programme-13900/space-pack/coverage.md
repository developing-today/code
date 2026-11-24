# Project Management Space — Coverage: the completion test

> Pack file 8 of 8 · Programme developing-today/code#13900 · Coordinator PM-BOOK
> Sources: `coverage-ledger.md` (lane PM-CENSUS, census date 2026-11-23,
> settled by lane TURSO-SETTLE 2026-11-23) + `turso-estate-list.md`
> (definitive live list, `turso db list` on the laptop, 2026-11-23).
> Book chapter citations are from each book's `SUMMARY.md` as it exists in
> the repo at census: fleet = `pkgs/fleet/book/`, atlas =
> `pkgs/atlas-search/book/`, id = `pkgs/id/book/`, mcpx = `pkgs/mcpx/docs/`,
> PM book (new) = `pm-stack/book/` per the reconciled skeleton.
>
> **Completion test:** every one of the **22 workers** and every one of the
> **50 Turso databases** has a covering book, and — for gaps — an assigned
> book. **Gap-closure status as of today (2026-11-23):** all 35 gaps
> (19 workers + 16 DBs) are **OPEN — assigned, not yet closed**. Closure
> means a chapter naming the item lands in the assigned book. The PM book is
> in drafting (skeleton reconciled 2026-11-23 ~18:45 CST; drafts under
> `~/workspace/pm-book/drafts/`); its planned covering chapters are named in
> the tables. The fleet/id/mcpx gaps are programme sidelines assigned to
> their owning books. No gap is recorded closed in any source read for this
> pack — do not present any as closed.

## Totals

| Set | Total | Covered at census | Gap (assigned) | Gap → PM book | Gap → fleet | Gap → id | Gap → mcpx |
|---|---:|---:|---:|---:|---:|---:|---:|
| Workers/doors | 22 | 3 | 19 | 12 | 3 | 3 | 1 |
| Turso DBs | 50 | 34 | 16 | 15 | 1 | 0 | 0 |
| **Combined** | **72** | **37** | **35** | **27** | **4** | **3** | **1** |

Count reconciliation notes:
- Workers: census 22 vs `platform/workers.json` 20 vs platform README 18 —
  the book reconciles all three (D6); the ledger's 22 is the test set.
- Turso: live `turso db list` returned exactly **50** (TURSO-SETTLE,
  2026-11-23). The #13844 audit's 50-vs-52 discrepancy is resolved: its "30
  `atlas-core-internal-*`" figure was wrong — the live family is 29 names
  matching that glob (28 content shards + `atlas-core-internal-content`
  itself, which the audit also double-counted as a separate item). True
  Atlas breakdown: `atlas` 1 + internal shards (excl. content) 28 + secret
  shards 2 + `atlas-core-internal-content` 1 + `atlas-ask-ledger` 1 = **33**.
  All 50 DBs sit in the single Turso group `great-star-lord`; hostnames are
  all `…-dezren39.aws-us-east-1.turso.io` (`turso-estate-list.md`).
- D1 databases named in the audit (`atlas-hot-cache`, `decision-events`,
  `cold-store-manifest`) and in `pkgs/lab-ops-hub` (`labops-db`) are D1, not
  Turso, and are excluded from Table B.

---

## Table A — All 22 workers/doors

Status key: **COVERED** = a covering chapter exists at census ·
**GAP→\<book\>** = assigned, OPEN as of 2026-11-23. Planned PM-book chapter
names are from `skeleton.md` (`reference/workers.md` covers the 12 PM-assigned
workers in full; the estate's other 10 are named there with their covering
books).

| # | Worker | Covering book + chapter | Status (2026-11-23) |
|---:|---|---|---|
| 1 | `atlas-search` (`pkgs/atlas-search/worker/`) | atlas — `concepts/service-and-worker.md`, `reference/worker-api.md`, `operations/services.md` | COVERED |
| 2 | `id-hosted-worker` (`pkgs/id/hosted-worker/`) | id — `concepts/hosted.md`, `reference/hosted-worker.md` | COVERED |
| 3 | `message-bridge` (`pkgs/message-bridge/worker/`) | fleet — `concepts/bridge.md`, `reference/bridge.md` (concept/CLI covered; Worker deployment detail is the fleet sideline extension) | COVERED |
| 4 | `desktop-commander` (commander door, `commander.security.cab`) | none — fleet book has proxy/bridge chapters but no commander-door chapter | GAP→fleet — OPEN |
| 5 | `mcpx` (mcpx door, `mcpx.security.cab`) | none — mcpx book has no door-Worker chapter (`remote.md` covers the daemon, not the Worker) | GAP→mcpx — OPEN |
| 6 | `mail` (id-mail worker; `mail.security.cab`, `id-mail.security.cab`) | none — id book SUMMARY has no mail-worker chapter | GAP→id — OPEN |
| 7 | `id-worker-poc` (`pkgs/id/worker-poc/`) | none | GAP→id — OPEN |
| 8 | `id-d1-journal-poc` (`pkgs/id/worker-poc/d1-journal-poc/`) | none | GAP→id — OPEN |
| 9 | `fleet-hosted` (`pkgs/fleet/hosted-worker/`) | none — fleet book SUMMARY has no hosted-Worker chapter | GAP→fleet — OPEN |
| 10 | `queue-transport-alert` (`pkgs/queue-transport-alert/worker/`) | none | GAP→fleet — OPEN |
| 11 | `decision-events` (`pm-stack/decision-events-worker/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 12 | `voice-line` (`pm-stack/voice-line-worker/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 13 | `pm-notify` (notify-relay, `pm-stack/notify-relay/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 14 | `pm-console-edge` (`pm-stack/console/edge/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 15 | `pkarr` (`pkgs/pkarr-worker/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 16 | `lab-ops-hub` (`pkgs/lab-ops-hub/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 17 | `cf-container-hello` (`pkgs/cf-container-hello/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 18 | `muse-agent-handoff` (`scripts/muse/handoff/worker/`) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 19 | `github-ci` (`pkgs/github-ci/worker/`; no wrangler config at package root) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 20 | exec door (security.cab) — no wrangler config found in repo census; issues #13404/#13657/#13774 family | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 21 | pm-stack aggregator worker — named by audit #13844; no wrangler config in `pm-stack/aggregator/` (deploy path unverified) | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |
| 22 | pm-stack dashboard worker — named by audit #13844; no wrangler config found in repo census | none → PM book, planned `reference/workers.md` | GAP→PM book — OPEN |

Notes (from the ledger): `pkgs/cf` is wrangler tooling, not a Worker. The
`cloudflare-export` item in `pm-stack/` is a laptop systemd exporter, not a
Worker. Commander/mcpx "link" hostnames are bridge doors of the same two
Workers, not separate Workers.

---

## Table B — All 50 Turso databases (individual names, definitive list)

Status key as Table A. Planned PM-book chapter: `reference/data-stores.md`
(PM's 15 in full, incl. the frozen eight as a group; the covered 35 named
with their covering chapters — `skeleton.md`).

### B1. Atlas group — 33 DBs, all COVERED (atlas book)

Covering chapters (ledger, grouped rows): shards →
`reference/data-stores.md` + `concepts/corpus-and-provenance.md`; secret
shards → `concepts/sensitivity.md` + `reference/data-stores.md`.

| # | Database | Covering book + chapter | Status |
|---:|---|---|---|
| 1 | `atlas` | atlas — `reference/data-stores.md`, `concepts/corpus-and-provenance.md` (monolith: sessions/messages/chunks, native vector + FTS5, 7.28 GB) | COVERED |
| 2 | `atlas-ask-ledger` | atlas — `reference/data-stores.md` (ask exports/queries/usage counters, 20 rows) | COVERED |
| 3 | `atlas-core-internal-agent-tooling-01` | atlas — `reference/data-stores.md`, `concepts/corpus-and-provenance.md` | COVERED |
| 4 | `atlas-core-internal-agent-tooling-02` | atlas — as above | COVERED |
| 5 | `atlas-core-internal-agent-tooling-03` | atlas — as above | COVERED |
| 6 | `atlas-core-internal-agent-tooling-04` | atlas — as above | COVERED |
| 7 | `atlas-core-internal-cloudflare-01` | atlas — as above | COVERED |
| 8 | `atlas-core-internal-content` | atlas — `reference/data-stores.md` (shards' content store: sessions/messages text, no vectors) | COVERED |
| 9 | `atlas-core-internal-fleet-repos-01` | atlas — `reference/data-stores.md`, `concepts/corpus-and-provenance.md` | COVERED |
| 10 | `atlas-core-internal-fleet-repos-02` | atlas — as above | COVERED |
| 11 | `atlas-core-internal-fleet-repos-03` | atlas — as above | COVERED |
| 12 | `atlas-core-internal-home-lab-01` | atlas — as above | COVERED |
| 13 | `atlas-core-internal-home-lab-02` | atlas — as above | COVERED |
| 14 | `atlas-core-internal-infra-nix-01` | atlas — as above | COVERED |
| 15 | `atlas-core-internal-infra-nix-02` | atlas — as above | COVERED |
| 16 | `atlas-core-internal-media-01` | atlas — as above | COVERED |
| 17 | `atlas-core-internal-misc-01` | atlas — as above | COVERED |
| 18 | `atlas-core-internal-models-and-plans-01` | atlas — as above | COVERED |
| 19 | `atlas-core-internal-models-and-plans-02` | atlas — as above | COVERED |
| 20 | `atlas-core-internal-people-pets-01` | atlas — as above | COVERED |
| 21 | `atlas-core-internal-personal-admin-01` | atlas — as above | COVERED |
| 22 | `atlas-core-internal-personal-admin-02` | atlas — as above | COVERED |
| 23 | `atlas-core-internal-personal-admin-03` | atlas — as above | COVERED |
| 24 | `atlas-core-internal-personal-admin-04` | atlas — as above | COVERED |
| 25 | `atlas-core-internal-security-01` | atlas — as above | COVERED |
| 26 | `atlas-core-internal-work-engineering-01` | atlas — as above | COVERED |
| 27 | `atlas-core-internal-work-engineering-02` | atlas — as above | COVERED |
| 28 | `atlas-core-internal-work-engineering-03` | atlas — as above | COVERED |
| 29 | `atlas-core-internal-work-engineering-04` | atlas — as above | COVERED |
| 30 | `atlas-core-internal-work-engineering-05` | atlas — as above | COVERED |
| 31 | `atlas-core-internal-work-engineering-06` | atlas — as above | COVERED |
| 32 | `atlas-core-secret-content` | atlas — `concepts/sensitivity.md`, `reference/data-stores.md` | COVERED |
| 33 | `atlas-core-secret-main-01` | atlas — `concepts/sensitivity.md`, `reference/data-stores.md` | COVERED |

Shard-family check: internal shards excl. content = rows 3–7, 9–31 = **28**;
plus content (row 8) = 29 names matching `atlas-core-internal-*`; plus
`atlas` + 2 secret + `atlas-ask-ledger` = **33**. ✓ (TURSO-SETTLE reconciliation.)

### B2. Fleet plane — 4 DBs

| # | Database | Purpose (ledger) | Covering book + chapter | Status (2026-11-23) |
|---:|---|---|---|---|
| 34 | `fleet-queue` | Fleet queue broker core (tasks/events/leases/placement, actions-exec tables) | fleet — `concepts/queue.md`, `library/queue-client.md`, `tutorials/run-the-brokers.md` | COVERED |
| 35 | `fleet-platform` | PM/dashboard plane store: agent_mirror, quota, agg, ingest families, alerts, decision events (87% of estate writes) | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |
| 36 | `fleet-mirror` | Lagging second copy of the agent_mirror dataset (175 MB) | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |
| 37 | `fleet-dispatch-jobs` | Dispatch jobs store (`jobs`, 6 rows) | none — fleet book dispatch chapters do not name this DB | GAP→fleet — OPEN |

### B3. Coordination — 1 DB

| # | Database | Purpose (ledger) | Covering book + chapter | Status (2026-11-23) |
|---:|---|---|---|---|
| 38 | `muse-meta` | Coordination store (coord_events/tasks/workers, meta_notes, mail_messages) | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |

### B4. Small product stores — 4 DBs

| # | Database | Purpose (ledger) | Covering book + chapter | Status (2026-11-23) |
|---:|---|---|---|---|
| 39 | `notes-submissions` | Notes submissions product store (<250 rows) | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |
| 40 | `bedside-and-away` | Small product store (<15 rows) | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |
| 41 | `voice-line-ledger` | Voice-line Worker ledger (4 rows) | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |
| 42 | `great-star-lord` | Third-party app's DB (Charm/Wish); standing keep-by-name, zero usage — STAYS on Turso per audit #13844 | none → PM book, planned `reference/data-stores.md` | GAP→PM book — OPEN |

### B5. Frozen staging/recovery copies — 8 DBs, all GAP→PM book — OPEN

The audit recommends export-to-object-storage + delete for these 8 rather
than migration; they are assigned to the PM book so the completion test holds
until that decision is taken — one grouped PM chapter can cover all 8
(planned: `reference/data-stores.md`, frozen-eight group).

| # | Database | Purpose (ledger) | Status (2026-11-23) |
|---:|---|---|---|
| 43 | `fleet-supabase-recovery-20261117` | Frozen 2026-11-17 Supabase recovery copy (422 MB) | GAP→PM book — OPEN |
| 44 | `fleet-supabase-runtime-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |
| 45 | `fleet-supabase-runtime-legacy-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |
| 46 | `fleet-supabase-runtime-tx-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |
| 47 | `fleet-platform-reconcile-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |
| 48 | `fleet-platform-batched-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |
| 49 | `fleet-platform-short-tx-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |
| 50 | `fleet-mirror-recovery-stage-20261118` | Frozen 2026-11-18 staging copy | GAP→PM book — OPEN |

**Table B check:** 33 (Atlas) + 4 (fleet plane) + 1 (coordination) + 4
(small stores) + 8 (frozen) = **50** ✓ · Covered 34 (33 Atlas + `fleet-queue`)
· Gap 16 (15 PM book + 1 fleet book) ✓ — matches the settled ledger.

---

## Gap-closure status as of today (2026-11-23) — summary

| Assigned book | Worker gaps | DB gaps | Total open | Closure condition |
|---|---:|---:|---:|---|
| PM book (new) | 12 | 15 | **27 OPEN** | Chapters land in `pm-stack/book/`: `reference/workers.md` (12 in full) + `reference/data-stores.md` (15 in full, frozen eight as a group). Book in drafting; skeleton reconciled 2026-11-23. |
| fleet book | 3 | 1 | **4 OPEN** | Commander-door, hosted-Worker, queue-transport-alert chapters + a dispatch chapter naming `fleet-dispatch-jobs`, in `pkgs/fleet/book/`. Programme sidelines. |
| id book | 3 | 0 | **3 OPEN** | Mail-worker chapter + the two PoC Workers, in `pkgs/id/book/`. Programme sidelines. |
| mcpx book | 1 | 0 | **1 OPEN** | Door-Worker chapter (distinct from `remote.md`'s daemon coverage), in `pkgs/mcpx/docs/`. Programme sideline. |
| **Total** | **19** | **16** | **35 OPEN, 0 closed** | — |

No source read for this pack records any of the 35 gaps as closed. The
Space build must render all 35 as assigned-and-open, with the assigned book
named — not as covered.
