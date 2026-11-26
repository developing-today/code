# fleet agent

`fleet agent` is the agent's front door: enrol an agent of any kind, check what this seat reaches, and take the next piece of work down the intake ladder. The fleet already has the legs — bridge mailboxes, the queue, the laptop transport, GitHub, secrets by name; what a new kind of agent lacked was one place to enroll, check its seat, and take work. `fleet agent` is that place. Its sibling domain [fleet shell](shell.md) rides on the same `agent` capability — there is no separate `shell` capability atom.

Every verb prints **one JSON object**. Exit codes for the domain: `0` ok · `2` usage · `3` a needed leg is down · `5` no work on any rung (`next` only).

See also [Agents: enrolment, doctor and the intake ladder](../concepts/agents.md) and [fleet bridge](bridge.md).

Help: `fleet agent <verb> --help` prints that verb's own usage — its synopsis, a short description, and the shared output/exit-code footer. An unknown verb (or no verb) falls back to the shared domain USAGE. The per-verb sections below are written from source semantics.

## fleet agent kinds

```
fleet agent kinds
```

| Flag | Argument | Default | Meaning                          |
| ---- | -------- | ------- | -------------------------------- |
| —    | —        | —       | Takes no arguments and no flags. |

### Behaviour

Prints the kind table as JSON: for each kind its name, summary, bridge groups, seat, and enrolment note, plus the three seat definitions. A kind says three things: which bridge groups it joins, how it reaches the fleet from where it runs, and what to tell it at enrolment. Adding a kind is adding a row; no verb changes. The 16 kinds:

| Kind          | Groups                | Seat      | Note (abridged)                                                                                                                                                        |
| ------------- | --------------------- | --------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `muse`        | muse                  | binary    | Fresh Muse gets a bridge-capable static build from `scripts/muse/bootstrap.sh`; older installs run the same commands on the laptop install via `~/tools/laptop-ssh.sh` |
| `opencode`    | opencode              | binary    | Runs on the laptop seat, so every leg is local                                                                                                                         |
| `openchamber` | opencode, openchamber | binary    | Joins `opencode` too, so `group:opencode` reaches both pipelines                                                                                                       |
| `openclaw`    | openclaw              | binary    | —                                                                                                                                                                      |
| `claude-code` | claude-code           | binary    | A cloud session has no tailnet: messaging and the priority rung work over HTTPS; builds go through `fleet shell`                                                       |
| `codex`       | codex                 | binary    | —                                                                                                                                                                      |
| `gemini`      | gemini                | binary    | —                                                                                                                                                                      |
| `antigravity` | antigravity           | binary    | —                                                                                                                                                                      |
| `grok`        | grok                  | binary    | A bot with no shell enrolls as kind `http` under the same name and groups                                                                                              |
| `dot`         | dot                   | http      | `pkgs/dot-bridge` carries its Slack and ChatGPT transports; the mailbox is the same store                                                                              |
| `duet`        | duet                  | http      | —                                                                                                                                                                      |
| `claude`      | (none)                | connector | Fixed bridge address `claude`; set a wake hook for channel:claude so a send starts a session                                                                           |
| `chatgpt`     | (none)                | connector | Fixed address `chatgpt`; woken by its MCP Events subscription                                                                                                          |
| `laptop`      | laptop                | binary    | A job or service on the fleet laptop                                                                                                                                   |
| `http`        | (none)                | http      | Anything that can make an HTTPS request and nothing else                                                                                                               |
| `generic`     | (none)                | binary    | Any other agent with a shell — the fallback `enroll` names in its unknown-kind error                                                                                   |

Seats: **binary** = has a shell and runs the fleet binary itself (`enroll`, then `next`); **http** = no shell of its own, speaks the bridge's fleet routes over HTTPS (`scripts/fleet/bridge-mailbox.py` is the reference client) and gets work from its mailbox; **connector** = a chat assistant with the Message Bridge connector attached — it has a channel address, not a mailbox.

### Exit codes

`0` ok · `2` usage.

### Example

```console
$ fleet agent kinds
```

Prints all 16 kinds with their groups, seats, and enrolment notes as one JSON object.

## fleet agent enroll

