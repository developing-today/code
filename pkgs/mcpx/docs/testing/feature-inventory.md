# mcpx feature↔integration-test inventory (for #1978)

Enumerated on current `main` at `732ca26d` (2026-10-30) by executing the
registries — `api.Ops()`, `cli.Commands()`, `settings.Registry()` — not by grep.
Generator (throwaway, not committed): a `go run` harness in the module root;
counts below are its output. Supersedes the issue-body table (53 ops / 152 settings / 209 e2e tests at `05c78b2`).

## Headline counts

| Surface | Count | Source |
| --- | --- | --- |
| `/v1` ops | **81** | `api.Ops()` (`internal/api/ops*.go`: literal + proto/consumer/exec/settings/resolve/secrets) |
| CLI commands | **69** | `cli.Commands()` = hand-written + generated (`internal/cli/manual.go:43`) |
| Settings | **197** | `settings.Registry()` (`internal/settings/registry.go:15`) |
| Plugin test files | **4** | `plugin/opencode/mcpx/*.test.ts` (daemon, ceiling, ops, instructions — issue says 1) |
| e2e `Test*` funcs | **339** | `internal/e2e/*_test.go` |

The issue’s stale row is confirmed stale: CLI commands are enumerable — `cli.Commands()` for help/man/completion and the runtime dispatch table `(&cli.App{}).Handlers()` (`internal/cli/dispatch.go:92`).

## Ops (81)

`tool` = MCP tool name (or `—` when `NoTool`); `covered` = hand-written tool in `CoveredBy`.
`S` marks the single streaming op (out of guard scope for the MCP parity test).

