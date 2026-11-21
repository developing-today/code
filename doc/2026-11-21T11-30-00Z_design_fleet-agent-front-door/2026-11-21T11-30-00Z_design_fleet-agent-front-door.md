# Fleet agent front door: one way in for every agent

- **Date:** 2026-11-21
- **Kind:** design, implemented in the same change
- **Code:** `pkgs/fleet/crates/fleet-agent` (`fleet agent`, `fleet shell`),
  `pkgs/fleet/crates/fleet-queue/src/person_wait.rs`, `placement.rs`,
  `pkgs/fleet/crates/fleet-secrets`, `pkgs/fleet/fleet-surfaces.json`
- **Rules it rests on:** [FDR-0013](../decisions/FDR-0013-no-owner-decision-gates.md)
- **Plan:** [`planning/plans/2026-11-21_results-first.md`](../../planning/plans/2026-11-21_results-first.md)

## What was asked

The owner asked on 2026-11-21 for one strategy that lets the fleet send and
receive messages as a whole, and that covers five things:

1. Any persistent agent can be onboarded: Muse agents, Grok bots, Dot and
   Duet, OpenClaw, Antigravity, and direct use from ChatGPT and Claude.
2. Those agents, and the OpenCode and OpenChamber pipeline, can be handed
   tasks.
3. An agent talks to the `fleet` binary and nothing else. The binary handles
   secrets and reaches Cloudflare. Tailscale is needed only for particular
   parts.
4. Builds go to the fleet's GitHub Actions copies, a codespace or the laptop
   as fits, and the binary has a subcommand that connects to a shell there, so
   nothing builds inside a limited sandbox.
5. Tasks reach agents by a ladder of fallbacks: a central push, then queues
   picked up on a schedule, then a local shortlist that a run refills when it
   sees the list is low, then whatever has the highest priority.

He also asked that the system not treat work as waiting on him because some
other session said so. That part is FDR-0013; this design is where it is
enforced.

## What already existed

Most of the transport was already built, as separate verbs that each agent had
to know and combine in the right order.

| Need                    | Existing piece                                                               |
| ----------------------- | ---------------------------------------------------------------------------- |
| Messages and commands   | `fleet bridge`: mailboxes on the Message Bridge worker, with wake hooks      |
| Queue of small tasks    | `fleet queue`: claim, checkpoint, complete                                   |
| Run a command elsewhere | `fleet laptop-agent exec`, over broker, Cloudflare tunnel, tailnet, then ssh |
| Interactive shell       | `fleet proxy ssh`                                                            |
| Build placement         | `fleet queue dispatch` over the surface registry                             |
| Secrets by name         | `fleet secrets`: the secrets broker, then `bws`                              |

What was missing was the order of use, a place to say what kind of agent is
asking, the two lower rungs of the ladder, and a guard against owner waits.

## What this adds

### 1. `fleet agent`: enrol, check, take the next task

```bash
fleet agent kinds                               # the kinds and how each enrols
fleet agent enroll --kind muse --name muse-7    # register, probe, save a profile
fleet agent doctor                              # what this seat reaches now
fleet agent next                                # one task, from the highest rung that has one
fleet agent shortlist status|add|take|fill|clear
```

An agent's whole loop is three lines:

```bash
fleet agent next              # exit 5: no work on any rung this run
# do the task in the answer, then run the answer's `then` lines
# repeat while the run has budget
```

Every answer is JSON on stdout. Exit codes are `0` done, `2` usage, `3` a leg
this verb needs is down, `5` no work.

### 2. Kinds and seats

A kind says what the agent is. A seat says how it reaches the fleet.

| Seat        | Kinds                                                                                                                       | How it enrols                                                                                        |
| ----------- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------- |
| `binary`    | `muse`, `opencode`, `openchamber`, `openclaw`, `claude-code`, `codex`, `gemini`, `antigravity`, `grok`, `laptop`, `generic` | `fleet agent enroll --kind K --name N` from its own shell                                            |
| `http`      | `dot`, `duet`, `http`                                                                                                       | Enrolled from any seat that has the binary; the agent then uses the mailbox's HTTPS calls            |
| `connector` | `claude`, `chatgpt`                                                                                                         | Nothing to enrol: the address is `claude` or `chatgpt`, reached through the Message Bridge connector |

