# Sampling

```
created:      2026-09-30T08:00:00-05:00
last-updated: 2026-11-01T16:20:00-06:00
increment:    1
status:       standard
tags:         area:compare
description:  sampling/createMessage fields and capabilities per revision, tool use in sampling, its 2026 deprecation, and how mcpx relays it
```

Sampling lets a server ask the client's model for a completion: nearly unchanged from 2024-11-05 to 2025-06-18, given tool
use and sub-capabilities in 2025-11-25, and deprecated in 2026-07-28, where it survives only as an embedded input request
([mrtr.md](mrtr.md)); sampling as a client-hosted task (2025-11-25 only) is in [tasks.md](tasks.md). mcpx has no model of
its own, so sampling is a relay: upstream requests become broker rows for an agent to answer. The client declares
sampling only when a request handler exists (SMP-09); host-bound relays check `sampling.tools` and
`sampling.context` (SMP-11).

Status column: SMP-03/09/11 are current (audited at `2a93f3f64`); other rows retain the **mcpx @ 05c78b2** snapshot. Legend and citation conventions: [../README.md](../README.md).

| ID     | Difference                                                              | Labels                                                            | Where it exists                                                                   | mcpx @ 05c78b2                                                                                                                            | Value        | Effort | Risk |
| ------ | ----------------------------------------------------------------------- | ----------------------------------------------------------------- | --------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- | ------------ | ------ | ---- |
| SMP-01 | `sampling/createMessage` request, result and baseline fields            | `2024-11-05 has` `2026-07-28 deprecates` `mcpx has, others don't` | `24-11 ✓ · 25-03 ✓ · 25-06 ✓ · 25-11 ✓ · 26-07 dep.`; opencode v1/v2 ✗, lootbox ✗ | ✓ — relayed both directions through the broker                                                                                            | + low        | S      | low  |
| SMP-02 | `includeContext` other than `none` soft-deprecated, then deprecated     | `2025-11-25 deprecates` `2026-07-28 deprecates`                   | values in all five; gated by `sampling.context` from 25-11                        | ✓ — never declares `sampling.context` upstream                                                                                            | − deprecated | S      | low  |
| SMP-03 | `tools` and `toolChoice` in sampling requests                           | `2025-11-25 has` `2026-07-28 deprecates`                          | `25-11 ✓ · 26-07 dep.`                                                            | ✓ — not declared upstream; undeclared requests are refused, and host relay requires `sampling.tools` (modern.go:142-179; conn.go:572-581) | + low        | M      | low  |
| SMP-04 | `stopReason: "toolUse"`                                                 | `2025-11-25 has`                                                  | `25-11 ✓ · 26-07 ✓`; open string in all                                           | n/a — never inspected                                                                                                                     | − moot       | S      | low  |
| SMP-05 | `SamplingMessage.content` may be an array                               | `2025-11-25 has`                                                  | `24-11..25-06 single · 25-11 single or array · 26-07 same`                        | n/a — relayed raw; broker decoding not examined                                                                                           | + low        | S      | low  |
| SMP-06 | Tool-result messages hold only `tool_result` blocks, paired by id       | `2025-11-25 has`                                                  | `25-11 ✓ · 26-07 ✓ (dep.)`                                                        | n/a — mcpx runs no tool loop                                                                                                              | − moot       | S      | low  |
| SMP-07 | Sampling feature deprecated (SEP-2577)                                  | `2026-07-28 deprecates`                                           | `26-07 dep.; removable from 2027-07-28`                                           | ✓ — relay exists; nothing new to build                                                                                                    | − deprecated | S      | low  |
| SMP-08 | 2026 `CreateMessageRequest` embedded; no `_meta`; result not a `Result` | `2026-07-28 has`                                                  | `26-07 only`                                                                      | ✓ — decoded per `inputRequests` key                                                                                                       | + low        | S      | low  |
| SMP-09 | Client declares `sampling` only when something can answer               | `2026-07-28 deprecates`                                           | spec: declare what you support; mcpx: handler-gated                               | ✓ — declared iff a request handler is configured (modern.go:120-123; daemon/hooks.go:82)                                                  | + med        | S      | med  |
| SMP-10 | Upstream sampling answered by an agent through the broker               | `mcpx has, others don't` `2026-07-28 deprecates`                  | mcpx only                                                                         | partial — decline becomes an error; `role`/`model` filled in                                                                              | + low        | S      | med  |
| SMP-11 | Relaying sampling to a host checks declared sub-capabilities            | `2025-11-25 has`                                                  | `25-11 ✓ · 26-07 ✓`                                                               | ✓ — tools require `sampling.tools`; context is dropped without `sampling.context` (conn.go:561-581,696-703)                               | + low        | S      | low  |

