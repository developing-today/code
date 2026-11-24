# Coverage ledger — PM-BOOK programme (#13900), lane PM-CENSUS

**Census date: 2026-11-23.** Sources: (workers) wrangler configs + `.github/workflows/`
on the laptop clone `~/git/code` (HEAD `4d937b36409`), `pm-stack/` tree, security.cab door
configs in `pkgs/desktop-commander/worker/wrangler.toml`, `pkgs/mcpx/worker/wrangler.toml`,
`home/common/default.nix`, issues #13404/#13657/#13774 as named in the brief;
(Turso) the #13844 Turso→D1 audit, merged via PR #13845 as
`doc/2026-11-23T23-15-07Z_design_turso-d1-migration-audit/2026-11-23T23-15-07Z_design_turso-d1-migration-audit.md`
(live Turso API inventory read 2026-11-23 ~23:05Z), cross-checked against repo config.
Book chapter citations are from each book's `SUMMARY.md` as it exists in the repo today:
fleet = `pkgs/fleet/book/`, atlas = `pkgs/atlas-search/book/`, id = `pkgs/id/book/`,
mcpx = `pkgs/mcpx/docs/` (note: the brief's `docs/` paths for fleet/atlas/id are the
packages' non-book docs dirs; the books live in `book/`).

## Gap summary (top)

**Workers: 22 censused — 3 COVERED, 19 GAP.**
**Workers, corrected 2026-11-23 (D17 correction, lane MISSED-WORKERS): 24 in the estate union — 3 COVERED, 21 GAP.** The original 22-row census above stands as written; it was incomplete against the platform registry — see the D17 note below the per-book table.
**Turso DBs: 50 in the estate (audit counting rule, below; confirmed live 2026-11-23, TURSO-SETTLE) — 34 COVERED, 16 GAP.**

Coverage / gaps per book:

| Book | Workers covered | Worker gaps assigned | Turso DBs covered | Turso gaps assigned |
|---|---:|---:|---:|---:|
| PM book (new, `pkgs/pm-stack/docs/`) | 0 | ~~12~~ 14 (D17 correction, 2026-11-23) | 0 | 15 |
| fleet (`pkgs/fleet/book/`) | 1 | 3 | 1 | 1 |
| atlas (`pkgs/atlas-search/book/`) | 1 | 0 | 33 | 0 |
| id (`pkgs/id/book/`) | 1 | 3 | 0 | 0 |
| mcpx (`pkgs/mcpx/docs/`) | 0 | 1 | 0 | 0 |
| **Total** | **3** | ~~19~~ **21** (D17 correction, 2026-11-23) | **34** | **16** |

**D17 CORRECTION 2026-11-23 (lane MISSED-WORKERS):** the reference writer's set
comparison (canonical disagreement D17) found the census and the platform
registry are NOT nested sets. Census 22 and `pm-stack/platform/workers.json`'s
20 share only **18** names: the census carries 4 registry-absent entries
(`mail`, exec door, pm-stack aggregator worker, pm-stack dashboard worker) and
the registry carried 2 workers with no census row and no covering-book
assignment anywhere — `cold-store-rehydrator` and `developing-today-backup`.
Both are now censused (Table A rows below, marked D17) and assigned
**GAP→PM book**, so the estate union is **24 workers (22 + 2)**, covered stays
**3**, gaps go ~~19~~ → **21**, and PM-book worker gaps go ~~12~~ → **14**.
Survey + assignment reasoning: `~/workspace/pm-book/survey/missed-workers.md`;
book entries: `~/workspace/pm-book/drafts/reference/workers-supplement.md`.
The original census text above and below is kept visible, per the
TURSO-SETTLE annotation precedent — nothing is erased.

### Full gap list, with assigned book

Workers — GAP→PM book (~~12~~ 14 — D17 correction, 2026-11-23): `decision-events`, `voice-line`, `pm-notify` (notify-relay),
`pm-console-edge`, `pkarr`, `lab-ops-hub`, `cf-container-hello`, `muse-agent-handoff`,
`github-ci`, exec door (security.cab), pm-stack aggregator worker, pm-stack dashboard worker,
`cold-store-rehydrator` (D17 addition, 2026-11-23), `developing-today-backup` (D17 addition, 2026-11-23).
Workers — GAP→fleet book (3): `desktop-commander` (commander door; programme sideline),
`fleet-hosted`, `queue-transport-alert`.
Workers — GAP→id book (3): `mail` (id-mail worker), `id-worker-poc`, `id-d1-journal-poc`.
Workers — GAP→mcpx book (1): `mcpx` (mcpx.security.cab door; programme sideline).

Turso — GAP→PM book (15): `fleet-platform`, `fleet-mirror`, `muse-meta`,
`notes-submissions`, `bedside-and-away`, `voice-line-ledger`, `great-star-lord`,
`fleet-supabase-recovery-20261117`, `fleet-supabase-runtime-stage-20261118`,
`fleet-supabase-runtime-legacy-stage-20261118`, `fleet-supabase-runtime-tx-stage-20261118`,
`fleet-platform-reconcile-stage-20261118`, `fleet-platform-batched-stage-20261118`,
`fleet-platform-short-tx-stage-20261118`, `fleet-mirror-recovery-stage-20261118`.
Turso — GAP→fleet book (1): `fleet-dispatch-jobs`.

**Counting discrepancy flagged (Turso):** the audit's headline is 50 DBs and its group
arithmetic (Atlas 33 + fleet plane 4 + `muse-meta` 1 + small product stores 4 + frozen 8)
sums to exactly 50 — that is the counting rule used here. But the audit's own §G7
enumeration (atlas + 30 `atlas-core-internal-*` + 2 `atlas-core-secret-*` +
`atlas-core-internal-content` + `atlas-ask-ledger`) expands to 35 Atlas DBs, i.e. 52
estate-wide. The audit never lists the shard names individually. Resolving which two
names are double-counted needs a live `turso db list`; until then the Atlas shard rows
below are grouped, and the covered count uses the audit's 33-DB Atlas group figure.

**RESOLVED 2026-11-23 (lane TURSO-SETTLE):** live `turso db list` on the laptop
returned exactly **50** databases — the audit's headline, group arithmetic, and
33-DB Atlas figure were all correct; no counts in this ledger change (50 total,
34 covered / 16 gap stand). The sole error was the audit's "30
`atlas-core-internal-*`" figure: the live family is 29 names matching that glob —
28 content shards plus `atlas-core-internal-content` itself — so the §G7 expansion
overcounts by 2: (1) `atlas-core-internal-content` double-counted (inside the
`atlas-core-internal-*` family and again as a separate "plus" item), and (2) a
phantom 30th internal shard that does not exist. True non-overlapping Atlas
breakdown: `atlas` 1 + internal shards (excl. content) 28 + secret shards 2 +
`atlas-core-internal-content` 1 + `atlas-ask-ledger` 1 = 33. Full definitive name
list + reconciliation: `~/workspace/pm-book/turso-estate-list.md`.

**RESOLVED 2026-11-23 (lane WORKER-EVIDENCE):** the three WEAK-EVIDENCE
worker rows in Table A are all **NOT Workers** — the exec door is a
path-scoped Cloudflare Tunnel ingress to the laptop queue broker, the
pm-stack aggregator is a laptop Bun systemd/NixOS service (port 4870), and
"the dashboard" is that same aggregator service under its other name. All
three rows are annotated in place below; the worker census therefore drops
by three (**22 → 19 censused, 19 → 16 worker gaps; PM-book worker gaps
12 → 9**), while all three documentation gaps stay assigned to the PM book.
Full evidence: `~/workspace/pm-book/survey/worker-evidence.md`.

## TABLE A — Cloudflare workers / doors

| Worker name | What it is | Covering book + chapter | Status |
|---|---|---|---|
| `atlas-search` (`pkgs/atlas-search/worker/wrangler.toml`) | Atlas search service Worker fronting the Turso corpus (D1 hot cache, #9134) | atlas — `concepts/service-and-worker.md`, `reference/worker-api.md`, `operations/services.md` | COVERED |
| `id-hosted-worker` (`pkgs/id/hosted-worker/wrangler.toml`) | ID hosted-edition Worker (UserDO, fail-closed routing) | id — `concepts/hosted.md`, `reference/hosted-worker.md` | COVERED |
| `message-bridge` (`pkgs/message-bridge/worker/wrangler.jsonc`) | Bridge messaging Worker (MessageStore DO); standalone build in flight, deployed by `message-bridge-deploy.yml` | fleet — `concepts/bridge.md`, `reference/bridge.md` (concept/CLI covered; Worker deployment detail is the fleet sideline extension) | COVERED |
| `desktop-commander` (`pkgs/desktop-commander/worker/wrangler.toml`) | Commander door: Desktop Commander MCP behind Access at commander.security.cab (+ commander-link bridge door) | none — fleet book has proxy/bridge chapters but no commander-door chapter; programme sideline assigns it | GAP→owning book (fleet) |
| `mcpx` (`pkgs/mcpx/worker/wrangler.toml`) | mcpx door: MCP gate/proxy at mcpx.security.cab (+ mcpx-link bridge door), `?ns=` enforcement | none — mcpx book has no door-Worker chapter (`remote.md` covers the daemon, not the Worker); programme sideline assigns it | GAP→owning book (mcpx) |
| `mail` (id-mail worker, `nixos/services/id-mail/worker/`; routes mail.security.cab, id-mail.security.cab, …; retires `id-mail`) | ID mail worker: address-confirmation mail for id | none — id book SUMMARY has no mail-worker chapter | GAP→owning book (id) |
| `id-worker-poc` (`pkgs/id/worker-poc/wrangler.toml`) | ID Worker proof-of-concept (World DO) | none | GAP→owning book (id) |
| `id-d1-journal-poc` (`pkgs/id/worker-poc/d1-journal-poc/wrangler.toml`) | ID D1 journal proof-of-concept Worker | none | GAP→owning book (id) |
| `fleet-hosted` (`pkgs/fleet/hosted-worker/wrangler.toml`) | Fleet hosted Worker (webhook-route dual ingestion) | none — fleet book SUMMARY has no hosted-Worker chapter | GAP→owning book (fleet) |
| `queue-transport-alert` (`pkgs/queue-transport-alert/worker/wrangler.jsonc`) | Alerts on queue-transport/broker route health via an Access service token | none | GAP→owning book (fleet) |
| `decision-events` (`pm-stack/decision-events-worker/wrangler.toml`) | PM stack decision-events Worker; writes `decision_set_events`, sibling D1 `decision-events` | none | GAP→PM book |
| `voice-line` (`pm-stack/voice-line-worker/wrangler.toml`) | PM stack voice line Worker (speech route, #9800); owns `voice-line-ledger` DB | none | GAP→PM book |
| `pm-notify` (`pm-stack/notify-relay/wrangler.toml`) | PM stack notify relay Worker (push/notification relay — it is a Worker) | none | GAP→PM book |
| `pm-console-edge` (`pm-stack/console/edge/wrangler.jsonc` + `edge/init/wrangler.jsonc`) | PM console edge Worker; route attached by host `scripts/expose.py` after health check | none | GAP→PM book |
| `pkarr` (`pkgs/pkarr-worker/wrangler.jsonc`) | Pkarr relay Worker (PkarrStore DO, rate limiters, DoH) | none | GAP→PM book |
| `lab-ops-hub` (`pkgs/lab-ops-hub/wrangler.jsonc`) | Lab ops hub Worker (LiveRoom + CrawlerContainer DOs, D1 `labops-db`, R2 `labops-files`) | none | GAP→PM book |
| `cf-container-hello` (`pkgs/cf-container-hello/wrangler.jsonc`) | Cloudflare Containers hello/probe Worker (HelloContainer, CrawlerProbe) | none | GAP→PM book |
| `muse-agent-handoff` (`scripts/muse/handoff/worker/wrangler.jsonc`) | Agent handoff Worker (Handoff DO) | none | GAP→PM book |
| `github-ci` (`pkgs/github-ci/worker/`; `github-ci-worker-tests.yml`) | GitHub CI helper Worker | none — name taken from its directory/workflow; no wrangler config at package root | GAP→PM book |
| exec door (security.cab) | Third security.cab door alongside commander/mcpx (issues #13404/#13657/#13774 family) | none — **no wrangler config found in repo census**; documented only in home config/issues | GAP→PM book — **ANNOTATED 2026-11-23 (lane WORKER-EVIDENCE): NOT-A-WORKER.** It is a path-scoped Cloudflare Tunnel ingress (`exec.security.cab/actions-exec-*` → laptop queue broker :4801, GitHub-OIDC auth at the broker, no Access app, no Worker) for `fleet actions exec` (#13774 Slice 5 deploy record). It leaves the worker census; the documentation gap stays GAP→PM book as a tunnel-door chapter. Evidence: `survey/worker-evidence.md` §1 |
| pm-stack aggregator worker | Audit (#13844) names the aggregator as a Cloudflare Worker writing `fleet-platform` via the Turso HTTP API | none — **no wrangler config in `pm-stack/aggregator/` in repo census** (deploy path unverified) | GAP→PM book — **ANNOTATED 2026-11-23 (lane WORKER-EVIDENCE): NOT-A-WORKER.** It is the laptop Bun service `pm-aggregator` (systemd/NixOS unit, `pkgs/pm-aggregator`, port 4870) that writes `fleet-platform` via the Turso HTTP API from the laptop. It leaves the worker census; the documentation gap stays GAP→PM book. Evidence: `survey/worker-evidence.md` §2 |
| pm-stack dashboard worker | Audit names the dashboard Worker as reader/writer of `fleet-platform` ingest families | none — no wrangler config found in repo census | GAP→PM book — **ANNOTATED 2026-11-23 (lane WORKER-EVIDENCE): NOT-A-WORKER — duplicate row.** "The dashboard" is the aggregator UI itself (same laptop service, port 4870); the only Worker in its public surface is `pm-console-edge`, already a separate row above. It leaves the worker census as a duplicate of the aggregator row; its documentation gap is the aggregator's gap, GAP→PM book. Evidence: `survey/worker-evidence.md` §3 |
| `cold-store-rehydrator` (`pkgs/cold-store-reaper/worker/wrangler.toml`) — **D17 addition, 2026-11-23, lane MISSED-WORKERS** | Cold-store read-path Worker: given an object's sha256, pulls its zstd frame from its Drive bundle via the D1 `cold-store-manifest` manifest + `BundleFlight` DO, verifies the hash, returns the bytes; registry (`pm-stack/platform/workers.json`, group `data`) "in service", deploy by hand, `https://cold-store-rehydrator.developing-today.workers.dev` | none — no book carries a cold-store chapter (atlas book's backup chapter covers the separate `atlas-backup` unit, not this Worker); survey `survey/missed-workers.md` | GAP→PM book |
| `developing-today-backup` (`infra/github-backup/worker/wrangler.toml`) — **D17 addition, 2026-11-23, lane MISSED-WORKERS** | GitHub backup webhook Worker: verifies GitHub App webhooks (HMAC) and queues them in KV `GHB_EVENTS` for the laptop mirror system's 5-min drainer; the rest of the system is laptop systemd timers, not a Worker; registry (group `data`) "in service", deploy by hand, `https://developing-today-backup.developing-today.workers.dev` | none — fleet book only names its App as a `gh` credential source (`reference/gh.md`), which is not Worker coverage; survey `survey/missed-workers.md` | GAP→PM book |

Notes: `pkgs/cf` is wrangler tooling (authed/secret wrappers), not a Worker. The
`cloudflare-export` item in `pm-stack/` is a laptop systemd exporter
(`cloudflare-quota-export.*`), not a Worker. Commander/mcpx "link" hostnames are bridge
doors of the same two Workers, not separate Workers.
D17 note (2026-11-23, lane MISSED-WORKERS): the two rows marked "D17 addition"
were absent from the original census because the census was built from
wrangler configs/workflows without a set comparison against
`pm-stack/platform/workers.json`; both workers do have tracked wrangler
configs (cited in their rows) and are held to the registry by
`pm-stack/platform/workers.test.ts`. Table A now carries the full estate
union: **24 rows**.

## TABLE B — Turso databases

Counting rule: one row per database named in the #13844 audit; the two Atlas shard
families are grouped rows with their audit counts. Estate total **50** per the audit's
group arithmetic (Atlas 33 + fleet plane 4 + muse-meta 1 + small stores 4 + frozen 8) —
see the discrepancy note in the gap summary. **Confirmed live 2026-11-23
(TURSO-SETTLE): total 50 verified by `turso db list`; the audit's ~~30~~ internal-shard
figure is corrected to 28 (excl. the content store's own row below) in the Atlas rows
— see the RESOLVED note in the gap summary and `turso-estate-list.md`.** Audit date **2026-11-23** (live Turso API
read ~23:05Z that day). Repo-config cross-check (Turso URLs/DB names in `pm-stack/`,
`pkgs/`, home config) found no Turso DB beyond the audit set; D1 databases named in the
audit (`atlas-hot-cache`, `decision-events`, `cold-store-manifest`) and in
`pkgs/lab-ops-hub` (`labops-db`) are D1, not Turso, and are excluded from this table.

| DB name | Purpose | Covering book + chapter | Status |
|---|---|---|---|
| `atlas` | Atlas monolith: sessions/messages/chunks, native vector + FTS5 store (7.28 GB) | atlas — `reference/data-stores.md`, `concepts/corpus-and-provenance.md` | COVERED |
| `atlas-core-internal-*` (~~×30 shards~~ ×28 shards, excl. `atlas-core-internal-content` below — corrected 2026-11-23, TURSO-SETTLE; individual names in `turso-estate-list.md`) | Atlas content shards (manifest/chunks/vector tables) | atlas — `reference/data-stores.md`, `concepts/corpus-and-provenance.md` | COVERED |
| `atlas-core-secret-*` (×2 shards) | Atlas secret-tier shards | atlas — `concepts/sensitivity.md`, `reference/data-stores.md` | COVERED |
| `atlas-core-internal-content` | Atlas shards' content store (sessions/messages text, no vectors) | atlas — `reference/data-stores.md` | COVERED |
| `atlas-ask-ledger` | Atlas ask exports/queries/usage counters (20 rows) | atlas — `reference/data-stores.md` | COVERED |
| `fleet-queue` | Fleet queue broker core (tasks/events/leases/placement, actions-exec tables) | fleet — `concepts/queue.md`, `library/queue-client.md`, `tutorials/run-the-brokers.md` | COVERED |
| `fleet-platform` | PM/dashboard plane store: agent_mirror, quota, agg, ingest families, alerts, decision events (87% of estate writes) | none | GAP→PM book |
| `fleet-mirror` | Lagging second copy of the agent_mirror dataset (175 MB) | none | GAP→PM book |
| `fleet-dispatch-jobs` | Dispatch jobs store (`jobs`, 6 rows) | none — fleet book dispatch chapters do not name this DB | GAP→owning book (fleet) |
| `muse-meta` | Coordination store (coord_events/tasks/workers, meta_notes, mail_messages) | none | GAP→PM book |
| `notes-submissions` | Notes submissions product store (<250 rows) | none | GAP→PM book |
| `bedside-and-away` | Small product store (<15 rows) | none | GAP→PM book |
| `voice-line-ledger` | Voice-line Worker ledger (4 rows) | none | GAP→PM book |
| `great-star-lord` | Third-party app's DB (Charm/Wish); standing keep-by-name, zero usage — STAYS on Turso per audit | none | GAP→PM book |
| `fleet-supabase-recovery-20261117` | Frozen 2026-11-17 Supabase recovery copy (422 MB) | none | GAP→PM book |
| `fleet-supabase-runtime-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |
| `fleet-supabase-runtime-legacy-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |
| `fleet-supabase-runtime-tx-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |
| `fleet-platform-reconcile-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |
| `fleet-platform-batched-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |
| `fleet-platform-short-tx-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |
| `fleet-mirror-recovery-stage-20261118` | Frozen 2026-11-18 staging copy | none | GAP→PM book |

Frozen-copy note: the audit recommends export-to-object-storage + delete for the 8
staging/recovery copies rather than migration; they are assigned to the PM book so the
completion test holds until that decision is taken — one grouped PM chapter can cover
all 8.

— Lane PM-CENSUS, 2026-11-23. Ledger only; no chapters written.
— Annotated 2026-11-23 by lane MISSED-WORKERS (D17 correction): two Table A
rows added, worker counts corrected 22→24 union / 19→21 gaps / PM gaps
12→14, original census text kept visible throughout.
