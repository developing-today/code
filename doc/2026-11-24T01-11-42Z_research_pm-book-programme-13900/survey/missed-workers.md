# Missed workers — census supplement (lane MISSED-WORKERS, 2026-11-23)

Programme #13900 (developing-today/code), coordinator PM-BOOK. Canonical
disagreement D17: the worker census (coverage ledger Table A, 22 rows) and the
platform registry (`pm-stack/platform/workers.json`, 20 workers) are **not
nested sets** — they share only 18 names. The census carries 4 registry-absent
entries (`mail`, the exec door, the pm-stack aggregator worker, the pm-stack
dashboard worker); the registry carries 2 census-absent workers,
`cold-store-rehydrator` and `developing-today-backup`, which had **no census
row and no covering-book assignment anywhere**. This survey censuses both to
the ledger standard and assigns both, closing the completion-test hole
(every worker covered).

**Method / evidence base:** laptop clone `~/git/code` at HEAD
`44aaf731ad285f25723f9198f7dabed8a2d21992`, read via `~/tools/laptop-ssh.sh`
(no fresh clone). Sources: `pm-stack/platform/workers.json` (checked
2026-11-23), each worker's wrangler config + source + README + deploy app,
the wider system each worker belongs to (package/infra tree, systemd units,
workflows), book `SUMMARY.md`/chapter greps across the fleet, atlas, id and
mcpx books, and related issues via `fleet gh api` search on the laptop.

**Corrected estate arithmetic:** census 22 ∪ registry 20, intersection 18 →
**union 24 workers**. Covered stays 3; gaps go 19 → **21** (both new workers
are gaps, both assigned below).

---

## `cold-store-rehydrator`

### Identity — what it actually is

A real, live **Cloudflare Worker** — the read path ("rehydrator") of the
Google Drive cold store. It is **not** the cold-store reaper itself: the
reaper is the Python CLI in the same package (`pkgs/cold-store-reaper/`,
`cold_store.py`) that packs files into zstd-frame bundles, uploads them to
Drive, and maintains the manifest. The Worker is the hosted half that serves
objects back out.

- Registry entry (`pm-stack/platform/workers.json`): name
  `cold-store-rehydrator`, title "Cold store rehydrator", group **`data`**,
  path `pkgs/cold-store-reaper/worker`, state **"in service"**, deploy
  **"by hand"**, workflow `null`, address
  `https://cold-store-rehydrator.developing-today.workers.dev`, does: "The
  read path of the Drive cold store: given an object's hash, pulls it out of
  its bundle, checks it and returns it."
- Wrangler config (file evidence): `pkgs/cold-store-reaper/worker/wrangler.toml`
  — `name = "cold-store-rehydrator"`, `main = "dist/worker.js"`,
  `compatibility_date = "2026-10-25"`. Header comment: "issue #5767, phase 2.
  Formerly wrangler.toml.example, when deployment was a manual REST PUT of
  the committed bundle; the deploy app (`nix run .#cold-store-rehydrator-deploy`)
  now stages this file with the flake-built bundle (package.nix) and drives
  the wrangler versions flow."
- Source: `pkgs/cold-store-reaper/worker/src/index.js` (Worker entry +
  `BundleFlight` Durable Object) and `src/decoder.js` (fzstd zstd-frame
  decoder, shared with the local proof test); worker README at
  `pkgs/cold-store-reaper/worker/README.md`.

### Surface — bindings, routes, secrets

- **Routes** (worker README + `src/index.js` header):
  - `GET /health` — no auth → `{"ok": true}`
  - `GET /fetch/<sha256>` — `Authorization: Bearer <REHYDRATOR_API_KEY>` →
    `200` + the object's original bytes (headers `X-Cold-Store-Sha256`,
    `X-Cold-Store-Bundle`, `Content-Length`); `401` without the key; `404`
    unknown sha; `502` on any Drive/decode/hash failure.
- **Bindings** (wrangler.toml):
  - D1 `MANIFEST` → database `cold-store-manifest`
    (id `15d5f460-835c-4f24-b229-222908a5b30c`) — the
    objects → members → bundles lookup; kept current by the reaper's
    `cold-store sync-manifest`. (This is the D1 named in the ledger's
    Table B note as D1, not Turso — correctly excluded from the Turso table.)
  - Durable Object `BUNDLE_FLIGHT` → class `BundleFlight`, migration tag
    `v1` (`new_sqlite_classes`) — single-flight per bundle id: concurrent
    requests for one bundle join one Drive pull; the Google access token is
    cached in DO storage until ~60 s before expiry.