## SMP-01 `sampling/createMessage` request, result and baseline fields

- **What.** A server asks the client's model for a completion: `messages`, `modelPreferences` (`hints[]`,
  `costPriority`, `speedPriority`, `intelligencePriority`), `systemPrompt`, `includeContext`, `temperature`, `maxTokens`
  (required), `stopSequences`, `metadata`. The result is a `SamplingMessage` plus `model` and `stopReason`.
- **Where.** All five revisions; deprecated in 2026-07-28. opencode v1 comments `sampling` out and v2 does not declare
  it; neither registers a handler. lootbox declares no capabilities.
- **mcpx @ 05c78b2.** Relayed both ways: an upstream's request goes to a downstream handler, which in the daemon is the
  broker (`internal/mcpclient/modern.go:121-128`); host-bound requests are gated by `CanSample`
  (`internal/mcpserver/conn.go:211`).
- **Value to mcpx.** + low: the relay exists; the feature is deprecated, so it should not grow.
- **Effort.** S — done.
- **Risk.** low.
- **Detail.** `metadata` is retyped from `object` to `JSONObject` in 2026. `ModelHint.name` is a substring hint, not an
  exact model id. opencode v2 plugins could answer a forwarded sampling request with `ctx.generate.text`, but only as a
  flattened prompt string with no messages, system prompt or tools.
- **Sources.** `schema/2024-11-05/schema.ts:762` "sampling/createMessage"; `schema/2024-11-05/schema.ts:784`
  "maxTokens: number;"; `schema/2026-07-28/schema.ts:2140` "metadata?: JSONObject;";
  `internal/mcpclient/modern.go:121-128`; `internal/mcpserver/conn.go:211`;
  `v1:packages/opencode/src/mcp/index.ts:42` "// sampling: {},"; `v2:packages/core/src/mcp/client.ts:142`;
  `.lootbox/src/lib/external-mcps/mcp_client_manager.ts:150-153`.

## SMP-02 `includeContext` other than `none` soft-deprecated, then deprecated

- **What.** `includeContext: "none" | "thisServer" | "allServers"` exists in every revision. 2025-11-25 soft-deprecates the
  two non-`none` values and gates them on `sampling.context` (servers SHOULD send only `none` or omit it otherwise). 2026
  reclassifies them as Deprecated under SEP-2596, removable no later than Sampling itself.
- **Where.** Values in all five; soft-deprecated in 2025-11-25; Deprecated in 2026-07-28.
- **mcpx @ 05c78b2.** Never declares `sampling.context` upstream (`internal/mcpclient/modern.go:55`), so conforming
  upstreams send only `none`. That is right for a proxy, which has no "context from other servers" to inject. The host
  direction is SMP-11.
- **Value to mcpx.** − deprecated: nothing to build.
- **Effort.** S.
- **Risk.** low: SHOULD-level only, so a server may still send the other values and the client MAY ignore them.
- **Detail.** Changelog-silent: the 2025-11-25 changelog mentions neither the soft-deprecation nor `sampling.context`; the
  2026 deprecated registry dates it to 2025-11-25. The 2024-11-05 and 2025-06-18 sampling prose never mention
  `includeContext` at all; it is schema-only there.