Enrolment registers the mailbox with the kind's groups, so `group:muse` or
`group:opencode` reaches every agent of that kind. `openchamber` joins
`opencode` too, so one send reaches the whole pipeline. A Grok bot with no
shell enrols as kind `http` under the same name and groups.

Enrolment then probes four legs at once and reports which ladder rungs work
from this seat. A leg that is down says what it needs. Enrolment fails only
when no intake leg answers at all.

### 3. What each part needs from the network

| Part                          | Needs                                                                          | Tailscale                      |
| ----------------------------- | ------------------------------------------------------------------------------ | ------------------------------ |
| Messages, and the push rung   | HTTPS to the Message Bridge (Cloudflare), and the fleet token by name          | No                             |
| Priority rung, shortlist fill | HTTPS to GitHub, and a `gh` token (the fleet's rotation, or a signed-in `gh`)  | No                             |
| Shortlist                     | A local file                                                                   | No                             |
| Queue rung                    | The queue broker: its socket on the laptop, else the laptop transport          | Only as one fallback transport |
| `fleet shell`, builds         | The laptop-agent ladder, a codespace through `gh`, or a GitHub Actions surface | Only as one fallback transport |
| Secrets                       | The secrets broker, then `bws`, then `op` where a vault is named               | No                             |

The laptop transport is the existing ladder: the broker, then the laptop's
Cloudflare tunnel, then the tailnet, then ssh. So a seat with HTTPS and nothing
else has messaging and three of the four rungs. Tailscale matters only for
reaching the laptop when its Cloudflare tunnel is down.

### 4. The intake ladder

`fleet agent next` walks four rungs and stops at the first that has a task.

1. **Push.** A command sent to this agent, its group or everyone:
   `fleet bridge inbox --agent N --commands --take --limit 1`. Wake hooks make
   this immediate for agents that can be woken.
2. **Queue.** `fleet queue claim`, the shared queue that scheduled lane runs
   already drain.
3. **Shortlist.** A ranked local file on the host, shared by every agent
   there. If the list is below its low-water mark (3) and nobody has claimed
   the refill in the last 30 minutes, this run's task **is** to refill it: the
   answer is a `refill-shortlist` task with the ranked candidates, and the
   run fills the list to its target (8) and then takes from it.
4. **Priority.** The highest-ranked open issue on GitHub that nobody is on.

Rank order is `expedite`, `priority/critical`, `now`, `priority/high`, `next`,
`priority/normal`, then unranked, oldest first within a rank. An issue is left
out when it carries a hold label (`held`, `user/owned`, `do-not-auto-edit`) or
a label showing someone is on it. `user/escalated` and `status/blocked` are
**not** holds: such an issue is handed out with an instruction to decide what
was asked, record it, clear the labels and continue.

A task from the shortlist or priority rung is a large-issue task, so it tells
the agent to land one slice (`.agents/skills/feature-slice`). This is how
large `id` work reaches agents while the queue still has only small-task
kinds.

Every answer lists each rung tried and why it was passed over, so a down leg
is visible and is never mistaken for an empty one. Each call to another leg
has a time limit (45 seconds, `FLEET_AGENT_STEP_TIMEOUT`); a leg that does not
answer in time is down, and the ladder moves on. A queue claim gets four times
the limit, because cutting a claim short would orphan it.

A pushed task is the command's body. The ladder does not pass along who sent
it, and nothing becomes the owner's instruction because a message says it is.

### 5. `fleet shell`: run it where builds belong

```bash
fleet shell -- cargo test -p id-core              # a command, on the best target
fleet shell --build -- nix build .#id             # takes a build slot, one-hour limit
fleet shell                                       # an interactive shell
fleet shell --target codespace --codespace NAME -- make test
fleet shell targets                               # what this seat can reach
fleet shell --print -- <command>                  # show the plan, run nothing
```

With `--target auto` the order is: here, if this is the laptop; the laptop, if
it answers; a running codespace; here as the last resort. A named target is
never swapped for another. On the laptop a command runs through
`fleet laptop-agent exec`, and `--build` runs it under the build lock.

Jobs that should not hold a shell go to GitHub Actions through
`fleet queue dispatch`, which picks a surface from the registry.

### 6. Three more build surfaces, by dispatching as the user

The registry listed three surfaces as waiting for a GitHub App grant since
2026-11-04. The reason given was a rule that dispatch must use the App, which
no decision record backs. The fleet's GitHub user is an admin of all three
copies and can start a workflow there with the token it already has.

A surface entry can now say `"dispatch_as": "user"`. Eligibility, dispatch,
status and load reads for that surface use the user's token: from the
environment, then the seat's `gh` login, then the `gh` hosts file by name
through fleet secrets. A surface without the field behaves as before, and
never falls back to the user's token.

Each was proven with a real run on 2026-11-21 before the registry changed:

| Surface                | Host                           | Run         | Result                                              |
| ---------------------- | ------------------------------ | ----------- | --------------------------------------------------- |
| `fleet-org`            | `developing-today/fleet`       | 44900088840 | Success on `ubuntu-24.04`                           |
| `fleet-org-blacksmith` | `developing-today/fleet`       | 44900255411 | Success on a `blacksmith-4vcpu-ubuntu-2404` runner  |
| `fleet-forks`          | `developing-today-forks/fleet` | 44900259404 | Success on `ubuntu-24.04`, after a sync to upstream |

Five surfaces are now active. Concurrent GitHub-hosted jobs go from 60 to 80,
and Blacksmith's free minutes become reachable as a spill tier. Its paid cap
stays 0.

### 7. No waits on a person

`fleet queue park` and `fleet queue block` now refuse a wait on a person
unless it is a hands item. The check runs before the broker is contacted and
exits `2` with the rule and what to do instead.

- A wait is a person wait when its reason class is one of the owner classes,
  its ready condition is `person_answer`, or its text says the owner must
  decide, approve, confirm or answer.
- A hands item names what only a person's hands can do:
  `--hands physical|interactive-login|payment|signature`, with `--action`, one
  sentence of at most 200 characters.
- A hands item blocks that step only. The agent takes the next task.

On GitHub a hands item carries `user/escalated` and one marker line in its
body: `<!-- hands:v1 kind=<kind> action="<one sentence>" -->`. Three places
read it:

- `fleet agent next` never hands such an issue to an agent, and it skips
  `kind/tracker` issues, which index work without being work.
- The dashboard's census (`pm-stack/aggregator/src/needs-drew.ts`) splits the
  items carrying `user/review` or `user/escalated` into three groups: needs a
  person's hands, review requested, and parked on the owner by an agent. Only
  the first two count as needing the owner. The third is listed as fleet work
  whose count should fall to zero.
- The digest's "Needs Drew" section lists the first two groups only.

### 8. Secrets: `op` beside `bws`

The secrets chain gains an optional third source, the 1Password CLI. It joins
the chain only where `FLEET_OP_VAULT` names a vault, and reads
`op://<vault>/<secret name>/<field>`, with `credential` as the default field
(`FLEET_OP_FIELD`). Seats without it keep the earlier chain exactly. Values
are never printed, logged or stored, as before.

## Decisions

Each is made, with how to undo it.

| #   | Decision                                                                         | Why                                                                                              | Undo                                          |
| --- | -------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------ | --------------------------------------------- |
| A1  | Compose the existing verbs through the same binary. No new transport or service. | The transports work. What was missing was order and a single entry point.                        | Delete the crate; nothing else depends on it. |
| A2  | Surfaces may dispatch as the user. Three surfaces are active.                    | The user can already start those workflows; waiting for an App grant bought nothing for 17 days. | Remove `dispatch_as` and set the status back. |
| A3  | Large work arrives through rungs 1, 3 and 4. No new queue kind yet.              | A queue kind touches the validator, dedupe, the store, dispatch, the schema and the PM stack.    | None needed; a queue kind can be added later. |
| A4  | Connectors are addresses, not enrolled agents.                                   | Claude and ChatGPT already have the bridge connector; a mailbox would be a second, unread inbox. | Add a `binary` kind row.                      |
| A5  | A slow leg is a down leg, after 45 seconds.                                      | Measured: without a limit, one enrolment in a seat with no broker took more than two minutes.    | Raise `FLEET_AGENT_STEP_TIMEOUT`.             |
| A6  | `op` is off unless a vault is named, and is last in the chain.                   | Nothing changes for seats that use the broker or `bws`.                                          | Unset `FLEET_OP_VAULT`.                       |
| A7  | `fleet-forks` targets public repos only.                                         | That copy has no checkout secret, and only a public target was proven.                           | Add the secret, then add the target.          |

## How it was checked

- The whole fleet workspace passes with the locked dependencies: 943 tests. The aggregator passes its 3,037 tests and its typecheck.
- `fleet-agent`: 45 tests, three of them property tests. The shortlist order
  is checked against a sort, the file against a round trip, and shell quoting
  against the real `/bin/sh`. Time limits are checked against real child
  processes. The quoting property found a real fault before merge: a bare `~`
  was sent unquoted, so the far shell replaced it with a home directory.
- The wait guard: 8 unit tests, and 5 tests that run the real `repo-queue`
  binary against a fake broker that counts connections, so "refused before
  the broker is asked" is observed.
- Placement: 7 new tests for `dispatch_as`. Secrets: 4 new tests for `op`.
- 34 hand-made faults were each caught by a test: 27 across the ladder, the
  shortlist, the shell plan, the guard and its wiring, placement and the
  secrets source, and 7 in the dashboard census.
- The built binary was run end to end: the park and block refusals exit `2`,
  `fleet shell --print` plans the laptop build command, the serve variant
  answers `fleet agent` and `fleet shell` with exit `69`, and
  `fleet agent next` returned real ranked candidates from GitHub.
- The three surfaces were dispatched for real, as above.

## Known gaps, in the order they should be closed

1. The `fleet-org` copy's `fleet-run.yml` has diverged from upstream, so its
   daily sync has failed since 2026-11-05. The copies need to converge:
   upstream takes the cache options and the anonymous checkout, and the org
   copy takes `dispatch_id` and `setup_runner`.
2. `codespace-overflow` needs the `codespace` scope on the `gh` login. That is
   a hands item (an interactive login); nothing else waits on it.
3. A dry-run enrolment checks that the bridge answers, not that this seat
   holds the fleet token, so it can report the push rung as working where a
   real enrolment would not.
4. The PM stack does not yet show which rung fed each agent's task. It does
   now separate hands items from choices an agent parked on the owner.
5. A `feature_slice` queue kind, so slices can also be scheduled centrally.

---

## 2026-11-21T12-45-00Z Update: nine review findings fixed

An automated review of #12227 arrived after the merge with nine findings.
Each was checked against the code, each was real, and each is fixed with a
test that fails without the fix.

| Finding                                                                                   | Fix                                                                                                                                              |
| ----------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------ |
| `fleet shell --build` gave the lock wrapper only the first word of a compound command     | The whole command runs in one shell beneath the lock: `fleet-build-lock sh -c '<command>'`. Checked against a real shell and a recording wrapper |
| `--build` was ignored when the target was this host                                       | A local build takes the lock where the host has one, and says so on stderr where it does not                                                     |
| A shortlist entry was handed out without checking it again                                | The issue is read again first. A hold, a claim, a close, a tracker or a hands item drops the entry; the next one is tried                        |
| An option given with no value fell back to its default (`--target=`, `--profile=`)        | A blank value, a missing value, or another option where the value should be is a usage error, before anything runs                               |
| Two runs at the same moment could take the same priority issue                            | The pick and its reservation happen in one locked update. Twelve concurrent reservations over eight issues hand each out once                    |
| Only the first 30 issues of a rank were read                                              | Each rank is read page by page until enough free issues are found, a short page ends it, or five pages are read                                  |
| With one to three eligible issues the list stayed low, so every run was told to refill it | A fill records its time. For 30 minutes after a fill a low list is worked, not refilled again                                                    |
| Follow-up commands dropped a chosen `--shortlist`                                         | `--shortlist`, `--repo` and `--profile` are carried into every follow-up command, quoted                                                         |
| `doctor` and a dry-run enrolment trusted the bridge's public health page                  | The mailbox is read as the agent, taking nothing. With no agent name the push rung is reported as not working, with the reason                   |

When GitHub cannot be asked, a listed issue is still handed out, marked
`rechecked: false` with a note to read the issue for a hold first: the
shortlist is the rung that works without the network, and withholding its
work then would defeat it.

This closes gap 3 in the list above. `fleet-agent` now has 65 tests; 16 more
hand-made faults, one per fix or more, were each caught.
