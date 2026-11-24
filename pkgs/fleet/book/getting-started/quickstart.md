# Quickstart

The five-minute path. It assumes fleet is installed (see
[Installation and bootstrap](installation.md)) and runs five commands, in an
order that checks each layer before the next one depends on it: the binary,
the seat, a secret, the queue, and finally taking work. Every command here is
read-only except the last, which claims a task only when work is available.

## 1. `fleet version` and `fleet capabilities` — the binary

```sh
fleet version
fleet capabilities
```

`fleet version` prints five lines: version, build name, derived version,
source revision, and enabled capabilities. What you are checking: the build
name is `fleet` (the full build) and the enabled-capabilities line is not
`none`. The short form `fleet --version` prints only the first two lines.

`fleet capabilities` prints the registry as JSON. What you are checking: the
`capabilities` and `compiled_capabilities` lists match (both are the full 15
on an unnarrowed full build), and every entry in `domains` has
`"available": true`. If a domain shows `"state": "narrowed-by-name"`, you
invoked the binary under a variant name such as `fleet-connect`; invoke it as
`fleet`. If it shows `"disabled-by-config"`, the host's `fleet.toml` disables
that domain under `[domains]`. If it shows `"not-in-build"`, the binary is a
narrowed build. Invoking an unavailable domain prints a stub and exits 69;
see [The shape of the binary](shape.md).

## 2. `fleet agent doctor` — the seat

```sh
fleet agent doctor
```

Doctor probes the seat's four _legs_ — a leg is one route the seat needs —
in parallel, each by running a real fleet verb as this seat, and prints one
JSON object:

| Leg      | Probe                                                                           | What "up" needs                                                                         |
| -------- | ------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------- |
| `bridge` | `bridge inbox --agent <name> --limit 1` (with a known agent name)               | HTTPS to the message bridge, and the fleet token resolved by name through fleet secrets |
| `queue`  | `queue health`                                                                  | The queue broker: its socket on the laptop, or the laptop transport from elsewhere      |
| `github` | `gh api rate_limit` through `fleet gh` (falling back to a plain signed-in `gh`) | A working `gh` token, by fleet rotation or by sign-in                                   |
| `shell`  | `laptop-agent health`                                                           | A route to the laptop: its broker, the Cloudflare tunnel, the tailnet, or SSH           |

The bridge probe is deliberately a read made _as_ an agent: service health
alone proves nothing about whether this seat holds the fleet token, so with
no agent name known the bridge leg is reported not ok even when the service
answers.

From the legs, doctor derives the _rungs_ map — which rungs of the intake
ladder (see step 5) this seat still has: `push` follows the bridge leg,
`queue` follows the queue leg, `shortlist` is always true (it is a local
file), and `priority` follows the github leg. It also reports `builds`:
whether `fleet shell` reaches the laptop from here.

Exit codes: 0 when any intake rung is live (bridge, queue, or github), 3
when none is, 2 for a usage error. A seat with only HTTPS still has rungs 1
and 4 of the ladder; doctor tells you exactly which world you are in. The
full treatment is in
[Agents: enrolment, doctor and the intake ladder](../concepts/agents.md).

## 3. `fleet secrets get <name>` — a secret, by name

```sh
fleet secrets get tailscale.auth-key
```

(Any secret name your seat is entitled to works; `tailscale.auth-key` is the
name the bootstrap check phase uses.)

This resolves the secret _by name_: the secrets broker first, Bitwarden
(`bws`) as fallback. The value goes to stdout and nowhere else — not to a
log, not to an error message. Resolution order and cache semantics are in
[Secrets and the secrets broker](../concepts/secrets.md); the short version
is that a broker cache hit answers immediately and carries the value's age,
and a miss on a loaded cache reads through to Bitwarden so a freshly minted
secret is visible at once.

Exit codes: 0 ok, 2 usage (for example, no name, or more than one name), 3
unresolved / broker unavailable and the fallback also failed. The companion
verb `fleet secrets list` prints the names in the broker's cache, one per
line, never values; unlike `get`, `list` has **no** `bws` fallback — a broker
failure is exit 3, because a vault listing is not the same thing as a cache
listing.

## 4. `fleet queue health` — the queue

```sh
fleet queue health
```

This asks the queue broker for its health table. The first output line is
`project: <netloc> (resolved via <source>)`; the table that follows carries
per-queue counts by task status, or `(no queues have tasks)` when empty.
Exit 0 means the broker answered. Exit 3 means the transport path to the
broker failed. Exit 6 is special: it is the _fallback switch_ — the primary
queue store failed and this work has been switched to the GitHub fallback
queue; the output is one JSON object saying so, with a hint naming the
`fleet queue fallback …` verbs to continue with. See
[The queue: tasks, lanes and leases](../concepts/queue.md).

Note that `queue health` (this verb, an authenticated broker call) is a
different thing from the broker's unauthenticated `GET /healthz` probe.

## 5. `fleet agent next` — take work

```sh
fleet agent next
```

`next` walks the _intake ladder_ — four rungs, in order — and returns the
first piece of work it finds, as one JSON object
`{agent, rung, task, then, tried}`:

1. **Push** — a command addressed to this agent on the bridge. Work a person
   or central already chose outranks everything.
2. **Queue** — a claim on the fleet queue, taken with a 4-hour lease
   (14,400 s — the queue's own default lease is 45 minutes, which lapses
   mid-task). The `then` lines in the answer name the lease deadline and the
   `queue checkpoint` / `queue complete` commands for the claimed task.
3. **Shortlist** — the top of this host's local shortlist, re-checked against
   GitHub before hand-out. If the list is at or below its low-water mark
   (default 3 entries) with nobody refilling it, `next` returns a `refill`
   task instead of handing out a stale remnant.
4. **Priority** — the highest-priority open issue straight from GitHub,
   reserved into the shortlist file as it is handed out so two seats on one
   host do not take the same issue.

`tried` is the per-rung log: each rung is recorded as `skipped`, `empty`,
`down`, or `rejected`, with a reason. A rung that is _down_ is skipped, not
fatal — that is what doctor's rung map predicted. Exit codes: 0 when work
was handed out, 5 when no rung had work, 3 when a needed leg is down in a
way that leaves nothing to try, 2 for a usage error.

That is the whole loop. An agent's working cycle is: `next`, do the task,
run the `then` lines (checkpoint as you go, complete at the end), and call
`next` again while the run has budget.
