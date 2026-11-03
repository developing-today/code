# PM-stack shared schema — inventory + schema map (slice 1)

Tracks issue [#1744](https://github.com/developing-today/code/issues/1744)
(acceptance checkbox 1). Map only — no service migration in this slice.
Verified against origin/main `3001d371b` (2026-11-03); every entry cites
the code that owns the store.

Target table groups (per #1744 proposal): `pmp_` (projector/link
registry), `agg_` (aggregator stores), `shared_` entity tables
(`shared_items`, `shared_links`, `shared_comments`, `shared_events`)
written once and read by every service.

| # | Store (file / table) | Owner service | Rebuildable? | Target group |
|---|---|---|---|---|
| 1 | Aggregator JSON files, cwd-relative (`pm-stack/aggregator/src/config.ts:109-131`): `fleet-feed.json`, `comments.json`, `roadmap-annotations.json`, `actions.json`, `views.json`, `registry.json`, `quota-current.json`, `coverage-snapshot.json`, `snapshots/` | aggregator | Yes — derived views over GitHub/Kaneo/Grist/projector sources; snapshots regenerable point-in-time copies; quota/coverage files ephemeral | `agg_` (`agg_feed`, `agg_comments`, `agg_actions`, `agg_views`, `agg_registry_cache`, `agg_snapshots`) |
| 2 | Aggregator drill state `~/.local/state/pm-drills/` (drill scripts under `pm-stack/aggregator/scripts/`, units `pm-stack/aggregator/systemd/`) | aggregator | Yes — ephemeral drill run state | `agg_` (`agg_drills`) or drop (ephemeral, no shared table needed) |
| 3 | Aggregator external inputs (read-only): `~/atlas/cloudflare-export/latest.json`, `~/atlas/supabase-export/latest.json`, quota registry md, openchamber `state.json` (`config.ts:118-133`) | aggregator (reader only) | n/a (not owned) | n/a — stay as external feeds, not shared tables |
| 4 | Projector SQLite (`pm-stack/projector/src/store.ts:14`, default `file:data/projector.db`, override `DATABASE_URL`): `pmp_items`, `pmp_comments` | projector | Yes — rebuilt from GitHub (`reconcile.ts`, `github.ts`) | `pmp_` as-is; items/comments also projected into `shared_items` / `shared_comments` |
| 5 | Projector SQLite: `pmp_links` | projector | **No — the only non-rebuildable store in the stack** (human/agent link writes). Gets the backup legs (fleet backup + `data/` volume snapshot) | `pmp_` as-is; canonical source for `shared_links` |
| 6 | Projector SQLite: `pmp_deliveries`, `pmp_delivery_errors`, `pmp_sync_state`, `pmp_push_events` (`schema.sql:98-156`) | projector | Yes — operational/delivery bookkeeping, regenerable | `pmp_` as-is (operational, not shared entities) |
| 7 | Projector views `pmp_v_links_items`, `pmp_v_subject_activity` (`schema.sql:158-176`, `projector/README.md:33-34,106-109`) | projector | Yes — pure projections over 4–6 | Keep as views over the shared schema post-migration |
| 8 | Grist roadmap document (`GRIST_ROADMAP_DOC`, default `czYHYKgG9SHcm9CPwiKEq1`, `config.ts:148-153`) + registers, stored in Docker named volume `grist_data:/persist` (`pm-stack/grist/compose.yml:40-41`, `deploy.sh:27`) | grist | **No for user-edited doc content** (live roadmap edits); schema seedable via `pm-stack/grist/seed.sh` | Roadmap rows merge into `shared_items`; registers stay Grist-local until a later slice designs them |
| 9 | Kaneo PostgreSQL (`pm-stack/compose.yml:15-27`, `postgres_data:/var/lib/postgresql/data`, db `kaneo`) + repo config `pm-stack/kaneo-projects.json` (versioned, slug→repo map) | kaneo | **No for task/project rows** (primary user data; GitHub per-repo sync via `connect-repo.sh` is partial). Config file yes (git) | Tasks/projects merge into `shared_items` + `shared_events`; `kaneo-projects.json` stays the repo-mapping config |
| 10 | Pad SQLite `/data/pad.db` in Docker volume `pad_data` (`pm-stack/pad/compose.yml:32-35`) | pad | **No** (user notes) — needs a backup leg | Pad notes merge into `shared_items` (or a `shared_notes` entity if the pad slice wants it) |
| 11 | ready-for-agent SQLite `~/.local/share/ready-for-agent/` (upstream default, `pm-stack/ready-for-agent/systemd/ready-for-agent.service:13-15`) | ready-for-agent | Mostly yes — dispatch queue derived from GitHub labels; in-flight dispatch rows ephemeral | `shared_events` (dispatch queue as events, not a private DB) |

## Notes

- Registry backup legs (store 5): the fleet's existing backup covers the
  laptop file today; post-migration the shared database file gets the
  same legs (local file + fleet backup). Non-rebuildable Grist/Kaneo/Pad
  content (stores 8–10) rides along once merged into shared tables.
- Sibling design inputs: #1738 (roadmap files) reads `shared_items`;
  #1739 (event log) reads `shared_events`; #1734 (per-source lag) is
  surfaced by change polling on the shared DB; #1288 (Grist live-apply)
  consumes the shared layer instead of per-service polling.
- Out of scope for later slices (not stores): `cloudflare-export`,
  `supabase-export`, `meta-exporter` (export legs), `opencode-claimant`
  (queue worker), `dispatch` (stateless scripts).
