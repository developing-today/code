# Agents: enrolment, doctor and the intake ladder

The fleet already had the legs — bridge mailboxes, the queue, the laptop
transport, GitHub, secrets by name. What a new kind of agent lacked was one
place to enrol, check its seat, and take work. `fleet agent` is that place.
Its verbs are `kinds`, `enroll`, `doctor`, `next`, and `shortlist`; every
verb prints one JSON object. `fleet shell` is its sibling domain, riding on
the same `agent` capability.

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-agent/src/lib.rs

As with the bridge, every per-verb `--help` prints the same shared usage
text; there is no per-verb help in source. This chapter is the per-verb
reference.

## Kinds and seat shapes: 16 × 3

A *kind* says three things: which bridge groups the agent joins, how it
reaches the fleet from where it runs, and what to tell it at enrolment.
`fleet agent kinds` prints the table as JSON — for each kind its name,
summary, bridge groups, seat, and enrolment note, plus the three seat
definitions. Adding a kind is adding a row; no verb changes.

A *seat shape* is how the agent runs:

- **binary** — has a shell and runs the fleet binary itself: `enroll`, then
  `next`.
- **http** — no shell of its own; speaks the bridge's fleet routes over
  HTTPS (the reference client is `scripts/fleet/bridge-mailbox.py`) and gets
  work from its mailbox.
- **connector** — a chat assistant with the Message Bridge connector
  attached; it has a channel address, not a mailbox.

The 16 kinds:

| Kind | Groups | Seat | Note |
|---|---|---|---|
| `muse` | muse | binary | A fresh Muse gets a bridge-capable static build from `scripts/muse/bootstrap.sh`; older installs run the same commands on the laptop install via `~/tools/laptop-ssh.sh`. |
| `opencode` | opencode | binary | Runs on the laptop seat, so every leg is local. |
| `openchamber` | opencode, openchamber | binary | Joins `opencode` too, so `group:opencode` reaches both pipelines. |
| `openclaw` | openclaw | binary | |
| `claude-code` | claude-code | binary | A cloud session has no tailnet: messaging and the priority rung work over HTTPS; builds go through `fleet shell`. |
| `codex` | codex | binary | |
| `gemini` | gemini | binary | |
| `antigravity` | antigravity | binary | |
| `grok` | grok | binary | A bot with no shell enrols as kind `http` under the same name and groups. |
| `dot` | dot | http | Its Slack and ChatGPT transports are carried elsewhere; the mailbox is the same store. |
| `duet` | duet | http | |
| `claude` | (none) | connector | Fixed bridge address `claude`; set a wake hook for channel:claude so a send starts a session. |
| `chatgpt` | (none) | connector | Fixed address `chatgpt`; woken by its MCP Events subscription. |
| `laptop` | laptop | binary | A job or service on the fleet laptop. |
| `http` | (none) | http | Anything that can make an HTTPS request and nothing else. |
| `generic` | (none) | binary | Any other agent with a shell — the fallback the enrolment error names. |

## Enrolment and the profile

```
fleet agent enroll --kind K --name N [--groups a,b] [--dry-run] [--profile PATH]
```

The kind is validated (an unknown kind is a usage error listing all kinds —
"use generic for anything with a shell"). `--groups` adds to the kind's
built-in groups. For a binary or http seat, the bridge leg is probed in
*register* mode — a bridge `hello` with the name and groups, which is what
actually registers the mailbox. Connector kinds instead get the "attach the
connector" instruction and a fixed channel address. The four legs are
probed (below) and the saved *profile* — at the default path, or `--profile`
— records the agent, kind, groups, and leg state. `--dry-run` probes nothing
and registers nothing.

