# fleet bridge

`fleet bridge` is the mailbox client for the message bridge (at `https://bridge.security.cab`). Every fleet agent has a **mailbox** on the bridge named after it, and registers with optional **groups** (`muse`, `code-lanes`, …). Central sends a message or a command to one agent, a group, or all; the agent reads its mailbox, does the work, and posts its answer back, which marks the command handled.

See also [The bridge: fleet messaging](../concepts/bridge.md).

Each verb has its own help: `fleet bridge <command> --help` prints that verb's usage section plus the shared defaults-and-exit-codes footer, as the domain status line advertises. A bare `fleet bridge --help` prints the whole-domain usage text.

## Tokens, URLs, and defaults (all verbs)

Two token families, both resolved **by name** through fleet-secrets; values are never printed, logged, or put in argv:

| Family                      | Secret name                                                               | Opens                                                                        | URL shape                    |
| --------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ---------------------------- |
| Owner token (central verbs) | `message-bridge.access-token` (renamable via `[bridge] owner_token_name`) | `send`, `agents`, `log`                                                      | `/api/<owner-token>/fleet/…` |
| Fleet token (agent verbs)   | `message-bridge.fleet-token`                                              | `hello`, `inbox`, `post`, `ack`, `work` — agent routes only; it cannot send commands | `/fleet/<fleet-token>/…`     |
| Per-agent credential        | `bridge.<agent>-fleet-token` (the name the agent profile records)         | The same agent routes, as that one agent only; its groups are the ones bound when the owner minted its bootstrap token | `/fleet/<credential>/…` |
| Notes token (notes family)  | `bridge.<agent>-token` (renamable via `[bridge] notes_token_name`)        | `notes send`, `notes inbox`, `notes ack` for the token's class/name identity | `Authorization: Bearer` header on `/notes/…` |

Tokens travel **in the URL path**, so every error string is scrubbed of token values (`<token>`) before it is printed, and a 404 is reported as "a wrong token, an old bridge without fleet mailboxes, or a wrong URL". Plain `http` URLs are accepted only to this machine; anything else must be `https`, because tokens ride the path.

Derivation fallback: when the fleet token's secret is missing but the owner token resolves (the laptop seat), the fleet token is **derived from the owner token exactly as the bridge derives it**: `base64url(HMAC-SHA256(key = owner token bytes, message = "message-bridge/fleet-token/v1"))`. The derived value is used in-process only; it is never written anywhere.

Defaults: the agent name defaults to `$FLEET_AGENT`, then `[bridge] agent`; groups to `$FLEET_AGENT_GROUPS`, then `[bridge] groups`; the bridge URL to `$FLEET_BRIDGE_URL`, then `[bridge] url`, then `https://bridge.security.cab` (env beats file throughout). Output is the bridge's JSON, passed through.

Exit codes for every verb unless noted: `0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request (its JSON error goes to stderr).

## fleet bridge send

```
fleet bridge send --to <targets> [--command] [--sender NAME] <text | ->
```

| Flag                       | Argument    | Default             | Meaning                                                                                           |
| -------------------------- | ----------- | ------------------- | ------------------------------------------------------------------------------------------------- |
| `--to`                     | `<targets>` | required            | Targets: `agent:<name>` (or a bare name), `group:<name>`, `all`. Repeat `--to` or comma-separate. |
| `--command`                | —           | off (plain message) | Mark the send as a command.                                                                       |
| `--sender`                 | `NAME`      | —                   | Sender name recorded on the message.                                                              |
| `<text \| ->` (positional) | text        | required            | Message text; `-` reads it from stdin.                                                            |

### Behaviour

Central verb (owner token). Sends a message — or, with `--command`, a command — to one agent, a group, or all agents. Route: `POST /api/<owner-token>/fleet/send`. A command sits in the target mailboxes until an agent takes it (see `inbox --take`) and posts an answer back, which marks it handled.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge send --to group:muse --command "run the nightly triage"
```

