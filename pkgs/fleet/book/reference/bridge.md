# fleet bridge

`fleet bridge` is the mailbox client for the message bridge (at `https://bridge.security.cab`). Every fleet agent has a **mailbox** on the bridge named after it, and registers with optional **groups** (`muse`, `code-lanes`, …). Central sends a message or a command to one agent, a group, or all; the agent reads its mailbox, does the work, and posts its answer back, which marks the command handled.

See also [The bridge: fleet messaging](../concepts/bridge.md).

Known discrepancy: every per-verb help invocation returns **one shared USAGE text** for the whole domain — there is no per-verb help in source, despite the domain status line "(options: fleet bridge &lt;command&gt; --help)". The per-verb sections below are written from source semantics.

## Tokens, URLs, and defaults (all verbs)

Two token families, both resolved **by name** through fleet-secrets; values are never printed, logged, or put in argv:

| Family                      | Secret name                                                               | Opens                                                                        | URL shape                    |
| --------------------------- | ------------------------------------------------------------------------- | ---------------------------------------------------------------------------- | ---------------------------- |
| Owner token (central verbs) | `message-bridge.access-token` (renamable via `[bridge] owner_token_name`) | `send`, `agents`, `log`                                                      | `/api/<owner-token>/fleet/…` |
| Fleet token (agent verbs)   | `message-bridge.fleet-token`                                              | `hello`, `inbox`, `post`, `ack` — agent routes only; it cannot send commands | `/fleet/<fleet-token>/…`     |

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
