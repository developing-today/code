# Agents in worlds: the MCP bridge and policy

An agent in a world is a participant that is not a person at a keyboard: a model-driven client that joins a world with a capability, reads the screen, posts chat and invokes programs, under a ceiling set by the world's owner. ID's mechanism for that is described in the design series #6300 (v1 spike) through #6303 (hosted services). This chapter describes what has shipped, and it starts with the boundary of what has shipped, stated plainly:

**There is no MCP server for ID.** The shipped tree contains no standalone MCP server binary, no stdio or HTTP MCP transport, and no JSON-RPC `tools/list` implementation. There is no mcpx or home wiring for ID either: `home/common/mcpx-settings.nix` contains no ID or world MCP server entry (its remote MCP entry is Atlas Search), and `home/` and `nixos/` contain no ID MCP entry. There is no gate-token environment variable for the ID MCP bridge; the bridge reads none of `ID_WORLD_CAPABILITY`, `ID_WORLD_ADMIN_TOKEN` or `ID_WEB_TOKEN`.

What exists is a library-level translator plus a host policy point, in `id-caps`, exercised by two integration tests in the facade package: `pkgs/id/tests/mcp_bridge_e2e.rs` and `pkgs/id/tests/mcp_spike_e2e.rs`. An agent reaches a world today by a host program embedding these two components, not by connecting an MCP client to an ID server.

## The translator: id_caps::mcp_bridge

`id_caps::mcp_bridge` (in `crates/id-caps/src/mcp_bridge.rs`) is authority-free. It translates one MCP tool call into one of three things: an F13.1 world-session client frame (JSON), a locally rendered answer computed from server frames the bridge has already observed, or a session close. It holds no credentials. The only credential value it touches is the agent's own join capability, which it passes through into the `join` frame and does not store as authority of its own.

Tool definitions are pinned at session start. `BridgeSession::start` and `verify_definitions` take the SHA-256 over the canonical JSON of the definition set; if the definition set changes mid-session, the session terminates permanently. That is an anti-poisoning rule: a tool whose meaning changes under an agent is not the tool the agent was granted.

The bridge consumes three kinds of server frame — `view`, `semantic_diff` and `screen_subscription` — and ignores all others by design. Its answers for read tools come only from what it has observed.

## The v1 tool list: 8 tools

`v1_tool_definitions()` pins 8 tools, in this order:

| # | Tool | Maps to | Ceiling key | Behaviour |
|---|---|---|---|---|
| 1 | `world.join` | F13.1 `session.presence` | `presence` | Sends `{type:"join", capability, after, agent_profile:true}`. `capability` is a required string; `after` is an optional `u64`. A second join returns `AlreadyJoined`. |
| 2 | `world.leave` | F13.1 `session.presence` | `presence` | No frame exists for leaving: returns `ToolAction::Close`. Presence is connection-scoped, so closing is leaving. |
| 3 | `screen.read` | `screen.read` | `screen.read` | A local answer: the latest full `view` text plus pending semantic-diff changes, which it drains. No frame is sent. |
| 4 | `screen.subscribe` | `screen.subscribe` | `screen.read` | Sends `{type:"screen_subscribe", …}`. Takes an optional `region` (`x`, `y`, `width`, `height`, each `u64`) and an optional `event_classes` list; at least one of the two is required. |
| 5 | `object.inspect` | `object.inspect` | `screen.read` | A local answer from observed `object_changed` diffs, keyed by the JSON string of the required `entity_ref` (any JSON value). If nothing was observed: `"object not in the observed screen state"`. |
| 6 | `program.invoke` | `input` | `program.invoke` | Sends exactly one `{type:"input", data_hex: hex(arguments), screen_revision?}` frame; no chaining. `program` and `object` are required non-empty strings, `arguments` is a required string, `screen_revision` is optional. The bridge validates the fields; the policy layer enforces the allowlist. |
| 7 | `chat.post` | `chat.post` | `chat.post` | Sends `{type:"chat", text, screen_revision?}`. `text` is a required string; `screen_revision` is optional. |
| 8 | `presence.set` | `presence.set` | `presence` | Listed but unimplemented in v1 — see below. |

