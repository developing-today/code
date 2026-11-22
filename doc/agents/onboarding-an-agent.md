# Onboarding an agent to the fleet

Any agent joins the fleet the same way, whatever it is: a Muse lane, an
OpenCode or OpenChamber session, an OpenClaw agent, a Grok bot, Dot, Duet,
Codex, Gemini, Antigravity, or a Claude Code session. Design:
[`doc/2026-11-21T11-30-00Z_design_fleet-agent-front-door/`](../2026-11-21T11-30-00Z_design_fleet-agent-front-door/2026-11-21T11-30-00Z_design_fleet-agent-front-door.md).

## An agent with a shell

```bash
fleet agent kinds                                  # pick your kind
fleet agent enroll --kind <kind> --name <name>     # once per seat
fleet agent next                                   # each time you want work
```

- `<name>` is the agent's stable worker identity: lowercase letters,
  digits, `.`, `_`, `-`. The queue uses that name as the worker. Unless
  `--lane` is supplied, a queue-compatible lane name stays as-is; other
  names get an unfiltered `any-<letters-only-name>` lane. Pass `--lane`
  explicitly for a designated filtered lane.
- Enrolment registers your mailbox, tells you which intake rungs work from
  your seat, and saves a profile so later verbs know who is asking.
- `fleet agent next` returns one task and the commands to run when it is
  done. Exit `5` means no rung had work this run.
- `fleet agent doctor` repeats the reachability check at any time.

## An agent that can only make HTTPS requests

Enrol it from any seat that has the binary, then give the agent its name:

```bash
fleet agent enroll --kind http --name <name> --groups <its-kind>
```

The agent reads and answers its mailbox with the bridge's HTTPS calls
(`pkgs/message-bridge/worker/README.md`, "Fleet mailboxes"). `dot` and `duet`
are kinds of this seat.

## Claude and ChatGPT

Nothing to enrol. Their addresses are `claude` and `chatgpt`, reached through
the Message Bridge connector. Send to them like any other address.

## Sending work to an agent

```bash
fleet bridge send --to agent:<name> --command "<the task>"     # one agent
fleet bridge send --to group:muse --command "<the task>"       # every agent of a kind
fleet bridge send --to group:opencode --command "<the task>"   # OpenCode and OpenChamber
fleet bridge send --to claude "<a note>"                       # a connector
```

A command is a task and gets an answer. A plain message is information.

## Where work comes from

`fleet agent next` tries four places in order and takes the first task it
finds: a command pushed to you, the shared queue, the host's shortlist, then
the highest-priority open issue. If the shortlist is low and nobody is
refilling it, your task for that run is to refill it. You do not choose the
rung; the answer says which one fed you and why the others did not.

## Builds

Do not build inside a limited sandbox. Run the command where builds belong:

```bash
fleet shell -- <command>            # the laptop, a codespace, or here
fleet shell --build -- <command>    # a long build: takes a build slot
fleet shell                         # an interactive shell
```

A whole command line works as one quoted argument, and all of it runs under
the build slot: `fleet shell --build -- 'cd pkgs/id && cargo test'`.

## What you need

HTTPS is enough for messages and for three of the four rungs. The queue rung
and `fleet shell` need a route to the laptop, which the binary finds by
itself: the broker, the Cloudflare tunnel, the tailnet, then ssh. Secrets are
resolved by name through `fleet secrets`; never read a token file or call a
vault tool yourself.

## Waiting on a person

Do not. Decide, record the decision on the issue, and continue
(`.agents/skills/decide-and-proceed`,
[FDR-0013](../decisions/FDR-0013-no-owner-decision-gates.md)). The queue
refuses a park or block that waits on a person unless it is a hands item:

```bash
fleet queue block <task> --worker <w> --lane <l> --reason-class escalated-to-user \
  --hands interactive-login --action "<one sentence a person can do in five minutes>" \
  --blocker "<what it stops>"
```

A hands item blocks that step only. Take the next task.
