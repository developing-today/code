# Configuration and environment

This chapter covers only verified names: configuration files, environment variables, secret names, and named constants that the 2026-11-23 surveys record verbatim — the original survey's detailed entries first, then the mechanical env/config census (survey top-up, lane CONFIG-CENSUS) that verified 318 distinct env/config names in source, listed by component at the end of this chapter. Where no survey gives a name, this chapter says so and invents none. Every detailed entry follows the same shape: what the item is, its shape, its default where the survey gives one, its behaviour, and its failure shapes where the survey records them.

## Configuration files

### `pm-stack/aggregator/src/pages.ts` — the page manifest

- **What it is:** The aggregator's page registry: the `PAGES` list (23 entries) and the `PAGE_GROUPS` navigation groups (Overview, Work, Ops, System), plus `SUBROUTE_PARENTS`.
- **Shape:** Each page entry carries its route, title, blurb, group, `nav` curation (`primary` or `dropdown`), and, for all but three pages, an `api:` twin.
- **Behaviour:** The manifest is rendered as the `/pages` page, reported in the `/api/health` render registry, and handed to the console, whose `#more` view renders the dashboard links from the same manifest data.
- **Known gap:** D8 — `SUBROUTE_PARENTS` omits `/decisions/&lt;setId&gt;`, so that page's renders sit outside the render registry.

### `pm-stack/PORTS.md` — the port registry

- **What it is:** The stack's port registry, audited 2026-11-05.
- **Shape:** The registered ports are: 4870 aggregator, 4871 freshness IPC, 4872 console, 4879 dispatch, 5173 Kaneo, 6056 ready-for-agent, 7777 Pad, 7801 projector, 8484 Grist. The registry also records a non-PM laptop service, OpenClaw's embedding server on 18791; no PM code references it.
- **Behaviour:** Ports in this file are the stack's allocation of record; the console gate proxies dashboard traffic from `pm.security.cab` to the dashboard URL behind it.

### `pm-stack/README.md` — the deployment definition

- **What it is:** The stack's deployment definition: one `nixos-rebuild switch` on laptop host `amd` deploys the stack; the user units `pm-aggregator`, `pm-projector`, `pm-dispatch`, and `pm-tunnel` are declared in `nixos/services/pm-stack`; Kaneo, Postgres, Pad, and Grist are containers.
- **Disagreements carried:** D1 (its fork framing for Kaneo against the upstream image `compose.yml` pins), D2 (its Grist version `gristlabs/grist-oss:v1.7.21` against the tagless digest pin in `grist/compose.yml`), D3 (its "the aggregator reads no Pad data" against the health probe and Pad configuration the code carries).

### `pm-stack/compose.yml` and the per-app compose files

- **What it is:** The container definitions for Kaneo (with its Postgres 16 sidecar), Pad, and Grist.
- **Shape:** Kaneo pins the upstream image `ghcr.io/usekaneo/kaneo:2.47.0` by digest (D1). Grist's compose file (line 28) pins `gristlabs/grist-oss` by digest with no tag (D2). Pad's pin carries tag and digest.
- **Behaviour:** Kaneo is public as `https://kaneo.security.cab` through the `amd-6-pm-stack` Cloudflare Tunnel; Grist has no tunnel hostname.

### `pm-stack/kaneo-projects.json` — the Kaneo project registry

- **What it is:** The workspace and project registry the Kaneo integration reads: workspace "Developing Today", four projects, one repo each — `code` to developing-today/code, `opencode` to developing-today-forks/opencode, `openchamber` to developing-today-forks/openchamber, `nix` to dezren39/darwin-nix.
- **Shape:** The file carries the GitHub App id and per-project installation ids. `pm-stack/connect-repo.sh` reads it.
- **Behaviour:** Kaneo's GitHub integration status transitions live only in Kaneo's Postgres (`integration.config.statusTransitions` per project); nothing in the repository sets them. The guard that keeps them correct is `pm-stack/kaneo-sync-guard.py`.

### `pm-stack/github-app-manifest.json` — the GitHub App manifest

- **What it is:** The manifest for the developing.today GitHub App behind the Kaneo integration.
- **Shape:** The integration's webhook is `https://kaneo.security.cab/api/github-integration/webhook`.

### `pm-stack/ready-for-agent/config.json`