Sends a command to every agent in the `muse` group.

## fleet bridge agents

```
fleet bridge agents
```

| Flag | Argument | Default | Meaning                          |
| ---- | -------- | ------- | -------------------------------- |
| —    | —        | —       | Takes no arguments and no flags. |

### Behaviour

Central verb (owner token). Lists the registered agents, their groups, and unread counts. Route: `GET /api/<owner-token>/fleet/agents`. The bridge's agent registry is exactly the set of agents that have said `hello` (name, groups, last-seen, unread counts).

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge agents
```

Prints the registered agents, groups, and unread counts as the bridge's JSON.

## fleet bridge log

```
fleet bridge log [--agent A] [--direction inbound|outbound] [--kind message|command] [--since-id N] [--before-id N] [--limit N] [--order asc|desc]
```

| Flag          | Argument            | Default        | Meaning                   |
| ------------- | ------------------- | -------------- | ------------------------- |
| `--agent`     | `A`                 | —              | Filter to one agent.      |
| `--direction` | `inbound\|outbound` | —              | Filter by direction.      |
| `--kind`      | `message\|command`  | —              | Filter by kind.           |
| `--since-id`  | `N`                 | —              | Only rows after this id.  |
| `--before-id` | `N`                 | —              | Only rows before this id. |
| `--limit`     | `N`                 | bridge default | Maximum rows.             |
| `--order`     | `asc\|desc`         | bridge default | Row order.                |

### Behaviour

Central verb (owner token). Reads the central message log. Route: `GET /api/<owner-token>/fleet/messages?…` with the filters as query parameters.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge log --kind command --limit 5
```

Shows the five most recent commands in the central log.

## fleet bridge mint-enroll

```
fleet bridge mint-enroll --agent A [--kind K] [--groups g1,g2] [--notes-identity I] [--ttl SECS]
```

| Flag               | Argument | Default  | Meaning                                                              |
| ------------------ | -------- | -------- | -------------------------------------------------------------------- |
| `--agent`          | `A`      | required | The new agent's name; the token is bound to it and enrolls no other. |
| `--kind`           | `K`      | `agent`  | The agent kind the token is bound to.                                |
| `--groups`         | `g1,g2`  | —        | Groups the mailbox registers with at exchange.                       |
| `--notes-identity` | `I`      | —        | A class/name notes identity to register at exchange.                 |
| `--ttl`            | `SECS`   | `86400`  | The token's lifetime in seconds (60 to 604800).                      |

### Behaviour

Central verb (owner token). Mints a bootstrap enroll token. Route: `POST /api/<owner-token>/enroll-tokens`. The answer carries the token value **once**: the bridge stores only its SHA-256 hash and can never show it again. Hand the value to the new seat, which runs `fleet agent enroll --name A --kind K --bootstrap <value>`; the exchange burns the token and returns the agent's fleet credential and notes token, likewise once. The PM console's Agents screen mints these from its Provisioning section and shows the value and the command once, on that screen only.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge mint-enroll --agent scout-2 --kind muse --groups muse
```

Mints a day-long bootstrap token for `scout-2` and prints it, once.

## fleet bridge enroll-tokens

```
fleet bridge enroll-tokens
```

| Flag | Argument | Default | Meaning                          |
| ---- | -------- | ------- | -------------------------------- |
| —    | —        | —       | Takes no arguments and no flags. |

### Behaviour

Central verb (owner token). Lists every minted bootstrap token's state: `pending`, `used` (with the exchange's time), `expired` (its TTL lapsed), or `revoked` (with the revocation time), each with its binding (agent, kind, groups, notes identity) and mint/expiry times. Route: `GET /api/<owner-token>/enroll-tokens`. State only: no hash or value is ever shown — the value existed once, in the mint answer.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge enroll-tokens
```

Shows which minted tokens are still outstanding, and how each closed one ended.

