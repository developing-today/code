# The bridge: fleet messaging

The bridge is the fleet's message bus. The bridge service itself lives at
https://bridge.security.cab (a separate worker, `pkgs/message-bridge`); the
`fleet bridge` domain is its mailbox client. The model: every fleet agent
has a *mailbox* on the bridge named after it, and registers with optional
*groups* (for example `muse`, `codex`, `code-lanes`). Central sends a
message or a command to one agent, a group, or all; the agent reads its
mailbox, does the work, and posts its answer back, which marks the command
handled.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-bridge/src/lib.rs

One documentation fact first: there is **one usage text for the whole
domain** and no per-verb help in source — every per-verb `--help` prints the
same shared usage block, despite the domain help's "(options: fleet bridge
&lt;command&gt; --help)" status line. This chapter is the per-verb reference the
help does not provide.

## The two verb families

The verbs split by which token they use and which routes they call.

**Central verbs** (the owner token — full control):

| Verb | Route | Behaviour |
|---|---|---|
| `send --to <targets> [--command] [--sender NAME] <text \| ->` | `POST /api/<owner-token>/fleet/send` | Targets: `agent:<name>` (or a bare name), `group:<name>`, `all`; repeat `--to` or comma-separate. `--command` marks the item a command (default: a plain message). `-` reads the text from stdin. |
| `agents` | `GET /api/<owner-token>/fleet/agents` | Registered agents, their groups, and unread counts. |
| `log [--agent A] [--direction inbound\|outbound] [--kind message\|command] [--since-id N] [--before-id N] [--limit N] [--order asc\|desc]` | `GET /api/<owner-token>/fleet/messages?…` | The central message log, filtered. |

**Agent verbs** (the fleet token — mailbox access only; it cannot send
commands):

| Verb | Route | Behaviour |
|---|---|---|
| `hello [--agent A] [--groups g1,g2]` | `POST /fleet/<fleet-token>/hello` with `{agent, groups}` | Register or refresh this agent and its groups. This is the registry write: the bridge's agent registry is exactly the set of agents that have said hello (name, groups, last-seen, unread counts). |
| `inbox [--agent A] [--groups g1,g2] [--commands \| --messages] [--all] [--since-id N] [--limit N] [--take] [--wait SECS] [--interval SECS]` | `GET /fleet/<fleet-token>/inbox?…` | Unread items by default; `--all` includes read ones; `--take` marks what it returns as read; `--wait` polls (default interval 15 s; wait capped at 24 h in source) until something arrives or the seconds pass. |
| `post [--agent A] [--reply-to ID] [--to ADDR] [--instruction \| --kind K] <text \| ->` | `POST /fleet/<fleet-token>/send` | A message to central by default. With `--reply-to` it answers that row (and marks it read). `--to` sends to any address instead: `drew`, `claude`, `chatgpt`, `connectors`, `agent:<name>` (or a bare name), `group:<name>`, `agents`, `all`; repeat or comma-separate. `--instruction` (or `--kind instruction`) makes it an instruction, which requires `--to`. |
| `ack [--agent A] (--up-to ID \| ID...)` | `POST /fleet/<fleet-token>/ack` | Mark messages read: either a list of ids, or everything up to an id. |
| `health` | `GET /healthz` | The bridge's health probe. No token. |

Defaults and config: the agent name defaults to `$FLEET_AGENT`, then
`[bridge] agent`; groups to `$FLEET_AGENT_GROUPS`, then `[bridge] groups`;
the bridge URL to `$FLEET_BRIDGE_URL`, then `[bridge] url`, then
`https://bridge.security.cab` — environment beats file throughout. Output is
the bridge's JSON, passed through.

Exit codes: 0 ok, 2 usage, 3 transport or credentials, **4 the bridge
refused the request** (its JSON error goes to stderr).

## Mailbox and ack semantics

A mailbox item is a *message* or a *command* (the kind is set by the
sender: `send --command`, and the intake ladder's push rung reads commands
only). Items are read or unread. Reading is non-destructive by default:
`inbox` without `--take` shows unread items and leaves them unread. Three
operations change read state:

- `inbox --take` marks exactly what it returns as read — the "I have this
  now" claim. `fleet agent next`'s push rung uses `--commands --take`, so a
  pushed command is consumed by exactly the hand-out that returns it.
- `post --reply-to ID` answers a row and marks that row read.
- `ack` marks explicit ids, or everything up to an id with `--up-to`.

The registry side is equally simple: there is no separate enrolment store
on the bridge. Saying `hello` *is* registering — name, groups, and last-seen
are refreshed on every hello — and `agents` reports the registry as the set
of agents that have said hello, with their unread counts. `fleet agent
enroll` builds on this: its bridge-leg probe is a `hello` in register mode
(see [Agents: enrolment, doctor and the intake ladder](agents.md)).

## Tokens and the HMAC derivation

Auth is by secret name through `fleet-secrets`; values are never printed,
logged, or put in argv. Central verbs use the **owner token**, secret name
`message-bridge.access-token` (`[bridge] owner_token_name` can rename it).
Agent verbs use the **fleet token**, secret name
`message-bridge.fleet-token`, which opens only the agent routes.

Tokens travel **in the URL path** (see the routes above), so every error
string is scrubbed of token values (replaced with `<token>`) before it is
printed, and a 404 is reported as "a wrong token, an old bridge without
fleet mailboxes, or a wrong URL". Plain `http` URLs are accepted only to
this machine; anything else must be `https`, because tokens ride the path.

**Derivation fallback.** When the fleet token's secret is missing but the
owner token resolves (the laptop seat's shape), the fleet token is
**derived from the owner token exactly as the bridge derives it**:
`base64url(HMAC-SHA256(key = owner token bytes, message =
"message-bridge/fleet-token/v1"))` — the fixed label constant is the string
`message-bridge/fleet-token/v1`. The derived value is used in-process only;
it is never written anywhere. (Construction and label named, no values —
that is the whole public shape of the derivation.)