- **Sources.** `schema/2024-11-05/schema.ts:776` "includeContext?:";
  `schema/2025-11-25/schema.ts:1592` "Values \"thisServer\" and \"allServers\" are soft-deprecated.";
  `schema/2025-11-25/schema.ts:332` "context?: object;";
  `2025-11-25/client/sampling.mdx:69` "With context inclusion support (soft-deprecated):";
  `2026-07-28/deprecated.mdx:30`; `internal/mcpclient/modern.go:120-123`.

## SMP-03 `tools` and `toolChoice` in sampling requests

- **What.** A server may pass `tools: Tool[]` (scoped to that request; they need not be registered tools) and
  `toolChoice{mode: "auto" | "required" | "none"}`. Clients MUST declare `sampling.tools` to receive them, and MUST
  return an error if they arrive anyway; servers MUST NOT send them to clients lacking it.
- **Where.** 2025-11-25; deprecated with Sampling in 2026-07-28.
- **mcpx @ 2a93f3f64.** Not declared upstream (`internal/mcpclient/modern.go:120-123`), so conforming upstreams do not
  send tools to mcpx. If a non-conforming upstream does, mcpx refuses tools/toolChoice with `-32602`
  (`internal/mcpclient/modern.go:142-179`). Toward hosts, a tool-enabled request is offered only to a host that declared
  `sampling.tools` (`internal/mcpserver/conn.go:572-581`).
- **Value to mcpx.** + low: only matters for an agentic-sampling relay, of a deprecated feature.
- **Effort.** M — tool-loop semantics (SMP-05, SMP-06).
- **Risk.** low while undeclared.
- **Detail.** `toolChoice` defaults to `{mode: "auto"}`. The prose gate (server MUST NOT send) and the schema gate (client
  MUST error) apply to opposite ends; a proxy sits at both.
- **Sources.** `schema/2025-11-25/schema.ts:1615` "tools?: Tool[];";
  `schema/2025-11-25/schema.ts:1636` "mode?: \"auto\" | \"required\" | \"none\";";
  `schema/2025-11-25/schema.ts:336` "tools?: object;";
  `2025-11-25/client/sampling.mdx:40` "Clients **MUST** declare support for tool use via the";
  `2025-11-25/changelog.mdx:18` "Add tool calling support to sampling via";
  `internal/mcpclient/modern.go:142-179`; `internal/mcpserver/conn.go:572-581`.

## SMP-04 `stopReason: "toolUse"`

- **What.** A new standard stop reason for "the model wants to call tools". Earlier standard values are `endTurn`,
  `stopSequence`, `maxTokens`.
- **Where.** 2025-11-25 and 2026-07-28. The field is an open string in every revision.
- **mcpx @ 05c78b2.** Not inspected; the broker answer path does not fill `stopReason` (it is optional; see SMP-10).
- **Value to mcpx.** − moot.
- **Effort.** S.
- **Risk.** None: open string.
- **Detail.** Only meaningful together with SMP-03.
- **Sources.** `schema/2024-11-05/schema.ts:804` "stopReason?:";
  `schema/2025-11-25/schema.ts:1673` "\"toolUse\" | string;".

## SMP-05 `SamplingMessage.content` may be an array

- **What.** A single content block through 2025-06-18; `SamplingMessageContentBlock | SamplingMessageContentBlock[]` from
  2025-11-25. Because `CreateMessageResult extends SamplingMessage`, results may be arrays too.
- **Where.** 2025-11-25 and 2026-07-28.
- **mcpx @ 05c78b2.** Upstream requests are relayed as raw params on the 2026 path (`internal/mcpserver/ask.go:256-266`);
  how the broker and the answering agent handle array content was not examined.
