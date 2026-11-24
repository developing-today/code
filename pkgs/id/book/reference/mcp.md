# The MCP bridge

Source: https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-caps/src/mcp_bridge.rs, https://github.com/developing-today/code/blob/main/pkgs/id/crates/id-caps/src/mcp_policy.rs

This chapter documents the MCP bridge **as it stands**. The short version: there is a translator and a policy layer, and there is no server.

## What does not exist

- There is **no MCP server** in `pkgs/id`: no standalone server binary.
- There is **no MCP transport**: no stdio transport and no HTTP transport.
- There is **no JSON-RPC `tools/list` implementation**.
- There is **no mcpx or home wiring** for ID. Searching `home/common/mcpx-settings.nix` finds no ID or world MCP server at all; its remote MCP entry is Atlas Search, with a gate token from `services.mcpx.environmentFile`. Neither `home/` nor `nixos/` contains an ID MCP entry.
- There is **no gate-token environment variable for the ID MCP bridge**. The bridge reads no environment variables. The nearest authentication environment names are the world and web ones in [Configuration and environment](configuration.md) — `ID_WORLD_CAPABILITY`, `ID_WORLD_ADMIN_TOKEN`, `ID_WEB_TOKEN` — and the bridge itself never reads any of them.

### What a reader would need for a live MCP client

A live MCP client needs a server process speaking MCP over a transport, advertising a tool list it can call. None of that exists for ID today. To use an agent with an ID world now, the agent must speak the world session protocol directly — the frames in [WebSocket protocols](websockets.md) — or be driven through the library translator below by a host that supplies its own transport. The design for a live bridge is carried in the design chapter [Design: agents in worlds and the MCP bridge](../design/agents-mcp.md); nothing in this chapter should be read as a connection recipe.

## What exists: the id-caps translator

`id_caps::mcp_bridge` is a library-level translator plus `id_caps::mcp_policy`, a host-side policy point. Together they are the "agent-in-worlds" spike surface. They are exercised by `pkgs/id/tests/mcp_bridge_e2e.rs` and `pkgs/id/tests/mcp_spike_e2e.rs`.

The bridge is **authority-free**:

- It translates one MCP tool call into one world-session client frame (JSON), into a locally-rendered answer built from observed server frames, or into a session close.
- It holds no credentials. The only value it touches that resembles a secret is the agent's own join capability, which it passes through into the `join` frame.
- It consumes three kinds of observed server frame — `view`, `semantic_diff` and `screen_subscription` — and ignores all others by design.

### Pinned tool definitions

Tool definitions are pinned at session start by SHA-256 over their canonical JSON (`BridgeSession::start` / `verify_definitions`). If the definition set changes, the session terminates permanently. This is an anti-poisoning rule: a tool list that changes mid-session is treated as hostile, not as an update.

## The 8 v1 tools

`v1_tool_definitions()` in `mcp_bridge.rs` defines 8 tools, in this pinned order.

| # | Tool | Maps to | Ceiling key | Parameters | Behaviour |
|---|---|---|---|---|---|
| 1 | `world.join` | `session.presence` | `presence` | `capability: string` (required), `after: u64` (optional) | Sends `{type:"join", capability, after, agent_profile:true}`. A second join returns `AlreadyJoined`. |
| 2 | `world.leave` | `session.presence` | `presence` | none | No frame exists for leaving: returns `ToolAction::Close`. Presence is connection-scoped, so leaving is closing. |
| 3 | `screen.read` | `screen.read` | `screen.read` | none | Local answer: the latest full `view` text plus pending semantic-diff changes. Reading drains the pending changes. Sends no frame. |
| 4 | `screen.subscribe` | `screen.subscribe` | `screen.read` | `region` (optional: `{x, y, width, height}`, all `u64`), `event_classes` (optional string array). At least one of the two is required. | Sends `{type:"screen_subscribe", …}`. |
| 5 | `object.inspect` | `object.inspect` | `screen.read` | `entity_ref` (required, any JSON value; keyed by its JSON string) | Local answer from observed `object_changed` diffs. When nothing is known: `"object not in the observed screen state"`. Sends no frame. |
| 6 | `program.invoke` | `input` | `program.invoke` | `program`: non-empty string, `object`: non-empty string, `arguments`: string, `screen_revision`: optional `u64` | Sends exactly one `{type:"input", data_hex: hex(arguments), screen_revision?}` frame. No chaining: one call, one frame. `program` and `object` are validated here and allowlist-enforced by the policy layer. |
| 7 | `chat.post` | `chat.post` | `chat.post` | `text: string` (required), `screen_revision`: optional `u64` | Sends `{type:"chat", text, screen_revision?}`. |
| 8 | `presence.set` | `presence.set` | `presence` | — | Listed but unimplemented in v1: always refused. See below. |

Session rule: every tool except `world.join` requires a joined session. Calling one before joining returns `NotJoined`.

### Forbidden tool names

Five names are explicitly **not** tools (`FORBIDDEN_TOOL_NAMES` in `mcp_bridge.rs`):

```text
capability.mint
credential.read
audit.mutate
world.export
host.exec
```

Any other unrecognised name returns `BridgeError::UnknownTool`. The forbidden list and the unknown-tool error are different answers: a forbidden name is a name the surface has considered and excluded; an unknown name was never part of the surface.