```
fleet agent enroll --kind K --name N [--groups a,b] [--dry-run] [--profile PATH]
fleet agent enroll --kind K --name N --bootstrap TOKEN
                     [--model-provider P] [--model-id M] [--model-variant V]
```

| Flag               | Argument | Default                | Meaning                                                                                                                                                                              |
| ------------------ | -------- | ---------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| `--kind`           | `K`      | required               | One of the 16 kinds (see `kinds`). An unknown kind is a usage error listing all kinds, with "use generic for anything with a shell".                                                 |
| `--name`           | `N`      | required               | The agent's name (its bridge mailbox name). The shared USAGE spells this `--name`; the domain help line abbreviates it as the enroll line's second value.                            |
| `--groups`         | `a,b`    | kind's built-in groups | Additional bridge groups; adds to the kind's groups, never replaces them.                                                                                                            |
| `--dry-run`        | —        | off                    | Probe nothing and register nothing.                                                                                                                                                  |
| `--profile`        | `PATH`   | default profile path   | Where to save the profile.                                                                                                                                                           |
| `--bootstrap`      | `TOKEN`  | —                      | Provision from the owner's single-use bootstrap token instead of the shared fleet token (binary seats only; never with `--dry-run` — the exchange burns the token).                  |
| `--model-provider` | `P`      | —                      | The lane's serving-model provider, recorded in the profile as its model stamp (#4919). Requires `--model-id`; re-enrolling without the model flags keeps a recorded stamp.           |
| `--model-id`       | `M`      | —                      | The lane's serving model (or the `muse-lane` marker for a lane that cannot name a leaf). Requires `--model-provider`.                                                                |
| `--model-variant`  | `V`      | —                      | The lane's effort variant. Requires `--model-provider` and `--model-id`. This is the effort producer: `next` stamps it on every queue claim, so completions carry a non-null effort. |

### Behaviour

Registers an agent and saves its profile. Flow: the kind is validated; `--groups` adds to the kind's built-in groups; for a binary/http seat the bridge leg is probed in **Register** mode — `bridge hello` with the name and groups, which is what actually registers the mailbox; connector kinds instead get the "attach the connector" instruction and a fixed channel address. The four legs are probed (see `doctor`) and the saved profile records agent, kind, groups, and leg state.