## fleet bridge revoke-enroll

```
fleet bridge revoke-enroll (--agent A | --token-hash H)
```

| Flag           | Argument | Default | Meaning                                                                                      |
| -------------- | -------- | ------- | -------------------------------------------------------------------------------------------- |
| `--agent`      | `A`      | —       | Take back every still-pending token minted for this agent; the answer counts them.           |
| `--token-hash` | `H`      | —       | Revoke the one token stored under this SHA-256 hash; the answer is `{revoked: true\|false}`. |

### Behaviour

Central verb (owner token). Revokes unused bootstrap tokens. Route: `POST /api/<owner-token>/enroll-tokens/revoke`. Exactly one form is required. The hash is not a credential — it cannot enroll anything — so it may travel where a token value never may. Used tokens are untouched: they became credentials, revoked with `fleet bridge revoke --agent`. Expired tokens are already dead and keep their state.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge revoke-enroll --agent scout-2
```

Takes back every outstanding token minted for `scout-2`.

## fleet bridge wake-hooks

```
fleet bridge wake-hooks [--set - | --remove --address A]
```

| Flag        | Argument | Default | Meaning                                                                            |
| ----------- | -------- | ------- | ---------------------------------------------------------------------------------- |
| `--set`     | `-`      | —       | Set one hook from a JSON object `{address, url, token?}` read from stdin.          |
| `--remove`  | —        | —       | Remove the hook for `--address`; the answer is `{removed: true\|false}`.           |
| `--address` | `A`      | —       | The hook's address (`channel:claude`, `channel:chatgpt` or `agent:<name>`); `--remove` only. |

### Behaviour

Central verb (owner token). With no flag, lists every wake hook — address, kind, host, and whether it holds a token; never the token or the URL's path. Routes: `GET`/`POST /api/<owner-token>/wake-hooks` (a POST with no `url` removes). The set form reads its JSON from stdin so a hook token never travels as an argument. This is the verb the PM console's Agents screen drives (#14544): the console lists hooks at the edge through the bridge's read-scope view (`GET /fleet/<credential>/wake-hooks`), and sets and removes through a machine, as with revoking.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge wake-hooks
$ echo '{"address":"agent:scout-2","url":"https://hooks.example/wake"}' | fleet bridge wake-hooks --set -
$ fleet bridge wake-hooks --remove --address agent:scout-2
```

## fleet bridge mint-credential

```
fleet bridge mint-credential --agent A [--scopes s1,s2] [--addr service:<name>]
```

| Flag       | Argument         | Default   | Meaning                                                                                              |
| ---------- | ---------------- | --------- | ---------------------------------------------------------------------------------------------------- |
| `--agent`  | `A`              | required  | The agent (or service) the credential belongs to; it acts only as itself.                            |
| `--scopes` | `s1,s2`          | see below | Any of `inbox`, `send`, `read`. Agent mints default to `inbox, send`; service mints to `send, read`. |
| `--addr`   | `service:<name>` | —         | Make it a service credential, bound to that address; the name must be the agent's own.               |

### Behaviour

Central verb (owner token). Mints a per-agent credential directly, for an agent already enrolled or for a service. Route: `POST /api/<owner-token>/fleet/credentials/mint`. Like the enroll mint, the answer carries the credential value **once** and the bridge stores only its hash. Enroll-minted credentials never carry `read`; that scope is for service credentials like the PM stack's.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge mint-credential --agent pm-stack --addr service:pm-stack
```

Mints the PM stack's service credential and prints it, once, to be stored in the vault as `bridge.pm-stack-token`.

## fleet bridge hello

```
fleet bridge hello [--agent A] [--groups g1,g2]
```

| Flag       | Argument | Default                                       | Meaning                         |
| ---------- | -------- | --------------------------------------------- | ------------------------------- |
| `--agent`  | `A`      | `$FLEET_AGENT`, then `[bridge] agent`         | Agent name to register.         |
| `--groups` | `g1,g2`  | `$FLEET_AGENT_GROUPS`, then `[bridge] groups` | Comma-separated groups to join. |

### Behaviour

Agent verb (fleet token). Registers / refreshes this agent and its groups — this is the registry write: the bridge's agent registry is exactly the set of agents that have said hello. Route: `POST /fleet/<fleet-token>/hello` with `{agent, groups}`.

Under a **per-agent credential** the hello only refreshes: the credential's groups were bound when the owner minted its bootstrap token and set by the exchange, and a credentialed hello (or inbox call) cannot change them — group membership drives `group:<name>` addressing, so it is not the caller's to reassign. The shared fleet token keeps caller-supplied groups.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge hello --agent archer --groups muse
```