- **Value to mcpx.** + low: a relay must not assume one block.
- **Effort.** S.
- **Risk.** low: decoding an array as an object fails.
- **Detail.** Not in the 2025-11-25 changelog. Allowed block types grow from text and image (2024-11-05) to text, image,
  audio, `tool_use` and `tool_result` (2025-11-25); the content-types register owns the blocks themselves.
- **Sources.** `schema/2025-06-18/schema.ts:1090` "content: TextContent | ImageContent | AudioContent;";
  `schema/2025-11-25/schema.ts:1683` "content: SamplingMessageContentBlock | SamplingMessageContentBlock[];".

## SMP-06 Tool-result messages hold only `tool_result` blocks, paired by id

- **What.** A user message containing `tool_result` blocks MUST contain only tool results, and every assistant message with
  `tool_use` blocks MUST be followed by a user message of only matching `tool_result`s.
- **Where.** 2025-11-25 and 2026-07-28 (deprecated).
- **mcpx @ 05c78b2.** mcpx runs no sampling tool loop, so nothing to check.
- **Value to mcpx.** − moot: needed only if mcpx relayed tool loops.
- **Effort.** S.
- **Risk.** low: `-32602` from strict clients if violated.
- **Detail.** The 2026 schema still lists "Missing tool result or tool results mixed with other content" as an
  `InvalidParamsError` case for sampling, but under MRTR a client has no way to return an error for one input request
  ([mrtr.md](mrtr.md), MRTR-17).
- **Sources.** `2025-11-25/client/sampling.mdx:372` "it **MUST** contain ONLY tool results.";
  `2025-11-25/client/sampling.mdx:430` "**MUST** be followed by a user message";
  `schema/2026-07-28/schema.ts:371` "Missing tool result or tool results mixed with other content".

## SMP-07 Sampling feature deprecated (SEP-2577)

- **What.** The capability, request, result and all sampling types are `@deprecated` in 2026-07-28; the migration is to
  call LLM provider APIs directly.
- **Where.** 2026-07-28; functional through at least 2027-07-28.
- **mcpx @ 05c78b2.** The relay exists and is declared whenever a handler is installed
  (`internal/mcpclient/modern.go:54-56`).
- **Value to mcpx.** − deprecated: keep what exists, do not grow it.
- **Effort.** S.
- **Risk.** None.
- **Detail.** Roots and Logging are deprecated by the same SEP; Elicitation is not.
- **Sources.** `2026-07-28/client/sampling.mdx:8` "The Sampling feature is deprecated as of protocol version";
  `2026-07-28/deprecated.mdx:27` "[Sampling](https://modelcontextprotocol.io/specification/2026-07-28/client/sampling)".

## SMP-08 2026 `CreateMessageRequest` embedded; no `_meta`; result not a `Result`

- **What.** In 2026 `CreateMessageRequest` no longer extends `JSONRPCRequest`; its params extend nothing (no `_meta`, so no
  `progressToken`); `CreateMessageResult extends SamplingMessage` only (no `resultType`, no `_meta`).
- **Where.** 2026-07-28.
- **mcpx @ 05c78b2.** The client decodes `{method, params}` per `inputRequests` key (`internal/mcpclient/modern.go:140-143`).
- **Value to mcpx.** + low: matches.
- **Effort.** S.
- **Risk.** None.
- **Detail.** None of this is in the changelog; it follows from MRTR. A long sampling request therefore has no progress
  channel in 2026.
- **Sources.** `schema/2026-07-28/schema.ts:2185` "export interface CreateMessageRequest {";
  `schema/2026-07-28/schema.ts:2210` "export interface CreateMessageResult extends SamplingMessage {";
  `schema/2025-11-25/schema.ts:1656` "export interface CreateMessageResult extends Result, SamplingMessage {";
  `internal/mcpclient/modern.go:140-143`.

## SMP-09 Client declares `sampling` only when something can answer

- **What.** A client should declare only what it supports. mcpx declares `sampling` only when its server-request handler
  exists.