| Op | Route | Tool | S |
| --- | --- | --- | --- |
| `artifacts_list` | `GET /v1/artifacts` | mcpx_artifacts_list |  |
| `artifact_put` | `POST /v1/artifacts` | mcpx_artifact_put |  |
| `artifact_delete` | `DELETE /v1/artifacts/{id}` | mcpx_artifact_delete |  |
| `artifact_get` | `GET /v1/artifacts/{id}` | mcpx_artifact_get |  |
| `ask_begin` | `POST /v1/ask` | mcpx_ask_begin |  |
| `ask_poll` | `GET /v1/ask/{id}` | mcpx_ask_poll |  |
| `ask_abandon` | `POST /v1/ask/{id}/abandon` | mcpx_ask_abandon |  |
| `ask_answers` | `POST /v1/ask/{id}/answers` | mcpx_ask_answers |  |
| `batch` | `POST /v1/batch` | mcpx_batch (via mcpx_batch) |  |
| `call` | `POST /v1/call` | mcpx_call (via mcpx_call) |  |
| `call_path` | `POST /v1/call/{server}/{tool}` | mcpx_call (via mcpx_call) |  |
| `catalog` | `GET /v1/catalog` | mcpx_catalog (via mcpx_catalog) |  |
| `catalog_history` | `GET /v1/catalog/history` | mcpx_catalog_history |  |
| `chat_completions` | `POST /v1/chat/completions` | mcpx_chat_completions |  |
| `client_module` | `GET /v1/client.ts` | mcpx_client_module |  |
| `complete` | `POST /v1/complete` | mcpx_complete |  |
| `daemon_restart` | `POST /v1/daemon/restart` | mcpx_daemon_restart |  |
| `dashboard` | `GET /v1/dashboard` | mcpx_dashboard |  |
| `diagnose` | `POST /v1/diagnose` | mcpx_diagnose |  |
| `elicit_list` | `GET /v1/elicit` | mcpx_elicit_list |  |
| `elicit_get` | `GET /v1/elicit/{id}` | mcpx_elicit_get |  |
| `elicit_answer` | `POST /v1/elicit/{id}/{action}` | mcpx_elicit_answer |  |
| `embeddings` | `POST /v1/embeddings` | mcpx_embeddings |  |
| `embeddings_export` | `GET /v1/embeddings/export` | mcpx_embeddings_export |  |
| `embeddings_import` | `POST /v1/embeddings/import` | mcpx_embeddings_import |  |
| `events` | `GET /v1/events` | mcpx_events | S |
| `exec` | `POST /v1/exec` | mcpx_exec (via mcpx_exec) |  |
| `feedback` | `POST /v1/feedback` | mcpx_feedback |  |
| `globals` | `GET /v1/globals.d.ts` | mcpx_globals |  |
| `health` | `GET /v1/health` | mcpx_health |  |
| `intent` | `POST /v1/intent` | mcpx_intent |  |
| `interactions_list` | `GET /v1/interactions` | mcpx_interactions_list |  |
| `interactions_get` | `GET /v1/interactions/{id}` | mcpx_interactions_get |  |
| `log_query` | `GET /v1/log` | mcpx_log_query |  |
| `log_record` | `POST /v1/log` | mcpx_log_record |  |
| `metrics_tokens` | `GET /v1/metrics/tokens` | mcpx_metrics_tokens |  |
| `metrics_tools` | `GET /v1/metrics/tools` | mcpx_metrics_tools |  |
| `namespaces` | `GET /v1/namespaces` | mcpx_namespaces (via mcpx_namespaces) |  |
| `openapi` | `GET /v1/openapi.json` | mcpx_openapi |  |
| `preflight` | `POST /v1/preflight` | mcpx_preflight |  |
| `prompt_get` | `POST /v1/prompt` | mcpx_prompt_get |  |
| `prompts` | `GET /v1/prompts` | mcpx_prompts |  |
| `protocol` | `GET /v1/protocol` | mcpx_protocol |  |
| `recipes_list` | `GET /v1/recipes` | mcpx_recipes_list |  |
| `recipe_get` | `GET /v1/recipes/{name}` | mcpx_recipe_get |  |
| `recipe_save` | `POST /v1/recipes/{name}` | mcpx_recipe_save |  |
| `recipe_run` | `POST /v1/recipes/{name}/run` | mcpx_recipe_run |  |
| `refresh` | `POST /v1/refresh` | mcpx_refresh |  |
| `registry_search` | `GET /v1/registry/search` | mcpx_registry_search |  |
| `reload` | `POST /v1/reload` | mcpx_reload |  |
| `resolve` | `GET /v1/resolve` | mcpx_resolve |  |
| `resource_read` | `POST /v1/resource` | mcpx_resource_read |  |
| `resource_templates` | `GET /v1/resource-templates` | mcpx_resource_templates |  |
| `resources` | `GET /v1/resources` | mcpx_resources |  |
| `restart` | `POST /v1/restart` | mcpx_restart |  |
| `search` | `GET /v1/search` | mcpx_search (via mcpx_search) |  |
| `secret_requests_list` | `GET /v1/secret-requests` | mcpx_secret_requests_list |  |
| `secret_request_decline` | `DELETE /v1/secret-requests/{id}` | mcpx_secret_request_decline |  |
| `secret_request_get` | `GET /v1/secret-requests/{id}` | mcpx_secret_request_get |  |
| `secrets_list` | `GET /v1/secrets` | mcpx_secrets_list |  |
| `secrets_unset` | `DELETE /v1/secrets/{name}` | mcpx_secrets_unset |  |
| `secrets_set` | `PUT /v1/secrets/{name}` | — |  |
| `servers_list` | `GET /v1/servers` | mcpx_servers_list |  |
| `servers_add` | `POST /v1/servers` | mcpx_servers_add |  |
| `servers_remove` | `DELETE /v1/servers/{name}` | mcpx_servers_remove |  |
| `servers_get` | `GET /v1/servers/{name}` | mcpx_servers_get |  |
| `session_release` | `POST /v1/session/release` | mcpx_session_release |  |
| `settings_list` | `GET /v1/settings` | mcpx_settings_list |  |
| `settings_unset` | `DELETE /v1/settings/{path}` | mcpx_settings_unset |  |
| `settings_get` | `GET /v1/settings/{path}` | mcpx_settings_get |  |
| `settings_set` | `PUT /v1/settings/{path}` | mcpx_settings_set |  |
| `shutdown` | `POST /v1/shutdown` | mcpx_shutdown |  |
| `stats_query` | `GET /v1/stats` | mcpx_stats_query |  |
| `status` | `GET /v1/status` | mcpx_status (via mcpx_status) |  |
| `tasks_list` | `GET /v1/tasks` | mcpx_tasks_list |  |
| `task_get` | `GET /v1/tasks/{id}` | mcpx_task_get |  |
| `task_cancel` | `POST /v1/tasks/{id}/cancel` | mcpx_task_cancel |  |
| `task_result` | `GET /v1/tasks/{id}/result` | mcpx_task_result |  |
| `tools` | `GET /v1/tools` | mcpx_tools |  |
| `tool_invoke` | `POST /v1/tools/{tool}` | mcpx_tool_invoke |  |
| `types` | `GET /v1/types` | mcpx_types (via mcpx_types) |  |

## CLI commands (69), by group

| Group | Commands |
| --- | --- |
| configuration | `adapter`, `api`, `completion`, `config`, `help`, `init`, `man`, `plugin`, `registry`, `secrets`, `servers`, `settings` |
| daemon | `cache`, `daemon`, `daemons`, `refresh`, `reload`, `restart`, `serve`, `status`, `stdio-proxy`, `stop`, `supervise`, `upgrade` |
| discovery | `batch`, `call`, `catalog`, `client`, `ls (list,namespaces)`, `openapi`, `prompts`, `resources`, `schema`, `search`, `types` |
| inspection | `dashboard`, `diagnose`, `doctor`, `elicit`, `explore`, `hook`, `interactions`, `log (logs)`, `stats`, `tui` |
| operations | `artifact (artifacts)`, `ask`, `complete`, `events`, `globals`, `health`, `history`, `metrics`, `preflight`, `protocol`, `resolve`, `session`, `task (tasks)`, `tool`, `tools` |
| running | `embeddings`, `exec`, `feedback`, `prompt`, `proxy`, `recipes`, `run`, `scripts`, `skills` |