Five names are explicitly forbidden (`FORBIDDEN_TOOL_NAMES`): `capability.mint`, `credential.read`, `audit.mutate`, `world.export`, `host.exec`. Any name that is neither a v1 tool nor handled returns `BridgeError::UnknownTool`. Every tool except `world.join` requires a joined session and returns `NotJoined` otherwise.

`presence.set` is the honesty case in the shipped surface, recorded as survey disagreement WEB-13. It is one of the 8 pinned definitions, so a client discovering the surface is offered it — and `tool_action` refuses it unconditionally with `BridgeError::NoFrameMapping`, because presence is the join itself and no presence frame exists. A v1 client must treat `presence.set` as a tool that can never succeed.

The world session the bridge targets carries contract revision 11 for the agent profile and semantic diffs (#6300 section 4.2): the subscribable event classes are `chat_posted`, `input_applied`, `object_changed`, `system_event` and `program_changed`, with `region_changed` and `notification` as non-subscribable change kinds. Revision 12 adds negotiated binary MessagePack screen patches, only on byte-capable transports.

## The policy point: PolicySession

`id_caps::mcp_policy` (in `crates/id-caps/src/mcp_policy.rs`) is the host-side policy point. `PolicySession` wraps a `BridgeSession` and runs three steps for every action, in this order:

1. Capability-ceiling check.
2. Quota charge, taken before execution.
3. Exactly one audit event per action — hash-chained and signed with the journal key — refusals included.

The grant the session runs under is `RoleGrant`, set by the owner at grant time and never widened by the session: `agent_key` (a 32-byte array), `agent_label`, `owner`, `world`, `capability_ref` (the delegation or grant record ID stamped on every audit event), `ceiling`, `quota`, and `rate_limits`.

The ceiling is `CapabilityCeiling`: four sets — `tools`, `worlds`, `object_classes`, `program_allowlist` — each a `BTreeSet<String>`. Anything unnamed is outside the ceiling. The per-tool ceiling keys in the table above are how a tool call is matched against the `tools` set.

Quota has three axes: `frames`, `tool_calls`, `program_invocations`. The charge per admitted action is fixed: 1 tool call always; plus 1 frame for `world.join`, `screen.subscribe`, `chat.post` and `program.invoke`; plus 1 program invocation for `program.invoke`. When quota is exhausted, the agent is parked, with a first-class world event: `QUOTA_PARK_EVENT = "agent parked, quota exhausted"`, a `ParkEvent` of kind `agent_parked` and reason `quota_exhausted`. Two further refusal rules apply: the stale-decision rule — a `screen_revision` argument must name the latest observed revision — and per-tool sliding-window rate limits, each a `RateLimit{tool, max_calls, window_secs}`.

Audit outcomes are the `AuditOutcome` variants: `Executed`, `RefusedCeiling`, `RefusedQuota{axis}`, `RefusedParked`, `RefusedStaleInput{token, current}`, `RefusedRateLimit{tool, max_calls, window_secs}`, plus refusals carrying the bridge error. The agent is the audit subject, never its editor: the log and the spend are exposed to it read-only. One action is exempt from refusal: `world.leave` is never refused by ceiling or quota and is charged nothing — but it is audited. An agent can always leave, and the leaving is always on the record. The audit chain those events join is the subject of [Events and the signed audit chain](events.md).

## What is design, not shipped

The agents-in-worlds programme beyond the spike — citizens (#6301), shared worlds (#6302), hosted services (#6303) and the v4 listing and registry records (#10662) — is design. The survey maps 6 records to this theme. None of them changes the statement at the top of this chapter: as shipped, the MCP bridge is a translator and a policy point in a library crate, with pinned tools, a pinned refusal, and no server, no transport and no mcpx wiring. The design chapter agents in worlds and the MCP bridge carries the four generations of that programme.