- **Secrets** (named in wrangler.toml comments + `deploy.mjs`
  `WORKER_SECRETS`; set out-of-band by the deploy app from Bitwarden by
  name, never committed — names only here): `REHYDRATOR_API_KEY`,
  `GOOGLE_CLIENT_ID`, `GOOGLE_CLIENT_SECRET`, `GOOGLE_REFRESH_TOKEN`
  (the same "Cold Store Reaper" Google grant the Python reaper uses;
  `drive.file` scope only).
- **Behaviour:** look the sha up in the D1 manifest → hand the job to the
  `BundleFlight` DO keyed by bundle id → refresh-token exchange at
  `oauth2.googleapis.com/token` → Drive `Range:` pull of exactly the
  member's zstd frame → fzstd decompress → **sha256-verify before serving;
  unverified bytes are never returned** (a hash mismatch is a 502, not a
  best-effort serve). Bounded decompression by the manifest's expected
  size and shape-validated token caching are in the code with issue
  references (#6590/#6726, #6989/#8702).
- **Deploy:** the deploy app `pkgs/cold-store-reaper/worker/deploy.mjs`
  (+ `deploy.sh`/`deploy.nix`, flake package `package.nix`) — wrangler
  versions flow (upload a git-SHA-tagged version, deploy at 100% as a
  separate step, rollback = `--deploy-version <id>`), secrets (re)set from
  Bitwarden by name after a live deploy. Registry says "by hand" because
  there is no Actions workflow for it; the package's Actions workflow
  `.github/workflows/cold-store-reaper.yml` is **tests only** (Python
  suites + worker local proof, nightly + dispatch).

### Status

**Live / in service.** Registry state "in service" (checked 2026-11-23
from outside, per the registry's own method); system living doc
`doc/living/cold-store-backup.md` records the wider cold-store system as
"shipped / operating (nightly)" (register entry X6, tracking epic #8805).
Not a PoC, not retired. Caveat from the worker README: until
`sync-manifest` has run at least once against a manifest containing
verified bundles, every `/fetch/<sha>` is a well-formed 404.

### Related issues (via `fleet gh api` search, 2026-11-23)

- #5767 (open) — deploy tool: Cloudflare target class — generalize the
  `<name>-deploy` apps to all Workers (the rehydrator's deploy app is its
  phase 2).
- #9643 / PR #929 (closed) — cold-store-reaper v2: D1 manifest sync,
  DB-backup class, rehydrator Worker (the Worker's origin).
- #11252 (closed) — deploy: register lab-ops-hub, cf-container-hello,
  cold-store-rehydrator as Cloudflare targets (#5767 phase 2).
- #6590 (closed) + #6726 — unbounded zstd decompression, bounded by the
  manifest expected size.
- #6989 (closed) + #8702 — Google token response shape-trusted and cached;
  validate before caching.
- #12569 (open) — ci(cold-store-rehydrator): upload with cf, gate
  promotion on private staging.
- #13493 (open) — Platform: deploy the twelve hand-deployed Workers from
  Actions (this worker is one of the twelve).
- Wider system: #8805 (cold-store tracking epic), #8833 (open, cold-store
  backup design + nightly run — shipped, tracking home), #1427 (the
  package's test workflow).

### Assignment recommendation → **PM book**

**GAP→PM book.** Reasons:

1. The brief's default stands: it is a platform-registry worker
   (group `data`) with no owning package book — `pkgs/cold-store-reaper`
   has no book of its own, and no existing book's `SUMMARY.md` carries a
   cold-store chapter (verified by grep across fleet/atlas/id/mcpx books).
2. It does **not** clearly belong to the atlas book. The atlas book's
   backup chapter (`operations/backup-and-census.md`) covers a different
   system — the `atlas-backup` Nix/systemd unit backing up the local
   `atlas.db` — and mentions Drive only as one filing destination
   (`Cloud/cold-store/db-backups/local-atlas/`). The rehydrator serves
   the estate-wide cold store (the nightly backs up Turso + Supabase +
   D1 across the fleet, per `doc/living/cold-store-backup.md`), not an
   Atlas surface; the Atlas search Worker (`atlas-search`) is a separate
   registry worker already covered by the atlas book.
3. It does not belong to fleet/id/mcpx either: it is not a fleet package,
   not part of the id system, and not an MCP door. Its programme home is
   the PM register (X6) / PM-stack tracking — the PM book already takes
   the other cross-cutting platform/data gaps (e.g. the `fleet-platform`
   Turso store).

---

## `developing-today-backup`

### Identity — what it actually is

Also a real, live **Cloudflare Worker** — but only the **webhook half** of
a larger laptop system. The system is the GitHub estate mirror system in
`infra/github-backup/` (bare `git clone --mirror` mirrors of the whole
GitHub estate kept on the laptop at `~/backups/github`); the Worker is its
freshness signal. The rest of the system is **not** a Worker: Python
scripts in `infra/github-backup/bin/` driven by laptop **systemd user
timers** (`nixos/home/github-backup.nix`, `services.github-backup.enable`).

- Registry entry (`pm-stack/platform/workers.json`): name
  `developing-today-backup`, title "GitHub backup webhook", group
  **`data`**, path `infra/github-backup/worker`, state **"in service"**,
  deploy **"by hand"**, workflow `null`, address
  `https://developing-today-backup.developing-today.workers.dev`, does:
  "The webhook half of the GitHub mirrors: tells the mirrors what changed
  so they stay fresh."
- Wrangler config (file evidence): `infra/github-backup/worker/wrangler.toml`
  — `name = "developing-today-backup"`, `main = "src/index.js"`,
  `compatibility_date = "2024-11-01"`.
- Source: `infra/github-backup/worker/src/index.js` (dependency-free plain
  JS); tests `infra/github-backup/worker/test/index.test.mjs`; system
  README `infra/github-backup/README.md` ("This directory is the canonical
  home of the whole system") and `infra/github-backup/docs/RUNBOOK.md`.

### Surface — bindings, routes, secrets

- **Routes** (`src/index.js` header + code):
  - `POST /github` — GitHub webhook. Verifies `X-Hub-Signature-256`
    (HMAC-SHA-256 with `GITHUB_WEBHOOK_SECRET`, constant-time compare;
    body read bounded at 25 MB, #11335; fail-closed 500 when the secret
    is unconfigured, #6951); `ping` → `pong`; a payload with no
    repository → 202 with nothing queued; otherwise queues
    `{id, repo, event, action, ts}` in KV with a 7-day TTL → `202 queued`.
  - `GET /queue` — Bearer-authenticated (`DRAIN_BEARER`); returns a page
    of queued events (prefix `ev:`, limit 500, `?cursor` follows; bulk KV
    reads in chunks of 100, #5496).
  - `POST /queue/ack` — Bearer-authenticated; body `{"ids":[…]}` deletes
    those events (parallel chunked deletes, #5496); ids validated as
    non-empty strings.
  - `GET /health` — liveness → `ok`. Everything else 404.
- **Bindings** (wrangler.toml): KV namespace `GHB_EVENTS`
  (id `46b1fe46e6434c5fa509556ed3a53ea2`) — the webhook event queue the
  laptop drainer consumes. No D1, no Durable Objects, no R2.
- **Secrets** (wrangler.toml comments + `deploy.mjs` `WORKER_SECRETS`;
  by Bitwarden name only): `GITHUB_WEBHOOK_SECRET` (same value as on the
  GitHub App) and `DRAIN_BEARER` (the laptop drainer authenticates with
  it).
- **The GitHub App:** `developing-today-backup` (metadata:read +
  contents:read; push + repository events) sends the webhooks; its
  installation-token pool is separate from the personal/fleet token pool
  (`infra/github-backup/README.md`, `config.json` →
  `app.bitwarden_secret_names`).
- **The laptop half (where it runs):** systemd user timers from
  `nixos/home/github-backup.nix` — `github-backup-drain` every 5 min
  (`bin/drain_queue.py` via `systemd/drain-wrapper.sh`: drain the Worker
  queue, fetch just those mirrors, per-repo coalescing 15 min; an event
  is acked only when its fetch landed `ok`), `github-backup-fetch` daily
  (every mirror fetched regardless — total webhook loss degrades to
  daily freshness), `github-backup-reconcile` weekly (re-inventory
  GitHub, diff the tree, clone new in-scope repos — the correctness
  floor: "Webhooks may be missed; the sweep may not"),
  `github-backup-r2` daily 01:30 (verify + upload planned cold bundles
  to R2). Scope (`config.json`): all non-fork repos + forks under
  500 MB across 17 owners plus force-includes; recorded gaps are
  vertexinc (SAML) and dezren39/edsebooks (HTTP 451) — gaps, not silent
  misses.
- **Deploy:** `infra/github-backup/worker/deploy.mjs` (+ `deploy.sh`,
  `just deploy-developing-today-backup` / `nix run .#developing-today-backup-deploy`)
  — wrangler versions flow, same standard shape as the rehydrator (tool
  gap vs the cf CLI recorded on #3215); secrets (re)set from Bitwarden
  by name. No Actions deploy workflow (registry "by hand" is accurate);
  `.github/workflows/github-backup-tests.yml` runs the system's tests.

### Status

**Live / in service.** Registry state "in service" (checked 2026-11-23);
the mirror system is operating under its systemd timers (drain 5-min,
fetch daily, reconcile weekly, R2 daily). Not a PoC, not retired.

### Related issues (via `fleet gh api` search, 2026-11-23)

- #3215 (closed) — github-backup: estate mirror system (laptop mirrors +
  dedicated App + Worker webhooks + weekly sweep) — the system's origin;
  also the cf-CLI tool-gap record for its deploy.
- #3280 (closed) — github-backup: rename to developing-today-backup +
  just/nix deploy standard.
- #3326 (closed) — drainer needs an owner allowlist.
- #3474 (closed), #3287 (closed) — deploy-app argument strictness
  (`--dry-run` / unrecognized args must never perform a real deploy).
- #6849 (closed) — app_setup unguarded parsing.
- #8886 (open) — Epic: fleet GitHub Apps — permission parity, org
  coverage, registry (the backup App's family).
- #12567 (open) — github-backup: add protected cf Versions release
  workflow.
- #13493 (open) — Platform: deploy the twelve hand-deployed Workers from
  Actions (this worker is one of the twelve).
- #5767 (open) — the deploy-target class both workers' deploy apps
  belong to.

### Assignment recommendation → **PM book**

**GAP→PM book.** Reasons:

1. The brief's default stands: a platform-registry worker (group
   `data`) with no owning book — `infra/github-backup/` has no book,
   and no existing book carries a chapter for this Worker (verified by
   grep across fleet/atlas/id/mcpx books).
2. It does **not** clearly belong to the fleet book. The system is not
   a fleet package: it lives in `infra/`, runs as laptop systemd
   timers, and mirrors the whole GitHub estate (17 owners), not the
   fleet queue/placement system the fleet book documents. The fleet
   book's only mention is a credential reference — `reference/gh.md`
   names the `developing-today-backup` App installations as one token
   source for the `gh` verb — which is not coverage of the Worker, its
   routes, its KV queue, or the drain contract.
3. Nor atlas/id/mcpx: it touches none of their systems. Its natural
   home is with the other cross-cutting platform/estate-infrastructure
   gaps the PM book already owns, alongside `cold-store-rehydrator`
   (the two are the registry's remaining `data`-group pair, and the
   backup system's cold bundles even feed the same cold-store/R2
   estate the rehydrator serves from).

---

## Ledger / book actions taken with this survey

- Coverage ledger (`~/workspace/pm-book/coverage-ledger.md`): both rows
  added to Table A as GAP→PM book, annotated "D17 correction,
  2026-11-23"; counts corrected in place (original text kept visible):
  workers 22 → **24 union**, covered 3, gaps 19 → **21**, PM-book worker
  gaps 12 → **14**.
- Reference supplement
  (`~/workspace/pm-book/drafts/reference/workers-supplement.md`): both
  workers as full entries in the exact shape of
  `drafts/reference/workers.md`, plus a corrected reconciliation
  paragraph (census 22 / registry 20 / union 24, intersection 18) for
  the assembly lane to swap in. `workers.md` itself was **not** edited.
- Finding + assignments commented on #13900.

— Lane MISSED-WORKERS, 2026-11-23.
