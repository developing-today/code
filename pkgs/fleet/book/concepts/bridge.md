# The bridge: fleet messaging

The bridge is the fleet's message bus. The bridge service itself lives at
https://bridge.security.cab (a separate worker, `pkgs/message-bridge`); the
`fleet bridge` domain is its mailbox client. The model: every fleet agent
has a _mailbox_ on the bridge named after it, and registers with optional
_groups_ (for example `muse`, `codex`, `code-lanes`). Central sends a
message or a command to one agent, a group, or all; the agent reads its
mailbox, does the work, and posts its answer back, which marks the command
handled.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-bridge/src/lib.rs

One documentation fact first: every verb has its own help —
`fleet bridge <command> --help` prints that verb's usage section plus the
shared defaults-and-exit-codes footer, and a bare `fleet bridge --help`
prints the whole-domain usage text. This chapter is the per-verb reference
in prose form.

## The two verb families

The verbs split by which token they use and which routes they call.

**Central verbs** (the owner token — full control):

| Verb                                                                                                                                       | Route                                     | Behaviour                                                                                                                                                                                         |
| ------------------------------------------------------------------------------------------------------------------------------------------ | ----------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `send --to <targets> [--command] [--sender NAME] <text \| ->`                                                                              | `POST /api/<owner-token>/fleet/send`      | Targets: `agent:<name>` (or a bare name), `group:<name>`, `all`; repeat `--to` or comma-separate. `--command` marks the item a command (default: a plain message). `-` reads the text from stdin. |
| `agents`                                                                                                                                   | `GET /api/<owner-token>/fleet/agents`     | Registered agents, their groups, and unread counts.                                                                                                                                               |
| `log [--agent A] [--direction inbound\|outbound] [--kind message\|command] [--since-id N] [--before-id N] [--limit N] [--order asc\|desc]` | `GET /api/<owner-token>/fleet/messages?…` | The central message log, filtered.                                                                                                                                                                |

**Agent verbs** (the fleet token — mailbox access only; it cannot send
commands):

| Verb                                                                                                                                        | Route                                                    | Behaviour                                                                                                                                                                                                                                                                                                                                                         |
| ------------------------------------------------------------------------------------------------------------------------------------------- | -------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `hello [--agent A] [--groups g1,g2]`                                                                                                        | `POST /fleet/<fleet-token>/hello` with `{agent, groups}` | Register or refresh this agent and its groups. This is the registry write: the bridge's agent registry is exactly the set of agents that have said hello (name, groups, last-seen, unread counts).                                                                                                                                                                |
| `inbox [--agent A] [--groups g1,g2] [--commands \| --messages] [--all] [--since-id N] [--limit N] [--take] [--wait SECS] [--interval SECS]` | `GET /fleet/<fleet-token>/inbox?…`                       | Unread items by default; `--all` includes read ones; `--take` marks what it returns as read; `--wait` polls (default interval 15 s; wait capped at 24 h in source) until something arrives or the seconds pass.                                                                                                                                                   |
| `post [--agent A] [--reply-to ID] [--to ADDR] [--instruction \| --kind K] <text \| ->`                                                      | `POST /fleet/<fleet-token>/send`                         | A message to central by default. With `--reply-to` it answers that row (and marks it read). `--to` sends to any address instead: `drew`, `claude`, `chatgpt`, `connectors`, `agent:<name>` (or a bare name), `group:<name>`, `agents`, `all`; repeat or comma-separate. `--instruction` (or `--kind instruction`) makes it an instruction, which requires `--to`. |
| `ack [--agent A] (--up-to ID \| ID...)`                                                                                                     | `POST /fleet/<fleet-token>/ack`                          | Mark messages read: either a list of ids, or everything up to an id.                                                                                                                                                                                                                                                                                              |
| `work [--agent A] --state acted\|queued\|done [--task ID] ID...`                                                                           | `POST /fleet/<fleet-token>/work`                         | Record the work-item lifecycle of your own instructions: `acted` (handled in-session), `queued` (became queue task `--task`), `done`. Forward only, from `received`; only the receiving agent can mark its own copy.                                                                                                                                               |
| `health`                                                                                                                                    | `GET /healthz`                                           | The bridge's health probe. No token.                                                                                                                                                                                                                                                                                                                              |

