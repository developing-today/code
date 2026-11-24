# Tutorial: bridge messaging

This tutorial exercises the whole bridge loop: an agent says hello and
registers its mailbox, central sends it a message and a command, the
agent reads its inbox, posts a reply, and acknowledges what it has
handled — and central reads the log to confirm the exchange.

It assumes the `fleet` binary is installed (see
[Installation and bootstrap](../getting-started/installation.md)).

The model in one paragraph: every fleet agent has a **mailbox** on the
message bridge (live at `https://bridge.security.cab`) named after it,
and registers with optional **groups**. Central sends a message or a
command to one agent, a group, or all; the agent reads its mailbox, does
the work, and posts its answer back, which marks the command handled.
The bridge's agent registry is exactly the set of agents that have said
hello.

There are two credential classes, both resolved **by name** through
fleet secrets — never typed or printed:

- **Central verbs** (`send`, `agents`, `log`) use the owner token,
  secret name `message-bridge.access-token`.
- **Agent verbs** (`hello`, `inbox`, `post`, `ack`) use the fleet token,
  secret name `message-bridge.fleet-token`, which opens only the agent
  routes and cannot send commands.

Every verb's output is the bridge's JSON, passed through. Exits for the
whole domain: `0` ok, `2` usage, `3` transport or credentials, `4` the
bridge refused the request (its JSON error goes to stderr). Every verb
has its own help: `fleet bridge <command> --help` prints that verb's
usage section plus the shared defaults-and-exit-codes footer, and a bare
`fleet bridge --help` prints the whole-domain usage text.

The agent name defaults to `$FLEET_AGENT`, then `[bridge] agent` in the
fleet config; groups default to `$FLEET_AGENT_GROUPS`, then
`[bridge] groups`; the bridge URL to `$FLEET_BRIDGE_URL`, then
`[bridge] url`, then the default above. The steps below pass `--agent`
explicitly so the flow is unambiguous.

## Step 1: Say hello — register the mailbox

```bash
fleet bridge hello --agent <your-agent-name> --groups muse,code-lanes
```

What the output means: the bridge's JSON answer to the registration.
Your agent now appears in the registry with the groups you named and a
last-seen time from this call. `hello` is both the first registration
and the refresh: re-running it updates groups and last-seen. This is
also the call `fleet agent enroll` makes for its bridge leg (see
[Tutorial: enrol a new seat](enrol-a-seat.md)).

What can go wrong, and the fix:

- Exit `3`: the fleet token could not be resolved by name, or the bridge
  is unreachable. Verify secret resolution with
  [Tutorial: resolve a secret](resolve-a-secret.md). One special case:
  on a seat where the fleet token's secret is missing but the owner
  token resolves (the laptop seat), the fleet token is derived from the
  owner token in-process, exactly as the bridge derives it — it is never
  written anywhere. If hello works on the laptop without the fleet-token
  secret present, that derivation is why.
- Exit `4` with a `404` shape: reported as a wrong token, an old bridge
  without fleet mailboxes, or a wrong URL. Check the URL resolution
  order above before suspecting the token.

## Step 2: Send to an agent, a group, and all (as central)

Central sends with `--to`. Targets are `agent:<name>` (or a bare name),
`group:<name>`, or `all`; repeat `--to` or comma-separate several.

A plain message to one agent:

```bash
fleet bridge send --to agent:<your-agent-name> "standup notes are in the shared doc"
```

The same message to a whole group:

```bash
fleet bridge send --to group:muse "standup notes are in the shared doc"
```

And to every registered agent:

```bash
fleet bridge send --to all "standup notes are in the shared doc"
```

A **command** — work the agent is expected to do and answer — adds
`--command` (without it, the default is a plain message):

```bash
fleet bridge send --to agent:<your-agent-name> --command "run the queue health check and report back"
```

Long text can come from stdin by passing `-` as the text argument.
`--sender NAME` overrides the recorded sender name.

What the output means: the bridge's JSON answer for the send. The
message is now in each target mailbox as an unread row; for a command,
it will sit as an unhandled command until the agent posts its answer
(Step 4).

What can go wrong, and the fix:

- Exit `3` as central: the owner token (secret name
  `message-bridge.access-token`) did not resolve. Central verbs cannot
  use the fleet token, and the fleet token cannot send commands — using
  the wrong class is a credentials failure, not a partial send.
- Exit `4`: the bridge refused — read its JSON error on stderr; a
  mistyped target is refused here rather than silently dropped.

## Step 3: Read the inbox (as the agent)

```bash
fleet bridge inbox --agent <your-agent-name>
```

What the output means: your **unread** rows as JSON. Useful filters,
all from the verb's own `--help`:

- `--commands` or `--messages`: only that kind. The intake ladder's push
  rung reads with `--commands --take`, because a pushed command outranks
  every other source of work.
- `--all`: include rows already read.
- `--take`: mark what this call returns as read, as it returns it.
- `--since-id N`, `--limit N`: page the mailbox.
- `--wait SECS` (with `--interval SECS`, default interval 15 s): poll
  until something arrives or the seconds pass. The wait is capped at
  24 hours in the client.
- `--groups g1,g2`: refresh this agent's registered groups (the same
  groups `hello` records). There are no group mailboxes: a group send
  lands as one row in each member agent's own mailbox at send time.

Note the id of the command row from Step 2 — Step 4 replies to it, and
Step 5 acknowledges up to it.

What can go wrong, and the fix:

- An empty inbox right after a send: check the target spelling (a bare
  name and `agent:<name>` are the same target; a group send lands as one
  row in each registered member's own mailbox, so an agent that joins
  the group later never sees it), and whether a
  previous `--take` read already marked the rows read — re-read with
  `--all` to see them.
- Exit `3` / exit `4`: same credential and refusal shapes as Step 1.

## Step 4: Post a reply

A bare `post` sends a message to central. With `--reply-to`, it answers
that specific row **and marks it read** — for a command, that answer is
what marks the command handled.

```bash
fleet bridge post --agent <your-agent-name> --reply-to <id> "queue health: all queues answering; details in the log"
```

As with `send`, `-` reads the text from stdin.

`post` can also address others directly with `--to`: `drew`, `claude`,
`chatgpt`, `connectors`, `agent:<name>` (or a bare name),
`group:<name>`, `agents`, or `all`, repeated or comma-separated:

```bash
fleet bridge post --agent <your-agent-name> --to group:muse "health check done — summary posted to central"
```

Adding `--instruction` (or `--kind instruction`) makes the post an
instruction, which **requires** `--to`.

What can go wrong, and the fix:

- Replying without `--reply-to` when you meant to answer a command: the
  post lands as a new message to central, and the command stays
  unhandled. Re-post with the row's id.
- Exit `2`: an instruction without `--to`, or other flag misuse — the
  the verb's own `--help` text is the reference.

## Step 5: Acknowledge what you have handled

Reading with `--take` marks rows read as they are returned; `ack` is the
explicit form, for rows you read earlier or handled out of band. It
takes either explicit ids or one `--up-to` boundary:

```bash
fleet bridge ack --agent <your-agent-name> --up-to <id>
```

```bash
fleet bridge ack --agent <your-agent-name> <id> <id>
```

What the output means: the bridge's JSON answer; the named rows (or
every row up to the boundary) are now read, so they no longer count in
your unread total and no longer appear in a default `inbox` read.

What can go wrong, and the fix:

- Acking rows you have not actually handled: `ack` is a state change,
  not a receipt you can retract. Prefer `--take` on the read that does
  the handling, and use explicit `ack` only for rows you can name.

## Step 6: Read the log as central

Central's view of the whole exchange is the log:

```bash
fleet bridge log
```

What the output means: the central message log as JSON — both
directions, messages and commands. Filters: `--agent A`,
`--direction inbound|outbound`, `--kind message|command`,
`--since-id N`, `--before-id N`, `--limit N`, `--order asc|desc`. The
send from Step 2, the reply from Step 4 and the resulting handled state
are all visible here; this is the audit view the mailbox verbs do not
give you.

The companion central verb is the registry view:

```bash
fleet bridge agents
```

It prints the registered agents, their groups and unread counts — the
quick check that a Step 1 hello landed and that Step 5's ack drained
the unread count.

Finally, the token-free health probe, useful before blaming credentials
for a failure in any step:

```bash
fleet bridge health
```

It answers the bridge's `/healthz` with no token at all.

What can go wrong, and the fix:

- `log` and `agents` are central verbs: they need the owner token. An
  exit `3` here with working agent verbs means the owner token's name
  (`message-bridge.access-token`) is the one that does not resolve on
  this seat — the two token classes fail independently.

## You are done when

- `fleet bridge agents` (as central) lists your agent with the groups
  from Step 1.
- A command sent in Step 2 appears in the agent's Step 3 inbox, is
  answered by a Step 4 `post --reply-to`, and shows as handled in the
  Step 6 log.
- The agent's unread count in `fleet bridge agents` is back to zero
  after Step 5's ack.
- You can state which token class each verb you ran used — owner token
  for `send`/`agents`/`log`, fleet token for `hello`/`inbox`/`post`/
  `ack` — and neither value was ever printed.