- **Where.** Spec rule in every revision; mcpx's `capabilities()` gates the field on a configured handler.
- **mcpx @ 2a93f3f64.** `capabilities()` adds `sampling` iff `OnServerRequest` is non-nil
  (`internal/mcpclient/modern.go:94-123`); the daemon installs its broker-backed handler
  (`internal/daemon/hooks.go:82,155-174`). The handler turns requests into broker work; an unanswered request may still
  expire. Thus declaration reflects configured support, not whether an agent is connected at that instant.
- **Value to mcpx.** − done: the declaration follows handler availability.
- **Effort.** S — done.
- **Risk.** med: a sampling server waits `elicit.ttl` (120 s) per call and gets an error when nobody answers.
- **Detail.** The capability check reports configured support. A broker request can remain open until a handler answers
  or the request expires; lack of an immediately connected agent does not remove the configured handler.
- **Sources.** `internal/mcpclient/modern.go:94-123`; `internal/daemon/hooks.go:82,155-174,280-328`.

## SMP-10 Upstream sampling answered by an agent through the broker

- **What.** An upstream's `sampling/createMessage` becomes a broker row for the agent audience (the opencode plugin can
  answer it with the session's model). An accept is returned as a `CreateMessageResult` with `role: "assistant"` and
  `model: "unknown"` filled in if missing; any other answer becomes a JSON-RPC error.
- **Where.** mcpx only; neither opencode version nor lootbox handles sampling at all.
- **mcpx @ 05c78b2.** `internal/daemon/hooks.go:180-232`.
- **Value to mcpx.** + low: lets a deprecated feature work for servers that still depend on it.
- **Effort.** S — done.
- **Risk.** med: without an answerer, 120 s of waiting per call (SMP-09).
- **Detail.** Sampling has no `decline`/`cancel` result, so mapping a declined broker row to an error is the only option
  in legacy; in 2026 it is also the only option, since MRTR has no per-input error (MRTR-17). `stopReason` is not filled;
  the schema makes it optional.
- **Sources.** `internal/daemon/hooks.go:215-217` "the sampling request was"; `internal/daemon/hooks.go:180-232`.

## SMP-11 Relaying sampling to a host checks declared sub-capabilities

- **What.** A `tools`/`toolChoice` request must not reach a host that did not declare `sampling.tools`; non-`none`
  `includeContext` must not be relayed unless the host declared `sampling.context`. The 2025-06-18 schema has no
  `sampling.tools` sub-capability; a conforming client from that revision cannot declare it.
- **Where.** The sub-capabilities are 2025-11-25 and later.
- **mcpx @ 2a93f3f64.** A tool-bearing question is sendable only when the host declared `sampling.tools`
  (`internal/mcpserver/conn.go:561-581`); `paramsFor` drops `includeContext` unless the host declared
  `sampling.context` (`internal/mcpserver/conn.go:652-704`). The host-facing gate is declaration-based, not a second
  explicit version predicate; a conforming pre-2025-11-25 peer cannot declare these sub-keys. In the reverse direction,
  sampling payload/result validation uses the negotiated revision (`internal/daemon/hooks.go:295-303,362-368`).
- **Value to mcpx.** − done: no sampling fields beyond the host's declared sub-capabilities are relayed.
- **Effort.** S.
- **Risk.** low: tools questions that are not sendable remain on the broker path; unsupported context is omitted.
- **Detail.** It needs a non-conforming upstream to send tools to mcpx because its upstream declaration is bare
  `sampling: {}`; mcpx refuses those requests. The host-side sub-capability checks protect the other end of the relay.
- **Sources.** `internal/mcpserver/conn.go:313-319,561-581,652-704`;
  `internal/mcpclient/modern.go:142-179`; `internal/daemon/hooks.go:295-303,362-368`;
  `schema/2025-11-25/schema.ts:327-336`.