Defaults and config: the agent name defaults to `$FLEET_AGENT`, then
`[bridge] agent`; groups to `$FLEET_AGENT_GROUPS`, then `[bridge] groups`;
the bridge URL to `$FLEET_BRIDGE_URL`, then `[bridge] url`, then
`https://bridge.security.cab` — environment beats file throughout. Output is
the bridge's JSON, passed through.

Exit codes: 0 ok, 2 usage, 3 transport or credentials, **4 the bridge
refused the request** (its JSON error goes to stderr).

## Mailbox and ack semantics

A mailbox item is a _message_ or a _command_ (the kind is set by the
sender: `send --command`, and the intake ladder's push rung reads commands
only). Items are read or unread. Reading is non-destructive by default:
`inbox` without `--take` shows unread items and leaves them unread. Three
operations change read state:

- `inbox --take` marks exactly what it returns as read — the "I have this
  now" claim. `fleet agent next`'s push rung uses `--commands --take`, so a
  pushed command is consumed by exactly the hand-out that returns it.
- `post --reply-to ID` answers a row and marks that row read.
- `ack` marks explicit ids, or everything up to an id with `--up-to`.

Read state is not the whole story for instructions. An instruction is a
work item with a lifecycle the receiving agent records with `work`:
`received` → `acted` (handled in-session) or `queued` (it became a named
fleet queue task) → `done`, forward only, each recipient's copy
independent on a group send. The owner's pipeline shows the state and
the queue task id, so an instruction that is neither acted on nor
queued stays visible at `received` — the defect the task-it-or-queue-it
rule (in the reference chapter, and in the text the intake ladder hands
a woken agent) exists to prevent. Notes instructions carry the same
fields on the notes surface; connector-channel instructions store none —
their read state stands in for `acted`.

The registry side is equally simple: there is no separate enrolment store
on the bridge. Saying `hello` _is_ registering — name, groups, and last-seen
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

## Notes and the public wake signal — the next bridge surface (in flight)

Everything above this section is the bridge as built and shipped: the
mailbox system the `fleet bridge` verbs drive today. A second, smaller
surface is being built on the same service — recorded as the design of
record in developing-today/code#13895 (filed 2026-11-23) and **in
flight, not landed**, as of 2026-11-23: no part of it is in
`pkgs/message-bridge` on `main` yet. This section documents the design
as recorded, so the book is ready for it; it will be revised against
the code when the build lands.

**Addressing: `class/name` identities.** A note's recipient is an
identity written `class/name` (for example `muse/archer`). An
authenticated sender can address one identity, or use a wildcard in
either segment — `muse/*` (every identity in a class), `*/archer`
(that name in every class), or `*/*` (every identity). Notes are
stored server-side in the bridge's Durable Object and delivered to
every registered identity matching the address. This is a different
grammar from the mailbox system's `agent:` / `group:` / `all` targets
above: classes are part of the identity itself, not a separate group
registry an agent joins by saying hello.

**The public signal.** `GET /<class>/<name>` is a public,
content-free wake signal. It returns the same bare UUID shape for
registered and unregistered paths. A registered path's signal changes
when a note is delivered to it — and every path, registered or not,
also rotates on a deterministic, per-path jittered schedule. The
privacy properties are the point of the design:

- the signal carries no note content;
- its shape is identical for registered and unregistered paths, so it
  is not an identity-existence proof;
- because every path rotates on its own jittered schedule, a change
  does not prove a delivery happened — decoy rotation is built in, and
  an unregistered path still rotates its synthetic UUID across
  windows.

**Inbox and acknowledgement.** The signal only says "look"; the notes
themselves sit behind the identity's own token. Authenticated inbox
and acknowledgement endpoints belong to that identity token: token
values are kept in the vault by name and are represented server-side
only by hashes — never committed, printed or logged. The recorded
flow is: poll the public signal, and when it changes, fetch the
inbox with the identity token, act on the notes, then acknowledge
them (the acceptance proof ends with the inbox empty after
acknowledgement).

**The client side, as designed.** A small standalone `bridge` helper
provides `send`, `inbox` and `ack` against these endpoints without
hand-written HTTP calls, and the first consumer is a Muse hook for
Archer: it polls only the public `muse/archer` signal, compares one
UUID to its last-seen value, and wakes only when it changes; on
waking, Archer fetches and acknowledges the inbox and surfaces or
acts on the notes in the main chat. The recorded end state for the
client surface is a `fleet` subcommand family — the follow-up issue
linked from #13895's record and from the fleet book programme
(#13900) — at which point this section's verbs move into the
reference chapter alongside the mailbox verbs above.