Registers `archer` in the `muse` group (or refreshes an existing registration).

## fleet bridge inbox

```
fleet bridge inbox [--agent A] [--groups g1,g2] [--commands | --messages] [--all] [--since-id N] [--limit N] [--take] [--wait SECS] [--interval SECS]
```

| Flag         | Argument | Default                                       | Meaning                                                               |
| ------------ | -------- | --------------------------------------------- | --------------------------------------------------------------------- |
| `--agent`    | `A`      | `$FLEET_AGENT`, then `[bridge] agent`         | Mailbox to read.                                                      |
| `--groups`   | `g1,g2`  | `$FLEET_AGENT_GROUPS`, then `[bridge] groups` | Groups whose mail is included.                                        |
| `--commands` | —        | off                                           | Only commands. Mutually exclusive in effect with `--messages`.        |
| `--messages` | —        | off                                           | Only plain messages.                                                  |
| `--all`      | —        | off (unread only)                             | Include read messages as well as unread.                              |
| `--since-id` | `N`      | —                                             | Only rows after this id.                                              |
| `--limit`    | `N`      | bridge default                                | Maximum rows.                                                         |
| `--take`     | —        | off                                           | Mark what this call returns as read.                                  |
| `--wait`     | `SECS`   | off                                           | Poll until something arrives or SECS pass (capped at 24 h in source). |
| `--interval` | `SECS`   | `15`                                          | Poll interval while waiting.                                          |

### Behaviour

Agent verb (fleet token). Reads this agent's mailbox: unread by default; `--all` includes read ones; `--take` marks what it returns as read; `--wait` polls (default interval 15 s) until something arrives or SECS pass. Route: `GET /fleet/<fleet-token>/inbox?…`.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge inbox --agent archer --commands --take
```

Takes this agent's unread commands, marking them read as they are returned.

## fleet bridge post

```
fleet bridge post [--agent A] [--reply-to ID] [--to ADDR] [--instruction | --kind K] <text | ->
```

| Flag                       | Argument | Default                               | Meaning                                                                                                                                                             |
| -------------------------- | -------- | ------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--agent`                  | `A`      | `$FLEET_AGENT`, then `[bridge] agent` | Posting agent.                                                                                                                                                      |
| `--reply-to`               | `ID`     | —                                     | Answer that row (and mark it read).                                                                                                                                 |
| `--to`                     | `ADDR`   | central                               | Send to any address instead: `drew`, `claude`, `chatgpt`, `connectors`, `agent:<name>` (or a bare name), `group:<name>`, `agents`, `all`. Repeat or comma-separate. |
| `--instruction`            | —        | off                                   | Make it an instruction. Requires `--to`.                                                                                                                            |
| `--kind`                   | `K`      | —                                     | Kind override; `--kind instruction` is equivalent to `--instruction`.                                                                                               |
| `<text \| ->` (positional) | text     | required                              | Message text; `-` reads it from stdin.                                                                                                                              |

### Behaviour

Agent verb (fleet token). Posts a message to central by default; with `--reply-to` it answers that row (and marks it read) — posting an answer back is what marks a command handled. Route: `POST /fleet/<fleet-token>/send`.

