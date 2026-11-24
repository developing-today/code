## PM-CENSUS — coverage ledger posted

Lane PM-CENSUS (coordinator: PM-BOOK) has built the coverage ledger — the programme's completion-test evidence. Full ledger: `~/workspace/pm-book/coverage-ledger.md` (sandbox workspace, PM-BOOK lane files).

**Census date: 2026-11-23.** Worker sources: wrangler configs + workflows on the laptop clone `~/git/code`, pm-stack tree, security.cab door configs. Turso source: the #13844 audit record in `doc/2026-11-23T23-15-07Z_design_turso-d1-migration-audit/` (PR #13845), cross-checked against repo config (no extra Turso DBs found; the D1 stores `atlas-hot-cache`, `decision-events`, `cold-store-manifest`, `labops-db` are excluded — not Turso).

### Counts

**Workers: 22 censused — 3 covered, 19 gap.**
**Turso DBs: 50 (audit counting) — 34 covered, 16 gap.**

| Book | Workers covered | Worker gaps | Turso covered | Turso gaps |
|---|---:|---:|---:|---:|
| PM book (new) | 0 | 12 | 0 | 15 |
| fleet | 1 | 3 | 1 | 1 |
| atlas | 1 | 0 | 33 | 0 |
| id | 1 | 3 | 0 | 0 |
| mcpx | 0 | 1 | 0 | 0 |

Covered workers: `atlas-search` (atlas book, service-and-worker/worker-api), `id-hosted-worker` (id book, hosted/hosted-worker), `message-bridge` (fleet book, bridge chapters — Worker deployment detail remains the fleet sideline extension).

### Full gap list

**Workers → PM book (12):** `decision-events`, `voice-line`, `pm-notify` (notify-relay), `pm-console-edge`, `pkarr`, `lab-ops-hub`, `cf-container-hello`, `muse-agent-handoff`, `github-ci`, exec door (security.cab — no wrangler config in repo; issues/home config only), pm-stack aggregator worker, pm-stack dashboard worker (both audit-named as Workers; no wrangler config found in repo — deploy path unverified).
**Workers → fleet book (3):** `desktop-commander` (commander door — sideline), `fleet-hosted`, `queue-transport-alert`.
**Workers → id book (3):** `mail` (id-mail worker), `id-worker-poc`, `id-d1-journal-poc`.
**Workers → mcpx book (1):** `mcpx` (mcpx.security.cab door — sideline).

**Turso → PM book (15):** `fleet-platform`, `fleet-mirror`, `muse-meta`, `notes-submissions`, `bedside-and-away`, `voice-line-ledger`, `great-star-lord`, and the 8 frozen 2026-11-17/18 staging/recovery copies (`fleet-supabase-recovery-20261117`, `fleet-supabase-runtime-stage-20261118`, `fleet-supabase-runtime-legacy-stage-20261118`, `fleet-supabase-runtime-tx-stage-20261118`, `fleet-platform-reconcile-stage-20261118`, `fleet-platform-batched-stage-20261118`, `fleet-platform-short-tx-stage-20261118`, `fleet-mirror-recovery-stage-20261118`).
**Turso → fleet book (1):** `fleet-dispatch-jobs`.

### Flagged discrepancy

The audit's headline is 50 DBs and its group arithmetic (Atlas 33 + fleet plane 4 + muse-meta 1 + small stores 4 + frozen 8) sums to 50 — the ledger uses that as the counting rule. But its §G7 enumeration (atlas + 30 internal shards + 2 secret shards + `atlas-core-internal-content` + `atlas-ask-ledger`) expands to 35 Atlas DBs / 52 estate-wide, and shard names are never listed individually. Settling it needs a live `turso db list`. Detail in the ledger.

— Lane PM-CENSUS, 2026-11-23. Ledger only; no chapters written.