### `presence.set`: a pinned tool the bridge always refuses (WEB-13)

`presence.set` is one of the 8 pinned `v1_tool_definitions()`, so a client discovering the surface is offered it. Calling it always fails: `tool_action` refuses it unconditionally with `BridgeError::NoFrameMapping`. The reason is structural — presence *is* the join. There is no presence frame in the world session protocol to map the call onto. That is disagreement WEB-13: the v1 surface lists a tool that can never succeed in v1.

## The policy layer: PolicySession

`id_caps::mcp_policy` provides `PolicySession`, which wraps a `BridgeSession` on the host side. Every action passes three steps, in order:

1. **Capability-ceiling check.** The action must be inside the session's ceiling.
2. **Quota charge, before execution.** The charge is taken first; an action that cannot pay does not run.
3. **Signed audit.** Exactly one hash-chained, journal-key-signed audit event per action — **refusals included**. A refused action produces an audit event and no frame.

Two further rules are enforced in the same layer:

- **Stale-decision rule.** A `screen_revision` supplied with `chat.post` or `program.invoke` must name the latest observed revision. An older revision is refused.
- **Rate limits.** Per-tool sliding-window rate limits.

### The grant: RoleGrant

The ceiling and quota come from a `RoleGrant`, set by the owner at grant time and never widened by the session:

| Field | Meaning |
|---|---|
| `agent_key` | `[u8; 32]`, the agent's key. |
| `agent_label` | The agent's label. |
| `owner` | The granting owner. |
| `world` | The world the grant covers. |
| `capability_ref` | The delegation/grant record ID, stamped on every audit event. |
| `ceiling` | A `CapabilityCeiling`. |
| `quota` | A `Quota`. |
| `rate_limits` | A list of `RateLimit{tool, max_calls, window_secs}`. |

`CapabilityCeiling` has four sets — `tools`, `worlds`, `object_classes`, `program_allowlist` — all `BTreeSet<String>`. Anything unnamed in the relevant set is outside the ceiling.

### Quota axes and charges

`Quota` has three axes: `frames`, `tool_calls`, `program_invocations`.

| Action | tool_calls | frames | program_invocations |
|---|---|---|---|
| Any admitted action | 1 | — | — |
| `world.join` | 1 | 1 | — |
| `screen.subscribe` | 1 | 1 | — |
| `chat.post` | 1 | 1 | — |
| `program.invoke` | 1 | 1 | 1 |
| `screen.read`, `object.inspect` | 1 | 0 | 0 |
| `world.leave` | 0 | 0 | 0 |

`world.leave` is never refused by ceiling or quota and is charged nothing. It is still audited.

### Quota exhaustion parks the agent

When a quota axis is exhausted, the agent is *parked*: the policy layer emits the first-class world event

```text
QUOTA_PARK_EVENT = "agent parked, quota exhausted"
```

as a `ParkEvent{kind: "agent_parked", reason: "quota_exhausted", …}`. Parking is a world-visible state, not a silent refusal: other participants can observe that the agent stopped because its quota ran out.

### Audit outcomes and refusal shapes

`AuditOutcome` records what happened to each action:

| Outcome | Meaning |
|---|---|
| `Executed` | The action ran. |
| `RefusedCeiling` | Outside the capability ceiling. |
| `RefusedQuota{axis}` | A quota axis could not pay; `axis` names which one. |
| `RefusedParked` | The agent is already parked. |
| `RefusedStaleInput{token, current}` | The supplied `screen_revision` (`token`) is not the latest observed revision (`current`). |
| `RefusedRateLimit{tool, max_calls, window_secs}` | The tool's sliding-window limit was hit. |
| Bridge-error refusals | The underlying bridge returned an error, for example `NotJoined`, `AlreadyJoined`, `UnknownTool` or `NoFrameMapping`. |

The agent is the audit subject, never its editor: the audit log and the spend figures are exposed to the agent read-only.

## Worked example

Toy data: an agent labelled `toy-agent`, granted a ceiling naming the tools `world.join`, `world.leave`, `screen.read`, `chat.post`, the world `lobby`, and a quota of 10 tool calls.

```text
world.join      {capability: "CAP"}        → frame {"type":"join","capability":"CAP","agent_profile":true}
                                             audit: Executed (charge: 1 tool call, 1 frame)
screen.read     {}                         → local answer: latest view text; no frame sent
                                             audit: Executed (charge: 1 tool call)
chat.post       {text: "hello", screen_revision: 3}
                                           → frame {"type":"chat","text":"hello","screen_revision":3}
                                             audit: Executed (charge: 1 tool call, 1 frame)
chat.post       {text: "again", screen_revision: 3}     (revision 4 has since been observed)
                                           → no frame
                                             audit: RefusedStaleInput{token: 3, current: 4}
presence.set    {…}                        → no frame
                                             audit: bridge-error refusal, NoFrameMapping
capability.mint {…}                        → no frame; forbidden name, not a tool
world.leave     {}                         → ToolAction::Close; no charge
                                             audit: Executed
```

After the tenth charged tool call, the next chargeable action is refused with `RefusedQuota` and the agent is parked with the `agent_parked` / `quota_exhausted` world event.