The JSON answer carries the profile, `registered` (true only when it is not
a dry run *and* the bridge leg answered), the legs, the *rungs* map (below),
and seat-specific next-step lines: for a binary seat, the run loop
(`fleet agent next --agent <name>`, do the task, run its `then` lines,
repeat while the run has budget) and the `fleet shell` line ("runs where
builds belong; never build in a limited sandbox"); for an http seat, the
token-by-name note and the reference-client commands; for central, the
`fleet bridge send` lines. Exit 0 for a dry run, or when any intake rung is
live; otherwise 3.

## Doctor: the four legs

```
fleet agent doctor [--agent N]
```

**The four legs are `bridge`, `queue`, `github`, and `shell`.** There is no
secrets leg: secrets is folded into the bridge leg's stated need ("HTTPS to
the message bridge, and the fleet token by name through fleet secrets").
Any text describing the legs as bridge/queue/secrets/shell is wrong — the
source is explicit, and this book follows the source.

The four probes are independent, issued together (one parallel call), and
each is a real fleet verb run as this seat:

| Leg | Probe | Needs |
|---|---|---|
| bridge | `bridge hello` (enrol) / `bridge inbox --agent A --limit 1` (doctor with a name) / `bridge health` (no name known) | HTTPS to the bridge + the fleet token by name through fleet secrets |
| queue | `queue health` | The queue broker: its socket on the laptop, or the laptop transport from elsewhere |
| github | `gh api rate_limit` via `fleet gh`; if that fails, plain signed-in `gh api rate_limit` (the answer records `route: "fleet"` or `"plain"`) | `gh` with a token: fleet gh's rotation, or a signed-in gh |
| shell | `laptop-agent health` | A route to the laptop: its broker, the Cloudflare tunnel, the tailnet, or SSH |

Doctor's bridge probe with a known agent is a read that takes nothing
(`inbox --limit 1`): service health alone proves nothing about whether this
seat holds the fleet token, so only a call made *as* an agent counts. With
no agent name, the bridge leg is reported **not ok** even when the service
answers ("the service answers, but no agent name is known to read a mailbox
with: enrol, or pass --agent").

From the legs, doctor derives the **rungs** map — which intake rungs this
seat still has: `push` = the bridge leg, `queue` = the queue leg,
`shortlist` = **always true** (it is a local file), `priority` = the github
leg. It also reports `builds`: "fleet shell reaches the laptop" when the
shell leg answers, else "fleet shell has no laptop route from here; it will
try a codespace, then run here". Exit 0 when any intake rung is live
(bridge, queue, or github), else 3.

## `next`: the intake ladder

```
fleet agent next [--agent N] [--lane L] [--repo OWNER/NAME] [--low-water N]
     [--shortlist PATH] [--platform P] [--pool P]
     [--remaining-budget-minutes N] [--filter EXPR] [--queue Q]
     [--no-push] [--no-queue] [--no-shortlist] [--no-priority]
```

`next` tries four rungs **in order** and returns the first that yields work.
A rung that is *down* (bridge unreachable, broker not answering) is not the
same as a rung that is *empty*: down rungs are recorded and skipped, so a
seat with only HTTPS still gets work from rungs 1 and 4. Every answer is
`{agent, rung, task, then, tried}` — `tried` is the per-rung log
(`skipped`/`empty`/`down`/`rejected`, with a reason), and `then` is the list
of follow-up commands the runner should execute for the handed-out task.
The default repo is `developing-today/code`. The `--no-*` flags skip
individual rungs (recorded as `skipped`); `--low-water N` overrides the
shortlist low-water mark for this call; `--lane`, `--queue`, `--platform`,
`--pool`, `--remaining-budget-minutes`, and `--filter` are carried into the
queue claim.

1. **Push** — a command addressed to this agent on the bridge
   (`bridge inbox --commands --take`). This rung outranks everything: a
   command a person (or central) pushed is work already chosen.
2. **Queue** — a claim on the one fleet queue (`queue claim` with the
   agent's lane; the lane defaults from the agent name, known lane families
   are validated by the queue, and a lane registered to a shared queue
   worker is honoured). `next` claims with a **4-hour lease** (14,400 s,
   overriding the queue's 45-minute default, which lapses mid-task), and
   the task's `then` lines name the lease deadline plus the
   `queue checkpoint` / `queue complete` commands with the claimed
   worker/lane. A broker refusal that is really a lane verdict (lane bound
   to another queue, registration did not land) is reported as rung
   `rejected`, distinct from `down`.
3. **Shortlist** — the top of this host's local shortlist. Entries are
   re-checked against GitHub before hand-out (an issue closed, assigned, or
   owner-parked since listing is dropped with a named reason), then handed
   out in rank order. **Refill switch:** if the list is at or below
   low-water with nobody refilling, `next` does not hand out a stale
   remnant — it returns rung **`refill`** instead, a purpose-switch task
   (`purpose: "refill-shortlist"`, target the shortlist default target)
   whose instructions and `then` lines are the `shortlist fill` /
   `shortlist add` commands. One run refills; the marker it leaves keeps
   the rest working.
4. **Priority** — the highest-priority open issue straight from GitHub.
   Label queries run soonest-first over the priority label family
   (`priority/critical`, then `priority/high`, … — the same ordering as the
   shortlist ranks below); candidates are filtered (owner-parked labels —
   the `user/owned`, `user/review`, `user/escalated` class — are kept out),
   and the winner is **reserved into the shortlist file** as it is handed
   out, so two seats on one host do not take the same issue.

## The shortlist: ranks, low-water, refill

The shortlist is a local JSON file on this host (default path from the
profile/state directory; `--shortlist PATH` overrides), guarded by a lock
file (stale after 30 s; 100 tries at 50 ms). Entries are
`{repo, issue, title, rank, why, added_at}`, plus handed-out records and an
optional refill marker. The queue verbs neither read nor write it — the
shortlist is the agent domain's concern, and any interplay is at the caller
level.

**Rank** (lower is sooner): `expedite` → 0, `priority/critical` → 1,
`now` → 2, `priority/high` → 3, `next` → 4, `priority/normal` → 5, anything
else unranked = 6. Hand-out order is lowest rank, then oldest `added_at`,
then lowest issue number.

**Defaults: low-water 3, target 8.** A list is *low* when
`entries ≤ low_water` — so a list of exactly 3 is already low.
`shortlist status [--low-water N]` reports the list, its count, `low`, and
the refill state. `shortlist add --issue N --title T [--repo R] [--rank N]
[--why TEXT]` adds one entry (rank defaults to unranked 6; duplicates —
same repo and issue already listed — are refused). `shortlist take` hands
out the soonest entry; `shortlist clear` empties the list and clears the
refill marker. `shortlist fill [--repo R] [--target N]` tops the list up to
the target (default 8, clamped 1–50) from the priority candidates: it
fetches **2× target** candidates from GitHub, adds them in rank order until
the target is met, records the fill, and clears the refill marker.

**Refill bookkeeping.** A refill marker `(by, at)` lasts **1800 s
(30 minutes)**; a list needs refill when it is low, no marker is live, and
no fill finished within the window. A handed-out issue is kept from being
handed out again on this host for **7200 s (2 hours)**; pruning drops
expired handed records and expired markers.

## Exit codes

The agent domain's vocabulary: **0** ok; **2** usage; **3** a needed leg is
down (doctor/enrol with no intake rung live); **5** — `next` only — no work
on any rung.