### Exit codes

`0` ok · `2` usage (including `--instruction` without `--to`) · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge post --agent archer --reply-to 412 "nightly triage done: 3 issues filed"
```

Answers message 412 and marks it read.

## fleet bridge ack

```
fleet bridge ack [--agent A] (--up-to ID | ID...)
```

| Flag                 | Argument | Default                               | Meaning                                                            |
| -------------------- | -------- | ------------------------------------- | ------------------------------------------------------------------ |
| `--agent`            | `A`      | `$FLEET_AGENT`, then `[bridge] agent` | Agent whose mailbox to ack in.                                     |
| `--up-to`            | `ID`     | —                                     | Mark every message up to this id read. Alternative to listing ids. |
| `ID...` (positional) | ids      | —                                     | Mark exactly these messages read.                                  |

### Behaviour

Agent verb (fleet token). Marks messages read without posting an answer. Route: `POST /fleet/<fleet-token>/ack`.

### Exit codes

`0` ok · `2` usage (neither `--up-to` nor any id) · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge ack --agent archer --up-to 412
```

Marks all of `archer`'s messages up to id 412 as read.

## fleet bridge work

```
fleet bridge work [--agent A] --state acted|queued|done [--task ID] ID...
```

| Flag                 | Argument              | Default                               | Meaning                                                            |
| -------------------- | --------------------- | ------------------------------------- | ------------------------------------------------------------------ |
| `--agent`            | `A`                   | `$FLEET_AGENT`, then `[bridge] agent` | Agent whose mailbox copy to mark.                                  |
| `--state`            | `acted\|queued\|done` | required                              | The lifecycle state to record.                                     |
| `--task`             | `ID`                  | —                                     | The fleet queue task the instruction became; required for `queued`. |
| `ID...` (positional) | ids                   | required                              | The instruction rows to mark; several ids are marked one by one.   |

### Behaviour

Agent verb (fleet token). Records the **work-item lifecycle** of instructions in this agent's own mailbox. Route: `POST /fleet/<fleet-token>/work` with `{agent, id, state, task_id?}` per id. An instruction starts at `received`; the receiving agent moves its own copy forward — `acted` (handled in-session), `queued` (it became a fleet queue task; `--task` names it), then `done`. States move only forward (`received` → `acted` | `queued` → `done`); re-marking the current state is a no-op. Only the receiving agent can mark a row: with a per-agent credential, naming another agent or another agent's row is the bridge's plain 404. The owner sees every row's state and task id in the pipeline, so an instruction that is neither acted on nor queued stays visible at `received`. Notes instructions carry the same lifecycle on the notes surface (`POST /notes/work`, same body shape); connector-channel instructions store no lifecycle fields — their read state stands in for `acted` in the pipeline.

The rule the lifecycle serves, carried verbatim by this book, the verb's help text, and the woken-agent text the intake ladder hands out with a pushed command:

> Task it or queue it. An instruction is a work item: do the work now, in this session, and reply — or queue it as a repo-task/v1 packet whose context_refs carry the bridge surface, the message id and the batch, and reply with the task id. Record the outcome with fleet bridge work: --state acted for work done in-session, --state queued --task <id> for queued work, --state done when the work is finished. An instruction that is neither acted on nor queued is a defect: it stays at received, where the owner can see it stuck in the pipeline.

### Exit codes