- **What it is:** The ready-for-agent configuration: four repositories (code, opencode, openchamber, nix — the same four as Kaneo).
- **Defaults:** Every repository sets `defaultModel: "opencode-go/kimi-k3.1-pro"`, `mergePolicy: "OFF"` (human merge always), and `includeAllIssueAuthors: true`; none is paused.
- **Behaviour:** `apply-config.sh` registers the repositories through the GraphQL API idempotently.

### `pm-stack/platform/workers.json` — the Workers registry

- **What it is:** The platform registry of Cloudflare Workers, checked 2026-11-23, with a `workers.test.ts` that holds the list to the repo's wrangler files. The console's `#more/platform` view is drawn from this file.
- **Shape:** 20 workers. D6: `pm-stack/platform/README.md` states eighteen; the JSON wins. Each entry records the worker's state — `pm-notify` is recorded as "not deployed" with a null address (D5); `decision-events` as "in service", deployed by hand.
- **Behaviour:** The README defines what "joined the platform" means: listed, Actions-deployed, version-reporting, named secrets, visible in the app.

### Wrangler configuration files

- **What it is:** One wrangler file per Worker, at the paths the ledger records: `pm-stack/decision-events-worker/wrangler.toml`, `pm-stack/voice-line-worker/wrangler.toml`, `pm-stack/notify-relay/wrangler.toml`, `pm-stack/console/edge/wrangler.jsonc` (plus `edge/init/wrangler.jsonc`), `pkgs/pkarr-worker/wrangler.jsonc`, `pkgs/lab-ops-hub/wrangler.jsonc`, `pkgs/cf-container-hello/wrangler.jsonc`, `scripts/muse/handoff/worker/wrangler.jsonc`, and the door and service configs named in [The workers](workers.md).
- **Known gaps:** No wrangler config exists in the census for the exec door, the pm aggregator worker, or the pm dashboard worker, and none at the package root for `github-ci`; those caveats are stated on their entries in [The workers](workers.md).

### Aggregator configuration in code (`aggregator/src/config.ts`)

- **What it is:** The aggregator's configuration module: service base URLs and keys, including the Grist connection (`grist.baseUrl`, `grist.docId`, `grist.apiKey`) and the Pad and ready-for-agent URLs below.
- **Behaviour:** The Grist values feed `gristRoadmap()` in `aggregator/src/sources/live.ts`, which reads the Roadmap table's records with the bearer API key.

### Other named configuration and seed files