## Settings (197)

Full list is `settings.Registry()` output; by prefix:

| Prefix | Count |
| --- | --- |
| `artifacts` | 10 |
| `autonomy` | 1 |
| `autostart` | 7 |
| `catalog` | 4 |
| `completion` | 1 |
| `daemon` | 22 |
| `diagnose` | 2 |
| `doctor` | 1 |
| `elicit` | 7 |
| `embeddings` | 5 |
| `events` | 2 |
| `exec` | 3 |
| `hooks` | 3 |
| `http` | 13 |
| `logging` | 11 |
| `logstore` | 2 |
| `mcp` | 2 |
| `output` | 2 |
| `paths` | 9 |
| `plugin` | 12 |
| `plumbing` | 8 |
| `pool` | 7 |
| `preset` | 1 |
| `presets` | 1 |
| `prompt` | 6 |
| `proto` | 9 |
| `protoMessages` | 3 |
| `protoTasks` | 1 |
| `recipes` | 3 |
| `registry` | 5 |
| `repair` | 1 |
| `retention` | 2 |
| `script` | 14 |
| `search` | 2 |
| `serve` | 1 |
| `session` | 1 |
| `spec` | 3 |
| `stats` | 1 |
| `tasks` | 2 |
| `transport` | 4 |
| `upstream` | 3 |

## Plugin (`plugin/opencode/mcpx/`)

| File | What it covers | Run in CI? |
| --- | --- | --- |
| `daemon.test.ts` | discovery ladder against real unix sockets (`Bun.serve({unix})`) | **no** — no workflow runs `bun test` |
| `ceiling.test.ts` | ceiling behaviour | **no** |
| `ops.test.ts` | `DaemonOps` against a fake recording daemon | **no** |
| `instructions.test.ts` | plugin instructions | **no** |

Local `bun test plugin/opencode/mcpx/`: 24 pass, 5 skip, 21 fail — **all 21 failures are `EPERM` on unix-socket `listen`, a sandbox restriction** (verified: zero non-socket failures). The tests require real sockets by design; GitHub-hosted runners allow socket binds, so they are expected green in CI. This run adds the CI step (see PR).

## What the existing guards cover — and the hole

| Guard | Covers | Does NOT assert |
| --- | --- | --- |
| `TestEveryDeclaredRouteAnswersOverTheSocket` (`e2e/v1ops_test.go:287`) | every op route answers | that any test *exercises* the op’s behaviour |
| `TestEverySettingIsReachableOverV1` (`e2e/settings_test.go:173`) | every setting reachable | behavioural exercise |
| `TestEveryExecFlagChangesSomething` (`e2e/e2e_test.go:2958`) | every exec flag changes something | per-op exercise |
| `TestEveryNonStreamingOpIsReachableAsAnMCPTool` (`api/parity_test.go:96`) | 80/81 ops as tools (streaming excluded) | exercise |
| `TestEverySettingIsReadSomewhere` (`settings/consumed_test.go:87`) | every setting read | exercise |
| `TestNoPackageRedeclaresADefault` (`defaults/guard_test.go:93`) | default uniqueness | n/a |
| `bun test` step (this run) | plugin tests run at all | per-hook exercise |

**The hole: no feature has a recorded behavioural-exercise mapping.** The `exercised-by` column this inventory needs can only be filled by the runtime ledger (Approach C, issue criterion 2): e2e harness logs the resolved command/op per test, `TestMain` writes `{test, kind, name}` JSON lines, a guard compares against these three registries plus a checked-in exemption file. Until that guard lands, *every* group above is trivially "a feature with no proven integration test" — per-group gap issues would just restate this section, so they wait on the guard.

## Guard design pin (Approach C — for the follow-up implementation issue)

- **Enumerate from:** `api.Ops()`, `cli.Commands()` (+ `App.Handlers()` keys for aliases), `settings.Registry()`, and the plugin op surface (`ops.gen.ts`, generated — assert freshness in the same guard).
- **Hook points:** the e2e client call path (log op name per call) and CLI dispatch (log resolved command); both keyed by current `t.Name()`.
- **`TestMain`:** appends `{test, kind, name}` JSON lines to a ledger file.
- **Guard test:** fails CI when a registry entry has no ledger row and no exemption-file entry (one reason per entry). Must fail on a fabricated unexercised op/command/setting and pass on main.
- **Build on** `scripts/gotest-report.py` + `MCPX_GO_TESTS` record conventions for skip/record handling; do not invent a second record system.

