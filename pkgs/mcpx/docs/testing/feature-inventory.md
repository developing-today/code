# mcpx feature↔integration-test inventory (for #1978)

Enumerated on current `main` at `732ca26d` (2026-10-30) by executing the
registries — `api.Ops()`, `cli.Commands()`, `settings.Registry()` — not by grep.
Generator (throwaway, not committed): a `go run` harness in the module root;
counts below are its output. Supersedes the issue-body table (53 ops / 152 settings / 209 e2e tests at `05c78b2`).

Refreshed 2026-11-11 at `2b3db37221` (task 14824) by re-executing the same
registries; the tables below are that run's output. The feature-coverage
guard this inventory specified is now landed (see the closing sections):
the counts are re-derivable at any time from the guard's own enumeration
(`requiredFeatures()` in `internal/e2e/feature_coverage_guard_test.go`).

## Headline counts

| Surface           | Count   | Source                                                                                                                                                 |
| ----------------- | ------- | ------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `/v1` ops         | **87**  | `api.Ops()` (`internal/api/ops*.go`: literal + proto/consumer/exec/settings/resolve/secrets)                                                           |
| CLI commands      | **72**  | `cli.Commands()` = hand-written + generated (`internal/cli/manual.go:43`); the guard's required set adds the 6 alias-only `App.Handlers()` keys for 78 |
| Settings          | **238** | `settings.Registry()` (`internal/settings/registry.go:15`)                                                                                             |
| Plugin test files | **3**   | `plugin/opencode/mcpx/*.test.ts` (ceiling, instructions, session-created)                                                                              |
| e2e `Test*` funcs | **377** | `internal/e2e/*_test.go` (including the coverage guard's own tests)                                                                                    |

The issue’s stale row is confirmed stale: CLI commands are enumerable — `cli.Commands()` for help/man/completion and the runtime dispatch table `(&cli.App{}).Handlers()` (`internal/cli/dispatch.go:92`).

## Ops (87)

`tool` = MCP tool name (or `—` when `NoTool`); `covered` = hand-written tool in `CoveredBy`.
`S` marks the single streaming op (out of guard scope for the MCP parity test).

| Op                       | Route                             | Tool                                  | S   |
| ------------------------ | --------------------------------- | ------------------------------------- | --- |
| `adapters_add`           | `POST /v1/adapters`               | mcpx_adapters_add                     |     |
| `adapters_remove`        | `DELETE /v1/adapters/{name}`      | mcpx_adapters_remove                  |     |
| `artifact_delete`        | `DELETE /v1/artifacts/{id}`       | mcpx_artifact_delete                  |     |
| `artifact_get`           | `GET /v1/artifacts/{id}`          | mcpx_artifact_get                     |     |
| `artifact_put`           | `POST /v1/artifacts`              | mcpx_artifact_put                     |     |
| `artifacts_list`         | `GET /v1/artifacts`               | mcpx_artifacts_list                   |     |
| `ask_abandon`            | `POST /v1/ask/{id}/abandon`       | mcpx_ask_abandon                      |     |
| `ask_answers`            | `POST /v1/ask/{id}/answers`       | mcpx_ask_answers                      |     |
| `ask_begin`              | `POST /v1/ask`                    | mcpx_ask_begin                        |     |
| `ask_poll`               | `GET /v1/ask/{id}`                | mcpx_ask_poll                         |     |
| `authz_decisions`        | `GET /v1/authz/decisions`         | mcpx_authz_decisions                  |     |
| `authz_decisions_clear`  | `DELETE /v1/authz/decisions`      | mcpx_authz_decisions_clear            |     |
| `batch`                  | `POST /v1/batch`                  | mcpx_batch (via mcpx_batch)           |     |
| `call`                   | `POST /v1/call`                   | mcpx_call (via mcpx_call)             |     |
| `call_path`              | `POST /v1/call/{server}/{tool}`   | mcpx_call (via mcpx_call)             |     |
| `catalog`                | `GET /v1/catalog`                 | mcpx_catalog (via mcpx_catalog)       |     |
| `catalog_history`        | `GET /v1/catalog/history`         | mcpx_catalog_history                  |     |
| `chat_completions`       | `POST /v1/chat/completions`       | mcpx_chat_completions                 |     |
| `client_module`          | `GET /v1/client.ts`               | mcpx_client_module                    |     |
| `complete`               | `POST /v1/complete`               | mcpx_complete                         |     |
| `daemon_restart`         | `POST /v1/daemon/restart`         | mcpx_daemon_restart                   |     |
| `dashboard`              | `GET /v1/dashboard`               | mcpx_dashboard                        |     |
| `diagnose`               | `POST /v1/diagnose`               | mcpx_diagnose                         |     |
| `elicit_answer`          | `POST /v1/elicit/{id}/{action}`   | mcpx_elicit_answer                    |     |
| `elicit_get`             | `GET /v1/elicit/{id}`             | mcpx_elicit_get                       |     |
| `elicit_list`            | `GET /v1/elicit`                  | mcpx_elicit_list                      |     |
| `embeddings`             | `POST /v1/embeddings`             | mcpx_embeddings                       |     |
| `embeddings_export`      | `GET /v1/embeddings/export`       | mcpx_embeddings_export                |     |
| `embeddings_import`      | `POST /v1/embeddings/import`      | mcpx_embeddings_import                |     |
| `events`                 | `GET /v1/events`                  | mcpx_events                           | S   |
| `exec`                   | `POST /v1/exec`                   | mcpx_exec (via mcpx_exec)             |     |
| `feedback`               | `POST /v1/feedback`               | mcpx_feedback                         |     |
| `globals`                | `GET /v1/globals.d.ts`            | mcpx_globals                          |     |
| `health`                 | `GET /v1/health`                  | mcpx_health                           |     |
| `intent`                 | `POST /v1/intent`                 | mcpx_intent                           |     |
| `interactions_get`       | `GET /v1/interactions/{id}`       | mcpx_interactions_get                 |     |
| `interactions_list`      | `GET /v1/interactions`            | mcpx_interactions_list                |     |
| `log_query`              | `GET /v1/log`                     | mcpx_log_query                        |     |
| `log_record`             | `POST /v1/log`                    | mcpx_log_record                       |     |
| `metrics_tokens`         | `GET /v1/metrics/tokens`          | mcpx_metrics_tokens                   |     |
| `metrics_tools`          | `GET /v1/metrics/tools`           | mcpx_metrics_tools                    |     |
| `namespaces`             | `GET /v1/namespaces`              | mcpx_namespaces (via mcpx_namespaces) |     |
| `openapi`                | `GET /v1/openapi.json`            | mcpx_openapi                          |     |
| `preflight`              | `POST /v1/preflight`              | mcpx_preflight                        |     |
| `prompt_get`             | `POST /v1/prompt`                 | mcpx_prompt_get                       |     |
| `prompts`                | `GET /v1/prompts`                 | mcpx_prompts                          |     |
| `protocol`               | `GET /v1/protocol`                | mcpx_protocol                         |     |
| `recipe_get`             | `GET /v1/recipes/{name}`          | mcpx_recipe_get                       |     |
| `recipe_run`             | `POST /v1/recipes/{name}/run`     | mcpx_recipe_run                       |     |
| `recipe_save`            | `POST /v1/recipes/{name}`         | mcpx_recipe_save                      |     |
| `recipes_list`           | `GET /v1/recipes`                 | mcpx_recipes_list                     |     |
| `refresh`                | `POST /v1/refresh`                | mcpx_refresh                          |     |
| `registry_search`        | `GET /v1/registry/search`         | mcpx_registry_search                  |     |
| `reload`                 | `POST /v1/reload`                 | mcpx_reload                           |     |
| `resolve`                | `GET /v1/resolve`                 | mcpx_resolve                          |     |
| `resource_read`          | `POST /v1/resource`               | mcpx_resource_read                    |     |
| `resource_templates`     | `GET /v1/resource-templates`      | mcpx_resource_templates               |     |
| `resources`              | `GET /v1/resources`               | mcpx_resources                        |     |
| `restart`                | `POST /v1/restart`                | mcpx_restart                          |     |
| `search`                 | `GET /v1/search`                  | mcpx_search (via mcpx_search)         |     |
| `secret_request_decline` | `DELETE /v1/secret-requests/{id}` | mcpx_secret_request_decline           |     |
| `secret_request_get`     | `GET /v1/secret-requests/{id}`    | mcpx_secret_request_get               |     |
| `secret_requests_list`   | `GET /v1/secret-requests`         | mcpx_secret_requests_list             |     |
| `secrets_list`           | `GET /v1/secrets`                 | mcpx_secrets_list                     |     |
| `secrets_set`            | `PUT /v1/secrets/{name}`          | —                                     |     |
| `secrets_unset`          | `DELETE /v1/secrets/{name}`       | mcpx_secrets_unset                    |     |
| `servers_add`            | `POST /v1/servers`                | mcpx_servers_add                      |     |
| `servers_get`            | `GET /v1/servers/{name}`          | mcpx_servers_get                      |     |
| `servers_list`           | `GET /v1/servers`                 | mcpx_servers_list                     |     |
| `servers_remove`         | `DELETE /v1/servers/{name}`       | mcpx_servers_remove                   |     |
| `session_release`        | `POST /v1/session/release`        | mcpx_session_release                  |     |
| `sessions_read`          | `GET /v1/sessions`                | mcpx_sessions_read                    |     |
| `settings_get`           | `GET /v1/settings/{path}`         | mcpx_settings_get                     |     |
| `settings_list`          | `GET /v1/settings`                | mcpx_settings_list                    |     |
| `settings_set`           | `PUT /v1/settings/{path}`         | mcpx_settings_set                     |     |
| `settings_unset`         | `DELETE /v1/settings/{path}`      | mcpx_settings_unset                   |     |
| `shutdown`               | `POST /v1/shutdown`               | mcpx_shutdown                         |     |
| `stats_query`            | `GET /v1/stats`                   | mcpx_stats_query                      |     |
| `status`                 | `GET /v1/status`                  | mcpx_status (via mcpx_status)         |     |
| `task_cancel`            | `POST /v1/tasks/{id}/cancel`      | mcpx_task_cancel                      |     |
| `task_get`               | `GET /v1/tasks/{id}`              | mcpx_task_get                         |     |
| `task_result`            | `GET /v1/tasks/{id}/result`       | mcpx_task_result                      |     |
| `tasks_list`             | `GET /v1/tasks`                   | mcpx_tasks_list                       |     |
| `tool_invoke`            | `POST /v1/tools/{tool}`           | mcpx_tool_invoke                      |     |
| `tools`                  | `GET /v1/tools`                   | mcpx_tools                            |     |
| `types`                  | `GET /v1/types`                   | mcpx_types (via mcpx_types)           |     |
| `writes_clear`           | `POST /v1/writes/{server}/clear`  | mcpx_writes_clear                     |     |

## CLI commands (72), by group

| Group         | Commands                                                                                                                                                                                                      |
| ------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| configuration | `config`, `settings`, `servers`, `secrets`, `init`, `help`, `man`, `completion`, `adapter`, `plugin`, `registry`, `api`                                                                                       |
| daemon        | `status`, `cache`, `daemons`, `refresh`, `reload`, `upgrade`, `restart`, `stop`, `supervise`, `daemon`, `serve`, `stdio-proxy`                                                                                |
| discovery     | `ls (list,namespaces)`, `catalog`, `types`, `search`, `call`, `batch`, `client`, `schema`, `prompts`, `resources`, `openapi`                                                                                  |
| inspection    | `diagnose`, `elicit`, `doctor`, `hook`, `tui`, `explore`, `dashboard`, `interactions`, `log (logs)`, `stats`                                                                                                  |
| operations    | `artifact (artifacts)`, `ask`, `authz`, `complete`, `events`, `globals`, `health`, `history`, `metrics`, `preflight`, `protocol`, `resolve`, `session`, `sessions`, `task (tasks)`, `tool`, `tools`, `writes` |
| running       | `run`, `exec`, `scripts`, `skills`, `recipes`, `prompt`, `proxy`, `embeddings`, `feedback`                                                                                                                    |

## Settings (238)

Full list is `settings.Registry()` output; by prefix:

| Prefix          | Count |
| --------------- | ----- |
| `artifacts`     | 12    |
| `autonomy`      | 1     |
| `autostart`     | 7     |
| `catalog`       | 4     |
| `completion`    | 3     |
| `daemon`        | 23    |
| `deps`          | 4     |
| `diagnose`      | 2     |
| `doctor`        | 1     |
| `elicit`        | 7     |
| `embeddings`    | 5     |
| `events`        | 2     |
| `exec`          | 4     |
| `hooks`         | 3     |
| `http`          | 13    |
| `logging`       | 11    |
| `logstore`      | 2     |
| `mcp`           | 2     |
| `model`         | 8     |
| `output`        | 2     |
| `paths`         | 10    |
| `plugin`        | 13    |
| `plumbing`      | 8     |
| `policy`        | 1     |
| `pool`          | 7     |
| `preset`        | 1     |
| `presets`       | 1     |
| `prompt`        | 6     |
| `proto`         | 12    |
| `protoMessages` | 3     |
| `protoTasks`    | 5     |
| `proxy`         | 1     |
| `recipes`       | 3     |
| `registry`      | 5     |
| `repair`        | 1     |
| `retention`     | 2     |
| `sampling`      | 1     |
| `script`        | 23    |
| `search`        | 2     |
| `serve`         | 1     |
| `session`       | 1     |
| `spec`          | 3     |
| `stats`         | 1     |
| `tasks`         | 3     |
| `transport`     | 5     |
| `upstream`      | 3     |

## Plugin (`plugin/opencode/mcpx/`)

| File                      | What it covers                 | Run in CI?                                                              |
| ------------------------- | ------------------------------ | ----------------------------------------------------------------------- |
| `ceiling.test.ts`         | ceiling behaviour              | **yes** — `mcpx.yml` runs `bun test plugin/opencode/mcpx/ plugin/core/` |
| `instructions.test.ts`    | plugin instructions            | **yes** — same step                                                     |
| `session-created.test.ts` | session-created hook behaviour | **yes** — same step                                                     |

The 2026-10-30 table's `daemon.test.ts` / `ops.test.ts` no longer live in
this directory. The plugin op surface itself (`plugin/core/ops.gen.ts`,
generated from `api.Ops()`) is freshness-asserted by the coverage guard
below, so a stale generated surface fails the e2e run.

## What the existing guards cover — and the hole

| Guard                                                                     | Covers                                      | Does NOT assert                              |
| ------------------------------------------------------------------------- | ------------------------------------------- | -------------------------------------------- |
| `TestEveryDeclaredRouteAnswersOverTheSocket` (`e2e/v1ops_test.go:287`)    | every op route answers                      | that any test _exercises_ the op’s behaviour |
| `TestEverySettingIsReachableOverV1` (`e2e/settings_test.go:173`)          | every setting reachable                     | behavioural exercise                         |
| `TestEveryExecFlagChangesSomething` (`e2e/e2e_test.go:2958`)              | every exec flag changes something           | per-op exercise                              |
| `TestEveryNonStreamingOpIsReachableAsAnMCPTool` (`api/parity_test.go:96`) | 86/87 ops as tools (streaming excluded)     | exercise                                     |
| `TestEverySettingIsReadSomewhere` (`settings/consumed_test.go:87`)        | every setting read                          | exercise                                     |
| `TestNoPackageRedeclaresADefault` (`defaults/guard_test.go:93`)           | default uniqueness                          | n/a                                          |
| `bun test` step (`mcpx.yml`)                                              | plugin tests run at all                     | per-hook exercise                            |
| **Feature-coverage guard (#1978, landed 2026-11-11)**                     | **per-feature behavioural exercise, below** | —                                            |

**The hole is closed.** The runtime ledger (Approach C, issue criterion 2)
records the exercised-by mapping at runtime — the e2e harness logs the
resolved op/command/setting per test, `TestMain` writes `{test, kind, name}`
JSON lines — and the guard now compares that ledger against the registries.

## The landed guard (Approach C, implemented 2026-11-11, task 14824)

- **Enumerate from:** `api.Ops()`, `cli.Commands()` (+ `App.Handlers()` keys
  for aliases), `settings.Registry()` — 403 required features at refresh
  time (87 ops, 78 cli names, 238 settings) — and the plugin op surface
  (`plugin/core/ops.gen.ts`, freshness-asserted in the same evaluation:
  it must name exactly the `api.Ops()` set).
- **Where it evaluates:** `guardedMain` in `internal/e2e/e2e_test.go`, after
  `m.Run()`, on a full unfiltered green run (a `-run`-filtered or failing
  run's ledger is partial by construction and is not judged). Implementation:
  `internal/e2e/feature_coverage_guard_test.go` (`requiredFeatures`,
  `evaluateFeatureCoverage`, `featureCoverageGuardViolations`), reusing the
  existing `coverageViolations` and ledger — no second record system.
- **Exemptions:** `internal/e2e/feature_coverage_exemptions.txt` (checked
  in): one line per feature with a one-line reason naming its burn-down
  issue. Empty reasons, exemptions for unknown features, and exemptions
  for features that have since gained a ledger row are all violations —
  burn-down is self-verifying. `TestFeatureCoverageGuardAgainstRecordedLedger`
  re-evaluates any recorded ledger (set
  `MCPX_FEATURE_COVERAGE_EVALUATE_LEDGER`) without a 25-minute suite run.
- **Measured state at landing (full-suite ledger on `2b3db37221`):** 152 of
  403 features covered by a ledger row; 251 exempt — ops 9 (#8466), CLI 29
  (#8460–#8465), settings 213 (#8467–#8474). The fabricated-gap tests
  (`TestFeatureCoverageGuardFailsForFabricatedGap`,
  `TestFeatureCoverageGuardFailsForFabricatedRegistryEntry`) demonstrate
  the failure shape; the recorded-ledger evaluation demonstrates zero
  violations on today's coverage.
- **Caveat:** the e2e suite itself is red on main for four pre-existing
  failures (#8457), so until that lands the guard's guardedMain evaluation
  is armed but not reached on red runs; the recorded-ledger evaluation
  above is the current evidence.
