# Tutorial: enrol a new seat

This tutorial takes a fresh seat — a new agent instance on a new sandbox or
machine — from nothing to taking work. It assumes the `fleet` binary is
already installed on the seat. If it is not, install it first by following
[Installation and bootstrap](../getting-started/installation.md), then come
back here.

By the end, the seat will have its three broker token files, a registered
bridge mailbox, a saved agent profile, and a checked answer from every leg
of `fleet agent doctor`. Names, kinds and groups in the examples are
illustrative: pick your own agent name, and use a kind from Step 4.

Secrets in this tutorial are handled **by name only**. You never type,
print or paste a secret value; the bootstrap fetches token files for you,
and every later command resolves the values it needs by name.

## Step 1: Run the seat bootstrap

On a Muse sandbox seat, the bootstrap script rebuilds the whole seat —
tools, tokens, the tailnet link, the binary, and the local init stack —
from the pinned release. It is idempotent: re-running it skips work that
is already done.

```bash
bash scripts/muse/bootstrap.sh
```

What the output means: the script runs named phases in order —
`tools token restore tailnet fleet persist init check`. Each phase either
reports that its work was already present or does it and moves on. The
final `check` phase verifies the binary (including that the `bridge`
domain is available), the laptop link, secret resolution by name, and a
bridge hello/inbox probe as your agent.

What can go wrong, and the fix:

- A phase fails and the script stops, naming the phase. Exit `1` means "a
  phase failed"; exit `2` means the command line itself was wrong. Fix the
  named phase's cause (usually a missing download or a tailnet join that
  needs attention) and re-run the whole script — completed phases skip
  themselves.
- The check phase reports a missing token name. That is not fatal to
  enrolment itself, but the affected legs will fail in Step 5. Note which
  name is missing and continue to Step 2 to check the token files
  directly.

Seats that are not Muse sandboxes do not run this script. If your seat
was provisioned another way, continue at Step 2 and make sure the token
files and the binary are in place by your provisioning route instead.

## Step 2: Confirm the three token files appeared

The bootstrap's token phase fetches three Bitwarden Secrets Manager
secrets — by the names `fleet.secrets-broker-token`,
`fleet.queue-broker-token` and `fleet.laptop-agent-token` — into three
files under `~/tools/`. These are client-side files the seat's commands
name when they talk to the brokers; the brokers read the files per
request and never store or log their contents.

```bash
ls -l ~/tools/secrets-broker-token ~/tools/queue-broker-token ~/tools/laptop-agent-token
```

What the output means: three regular files, each with owner-only
permissions (`-rw-------`, mode `0600`). Their contents are secret values
— do not open, print or copy them anywhere. Only their presence, names
and permissions matter here.

What can go wrong, and the fix:

- A file is missing. The seat can still enrol (the bridge leg works over
  HTTPS with a by-name secret), but the queue, secrets and shell legs
  will fail with synthesised transport errors, because those legs read
  these shared files. Re-run the bootstrap's token provisioning; if the
  missing Bitwarden name itself does not resolve, that is a provisioning
  problem to raise with whoever stood the seat up — do not substitute
  another secret's value, and do not reuse the older, different name
  `secrets-broker.token`, which is a different value.
- Permissions are wider than `0600`. Tighten them:

```bash
chmod 600 ~/tools/secrets-broker-token ~/tools/queue-broker-token ~/tools/laptop-agent-token
```

Do not confuse these client-side `~/tools/*-token` files with the
daemon-side token files under `~/.config/secrets-broker/token` and
`~/.config/queue-broker/token`. The daemon-side files belong to the
brokers themselves (see [Tutorial: run the brokers](run-the-brokers.md));
a working seat needs its own `~/tools/` set.

## Step 3: Say hello on the bridge

Registering on the bridge is a single command. The bridge's agent
registry is exactly the set of agents that have said hello: name, groups
and last-seen time all come from this call.

```bash
fleet bridge hello --agent <your-agent-name> --groups muse
```

