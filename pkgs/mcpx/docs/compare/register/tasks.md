# Tasks

```
created:      2026-09-30T08:00:00-05:00
last-updated: 2026-09-30T08:00:00-05:00
increment:    1
status:       standard
tags:         area:compare
description:  2025-11-25 core tasks versus the 2026 io.modelcontextprotocol/tasks extension, difference by difference, and what mcpx implements
```

A task turns a long request into a handle a client can poll, cancel and collect later. The 2025-11-25 core and 2026 extension have different wire formats. Task observations below reflect phases 1–2 of [#1554](https://github.com/developing-today/code/issues/1554); other observations retain their original snapshot. Limits, cancellation, audit, ID entropy and upstream input relay remain open questions on #1554.

Status column: updated task rows are current; other rows are **mcpx @ 05c78b2**. Legend and citation conventions: [../README.md](../README.md).

| ID | Difference | Labels | Where it exists | mcpx | Value | Effort | Risk |
| --- | --- | --- | --- | --- | --- | --- | --- |
| TASK-01 | Tasks leave the core and become an extension | `2025-11-25 has` `2026-07-28 removes` `has better replacement` `specs conflict` | `25-11 core · 26-07 extension only` | Separate legacy and extension paths (`internal/mcpserver/tasks.go:maybeTask`; `internal/mcpserver/server.go:capabilities`). | + high | L | high |
| TASK-02 | Client-hosted tasks (sampling, elicitation) removed; `tools/call` only | `2025-11-25 has` `2026-07-28 removes` | `25-11 server + client · 26-07 server only`; opencode, lootbox declare none | ✓ — `tools/call` only; no client tasks declared | − moot | S | low |
| TASK-03 | Opt-in moves from `params.task` to the server's discretion | `2026-07-28 removes` `has better replacement` | `25-11 client asks · 26-07 server decides` | Legacy opt-in; modern server decides (`internal/mcpserver/tasks.go:maybeTask`). | + med | M | med |
| TASK-04 | `tasks.list` declared without requestor binding | `2025-11-25 has` `mcpx missing` | `25-11 only` | Lists connection-owned tasks only; unowned tasks remain readable by ID (`internal/mcpserver/tasks.go:visible`, `handleTask`; #1554). | + med | S | med |
| TASK-05 | `CreateTaskResult`: `{task}` → flat `Task` with `resultType: "task"` | `2026-07-28 has` | `25-11 nested · 26-07 flat` | Legacy `{task}`; modern flat `resultType: "task"` (`internal/mcpserver/tasks.go:maybeTask`, `created`; `internal/mcpclient/tasks.go:finishModernTask`). | + high | S | high |
| TASK-06 | `Task.ttl` → `ttlMs` | `2026-07-28 has` | `25-11 ttl · 26-07 ttlMs` | Legacy `ttl`, modern `ttlMs` (`internal/tasks/tasks.go:Task`; `internal/mcpserver/tasks.go:modernTask`; `internal/mcpclient/tasks.go:modernTaskResult.UnmarshalJSON`). | + low | S | low |
| TASK-07 | `Task.pollInterval` → `pollIntervalMs` | `2026-07-28 has` | `25-11 pollInterval · 26-07 pollIntervalMs` | Legacy `pollInterval`, modern `pollIntervalMs` (`internal/tasks/tasks.go:Task`; `internal/mcpserver/tasks.go:modernTask`; `internal/mcpclient/tasks.go:finishModernTask`). | + low | S | low |
| TASK-08 | `tasks/get` returns a `DetailedTask` with inline result or error | `2026-07-28 has` `specs conflict` | `25-11 status only · 26-07 detailed` | Modern get inlines terminal result/error or pending input (`internal/mcpserver/tasks.go:detailedNow`, `handleModernTask`; `internal/mcpclient/tasks.go:finishModernTask`). | + med | M | med |
| TASK-09 | Blocking `tasks/result` removed | `2026-07-28 removes` `has better replacement` | `25-11 only`; extension: `-32601` | Modern method-not-found; legacy result remains (`internal/mcpserver/tasks.go:handleModernTask`, `handleTask`). | + low | S | low |
| TASK-10 | `tasks/list` removed | `2026-07-28 removes` `has no replacement` | `25-11 only` | Modern method-not-found; legacy list scoped to connection (`internal/mcpserver/tasks.go:handleModernTask`, `handleTask`). | + low | S | med |
| TASK-11 | `tasks/update` added for mid-task input | `2026-07-28 has` | `26-07 extension only` | Modern update answers outstanding asked-task questions, ignoring unknown keys (`internal/mcpserver/tasks.go:updateAsked`, `handleModernTask`). | + med | M | med |
| TASK-12 | `tasks/cancel` result: `Task` → empty acknowledgement | `2026-07-28 has` `mcpx missing` | `25-11 Task, MUST cancel · 26-07 empty ack, cooperative` | Modern empty acknowledgement, legacy snapshot; modern cancellation still forces terminal state (`internal/mcpserver/tasks.go:handleTask`, `handleModernTask`; `internal/tasks/tasks.go:CancelWithReason`; #1554). | + low | S | low |
| TASK-13 | `tasks/cancel` on a finished task | `2025-11-25 has` | `25-11 MUST -32602 · 26-07 empty ack` | Legacy terminal cancel rejected; modern acknowledged (`internal/mcpserver/tasks.go:handleTask`, `handleModernTask`). | + low | S | low |
| TASK-14 | Status push: `notifications/tasks/status` → `notifications/tasks` on listen | `2025-11-25 has` `2026-07-28 has` | `25-11 unsolicited · 26-07 listen filter taskIds` | Modern listen supports task notifications (`internal/tasks/tasks.go:Watch`; `internal/mcpserver/listen_tasks_test.go:TestListenDeliversTaskStatusNotifications`). | + low | M | low |
| TASK-15 | `io.modelcontextprotocol/related-task` `_meta` | `2025-11-25 has` `2026-07-28 removes` `mcpx missing` | `25-11 MUST · 26-07 —` | Legacy result and task-related input requests carry related-task metadata (`internal/mcpserver/tasks.go:handleTask`; `internal/conformance/stateful_test.go:TestStatefulServer`). | + low | S | low |
| TASK-16 | `io.modelcontextprotocol/model-immediate-response` hint | `2025-11-25 has` `2026-07-28 removes` `has no replacement` | `25-11 only` | n/a — never sent | − moot | S | low |
| TASK-17 | `failed` versus `isError` semantics inverted | `2026-07-28 has` `specs conflict` | `25-11 isError → failed · 26-07 isError → completed` | Legacy store marks isError failed; modern detailed result reports completed (`internal/tasks/tasks.go:finish`; `internal/mcpserver/tasks.go:detailedNow`). | + low | S | low |
| TASK-18 | Mid-task input: `tasks/result` side channel → `inputRequests` + `tasks/update` | `2026-07-28 has` `mcpx missing` | `25-11 side channel · 26-07 polled` | Server relays asked-task input via get/update; upstream input_required yields a named error instead (`internal/mcpserver/tasks.go:driveAsked`, `updateAsked`; `internal/mcpclient/tasks.go:TaskInputRequiredError`; #1554). | + med | M | med |
| TASK-19 | Progress on tasks: supported → not supported | `2025-11-25 has` `2026-07-28 removes` | `25-11 token lives with task · 26-07 none` | n/a — mcpx sends no progress | − moot | S | low |
| TASK-20 | Durable creation before `CreateTaskResult` | `2026-07-28 has` | `26-07 extension only` | Injected SQLite store saves before returning and can reopen; database write errors are ignored (`internal/tasks/tasks.go:StartWithDeadline`, `saveTaskLocked`, `Open`; `internal/mcpserver/tasks.go:tasks`). | + low | L | low |
| TASK-21 | HTTP routing: `Mcp-Name` = `taskId` on `tasks/*` | `2026-07-28 has` | `26-07 extension only` | Client sets task routing headers (`internal/mcpclient/headers_tasks_test.go:TestStandardHeadersTaskID`); server header validation is a separate transport concern. | + low | S | low |
| TASK-22 | Undeclared task support: SHOULD NOT create → `-32021` | `2026-07-28 has` | `25-11 SHOULD NOT · 26-07 MUST -32021` | Modern undeclared clients get -32021 for extension methods and required tools (`internal/mcpserver/tasks.go:missingTasks`, `handleTask`, `maybeTask`). | + low | S | low |
| TASK-23 | Task IDs unguessable | `2025-11-25 has` `2026-07-28 has` | `25-11 MUST when unbound · 26-07 MUST` | Open: 64 random bits (`internal/tasks/tasks.go:NewID`); entropy is an open question on #1554; #2869 is unmerged. | + low | S | med |
| TASK-24 | Tasks bound to the requestor; checked on every request | `2025-11-25 has` `mcpx missing` | `25-11 MUST with auth context · 26-07 MUST` | Connection-owned legacy tasks isolated; unowned tasks accessible by ID and ownership is in memory (`internal/mcpserver/tasks.go:startTask`, `visible`, `handleTask`; #1554). | + med | S | high |
| TASK-25 | Reserved names `tasks/`, `notifications/tasks/`, `resultType: "task"` | `2026-07-28 has` `specs conflict` | `26-07 extension only` | n/a — nothing reserved clashes | − moot | S | low |
| TASK-26 | MRTR rounds resolved before a `CreateTaskResult` | `2026-07-28 has` | `26-07 + extension` | Server answers sendable questions inline before returning a task (`internal/mcpserver/tasks.go:askTask`, `maybeTask`). | + low | M | low |
| TASK-27 | One task store across MCP and `/v1` | | mcpx only | Daemon injects its task store into MCP (`internal/daemon/ops.go:TaskStore`; `internal/mcpserver/tasks.go:tasks`; `internal/cli/serve.go:MCPServer`, `internal/cli/root.go`). | + med | S | med |

## TASK-01 Tasks leave the core and become an extension

- **What.** The 2025-11-25 core "experimental" tasks (`tasks/*` methods, `Task*` types, `tasks` capabilities,
  `Tool.execution`) are absent from the 2026 schema. The redesigned feature is `io.modelcontextprotocol/tasks`, negotiated
  through `capabilities.extensions`. SEP-2663 says it is not wire-compatible with 2025-11-25 and must not be enabled under
  2025-11-25.
- **Where.** Core in 2025-11-25; extension only with 2026-07-28.
- **mcpx (current).** Separate legacy and extension paths (`internal/mcpserver/tasks.go:maybeTask`; `internal/mcpserver/server.go:capabilities`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + high: a real extension implementation would let long upstream calls survive a 2026 host's
  disconnect; declaring what is not delivered is the build brief's "declared but not delivered" bug class.
- **Effort.** L — new result shape, update and poll flow, notifications; the rows below are the parts.
- **Risk.** high: 2026 hosts that trust the advertisement send and expect extension shapes mcpx does not produce.
- **Detail.** Dual support means parallel code paths, not one shape with renames. SEP-2663 calls the 2026 revision
  `2026-06-30`, presumably a pre-release name. The capability-level leaks (`extensions` to 2025-11-25 hosts, core `tasks`
  to 2026 hosts) are also registered in the capabilities and lifecycle registers.
- **Sources.** `2026-07-28/changelog.mdx:22` "Move experimental tasks out of the core protocol and into an official extension";
  `seps/2663-tasks-extension.md:944` "is **not wire-compatible** with this extension.";
  `seps/2663-tasks-extension.md:17`; `docs/extensions/tasks/overview.mdx:7`;
  `internal/mcpserver/server.go:1404-1417`; `internal/mcpserver/tasks.go:88-126`; `internal/mcpserver/revisions.go:90`.

## TASK-02 Client-hosted tasks (sampling, elicitation) removed; `tools/call` only

- **What.** 2025-11-25 lets a server task-augment `tools/call` and lets a *client* host tasks for server-sent
  `sampling/createMessage` and `elicitation/create` (client `tasks.requests.*`, `tasks.list`, `tasks.cancel`). The
  extension supports only `tools/call`; a `CreateTaskResult` for any other method MUST be treated as invalid. Client-hosted
  tasks cease to exist, because polling one would be a server-to-client request.
- **Where.** 2025-11-25 only for the client side. opencode v1 comments `tasks` out, v2 does not declare it, and lootbox
  declares no capabilities.
- **mcpx @ 05c78b2.** Server: `tools/call` only (`internal/mcpserver/server.go:477-500`). Client: no `tasks` declared
  upstream (`internal/mcpclient/modern.go:46-58`).
- **Value to mcpx.** − moot: already aligned; implementing client-hosted tasks would be work for one revision.
- **Effort.** S.
- **Risk.** None.
- **Detail.** In 2025-11-25 `CreateMessageRequestParams` and `ElicitRequestFormParams` extend `TaskAugmentedRequestParams`;
  in 2026 they extend nothing. The extension SHOULD be designed to accommodate more methods later.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:71`; `schema/2025-11-25/schema.ts:346` "tasks?: {";
  `schema/2025-11-25/schema.ts:1578` "export interface CreateMessageRequestParams extends TaskAugmentedRequestParams {";
  `schema/2026-07-28/schema.ts:2104` "export interface CreateMessageRequestParams {";
  `seps/2663-tasks-extension.md:35` "**Client-hosted tasks are no longer expressible.**";
  `seps/2663-tasks-extension.md:87`; `v1:packages/opencode/src/mcp/index.ts:48` "// tasks: {},";
  `v2:packages/core/src/mcp/client.ts:142`; `.lootbox/src/lib/external-mcps/mcp_client_manager.ts:150-153`.

## TASK-03 Opt-in moves from `params.task` to the server's discretion

- **What.** 2025-11-25 clients add `params.task: {ttl?}` to ask for task execution. The extension removes `task`; servers
  MUST ignore it, and the server alone decides per request whether to return a task to a client that declared the
  extension.
- **Where.** `task` param in 2025-11-25 only.
- **mcpx (current).** Legacy opt-in; modern server decides (`internal/mcpserver/tasks.go:maybeTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: server-chosen tasks would let mcpx hand back a handle for a slow upstream call without the
  host asking, which is the extension's whole model.
- **Effort.** M.
- **Risk.** med: a 2026 host never sends `task`, so mcpx's task path is unreachable for exactly the hosts it advertises the
  extension to.
- **Detail.** The 2026 changelog's "without per-request opt-in" still requires the extension in each request's
  `clientCapabilities`; the overview says "Never return a task to a client that did not declare support". The two read as
  contradictory until "opt-in" is taken to mean the removed `task` param.
- **Sources.** `schema/2025-11-25/schema.ts:44` "task?: TaskMetadata;";
  `seps/2663-tasks-extension.md:85` "The server is the sole decider; clients do not signal task preference on the request itself.";
  `docs/extensions/tasks/overview.mdx:231-233`; `internal/mcpserver/tasks.go:40-56`.

## TASK-04 `tasks.list` declared without requestor binding

- **What.** A 2025-11-25 receiver that cannot identify requestors SHOULD NOT declare `tasks.list`, because listing exposes
  every task's metadata to anyone.
- **Where.** 2025-11-25 only.
- **mcpx (current).** Lists connection-owned tasks only; unowned tasks remain readable by ID (`internal/mcpserver/tasks.go:visible`, `handleTask`; #1554).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: removes cross-host metadata exposure.
- **Effort.** S — stop declaring it, or bind first (TASK-24).
- **Risk.** med: on a shared daemon any host sees every host's task ids and can collect their results.
- **Detail.** Task ids are 64 random bits (TASK-23), so listing is the easy path to someone else's result.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:876` "**SHOULD NOT** declare the";
  `internal/mcpserver/server.go:1412`; `internal/mcpserver/tasks.go:89-95`; wire W23.

## TASK-05 `CreateTaskResult`: `{task}` → flat `Task` with `resultType: "task"`

- **What.** 2025-11-25 `CreateTaskResult` is `{ task: Task }`. The extension's is `Result & Task` with
  `resultType: "task"`, a value the extension reserves and advertises.
- **Where.** As stated.
- **mcpx (current).** Legacy `{task}`; modern flat `resultType: "task"` (`internal/mcpserver/tasks.go:maybeTask`, `created`; `internal/mcpclient/tasks.go:finishModernTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + high: without it no 2026 host can use an mcpx task.
- **Effort.** S for the shape, given the rest of TASK-01.
- **Risk.** high: a 2026 host reads `resultType: "complete"` and treats `{task: …}` as an empty tool result.
- **Detail.** `resultType` values beyond core are legal only when advertised through capabilities. As a client, mcpx treats
  any `resultType` other than `input_required` as complete (`internal/mcpclient/client.go:665`), so a modern upstream's
  `"task"` would be parsed as a tool result too.
- **Sources.** `schema/2025-11-25/schema.ts:1392` "task: Task;"; `seps/2663-tasks-extension.md:262`
  "type CreateTaskResult = Result & Task;"; `seps/2663-tasks-extension.md:128`; `docs/extensions/tasks/overview.mdx:54-56`;
  `internal/mcpserver/server.go:499-500`; `2026-07-28/basic/index.mdx:83`; wire W22.

## TASK-06 `Task.ttl` → `ttlMs`

- **What.** Renamed; still milliseconds, `null` for unlimited. The extension adds that it MAY change over the task's
  lifetime. The request-side `TaskMetadata.ttl` disappears with the `task` param.
- **Where.** `ttl` in 2025-11-25; `ttlMs` in the extension.
- **mcpx (current).** Legacy `ttl`, modern `ttlMs` (`internal/tasks/tasks.go:Task`; `internal/mcpserver/tasks.go:modernTask`; `internal/mcpclient/tasks.go:modernTaskResult.UnmarshalJSON`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: a rename.
- **Effort.** S.
- **Risk.** low: extension clients see no TTL.
- **Detail.** The TTL comes from the client's `task.ttl` or `plumbing.taskTTL` (10 m).
- **Sources.** `schema/2025-11-25/schema.ts:1378` "ttl: number | null;"; `seps/2663-tasks-extension.md:168`
  "ttlMs: number | null;"; `internal/tasks/tasks.go:36`.

## TASK-07 `Task.pollInterval` → `pollIntervalMs`

- **What.** Renamed. Servers MAY rate-limit clients that poll faster.
- **Where.** `pollInterval` in 2025-11-25; `pollIntervalMs` in the extension.
- **mcpx (current).** Legacy `pollInterval`, modern `pollIntervalMs` (`internal/tasks/tasks.go:Task`; `internal/mcpserver/tasks.go:modernTask`; `internal/mcpclient/tasks.go:finishModernTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low.
- **Effort.** S.
- **Risk.** low: extension clients see no interval.
- **Detail.** None beyond the rename.
- **Sources.** `schema/2025-11-25/schema.ts:1383` "pollInterval?: number;"; `seps/2663-tasks-extension.md:175`
  "pollIntervalMs?: number;"; `internal/tasks/tasks.go:37`.

## TASK-08 `tasks/get` returns a `DetailedTask` with inline result or error

- **What.** 2025-11-25 `GetTaskResult = Result & Task`, a status only. The extension returns status-discriminated
  variants: `input_required` carries `inputRequests`, `completed` carries `result`, `failed` carries `error`; `resultType`
  MUST be `"complete"` on it.
- **Where.** As stated.
- **mcpx (current).** Modern get inlines terminal result/error or pending input (`internal/mcpserver/tasks.go:detailedNow`, `handleModernTask`; `internal/mcpclient/tasks.go:finishModernTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: one polling call gives everything, which suits hosts that reconnect.
- **Effort.** M.
- **Risk.** med: see TASK-01.
- **Detail.** SEP-2663 contradicts itself: it requires `"complete"` here but its own execution-error examples show
  `"resultType": "task"`.
- **Sources.** `schema/2025-11-25/schema.ts:1415` "export type GetTaskResult = Result & Task;";
  `seps/2663-tasks-extension.md:337` "type GetTaskResult = Result & DetailedTask;";
  `seps/2663-tasks-extension.md:340`; `seps/2663-tasks-extension.md:844` "\"resultType\": \"task\",";
  `internal/mcpserver/tasks.go:97-102`.

## TASK-09 Blocking `tasks/result` removed

- **What.** 2025-11-25 `tasks/result` blocks until the task is terminal, returns its payload, and is where mid-task input
  requests travel. The extension removes it; clients calling it MUST get `-32601`, and results move into `tasks/get`.
- **Where.** 2025-11-25 only.
- **mcpx (current).** Modern method-not-found; legacy result remains (`internal/mcpserver/tasks.go:handleModernTask`, `handleTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: accept-liberally is harmless as long as it is not advertised to 2026 hosts, which it is
  (CAP-26).
- **Effort.** S.
- **Risk.** low.
- **Detail.** Blocking until the end is exactly what the redesign moved away from, because a blocking request is lost with
  its connection.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:465` "it **MUST** block the response until the task reaches a terminal status";
  `seps/2663-tasks-extension.md:949` "is removed; clients calling it"; `internal/mcpserver/tasks.go:104-117`.

## TASK-10 `tasks/list` removed

- **What.** Listing cannot be scoped without sessions, so the extension removes it with no replacement: clients must keep
  the task ids they were given.
- **Where.** 2025-11-25 only.
- **mcpx (current).** Modern method-not-found; legacy list scoped to connection (`internal/mcpserver/tasks.go:handleModernTask`, `handleTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: stop serving it to hosts that declared the extension; for legacy hosts the fix is TASK-24.
- **Effort.** S.
- **Risk.** med: SEP-2663's own argument, "a server cannot inadvertently leak the existence of one caller's tasks to
  another", applies directly to mcpx's shared store.
- **Detail.** 2025-11-25 required every task visible to `tasks/get` for a requestor to be listable for that requestor.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:490` "it **MUST** be retrievable via";
  `seps/2663-tasks-extension.md:957` "a server cannot inadvertently leak the existence of one caller's tasks to another.";
  `internal/mcpserver/tasks.go:89-95`; wire W23.

## TASK-11 `tasks/update` added for mid-task input

- **What.** `tasks/update {taskId, inputResponses}` answers a task's outstanding `inputRequests`. The reply is an empty
  acknowledgement and eventually consistent; unknown or already-answered keys SHOULD be ignored; partial sets are allowed.
- **Where.** Extension only.
- **mcpx (current).** Modern update answers outstanding asked-task questions, ignoring unknown keys (`internal/mcpserver/tasks.go:updateAsked`, `handleModernTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: input for long tasks without holding a request open; it maps directly onto the broker and
  `Ask.Reply` machinery MRTR already uses.
- **Effort.** M.
- **Risk.** med: extension hosts cannot answer mid-task questions.
- **Detail.** Input-request keys MUST be unique over the task's whole lifetime, stronger than MRTR's per-request uniqueness.
- **Sources.** `seps/2663-tasks-extension.md:358` "tasks/update"; `seps/2663-tasks-extension.md:352` "Each request key in";
  `docs/extensions/tasks/overview.mdx:36-39`; `internal/mcpserver/tasks.go:126` "return fail(codeMethodNotFound, \"no method \"+req.Method)".

## TASK-12 `tasks/cancel` result: `Task` → empty acknowledgement

- **What.** 2025-11-25 returns the `Task`, MUST move it to `cancelled` before responding, and `cancelled` is sticky. The
  extension returns an empty acknowledgement; cancellation is cooperative and the task may still end in another terminal
  state.
- **Where.** As stated.
- **mcpx (current).** Modern empty acknowledgement, legacy snapshot; modern cancellation still forces terminal state (`internal/mcpserver/tasks.go:handleTask`, `handleModernTask`; `internal/tasks/tasks.go:CancelWithReason`; #1554).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: an acknowledgement is simpler for an asynchronous daemon.
- **Effort.** S.
- **Risk.** low.
- **Detail.** In both designs `notifications/cancelled` MUST NOT be used to cancel a task.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:495` "**MUST** transition the task to";
  `seps/2663-tasks-extension.md:403` "type CancelTaskResult = Result; // empty acknowledgement";
  `seps/2663-tasks-extension.md:408` "Cancellation is **cooperative**"; `internal/mcpserver/tasks.go:119-124`.

## TASK-13 `tasks/cancel` on a finished task

- **What.** 2025-11-25: cancelling a task already `completed`, `failed` or `cancelled` MUST be rejected with `-32602`. The
  extension acknowledges with an empty result.
- **Where.** 2025-11-25 MUST; extension changes it.
- **mcpx (current).** Legacy terminal cancel rejected; modern acknowledged (`internal/mcpserver/tasks.go:handleTask`, `handleModernTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low.
- **Effort.** S.
- **Risk.** low.
- **Detail.** Neither revision's answer is what mcpx sends: 2025-11-25 wants an error, the extension an empty object.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:494` "**MUST** reject cancellation requests for tasks already in a terminal status";
  `internal/mcpserver/tasks.go:124` "return reply(snap)".

## TASK-14 Status push: `notifications/tasks/status` → `notifications/tasks` on listen

- **What.** 2025-11-25 receivers MAY push `notifications/tasks/status` unsolicited. The extension renames it
  `notifications/tasks`, carries the full `DetailedTask`, and delivers it only on a `subscriptions/listen` stream whose
  filter lists `taskIds` (a field the extension adds to the core filter).
- **Where.** As stated.
- **mcpx (current).** Modern listen supports task notifications (`internal/tasks/tasks.go:Watch`; `internal/mcpserver/listen_tasks_test.go:TestListenDeliversTaskStatusNotifications`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: saves hosts polling.
- **Effort.** M — needs the listen stream to accept `taskIds` ([notifications.md](notifications.md)).
- **Risk.** low: optional in both designs.
- **Detail.** The overview page does not mention `taskIds`; only SEP-2663 does.
- **Sources.** `schema/2025-11-25/schema.ts:1496` "notifications/tasks/status"; `seps/2663-tasks-extension.md:422`
  "notifications/tasks"; `seps/2663-tasks-extension.md:435` "taskIds?: string[];";
  `docs/protocol.md:297-299` "mcpx polls task state rather than tracking it"; `internal/mcpclient/client.go:478`.

## TASK-15 `io.modelcontextprotocol/related-task` `_meta`

- **What.** 2025-11-25 requires every task-related request, notification and response (including the `tasks/result`
  response) to carry `_meta["io.modelcontextprotocol/related-task"] = {taskId}`. Neither the 2026 core nor SEP-2663 defines
  it.
- **Where.** 2025-11-25 only.
- **mcpx (current).** Legacy result and task-related input requests carry related-task metadata (`internal/mcpserver/tasks.go:handleTask`; `internal/conformance/stateful_test.go:TestStatefulServer`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: lets a 2025-11-25 host tie a mid-task question to its task. The upstream direction would give
  mcpx exact attribution of questions on shared instances ([elicitation.md](elicitation.md), ELI-19).
- **Effort.** S.
- **Risk.** low.
- **Detail.** For `tasks/get`, `tasks/result` and `tasks/cancel`, receivers MUST prefer the `taskId` param over this key.
  With no server-to-client requests in 2026 there is nothing left to associate.
- **Also recorded from the meta register.** No `related-task` anywhere in `internal/`; `tasks/result` results and
  questions raised while a task runs carry none (`internal/mcpserver/tasks.go:104-117`). The tasks extension replaces
  it with `inputRequests` on `tasks/get`; see the tasks register.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:470` "All requests, notifications, and responses related to a task";
  `schema/2025-11-25/schema.ts:1332` "export interface RelatedTaskMetadata {"; `schema/2025-11-25/schema.ts:1328`;
  `internal/mcpserver/tasks.go:104-117`.

## TASK-16 `io.modelcontextprotocol/model-immediate-response` hint

- **What.** A non-binding, "provisional" `_meta` string on a 2025-11-25 `CreateTaskResult` that a host may show the model as
  an interim tool result. The extension has no such key.
- **Where.** 2025-11-25 only.
- **mcpx @ 05c78b2.** Never sent (no occurrence in `internal/`).
- **Value to mcpx.** − moot: mcpx could synthesise its own interim text if it ever needed one.
- **Effort.** S.
- **Risk.** None.
- **Detail.** None.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:175` "servers can provide an optional".

## TASK-17 `failed` versus `isError` semantics inverted

- **What.** 2025-11-25: a tool result with `isError: true` puts the task in `failed`. Extension: `failed` MUST NOT be used
  for `isError`; that is `completed` with the result, and `failed` is only for JSON-RPC errors, carried in `error`.
- **Where.** As stated.
- **mcpx (current).** Legacy store marks isError failed; modern detailed result reports completed (`internal/tasks/tasks.go:finish`; `internal/mcpserver/tasks.go:detailedNow`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: the extension's rule is the cleaner one for a proxy.
- **Effort.** S.
- **Risk.** low: clients switching on status misreport tool errors between the two designs.
- **Detail.** A dual-era server has to pick per revision.
- **Sources.** `schema/2025-11-25/schema.ts:1310` "For tool calls specifically, this includes cases where the tool call result has";
  `seps/2663-tasks-extension.md:835`; `internal/mcpserver/tasks.go:62-64`.

## TASK-18 Mid-task input: `tasks/result` side channel → `inputRequests` + `tasks/update`

- **What.** 2025-11-25: on `input_required` the requestor SHOULD call `tasks/result` so the receiver can send
  `elicitation/create` or sampling on that stream. Extension: pending requests appear in `tasks/get` as `inputRequests`
  and are answered with `tasks/update`.
- **Where.** As stated.
- **mcpx (current).** Server relays asked-task input via get/update; upstream input_required yields a named error instead (`internal/mcpserver/tasks.go:driveAsked`, `updateAsked`; `internal/mcpclient/tasks.go:TaskInputRequiredError`; #1554).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: fits mcpx's broker model, where questions are already data.
- **Effort.** M.
- **Risk.** med: a task whose upstream asks something sits at `input_required` with no way to answer through tasks.
- **Detail.** The statuses themselves (`working`, `input_required`, `completed`, `failed`, `cancelled`) are the same five in
  both designs. The task-phase `inputRequests` key space is separate from any MRTR phase before it (TASK-26).
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:449` "When the requestor encounters the";
  `seps/2663-tasks-extension.md:184` "will include outstanding requests in the"; `internal/mcpserver/tasks.go:69-73`.

## TASK-19 Progress on tasks: supported → not supported

- **What.** 2025-11-25: the original `progressToken` stays valid for the task's life, and notifications stop when it is
  terminal. Extension: `notifications/progress` and `notifications/message` are not supported on tasks and MUST NOT go on
  the listen stream for a task; `statusMessage` is the only progress channel left.
- **Where.** As stated.
- **mcpx @ 05c78b2.** Sends no progress at all ([progress-cancellation.md](progress-cancellation.md)).
- **Value to mcpx.** − moot.
- **Effort.** S.
- **Risk.** None.
- **Detail.** `SetTaskStatus` takes a message, so `statusMessage` is available to mcpx already.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:483` "Task-augmented requests support progress notifications";
  `2025-11-25/basic/utilities/progress.mdx:69`; `seps/2663-tasks-extension.md:511` "are not supported on tasks in general in this specification.".

## TASK-20 Durable creation before `CreateTaskResult`

- **What.** New MUST in the extension: a server returns `CreateTaskResult` only once `tasks/get` for that id would succeed.
- **Where.** Extension only.
- **mcpx (current).** Injected SQLite store saves before returning and can reopen; database write errors are ignored (`internal/tasks/tasks.go:StartWithDeadline`, `saveTaskLocked`, `Open`; `internal/mcpserver/tasks.go:tasks`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: satisfied in-process; restart durability is a separate, larger feature.
- **Effort.** L for durability across daemon restarts.
- **Risk.** low: tasks vanish on restart, which servers may do (they may purge), but it surprises hosts that persisted ids.
- **Detail.** The extension also lets servers fail tasks after their TTL.
- **Sources.** `seps/2663-tasks-extension.md:302` "A server **MUST NOT** return"; `internal/mcpserver/tasks.go:31-38`.

## TASK-21 HTTP routing: `Mcp-Name` = `taskId` on `tasks/*`

- **What.** Over Streamable HTTP, `tasks/get`, `tasks/update` and `tasks/cancel` MUST set `Mcp-Name` to `params.taskId`, so
  a load balancer can route to the instance that owns the task.
- **Where.** Extension only.
- **mcpx (current).** Client sets task routing headers (`internal/mcpclient/headers_tasks_test.go:TestStandardHeadersTaskID`); server header validation is a separate transport concern.
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: needed if mcpx becomes a 2026 HTTP client of task-returning upstreams.
- **Effort.** S.
- **Risk.** low: misrouted polls against multi-instance upstreams.
- **Detail.** The headers come from SEP-2243.
- **Sources.** `seps/2663-tasks-extension.md:515` "the client **MUST** set the".

## TASK-22 Undeclared task support: SHOULD NOT create → `-32021`

- **What.** 2025-11-25: if the peer's `capabilities.tasks` is undefined, a requestor SHOULD NOT create tasks. Extension: a
  server that can serve a request only as a task MUST return `-32021` with
  `requiredCapabilities.extensions["io.modelcontextprotocol/tasks"]`, and MUST do the same for `tasks/get`, `tasks/update`
  and `tasks/cancel` from clients that did not declare the extension.
- **Where.** As stated.
- **mcpx (current).** Modern undeclared clients get -32021 for extension methods and required tools (`internal/mcpserver/tasks.go:missingTasks`, `handleTask`, `maybeTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: honest failure for 2026 clients.
- **Effort.** S.
- **Risk.** low.
- **Detail.** Invalid or expired `taskId` → `-32602` (MUST for `tasks/get`, SHOULD for update and cancel); mcpx already
  answers an unknown id with `-32602`. Accepting `tasks/*` from legacy clients is accept-liberally; the extension's MUST
  applies only to 2026 clients.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:99` "attempt to create tasks during requests.";
  `seps/2663-tasks-extension.md:89` "the server **MUST** return an error with the code";
  `seps/2663-tasks-extension.md:799` "Servers **MUST** return this error for non-declaring clients issuing"; wire W22, W23.

## TASK-23 Task IDs unguessable

- **What.** Task ids may act as bearer tokens, so both designs require enough entropy: 2025-11-25 when tasks cannot be bound
  to an authorization context, the extension always.
- **Where.** 2025-11-25 and the extension.
- **mcpx (current).** Open: 64 random bits (`internal/tasks/tasks.go:NewID`); entropy is an open question on #1554; #2869 is unmerged.
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: 128 bits would remove the question.
- **Effort.** S.
- **Risk.** med: with no binding (TASK-24) the id is the only protection, and `tasks/list` hands ids out anyway (TASK-04).
- **Detail.** 2025-11-25 also suggests shorter TTLs when binding is unavailable.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:875` "receivers **MUST** generate cryptographically secure task IDs";
  `seps/2663-tasks-extension.md:955` "Servers **MUST** generate them with sufficient entropy"; `internal/tasks/tasks.go:86-90`.

## TASK-24 Tasks bound to the requestor; checked on every request

- **What.** 2025-11-25: when an authorization context exists, receivers MUST bind tasks to it and reject others' requests;
  without one they SHOULD document the limitation. Extension: authentication and authorization MUST be checked on every
  task request, and task `inputRequests` get the same trust model as direct ones.
- **Where.** 2025-11-25 and the extension.
- **mcpx (current).** Connection-owned legacy tasks isolated; unowned tasks accessible by ID and ownership is in memory (`internal/mcpserver/tasks.go:startTask`, `visible`, `handleTask`; #1554).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: isolation between hosts sharing one daemon.
- **Effort.** S — key tasks by the connection binding now, by principal once `/mcp` has one.
- **Risk.** high: result disclosure between hosts. `/mcp` has no authentication, so the 2025-11-25 MUST does not strictly
  apply, but the SHOULD to document it does, and the docs are silent.
- **Detail.** The same binding question as MRTR's `requestState` ([mrtr.md](mrtr.md), MRTR-08): a session id is what mcpx
  has, and 2026 removed it.
- **Sources.** `2025-11-25/basic/utilities/tasks.mdx:871-876`;
  `seps/2663-tasks-extension.md:956` "Servers **MUST** perform authentication and authorization checks on each task-related request";
  `internal/mcpserver/tasks.go:34-36`; wire W23.

## TASK-25 Reserved names `tasks/`, `notifications/tasks/`, `resultType: "task"`

- **What.** The extension reserves the `tasks/` method prefix, the `notifications/tasks/` notification prefix, and the
  `"task"` result discriminator.
- **Where.** Extension only.
- **mcpx @ 05c78b2.** Nothing of its own uses these names.
- **Value to mcpx.** − moot.
- **Effort.** S.
- **Risk.** None.
- **Detail.** The extension's own notification is `notifications/tasks`, with no trailing slash, so it falls outside the
  `notifications/tasks/` prefix it reserves.
- **Sources.** `seps/2663-tasks-extension.md:894` "prefix are reserved for this extension."; `seps/2663-tasks-extension.md:422`.

## TASK-26 MRTR rounds resolved before a `CreateTaskResult`

- **What.** A `tools/call` may go through one or more `input_required` rounds and then return `resultType: "task"`; servers
  SHOULD resolve all MRTR exchanges synchronously first. The two phases have separate key spaces.
- **Where.** 2026-07-28 with the extension.
- **mcpx (current).** Server answers sendable questions inline before returning a task (`internal/mcpserver/tasks.go:askTask`, `maybeTask`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + low: design guidance for when mcpx implements the extension.
- **Effort.** M.
- **Risk.** low.
- **Detail.** SHOULD, not MUST.
- **Sources.** `seps/2663-tasks-extension.md:304` "resolve all MRTR exchanges _synchronously_ before responding with a".

## TASK-27 One task store across MCP and `/v1`

- **What.** `internal/tasks` says one store is shared so that a task started on one surface is visible from the other. In
  practice the daemon now injects its store into the MCP server.
- **Where.** mcpx internal parity (config = /v1 = MCP).
- **mcpx (current).** Daemon injects its task store into MCP (`internal/daemon/ops.go:TaskStore`; `internal/mcpserver/tasks.go:tasks`; `internal/cli/serve.go:MCPServer`, `internal/cli/root.go`).
  The assessment and source references below preserve the original 05c78b2 comparison; implementation-specific claims there are historical, not current.
- **Value to mcpx.** + med: one view of background work.
- **Effort.** S — the daemon hands its store to the lazily built MCP server.
- **Risk.** med: a doc/code disagreement, and users look in the wrong place.
- **Detail.** None beyond the above.
- **Sources.** `internal/tasks/tasks.go:10-12` "Two stores would be two";
  `internal/mcpserver/tasks.go:35` "s.taskStore = tasks.New()"; `internal/daemon/ops.go:428-429`.