`0` ok · `2` usage (including `--state queued` without `--task`) · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge work --agent archer --state queued --task 23500 412
```

Records that archer queued instruction 412 as fleet queue task 23500.
## fleet bridge exchange

```
fleet bridge exchange --agent A --bootstrap TOKEN
```

| Flag          | Argument | Default                                       | Meaning                                                                 |
| ------------- | -------- | --------------------------------------------- | ----------------------------------------------------------------------- |
| `--agent`     | `A`      | `$FLEET_AGENT`, then `[bridge] agent`         | The agent name the bootstrap token was minted for.                      |
| `--bootstrap` | `TOKEN`  | required                                      | The single-use bootstrap token the owner minted (console hand-off).     |

### Behaviour

Provisioning verb (no stored token; the bootstrap token rides the request body and is scrubbed from every error). Trades the token at `POST /enroll/exchange` for the agent's own credentials — a per-agent fleet credential and, when the mint carried a notes identity, a notes token. The values come back exactly once and go straight into the vault through fleet-secrets' write path (Bitwarden, into the project named by `$FLEET_BWS_PROJECT_ID` or `[secrets] bws_project_id`) under `bridge.<agent>-fleet-token` and `bridge.<agent>-token`. The verb prints names and metadata only — never a value. A refused exchange (wrong, used, expired or revoked token, or a name that is not the minted name) stores nothing; if storing fails after a successful exchange, the error says so loudly, because the burned token cannot be replayed. `fleet agent enroll --bootstrap` runs this verb first, then enrols as usual.

### Exit codes

`0` ok · `2` usage · `3` transport, credentials or vault write · `4` the bridge refused the exchange.

### Example

```console
$ fleet bridge exchange --agent muse-9 --bootstrap <token from the console>
```

Stores `muse-9`'s credentials under `bridge.muse-9-fleet-token` and `bridge.muse-9-token` and prints the exchange's names and metadata.

## fleet bridge notes

```
fleet bridge notes send --to CLASS/NAME [--kind note|instruction] [--reply-to ID] <text | ->
fleet bridge notes inbox
fleet bridge notes ack (--up-to ID | ID...)
```

| Flag                       | Argument    | Default  | Meaning                                                                                                              |
| -------------------------- | ----------- | -------- | -------------------------------------------------------------------------------------------------------------------- |
| `--to`                     | `CLASS/NAME` | —       | Recipient identity; a `*` segment wildcards (`muse/*`, `*/archer`, `*/*`). Required unless `--reply-to` is given.   |
| `--kind`                   | `note\|instruction` | `note` | `instruction` makes the note a work item: it lands in the recipient's inbox **without a sender** (the bridge keeps the sender store-side). |
| `--reply-to`               | `ID`        | —        | Answer a note in your own inbox. The reply is routed store-side to that note's real sender — including an instruction whose sender you cannot see — and may replace `--to`. |
| `--up-to` (ack)            | `ID`        | —        | Acknowledge every note up to this id. Alternative to listing ids.                                                    |
| `ID...` (ack, positional)  | ids         | —        | Acknowledge exactly these notes.                                                                                     |

### Behaviour

The class/name notes family, authenticated by the calling identity's own notes token as a `Bearer` header (routes `POST /notes/send`, `GET /notes/inbox`, `POST /notes/ack`). Inbox rows carry `kind` and `reply_to`; a plain note names its sender in `from`, an instruction's `from` is null. Replying to an instruction therefore needs nothing but its id: the replier never learns who sent it, and the reply still reaches them.

### Exit codes

`0` ok · `2` usage · `3` transport or credentials · `4` the bridge refused the request.

### Example

```console
$ fleet bridge notes send --reply-to 41 "check done, filed as task 23101"
```

Answers note 41 — even if it arrived as a sender-free instruction.

## fleet bridge health

```
fleet bridge health
```

| Flag | Argument | Default | Meaning                                            |
| ---- | -------- | ------- | -------------------------------------------------- |
| —    | —        | —       | Takes no arguments and no flags, and **no token**. |

### Behaviour

Reads the bridge's `/healthz`, **unauthenticated** — the only bridge verb that needs no token. It proves the service answers; it proves nothing about whether this seat holds a usable token (that is what `fleet agent doctor`'s bridge leg checks, with a real mailbox read).

### Exit codes

`0` ok · `2` usage · `3` transport · `4` the bridge refused the request.

### Example

```console
$ fleet bridge health
```

Prints the bridge's health JSON.