Replace `<your-agent-name>` with the name you will enrol under in Step 4,
and the groups with the groups your kind carries (Step 4 shows where
groups come from; `--groups` here takes a comma-separated list). The
command prints the bridge's JSON answer.

What the output means: your mailbox now exists on the bridge at
`https://bridge.security.cab`, and central can address you as
`agent:<your-agent-name>` or through any group you named. Re-running
`hello` refreshes the registration and its groups; it is safe to repeat.

What can go wrong, and the fix:

- Exit `3` — transport or credentials. The fleet token (secret name
  `message-bridge.fleet-token`) could not be resolved by name, or the
  bridge could not be reached. Check that secret resolution works at all
  by following [Tutorial: resolve a secret](resolve-a-secret.md), then
  retry.
- Exit `4` — the bridge refused the request; its JSON error is printed on
  stderr. A `404` here is reported as a wrong token, an old bridge
  without fleet mailboxes, or a wrong URL — check the bridge URL
  resolution (`$FLEET_BRIDGE_URL`, then `[bridge] url` in the fleet
  config, then the default above).
- Exit `2` — a usage error; fix the flags and retry. Note that `fleet
bridge` has one shared usage text for the whole domain, so read the
  `hello` line in it carefully.

If you omit `--agent`, the name defaults to `$FLEET_AGENT`, then
`[bridge] agent` in the fleet config; groups default the same way via
`$FLEET_AGENT_GROUPS` and `[bridge] groups`. For a first enrolment, pass
both explicitly so the registration is the one you intend.

## Step 4: Enrol the agent

Enrolment validates the kind, adds any extra groups, probes the bridge in
register mode (the `hello` from Step 3 is what actually registers the
mailbox), probes the remaining legs, and saves a profile recording the
agent, kind, groups and leg state.

```bash
fleet agent enroll --kind muse --name <your-agent-name> --groups muse
```

`--kind` and `--name` are required. `--groups a,b` adds to the kind's
built-in groups. If you do not know which kind fits, list the table
first:

```bash
fleet agent kinds
```

That prints one JSON object describing every kind: its name, summary,
bridge groups, seat type (`binary`, `http` or `connector`) and enrolment
note. Sixteen kinds are registered, including `muse`, `opencode`,
`openchamber`, `openclaw`, `claude-code`, `codex`, `gemini`,
`antigravity`, `grok` and the fallback kind `generic` for any other agent
with a shell. Adding a kind is adding a row to that table; no verb
changes.

What the output means: one JSON object carrying the saved profile,
`registered` (true only when this was not a dry run **and** the bridge
leg answered), the four legs, the intake rungs this seat still has, and
seat-specific next-step lines — for a binary seat, the run loop
(`fleet agent next --agent <name>`, do the task, run its `then` lines,
repeat) and the `fleet shell` line. Exit `0` means a dry run, or that at
least one intake rung is live; exit `3` means no intake rung is live.

What can go wrong, and the fix:

- Unknown kind: a usage error (exit `2`) listing all kinds. Pick one from
  the list, or use `generic` for anything with a shell.
- `registered` is false even though enrolment exited `0`: the bridge leg
  did not answer, so the mailbox was not registered by this call. Re-run
  Step 3, fix what it reports, then enrol again.
- You only want to preview: add `--dry-run`. A dry run probes nothing and
  registers nothing, and always exits `0`.
- To save the profile somewhere other than the default path, pass
  `--profile PATH`.

## Step 5: Read all four legs with doctor

Doctor probes four legs **in parallel**, each with a real fleet verb run
as this seat. The four legs are `bridge`, `queue`, `github` and `shell`.
There is no separate secrets leg: secret resolution is folded into the
bridge leg's stated need.

```bash
fleet agent doctor --agent <your-agent-name>
```

Read the answer leg by leg:

| Leg      | Probe                                                                      | "Up" means                                             | It needs                                                                           |
| -------- | -------------------------------------------------------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| `bridge` | `bridge inbox --agent <name> --limit 1`                                    | "the mailbox answers"                                  | HTTPS to the bridge, and the fleet token by name through fleet secrets             |
| `queue`  | `queue health`                                                             | "answers"                                              | The queue broker: its socket on the laptop, or the laptop transport from elsewhere |
| `github` | `gh api rate_limit` via `fleet gh`, falling back to a signed-in plain `gh` | "answers", with `route: "fleet"` or `"plain"` recorded | A `gh` token: the fleet rotation, or a signed-in `gh`                              |
| `shell`  | `laptop-agent health`                                                      | "the laptop answers"                                   | A route to the laptop: its broker, the Cloudflare tunnel, the tailnet, or ssh      |

Two details matter when reading the result:

- Doctor's bridge probe with a known agent is a read that takes nothing
  (`inbox --limit 1`). Service health alone proves nothing about whether
  this seat holds the fleet token, so only a call made _as_ an agent
  counts. With no agent name known, the bridge leg is reported **not ok**
  even when the service answers — enrol, or pass `--agent`.
- From the legs, doctor derives the **rungs** map: `push` follows the
  bridge leg, `queue` follows the queue leg, `priority` follows the
  github leg, and `shortlist` is always true because it is a local file.
  The answer also reports `builds`: whether `fleet shell` reaches the
  laptop from here, or will try a codespace and then run locally.

Exit `0` means at least one intake rung (bridge, queue or github) is
live; exit `3` means none is.

What can go wrong, and the fix:

- `queue` down while `bridge` is up: the classic missing-token-file
  shape. Go back to Step 2 — a seat whose bootstrap skipped the token
  phase enrols fine and then fails exactly here.
- `github` down: neither the fleet rotation nor a signed-in `gh`
  answered. Sign `gh` in on the seat, or check secret resolution by name
  as in [Tutorial: resolve a secret](resolve-a-secret.md).
- `shell` down but the other legs up: the seat can still take work;
  builds will fall back as the `builds` line describes. Fix the laptop
  route when you need laptop builds.

## Step 6: Take the first piece of work

`fleet agent next` walks the intake ladder in order — a pushed bridge
command, a queue claim, the local shortlist, then the highest-priority
open issue from GitHub — and returns the first rung that yields work. A
rung that is _down_ is recorded and skipped; a rung that is _empty_ is
different, and the answer's `tried` log says which was which.

```bash
fleet agent next --agent <your-agent-name>
```

What the output means: one JSON object, `{agent, rung, task, then,
tried}`. Do the task, then run the commands in `then` — for a queue task
those lines name the lease deadline and the exact `fleet queue
checkpoint` / `fleet queue complete` commands for the claim. The queue
rung claims with a four-hour lease, overriding the queue's 45-minute
default, which would lapse mid-task. The full claim-and-complete flow is
[Tutorial: claim and complete a queue task](claim-and-complete.md).

What can go wrong, and the fix:

- Exit `5`: no rung had work. This is a normal, healthy answer for an
  idle fleet — the seat is enrolled and reachable, there is simply
  nothing to hand out right now. Read `tried` to confirm each rung was
  `empty` or `skipped` rather than `down`; a `down` rung points back at
  the matching doctor leg in Step 5.
- Exit `3`: a needed leg is down before the ladder could run. Fix it via
  Step 5.
- Exit `2`: a usage error in the flags.

## You are done when

All of the following hold:

- `ls -l ~/tools/*-token` shows the three token files, each mode `0600`.
- `fleet bridge hello --agent <your-agent-name>` exits `0`.
- `fleet agent enroll --kind <kind> --name <your-agent-name>` printed a
  profile with `registered: true`.
- `fleet agent doctor --agent <your-agent-name>` exits `0`, and you have
  read all four legs — `bridge`, `queue`, `github`, `shell` — and know
  which rungs they give you.
- `fleet agent next --agent <your-agent-name>` either handed you a task
  with its `then` lines, or exited `5` with every rung `empty` or
  `skipped` in `tried` — never `down` for a reason you have not fixed.