- `pm-stack/grist/seed/roadmap.json` — the Grist roadmap seed (217 lines), also bundled into the pm-aggregator Nix package because `fixtures.ts` imports it by relative path.
- `pm-stack/opencode-scheduled-jobs/jobs.json` — the scheduled-jobs registry; the one pilot job is `quota-freshness-summary`.
- `pm-stack/console/src/plan.ts` constants — `INTAKE_LABEL = "owner/intake"`, the intake `ROUTES` table, `CAPABILITIES`, and `PHASES` (`doing`, `now`, `next`, `later`, `built`).
- `pm-stack/aggregator/src/notifications.ts` constants — `NOTIFICATION_CLASSES` and `NOTIFICATION_CLASS_DEFAULTS` (the eight preference classes, all default on), `DEFAULT_QUIET` (quiet hours 23:00–07:00 America/Chicago), `NOTIFICATION_COOLDOWN_MS` (6 hours), `QUOTA_WELL_BEFORE_RESET_MS` (24 hours), `BLOCKED_AGE_REMINDER_MS` (24 hours), and `BLOCKED_AGE_BREAKTHROUGH_MS` (48 hours).
- `pm-stack/notify-relay/src/relay.ts` constants — `NOTIFY_RELAY_HEARTBEAT_MS` (the aggregator heartbeat, 60 seconds, named in `aggregator/src/notify-relay.ts`) and `SILENT_CLASS` (the relay's pushes ride the `incident` class).

## Environment variables

The original survey verifies these environment names in detail. The full verified set — 318 distinct names across ten components — is the census section at the end of this chapter.

### `PM_AGG_PAD_URL`

- **What it is:** The aggregator's Pad base URL (`aggregator/src/config.ts`).
- **Default:** `http://127.0.0.1:7777`.
- **Behaviour:** D3 — the aggregator reads no Pad planning data; this URL serves the health and topology probe and the Pad service link.

### `PM_AGG_RFA_URL`

- **What it is:** The aggregator's ready-for-agent base URL, feeding the source adapter `aggregator/src/sources/ready-for-agent.ts`.

### `PM_AGG_NOTIFY_RELAY_URL`

- **What it is:** The aggregator's notify-relay URL (`aggregator/src/notify-relay.ts`).
- **Default:** Unset.
- **Behaviour:** With no `PM_AGG_NOTIFY_RELAY_URL`, the aggregator side of the dead-man's switch is inert: no heartbeats are posted. D5 — the relay itself is not deployed.

### `ATLAS_SEARCH_MODEL` and `ATLAS_VEC_TABLE`

- **What it is:** Environment overrides in the Atlas Python reference tool only (`pkgs/atlas-search/atlas_search.py`) — the embedding model and vector table selection.
- **Behaviour:** No PM component reads them; they are recorded here because the survey's model census names them. See [Model dependencies](models.md).

## Secrets, by name

Secret values appear nowhere in this book. The survey records these names and routes:

- `pm-aggregator.alerts-intake-token` — the bearer token for `POST /api/alerts`, resolved by name through `src/secrets.ts`.
- `pm-aggregator.vapid-private-key` — the Web Push VAPID private key (`aggregator/src/push.ts`). The VAPID public key is a constant in code; the subject is a `mailto:` value in code.
- Per-database Turso tokens for `fleet-platform` and `fleet-mirror` reads are fetched by name through `fleet secrets get`, with a Bitwarden fallback once (`pm-stack/quota-check/turso-mirror-read.py`, #6828) — explicitly not the Turso CLI's org token.
- The decision-events Worker's shared secret gates its routes; with no secret configured, the Worker fails closed.
- Stack-token exchange (`POST /api/stack-token/exchange`) accepts exactly three targets: `kaneo`, `pad`, `grist`.

## The full environment census (318 verified names, by component)

The mechanical census (survey top-up, lane CONFIG-CENSUS, 2026-11-23) grepped the laptop clone's source for every env/config read — `process.env` / `Bun.env` / config-helper reads, Python `os.environ`, shell `${NAME:-default}` forms, systemd `Environment=` lines, NixOS `Environment` lists in `nixos/services/pm-stack` and `nixos/services/pm-apps`, wrangler `[vars]` and binding names, compose `environment:` keys, and `pm-stack/projector/.env.example` — excluding `node_modules/` and test files. It found **318 distinct verified names, 373 (component, name) rows**: aggregator 175, console 71, dispatch 23, projector 15, decision-events-worker 3, voice-line-worker 9, notify-relay 3, ready-for-agent 7, platform-scripts 28, nixos-pm 39. The three names the earlier survey verified in detail above (`PM_AGG_PAD_URL`, `PM_AGG_RFA_URL`, `PM_AGG_NOTIFY_RELAY_URL`) are all present in the aggregator list. The full name lists follow, by component; the census's per-name evidence (file and line, read kind, default) is in the survey record.

### aggregator (175)

`AA_OPENCODE_CONFIG_PATH`, `AA_OPENCODE_RUNTIME_CONFIG_PATH`, `AGE`, `ALERTS_INTAKE_URL`, `ALERT_ACK_WINDOW_MIN`, `BURN_HISTORY_STATE_PATH`, `BWS_ACCESS_TOKEN_FILE`, `BWS_BIN`, `CHARGES_SURFACED_STATE_PATH`, `CHARGE_CANDIDATES_PATH`, `CHARGE_INPUTS_DIR`, `CLAUDE_LAUNCH_RECORDS_PATH`, `CUR_STAMP`, `DECISION_EVENTS_OUTBOX_FILE`, `DECISION_EVENTS_STATE_FILE`, `DECISION_EVENTS_SYNC`, `DECISION_EVENTS_TURSO_TOKEN`, `DECISION_EVENTS_TURSO_URL`, `DECISION_EVENTS_WEBHOOK_SECRET`, `DECISION_EVENTS_WEBHOOK_URL`, `DEPLOY_COMMIT`, `DNS_REGISTER_JSON_PATH`, `DNS_REGISTER_PATH`, `FLEET_BIN`, `FLEET_FEED_OUT`, `GH_BILLING_USER`, `GH_TOKEN`, `GITHUB_TOKEN`, `GMAIL_CLI`, `GRIST_API_KEY`, `GRIST_ROADMAP_DOC`, `GRIST_URL`, `HOME`, `HTTPS_PROXY`, `ITEM_HISTORY_FILE`, `KANEO_ADMIN_EMAIL`, `KANEO_ADMIN_PASSWORD`, `KANEO_CLIENT_URL`, `KANEO_ORIGIN`, `KANEO_URL`, `LIVE_VERSION`, `METADATA_SWEEP_REPOS`, `META_CREDENTIAL_PATH`, `META_EXPORTER_LATEST_PATH`, `META_RENEWAL_DISABLED`, `META_RENEWAL_STATE_PATH`, `META_TEAM_ID`, `NO_PROXY`, `OPENCHAMBER_LIB_DIR`, `OPENCHAMBER_LIB_INSTALL_CMD`, `OPENCODE_BASE_URL`, `OPENCODE_CONFIG_PATH`, `OPENCODE_PORT`, `OPENCODE_SERVER_PASSWORD`, `OPENCODE_SERVER_PASSWORD_FILE`, `OPENCODE_SERVER_URL`, `OPENCODE_SESSION_SOURCE`, `OPENDESIGN_CDP_URL`, `OPENDESIGN_RENEWAL_DISABLED`, `OPENDESIGN_RENEWAL_FORCE_CAPTURE`, `OPENDESIGN_RENEWAL_STATE_PATH`, `ORCHESTRATION_PATH`, `ORCHESTRATION_SOURCE`, `PAGE_WARN`, `PLAID_CLI`, `PM_AGG_ACCESS_AUD`, `PM_AGG_ACCESS_TEAM_DOMAIN`, `PM_AGG_ACTIONS_FILE`, `PM_AGG_ACTIVITY_FILE`, `PM_AGG_BREAK_GLASS_FILE`, `PM_AGG_BUILD_VERSION`, `PM_AGG_CLOUDFLARE_EXPORT`, `PM_AGG_CODE_CHECKOUT`, `PM_AGG_COMMENTS_FILE`, `PM_AGG_CONSOLE_HOST`, `PM_AGG_COVERAGE_SNAPSHOT_FILE`, `PM_AGG_DECISIONS_FILE`, `PM_AGG_DECISION_EVENTS_FILE`, `PM_AGG_DECISION_OUTBOX_FILE`, `PM_AGG_DEV`, `PM_AGG_DIGESTS_DIR`, `PM_AGG_EVENTS_FILE`, `PM_AGG_FEED_FILE`, `PM_AGG_FLEET_INVOCATION_LEDGER_FILE`, `PM_AGG_FORECAST_ADMISSION`, `PM_AGG_FORECAST_ADMISSION_MARGIN_MS`, `PM_AGG_FORECAST_ADMISSION_POOL`, `PM_AGG_FREE_SERVICES_TRACKER`, `PM_AGG_FRESHNESS_FILE`, `PM_AGG_FRESHNESS_PORT`, `PM_AGG_FRESHNESS_URL`, `PM_AGG_GITHUB_TOKEN`, `PM_AGG_HOSTS`, `PM_AGG_HOST_HEALTH_FILE`, `PM_AGG_IDENTITY_REGISTRY_FILE`, `PM_AGG_ITEM_HISTORY_FILE`, `PM_AGG_LAST_GOOD_DIR`, `PM_AGG_MCPX_CALL_LEDGER_DIR`, `PM_AGG_NEEDS_DREW_FILE`, `PM_AGG_NIX_CHECKOUT`, `PM_AGG_NOTIFICATIONS_FILE`, `PM_AGG_NOTIFY_RELAY_STATE_FILE`, `PM_AGG_NOTIFY_RELAY_URL`, `PM_AGG_OPENCHAMBER_STATE`, `PM_AGG_OPENCODE_CONFIG_FILE`, `PM_AGG_OPENCODE_SERVER_STATE`, `PM_AGG_OUTAGE_QUEUE_FILE`, `PM_AGG_OUTCOME_BINDINGS_FILE`, `PM_AGG_OUTCOME_FILE`, `PM_AGG_OWNER_IDS`, `PM_AGG_PAD_URL`, `PM_AGG_PORT`, `PM_AGG_PROMPTS_FILE`, `PM_AGG_PROMPT_ATTACHMENTS_DIR`, `PM_AGG_PROMPT_SYNC_TIMEOUT_MS`, `PM_AGG_PUBLIC_ORIGIN`, `PM_AGG_QUEUE_BROKER_TOKEN_FILE`, `PM_AGG_QUEUE_BROKER_URL`, `PM_AGG_QUOTA_CURRENT_FILE`, `PM_AGG_QUOTA_HEARTBEAT_FILE`, `PM_AGG_QUOTA_REGISTRY`, `PM_AGG_QUOTA_VIEW`, `PM_AGG_REGISTRY_FILE`, `PM_AGG_REPORT_RECORDS_FILE`, `PM_AGG_RFA_URL`, `PM_AGG_ROADMAP_ANNOTATIONS_FILE`, `PM_AGG_SLO_JOURNAL_FILE`, `PM_AGG_SNAPSHOTS_DIR`, `PM_AGG_SOURCE`, `PM_AGG_STACK_HEALTH_FILE`, `PM_AGG_STACK_SENTINEL_STATE_FILE`, `PM_AGG_STACK_TOKEN_SECRET`, `PM_AGG_STORE_BACKEND`, `PM_AGG_STORE_WRITER`, `PM_AGG_VIEWS_FILE`, `PM_CORE_JOBS_AUDIT_PATH`, `PM_DRILL_AGE_MS`, `PM_DRILL_ATTENTION_CMD`, `PM_DRILL_FEED_FILE`, `PM_DRILL_HEALTH_CMD`, `PM_DRILL_INCIDENT_FLAG`, `PM_DRILL_MAX_WAIT_MIN`, `PM_DRILL_NO_POST`, `PM_DRILL_PAGE_CMD`, `PM_DRILL_RESTORE_WAIT_MIN`, `PM_DRILL_STATE_DIR`, `PM_KANEO_PROJECTS_FILE`, `PM_METADATA_DB_PATH`, `PM_PRIMARY_CHECKOUT_DIR`, `PM_PROJECTOR_API_TOKEN`, `PM_PROJECTOR_URL`, `POOL_STATE_PATH`, `PROVIDER_USAGE_SNAPSHOT_PATH`, `PUSH_FLEET_FEED_KNOWN_HOSTS`, `QUOTA_ANOMALY_HISTORY_PATH`, `QUOTA_ANOMALY_STATE_PATH`, `QUOTA_CLAUDE_RETRY_BASE_MS`, `QUOTA_CURRENT_PATH`, `QUOTA_FORECAST_PATH`, `QUOTA_GATHERER_STATE_PATH`, `QUOTA_HEARTBEAT_PATH`, `QUOTA_PREDICTIONS_PATH`, `QUOTA_REFRESH_CMD`, `QUOTA_REGISTRY_PATH`, `QUOTA_SHADOW_PLACEMENT_HISTORY_PATH`, `QUOTA_SHADOW_PLACEMENT_PATH`, `QUOTA_SNAPSHOTS_DIR`, `QUOTA_SUBCOMMAND`, `QUOTA_SUBCOMMAND_TIMEOUT_MS`, `QUOTA_VIEW_PATH`, `SANDBOX_OPENCODE_URL`, `TAILNET_IP`, `TURSO_QUOTA_SINK`, `USAGE_SNAPSHOT_OUTBOX_PATH`, `XDG_STATE_HOME`

### console (71)

`ANDROID_HOME`, `ASSETS`, `CLOUDFLARE_ACCOUNT_ID`, `CLOUDFLARE_API_TOKEN`, `EDGE`, `EDGE_DIRECT`, `GITHUB_API_ORIGIN`, `GITHUB_REPOSITORY`, `GITHUB_RUN_ID`, `GITHUB_SERVER_URL`, `GITHUB_SHA`, `GITHUB_STEP_SUMMARY`, `GITHUB_TOKEN`, `GRADLE_VERSION`, `HOME`, `HOST_ORIGIN`, `JAVA_HOME_17_X64`, `KEYSTORE_B64`, `KEYS_FROM`, `KEYS_TO`, `OWN_CLOCK_MS`, `PM_ANDROID_KEYSTORE`, `PM_ANDROID_KEYSTORE_PASSWORD`, `PM_ANDROID_KEYSTORE_PASSWORD_FILE`, `PM_ANDROID_REPO`, `PM_ANDROID_VERSION_CODE`, `PM_ANDROID_VERSION_NAME`, `PM_CONSOLE_ACCESS_AUD`, `PM_CONSOLE_ACCESS_TEAM_DOMAIN`, `PM_CONSOLE_ANDROID_APK`, `PM_CONSOLE_ANDROID_PACKAGE`, `PM_CONSOLE_AUDIT_FILE`, `PM_CONSOLE_BIND`, `PM_CONSOLE_BUN`, `PM_CONSOLE_CERT_NAMES`, `PM_CONSOLE_DASHBOARD_URL`, `PM_CONSOLE_DECISIONS_FILE`, `PM_CONSOLE_DEMO_ACT_LOG`, `PM_CONSOLE_DEVICES_FILE`, `PM_CONSOLE_EDGE_DONE_FILE`, `PM_CONSOLE_EDGE_KEY_FILE`, `PM_CONSOLE_EDGE_URL`, `PM_CONSOLE_FLEET_BIN`, `PM_CONSOLE_ISSUERS_FILE`, `PM_CONSOLE_MACHINE`, `PM_CONSOLE_OWNER_EMAILS`, `PM_CONSOLE_PORT`, `PM_CONSOLE_PROMPT_LABEL`, `PM_CONSOLE_PROTECTED_UNITS`, `PM_CONSOLE_QUEUE`, `PM_CONSOLE_QUEUE_DB_URL`, `PM_CONSOLE_QUEUE_TOKEN_NAME`, `PM_CONSOLE_REPO`, `PM_CONSOLE_SITE`, `PM_CONSOLE_UNITS`, `PM_GH`, `PM_RELEASE_API`, `PM_RELEASE_HEALTH`, `PM_RELEASE_HEALTH_TRIES`, `PM_RELEASE_HEALTH_WAIT_MS`, `PM_RELEASE_STAND_UP_TRIES`, `PM_RELEASE_STAND_UP_WAIT_MS`, `PM_SIGNED_HERE`, `PORT`, `PUBLIC_HOST`, `QUEUE_DB_ORIGIN`, `QUEUE_TOKEN`, `SMOKE_PRIMARY`, `VERSION`, `VERSION_CODE`, `VERSION_NAME`

### dispatch (23)

`DEPLOY_COMMIT`, `DISPATCH_API_TOKEN`, `DISPATCH_BUILD_VERSION`, `DISPATCH_HOSTS`, `DISPATCH_OIDC_AUDIENCE`, `DISPATCH_PORT`, `DISPATCH_REPO_ALLOWLIST`, `GITHUB_APP_ID`, `GITHUB_APP_PRIVATE_KEY_BASE64`, `LIVE_VERSION`, `OPENCODE_AUTH_FILE`, `OPENCODE_URL`, `PM_DISPATCH_PLACEMENT_GUARD`, `PM_DISPATCH_PLACEMENT_POLICY`, `PM_DISPATCH_PLACEMENT_RECORD`, `PM_DISPATCH_WORKTREE_REGISTRY`, `PM_DISPATCH_WORKTREE_ROOT`, `PM_DISPATCH_WORKTREE_SWEEP_SECONDS`, `QUEUE_BROKER_TOKEN`, `QUEUE_BROKER_TOKEN_FILE`, `QUEUE_BROKER_URL`, `QUOTA_CURRENT_PATH`, `TAILNET_IP`

### projector (15)

`DATABASE_URL`, `GITHUB_APP_ID`, `GITHUB_APP_INSTALL_ID`, `GITHUB_APP_PRIVATE_KEY`, `GITHUB_TOKEN`, `GITHUB_WEBHOOK_SECRET`, `HOSTNAME`, `LIBXML_SRC`, `PM_AGGREGATOR_URL`, `PM_AGG_FRESHNESS_URL`, `PM_PROJECTOR_API_TOKEN`, `PM_PROJECTOR_URL`, `PM_REPOS`, `PORT`, `TURSO_AUTH_TOKEN`

### decision-events-worker (3)

`DB`, `SET_EVENTS`, `WEBHOOK_SECRET`

### voice-line-worker (9)

`AI`, `GH_TOKEN`, `GITHUB_EVENT_NAME`, `GITHUB_REF`, `GITHUB_REPOSITORY`, `STATUS_INGEST_TOKEN`, `TURSO_TOKEN`, `TURSO_URL`, `VOICE_SESSIONS`

### notify-relay (3)

`PM_NOTIFY`, `RELAY_SHARED_SECRET`, `VAPID_PRIVATE_KEY`

### ready-for-agent (7)

`DRY_RUN`, `PATH`, `RFA_CONFIG`, `RFA_GRAPHQL_URL`, `RFA_REPOS_DIR`, `STUB_PID`, `XDG_STATE_HOME`

### platform-scripts (28)

`AUTH_SECRET`, `DISABLE_REGISTRATION`, `FENCE_LINE`, `FENCE_OUT`, `GITHUB_APP_ID`, `GITHUB_APP_NAME`, `GITHUB_PRIVATE_KEY_BASE64`, `GITHUB_WEBHOOK_SECRET`, `KANEO_CLIENT_URL`, `KANEO_OPEN_REGISTRATION`, `KCODE`, `NOTIFICATION_SECRET_ENCRYPTION_KEY`, `PATH`, `PM_AGG_DIGESTS_DIR`, `PM_GIT_SYNC_QUARANTINE`, `PM_GIT_SYNC_REPO`, `PM_STACK_SECRETS`, `PM_STACK_TAILNET_IP`, `PM_SWEEPER_SUBPROCESS_TIMEOUT`, `POSTGRES_DB`, `POSTGRES_PASSWORD`, `POSTGRES_USER`, `QUOTA_WATCHDOG_ADAPTERS_DIR`, `QUOTA_WATCHDOG_LIVE`, `QUOTA_WATCHDOG_LIVE_REPAIRS`, `SOPS_AGE_KEY_FILE`, `SRC_COUNTS`, `TGT_COUNTS`

### nixos-pm (39)

`BWS_ACCESS_TOKEN_FILE`, `BWS_BIN`, `CHARGES_SURFACED_STATE_PATH`, `CHARGE_CANDIDATES_PATH`, `CLAUDE_LAUNCH_RECORDS_PATH`, `DNS_REGISTER_PATH`, `FLEET_FEED_OUT`, `OPENCODE_SERVER_PASSWORD_FILE`, `OPENCODE_SERVER_URL`, `OPENCODE_SESSION_SOURCE`, `ORCHESTRATION_PATH`, `PATH`, `PM_AGG_ACTIVITY_FILE`, `PM_AGG_COVERAGE_SNAPSHOT_FILE`, `PM_AGG_DIGESTS_DIR`, `PM_AGG_EVENTS_FILE`, `PM_AGG_FRESHNESS_FILE`, `PM_AGG_FRESHNESS_PORT`, `PM_AGG_FRESHNESS_URL`, `PM_AGG_ROADMAP_ANNOTATIONS_FILE`, `PM_AGG_SOURCE`, `PM_AGG_STORE_BACKEND`, `PM_AGG_STORE_WRITER`, `PM_AGG_VIEWS_FILE`, `PM_APPS_HEALTH_APPS`, `PM_APPS_HEALTH_GRACE_SECONDS`, `PM_APPS_HEALTH_NOW`, `PM_APPS_HEALTH_STATE_DIR`, `PM_KANEO_PROJECTS_FILE`, `PM_PROJECTOR_URL`, `POOL_STATE_PATH`, `QUOTA_CURRENT_PATH`, `QUOTA_GATHERER_STATE_PATH`, `QUOTA_HEARTBEAT_PATH`, `QUOTA_REGISTRY_PATH`, `QUOTA_SNAPSHOTS_DIR`, `QUOTA_VIEW_PATH`, `SANDBOX_OPENCODE_URL`, `USAGE_SNAPSHOT_OUTBOX_PATH`

### Docs-only names (13) — labelled docs-only

These 13 names appear in READMEs, runbooks, or `PORTS.md` but no code or config-file read was found for them by any census pass. They are recorded here as **docs-only**: either the docs are stale, or the read happens outside the pm-stack tree (CI workflows, Android build scripts, another repo). They are not verified env names and no behaviour is asserted for them.

`AUTH_SECRET`, `CLOUDFLARE_VOICE_LINE_PROMOTION_TOKEN`, `CLOUDFLARE_VOICE_LINE_UPLOAD_TOKEN`, `GRIST_BOOT_KEY`, `PAD_API_KEY`, `PM_CONSOLE_ANDROID_KEYSTORE_B64`, `PM_CONSOLE_ANDROID_KEYSTORE_PASSWORD`, `PM_CONSOLE_ANDROID_SHA256`, `PM_HEALTH_HOOKS_INCIDENT_FILE`, `PORT_SECRETS_BROKER`, `REPO_QUEUE_BROKER_URL`, `REPO_QUEUE_TOKEN_FILE`, `UPLOAD_SECRET`

Sources: PM-SURVEY survey files `apps.md`, `website-pages.md`, `push.md`, `models.md`, and `disagreements.md` (D1, D2, D3, D5, D6, D8); PM-CENSUS coverage ledger, census date 2026-11-23.
