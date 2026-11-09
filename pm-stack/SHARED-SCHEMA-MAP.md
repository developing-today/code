# PM-stack shared schema — inventory + schema map (slice 1)

Tracks [#1744](https://github.com/developing-today/code/issues/1744),
acceptance checkbox 1. Inventory only: no store or service is migrated
here. Code citations were checked at current `origin/main` `2be05f724`
and remain at this branch's HEAD.

## How to read the map

The target groups from #1744 are `pmp_` projector/link-registry tables,
`agg_` aggregator-owned stores, and shared entity tables
`shared_items`, `shared_links`, `shared_comments`, and `shared_events`.
These are logical target groups, not approval to combine native service
databases. The PM-stack layer design says each product layer owns its
source of truth and the projector is glue, not a source of truth
([ownership and data flow](pad/seed/plans-pm-stack-design.md#L23-L50)).
The later #3142 decision supersedes #1744's one-local-libSQL default:
start new PM state in Supabase `muse-dev2`, and keep Kaneo, Grist, and
Pad databases separate unless a supported shared-database mode is proven
([durability decision](../doc/2026-11-01T21-12-00Z_pm-stack-durability-host-independence/2026-11-01T21-12-00Z_pm-stack-durability-host-independence.md#L7-L22)).
Rows below therefore name candidate entity groups while preserving each
service's current source of truth and native restore path.

### Aggregator-owned stores

| Current store (file / table; code citation) | Owner service | Rebuildable? | Target table group |
|---|---|---|---|
| `fleet-feed.json` at `~/.local/state/pm-aggregator/` ([Nix collector paths](../nixos/services/pm-stack/default.nix#L221-L254); [collector writer](aggregator/scripts/collect-fleet-feed.ts#L920-L948)) | fleet-feed-collector / aggregator | Yes, as a derived feed from upstream APIs and producer stores; historical versions are not equivalent to current refreshes | `agg_` feed/cache |
| `comments.json` ([store contract](aggregator/src/comments.ts#L1-L7), [default path](aggregator/src/config.ts#L108-L110)) | aggregator | No; dashboard-native notes/messages and posting/triage status are not recovered from GitHub | `shared_comments` with origin/kind preserved |
| `actions.json` ([store contract and append-only audit](aggregator/src/actions.ts#L1-L11), [record retention](aggregator/src/actions.ts#L47-L55)) | aggregator | No; mediated actions and outcomes are an audit/source record even if some effects appear upstream | `agg_` actions and `shared_events` for cross-service outcomes |
| `roadmap-annotations.json` ([annotation model](aggregator/src/roadmap-annotations.ts#L1-L35); [deployed path](../nixos/services/pm-stack/default.nix#L142-L146)) | aggregator | No; attributed annotations and row aliases are not derivable from Grist rows | `agg_` roadmap annotations |
| `views.json` ([default path](aggregator/src/config.ts#L108-L114); [saved-view contract](aggregator/src/views.ts#L1-L21)) | aggregator | No; saved-view metadata/defaults are user preferences. Current installer does not set `PM_AGG_VIEWS_FILE`, so default is checkout-relative; an external `.env` override is possible ([rendered env](aggregator/install.sh#L99-L118), [working directory](../nixos/services/pm-stack/default.nix#L127-L131)) | `agg_` views/preferences |
| Aggregator `registry.json` ([identity merge](aggregator/src/sources/live.ts#L736-L775); [deployed path](aggregator/install.sh#L99-L108)) | aggregator | Partial; live/feed records can be re-collected, but the durable identity history outlives the feed attention window | `agg_` session/identity registry. This is **not** projector `pmp_links` |
| `snapshots/usage-YYYY-MM.jsonl` and `.rollup.jsonl` ([retention and compaction](aggregator/src/snapshots.ts#L1-L18); [deployed directory](aggregator/install.sh#L104-L109)) | aggregator and usage writers | Partial; current readings can be recollected, but historical observations/rollups cannot be reconstructed | `agg_` usage history; shared event/measurement representation is a later design choice |
| `quota-current.json` ([gatherer paths](../nixos/services/pm-stack/default.nix#L202-L217); [aggregator default](aggregator/src/config.ts#L124-L129)) | quota-gatherer / aggregator | Yes for latest values; it's a refreshable last-known-good cache | `agg_` quota cache |
| `quota-gatherer-heartbeat.json` and `quota-gatherer-state.json` ([paths](../nixos/services/pm-stack/default.nix#L202-L217)) | quota-gatherer | Yes; operational heartbeat/cursor state can be regenerated, though losing the cursor may duplicate snapshot writes | `agg_` operational state |
| `coverage-snapshot.json` ([deployed path](../nixos/services/pm-stack/default.nix#L132-L146); [store](aggregator/src/coverage.ts#L213-L243)) | aggregator | Partial; current coverage is recomputable but the stored comparison baseline/history is not | `agg_` coverage baseline/cache |
| `usage-snapshot-outbox.jsonl` ([producer handoff](../nixos/services/pm-stack/default.nix#L242-L244); [drain contract](aggregator/scripts/collect-fleet-feed.ts#L62-L64)) | usage writers / fleet-feed-collector | No for records not yet drained; source writers may not retain them | `shared_events` or `agg_` ingestion, final representation deferred |
| `aa-last-good.json` beside the feed ([path and reuse contract](aggregator/scripts/collect-fleet-feed.ts#L401-L408)) | fleet-feed-collector | Yes as a cache; refreshed from Supabase and config when needed | `agg_` cache |
| Supabase `alerts.incidents`, `alerts.incident_events`, `alerts.push_subscriptions`, `alerts.notification_sends` ([store and writes](aggregator/src/alerts.ts#L275-L310), [incident mutations](aggregator/src/alerts.ts#L340-L363), [send ledger](aggregator/src/alerts.ts#L448-L460); [schema/location](aggregator/README.md#L299-L310)) | aggregator alerts service | No; acknowledgement state, subscriptions, and delivery history are writable durable records | `agg_` alerts; `incident_events` maps to `shared_events` |
| Drill state under `~/.local/state/pm-drills/` ([default state path](aggregator/scripts/kill-collector-drill.sh#L41-L49)) | laptop-only aggregator drill script | Yes; run results/locks/incident flags are operational drill artifacts | `agg_` operational, or omit from shared tables as ephemeral |

#### First action/event column contract (#6103)

The first Move 1 store group is now specified by the versioned
[action/event schema contract](shared-schema/README.md): `public.agg_actions`
owns the aggregator's `actions.json` records, and `public.shared_events`
holds their cross-service outcomes. The paired up/down migration and
disposable PostgreSQL test are under `shared-schema/`. This is DDL and a
contract only; `actions.json` / `events.json` remain authoritative, with no
rows copied and no live reader or writer changed.

Read-only producer inputs (quota register/view, OpenChamber state, Cloudflare
and Supabase exports, DNS/orchestration/charge/launch records) are not
aggregator-owned stores: their producer remains owner and their target is
`not migrated / no PM-stack table`. See [config inputs](aggregator/src/config.ts#L122-L133)
and [collector environment](../nixos/services/pm-stack/default.nix#L234-L243).
`kaneo-projects.json` is versioned slug-to-repository configuration, not
runtime data ([config reader](aggregator/src/config.ts#L140-L143)).

### Projector database and read views

The projector opens `DATABASE_URL`, default `file:data/projector.db`
([DB wiring](projector/src/store.ts#L13-L22)); the systemd unit runs from
`pm-stack/projector` and reads an external `.env`
([Nix unit](../nixos/services/pm-stack/default.nix#L105-L115)), so the
checked-in default is not proof the deployed instance is local SQLite.

| Current table / view (schema citation) | Owner service | Rebuildable? | Target table group |
|---|---|---|---|
| `pmp_items` ([schema](projector/schema.sql#L10-L39)) | projector | Yes, GitHub issue/PR projection | `pmp_` projection; common item fields in `shared_items` |
| `pmp_comments` ([schema](projector/schema.sql#L41-L70)) | projector | Mostly; GitHub comments/reviews can be re-fetched, but deletion/tombstone history may not be | `pmp_` projection; common comments in `shared_comments` |
| `pmp_links` ([schema](projector/schema.sql#L72-L92)) | projector | **No. The only non-rebuildable cross-service relationship in the projector schema:** human/agent-written subject↔GitHub links | `pmp_` registry and canonical `shared_links` source |
| `pmp_deliveries`, `pmp_delivery_errors`, `pmp_sync_state` ([schema](projector/schema.sql#L94-L136)) | projector | Current projection/cursors can be rebuilt or reset; delivery/error rows are operational idempotency and diagnostic history, not shared entities | `pmp_` operational/audit tables |
| `pmp_push_events` ([schema](projector/schema.sql#L138-L150)) | projector | Current state can be reconciled; exact webhook history is an operational retention concern | `shared_events` if retained cross-service, otherwise `pmp_` audit |
| `pmp_v_links_items`, `pmp_v_subject_activity` ([views](projector/schema.sql#L152-L175); [README inventory](projector/README.md#L31-L44)) | projector | Yes; SQL views have no independent rows | Recreate as read views over `pmp_` / shared entity tables |

### Native service stores (remain service-owned under #3142)

| Current store (file / table; code citation) | Owner service | Rebuildable? | Target table group |
|---|---|---|---|
| Grist `grist_data:/persist` document store (roadmap and registers; [volume](grist/compose.yml#L20-L24), [mount](grist/compose.yml#L40-L52), [document/table](README.md#L300-L303)) | Grist | No for live document edits; seed files are not a full restore source | Roadmap/register rows can project to `shared_items`; keep Grist's native documents and restore path separate unless supported shared mode is proven |
| Kaneo PostgreSQL 16 database (defaults to `kaneo`), volume `postgres_data:/var/lib/postgresql/data` ([Compose](compose.yml#L14-L24)) | Kaneo | No for task/workflow state; GitHub sync is partial and does not reproduce all native board data | `shared_items` for cross-service task records and `shared_events` for changes; Kaneo remains source of truth |
| Pad SQLite `/data/pad.db`, volume `pad_data:/data` ([Compose](pad/compose.yml#L20-L35); [owned planning content](pad/seed/plans-pm-stack-design.md#L27-L35)) | Pad | No for user plans/notes; seed Markdown is bootstrap content, not full restore | `shared_items` for cross-service plan/note references; Pad database, attachments, and keys remain native |
| ready-for-agent SQLite state under `~/.local/share/ready-for-agent/` ([unit default](ready-for-agent/systemd/ready-for-agent.service#L10-L15)) | ready-for-agent | Unknown from checked-in code: DB filename/schema and restore path are not specified. Preserve until audited. Clones/worktrees are reproducible ([installer](ready-for-agent/install.sh#L58-L71), [stack runbook](README.md#L213-L247)) | `shared_events` candidate for dispatch/run status; native service state remains separate pending schema evidence |

## Registry backup legs and current coverage

For `pmp_links`, the recovery legs are the PM dashboard archive's three
destinations: local dated archive, Cloudflare R2, and Google Drive
([backup destinations](../nixos/home/pm-dashboard-backup.sh#L1-L11)). As of
the #1744 P0 slice (task 9251), every archive run stages a `pmp_links`-only
export (`projector/pmp_links.sql`: schema + rows dumped from the ACTUAL
configured projector database) as a conditionally-required manifest member —
required whenever a projector DB is present at the configured path, so a
missing or corrupt export fails the run before any upload. The rest of
`projector.db` (a rebuildable projection) is deliberately NOT archived.
Restore is replaying the SQL file into a scratch SQLite database;
`nixos/home/pm-dashboard-backup-pmp-links.test.sh` proves export,
manifest inclusion, links-only content, and restore integrity against
fixture databases, and the three legs inherit coverage because all three
copy the same sha256-verified archive tarball. The aggregator's distinct
`registry.json` is already a required archive member.
Kaneo/Grist/Pad dumps are only included when the optional external dump script
exists ([conditional dump step](../nixos/home/pm-dashboard-backup.sh#L275-L299));
the #3142 design also calls for their own verified native restore paths.

**Rebuildability clarification:** `pmp_links` is the only non-rebuildable
business relationship in the projector's disposable projection database.
That does not make aggregator-authored records or Kaneo/Grist/Pad source data
rebuildable; those are separate owners' durable source-of-truth stores and
must not be discarded on the strength of a shared-table projection.