With `--bootstrap`, provisioning comes first: [`fleet bridge exchange`](bridge.md#fleet-bridge-exchange) trades the token — minted by the owner bound to this name, kind and groups — for the agent's own credentials, which land in the vault by name (`bridge.<agent>-fleet-token`, `bridge.<agent>-token`). The mint-bound groups replace the kind/`--groups` set (they are the truth a credentialed hello cannot change), the profile is saved early so the probe's hello resolves the new credential by its recorded name, and the profile records the credential **names** and the notes identity — never a value. For a `muse` seat whose mint carried a notes identity, enrolment also writes the wake hookup (the signal-poll detector under `~/hooks/scripts/` and the hook definition under `~/hooks/definitions/`, the hatch-personal `scripts/messages-hook/` pattern), so the agent is woken when a note lands. A refused exchange enrols nothing: no vault write, no profile, no hook.

The JSON answer carries: the profile, `registered` (true only when not a dry run _and_ the bridge leg answered), the legs, the **rungs** map, and seat-specific next-step lines — for a binary seat the run loop (`fleet agent next --agent <name>`, do the task, run its `then` lines, repeat while the run has budget) and the `fleet shell` line ("runs where builds belong; never build in a limited sandbox"); for an http seat the token-by-name note and the reference-client commands; for central, the `fleet bridge send --to <address> [--command]` lines.

### Exit codes

`0` — a dry run, or any intake rung is live. `2` — usage (including an unknown kind). `3` — no intake rung is live.

### Example

```console
$ fleet agent enroll --kind generic --name worker-1
```

Registers `worker-1`, probes its four legs, and saves its profile with the run-loop instructions.

## fleet agent doctor

```
fleet agent doctor [--agent N]
```

| Flag      | Argument | Default                  | Meaning                                                                                                                     |
| --------- | -------- | ------------------------ | --------------------------------------------------------------------------------------------------------------------------- |
| `--agent` | `N`      | profile / `$FLEET_AGENT` | Agent name to probe the bridge leg as. With no name known, the bridge leg is reported not ok even when the service answers. |

### Behaviour

Checks which legs answer from this seat. **The four legs are `bridge`, `queue`, `github`, `shell`** — there is no secrets leg; secrets is folded into the bridge leg's stated need ("HTTPS to the message bridge, and the fleet token by name through fleet secrets"). The four probes are independent and are issued **together** (one parallel call), each a real fleet verb run as this seat:

| Leg    | Probe                                                                                                               | "Up" detail                                            | What it needs                                                                      |
| ------ | ------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------ | ---------------------------------------------------------------------------------- |
| bridge | `bridge hello` (enroll) / `bridge inbox --agent A --limit 1` (doctor with a name) / `bridge health` (no name known) | "registered" / "the mailbox answers"                   | HTTPS to the bridge + the fleet token by name through fleet secrets                |
| queue  | `queue health`                                                                                                      | "answers"                                              | The queue broker: its socket on the laptop, or the laptop transport from elsewhere |
| github | `gh api rate_limit` via `fleet gh`; if that fails, plain signed-in `gh api rate_limit`                              | "answers", with `route: "fleet"` or `"plain"` recorded | `gh` with a token: fleet gh's rotation, or a signed-in gh                          |
| shell  | `laptop-agent health`                                                                                               | "the laptop answers"                                   | A route to the laptop: its broker, the Cloudflare tunnel, the tailnet or ssh       |

Doctor's bridge probe with a known agent is a **read that takes nothing** (`inbox --limit 1`): service health alone proves nothing about whether this seat holds the fleet token, so only a call made _as_ an agent counts. With no agent name, the bridge leg is reported **not ok** even when the service answers ("the service answers, but no agent name is known to read a mailbox with: enrol, or pass --agent").

From the legs, doctor derives the **rungs** map — which intake rungs this seat still has: `push` = bridge leg, `queue` = queue leg, `shortlist` = **always true** (it is a local file), `priority` = github leg. The answer also reports `builds`: "fleet shell reaches the laptop" when the shell leg answers, else "fleet shell has no laptop route from here; it will try a codespace, then run here".

### Exit codes

`0` — any intake rung is live (bridge, queue or github). `2` — usage. `3` — no intake rung is live.

### Example

```console
$ fleet agent doctor --agent archer
```

Probes all four legs in parallel and prints the legs, the rungs map, and the builds line as one JSON object.

## fleet agent next

```
fleet agent next [--agent N] [--lane L] [--repo OWNER/NAME] [--low-water N] [--shortlist PATH] [--platform P] [--pool P] [--remaining-budget-minutes N] [--filter EXPR] [--queue Q] [--model-provider P] [--model-id M] [--model-variant V] [--no-push] [--no-queue] [--no-shortlist] [--no-priority]
```

| Flag                                                  | Argument        | Default                     | Meaning                                                                                                                                                          |
| ----------------------------------------------------- | --------------- | --------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `--agent`                                             | `N`             | profile / `$FLEET_AGENT`    | Agent taking the work.                                                                                                                                           |
| `--lane`                                              | `L`             | derived from the agent name | Queue lane for the queue rung's claim.                                                                                                                           |
| `--repo`                                              | `OWNER/NAME`    | `developing-today/code`     | Repo the priority/shortlist rungs draw from.                                                                                                                     |
| `--low-water`                                         | `N`             | `3`                         | Shortlist low-water mark for this call.                                                                                                                          |
| `--shortlist`                                         | `PATH`          | profile/state default       | Shortlist file path.                                                                                                                                             |
| `--platform`                                          | `P`             | —                           | Carried into the queue claim.                                                                                                                                    |
| `--pool`                                              | `P`             | —                           | Carried into the queue claim.                                                                                                                                    |
| `--remaining-budget-minutes`                          | `N`             | —                           | Carried into the queue claim.                                                                                                                                    |
| `--filter`                                            | `EXPR`          | —                           | Carried into the queue claim.                                                                                                                                    |
| `--queue`                                             | `Q`             | —                           | Queue name for the queue rung's claim.                                                                                                                           |
| `--model-provider` / `--model-id` / `--model-variant` | `P` / `M` / `V` | profile stamp               | The claim's model stamp (#4919). Explicit flags win as given; with none, the stamp recorded in the agent's profile at enrolment is carried into the claim whole. |
| `--no-push`                                           | —               | off                         | Skip rung 1 (recorded as `skipped` in `tried`).                                                                                                                  |
| `--no-queue`                                          | —               | off                         | Skip rung 2.                                                                                                                                                     |
| `--no-shortlist`                                      | —               | off                         | Skip rung 3.                                                                                                                                                     |
| `--no-priority`                                       | —               | off                         | Skip rung 4.                                                                                                                                                     |

### Behaviour

Tries four rungs **in order** and returns the first that yields work. A rung that is _down_ (bridge unreachable, broker not answering) is not the same as a rung that is _empty_: down rungs are recorded and skipped, so a seat with only HTTPS still gets work from rungs 1 and 4. Every answer is `{agent, rung, task, then, tried}` — `tried` is the per-rung log (`skipped`/`empty`/`down`/`rejected` with a why), and `then` is the list of follow-up commands the runner should execute for the handed-out task.

1. **Push** — a command addressed to this agent on the bridge (`bridge inbox --commands --take`). This rung outranks everything: a command a person (or central) pushed is work already chosen.
2. **Queue** — a claim on the one fleet queue (`queue claim` with the agent's lane; the lane defaults from the agent name — known lane families are validated by the queue, and a lane registered to a shared queue worker is honoured). `next` claims with a **4-hour lease** (14400 s, overriding the queue's 45-minute default, which lapses mid-task) and the task's `then` lines name the lease deadline plus the `queue checkpoint` / `queue complete` commands with the claimed worker/lane. A broker refusal that is really a lane verdict (lane bound to another queue, registration did not land) is reported as rung `rejected`, distinct from `down`.
3. **Shortlist** — the top of this host's local shortlist: entries are re-checked against GitHub before hand-out (an issue closed, assigned or owner-parked since listing is dropped with a named why), then handed out in rank order. **Refill switch:** if the list is at or below low-water with nobody refilling, `next` does not hand out a stale remnant — it returns rung **`refill`** instead, a purpose-switch task (`purpose: "refill-shortlist"`, target 8) whose instructions and `then` lines are the `shortlist fill` / `shortlist add` commands. One run refills; the marker it leaves keeps the rest working.
4. **Priority** — the highest-priority open issue straight from GitHub. Label queries run soonest-first over the priority label family (`priority/critical`, then `priority/high`, … — the same ordering as the shortlist ranks); candidates are filtered (owner-parked labels — `user/owned`, `user/review`, `user/escalated` class — are kept out), and the winner is **reserved into the shortlist file** as it is handed out, so two seats on one host do not take the same issue.

### Exit codes

`0` — a rung yielded work. `2` — usage. `3` — a needed leg is down (domain-level). `5` — **no work on any rung**.

### Example

```console
$ fleet agent next --agent archer
{"agent": "archer", "rung": "queue", "task": {…}, "then": […], "tried": […]}
```

Returns the first available work down the ladder, with the follow-up commands in `then`.

## fleet agent shortlist status

```
fleet agent shortlist status [--low-water N]
```

| Flag          | Argument | Default               | Meaning                                      |
| ------------- | -------- | --------------------- | -------------------------------------------- |
| `--low-water` | `N`      | `3`                   | Low-water mark for the `low` computation.    |
| `--shortlist` | `PATH`   | profile/state default | Shortlist file path (shared shortlist flag). |

### Behaviour

Reports the local list `next` draws on: a JSON file on this host, guarded by a lock file (stale after 30 s). Entries are `{repo, issue, title, rank, why, added_at}` plus handed-out records and an optional refill marker. `status` prints the list, its count, `low` (count ≤ low-water — so a list of exactly 3 is already low), and the refill state. Defaults: low-water **3**, target **8**.

Ranks (lower is sooner): `expedite` → 0, `priority/critical` → 1, `now` → 2, `priority/high` → 3, `next` → 4, `priority/normal` → 5, anything else **unranked = 6**. Hand-out order is lowest rank, then oldest `added_at`, then lowest issue number.

### Exit codes

`0` ok · `2` usage · `3` a needed leg is down.

### Example

```console
$ fleet agent shortlist status
```

Prints the entries, the count, whether the list is low, and the refill state.

## fleet agent shortlist take

```
fleet agent shortlist take
```

| Flag          | Argument | Default               | Meaning                                                                              |
| ------------- | -------- | --------------------- | ------------------------------------------------------------------------------------ |
| `--shortlist` | `PATH`   | profile/state default | Shortlist file path (shared shortlist flag). This verb takes no verb-specific flags. |

### Behaviour

Hands out the soonest entry to this agent — the same selection `next` rung 3 uses (lowest rank, then oldest, then lowest issue number). A handed-out issue is kept from being handed out again on this host for **2 h** (7200 s); pruning drops expired handed records and expired refill markers.

### Exit codes

`0` ok · `2` usage · `3` a needed leg is down · `5` no work (empty list) by the domain's `next`-family convention.

### Example

```console
$ fleet agent shortlist take
```

Hands out the soonest shortlist entry as one JSON object.

## fleet agent shortlist clear

```
fleet agent shortlist clear
```

| Flag          | Argument | Default               | Meaning                                                                              |
| ------------- | -------- | --------------------- | ------------------------------------------------------------------------------------ |
| `--shortlist` | `PATH`   | profile/state default | Shortlist file path (shared shortlist flag). This verb takes no verb-specific flags. |

### Behaviour

Empties the list and clears the refill marker.

### Exit codes

`0` ok · `2` usage.

### Example

```console
$ fleet agent shortlist clear
```

Empties the local shortlist and clears any live refill marker.

## fleet agent shortlist add

```
fleet agent shortlist add --issue N --title T [--repo R] [--rank N] [--why TEXT]
```

| Flag          | Argument | Default                 | Meaning                                                    |
| ------------- | -------- | ----------------------- | ---------------------------------------------------------- |
| `--issue`     | `N`      | required                | Issue number to add.                                       |
| `--title`     | `T`      | required                | Issue title recorded on the entry.                         |
| `--repo`      | `R`      | `developing-today/code` | Repository the issue belongs to.                           |
| `--rank`      | `N`      | `6` (unranked)          | Rank for hand-out order (see `status` for the rank table). |
| `--why`       | `TEXT`   | —                       | Why this entry was listed.                                 |
| `--shortlist` | `PATH`   | profile/state default   | Shortlist file path (shared shortlist flag).               |

### Behaviour

Adds one entry `{repo, issue, title, rank, why, added_at}`. Duplicates (same repo+issue already listed) are refused — the add returns false rather than listing the issue twice.

### Exit codes

`0` ok · `2` usage.

### Example

```console
$ fleet agent shortlist add --issue 13165 --title "Write the fleet book" --rank 1
```

Adds issue 13165 at rank 1 (`priority/critical` rank).

## fleet agent shortlist fill

```
fleet agent shortlist fill [--repo R] [--target N]
```

| Flag          | Argument | Default                 | Meaning                                      |
| ------------- | -------- | ----------------------- | -------------------------------------------- |
| `--repo`      | `R`      | `developing-today/code` | Repository to draw candidates from.          |
| `--target`    | `N`      | `8` (clamped 1–50)      | Top the list up to this many entries.        |
| `--shortlist` | `PATH`   | profile/state default   | Shortlist file path (shared shortlist flag). |

### Behaviour

Tops the list up to target from the priority candidates: it fetches **2× target** candidates from GitHub (the same candidate scan as `next` rung 4), adds them in rank order until the target is met, records the fill (so a just-filled list suppresses an immediate second refill), and clears the refill marker. Refill bookkeeping: a refill marker `(by, at)` lasts **30 min** (1800 s); a list needs refill when it is low, no marker is live, and no fill finished within the window.

### Exit codes

`0` ok · `2` usage · `3` a needed leg is down (GitHub unreachable).

### Example

```console
$ fleet agent shortlist fill
```

Tops the local shortlist up to 8 entries from the priority candidates.
