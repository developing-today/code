# Glossary

One-to-three sentence definitions, alphabetised. Chapters use these terms in these senses.

**broker** — A resident fleet service that other components ask over a socket or HTTP: the queue-broker owns queue state, the secrets-broker owns the secrets cache, and the session broker (the session proxy's server half) owns laptop-channel connections. Brokers are the stateful centre; clients are replaceable.

**capability** — One of the 15 compile-time atoms in `fleet-registry` (`queue`, `proxy-connect`, `search-serve`, and so on) that decide what a build can do. A domain's availability is derived from its capabilities, the invocation's profile, and config, as one of four `CapState` values.

**disposition** — The verified account of what actually happened to a task's issue and its linked pull requests, read back live from GitHub by `verify-disposition`. A completion that requires one must carry the record, and the broker's gate checks it before the task may close.

**envelope** — The broker's reply shape: `{ok: true, …}` carrying a `row` (or `rows`), or `{ok: false, exit, error}` carrying the exit code the client exits with and the message it prints. Clients do not invent errors; they relay envelopes.

**fallback** — Two senses, both load-bearing. (1) _Transport fallback_: the GitHub-issues transport of the queue in `fleet-queue/src/fallback.rs`, used when the broker path fails; a work verb switches to it with exit 6. (2) _Resolution fallback_: the next source in a chain, as in the secrets path falling back from the secrets-broker protocol to the `bws` CLI.

**follow-up** — A recorded successor that owns the remainder of unfinished work: a task, an issue, or a pull request named on a completion (with a `--reason`, always) so a task may close while linked work is still open. The disposition gate verifies it exists and is in the state claimed.

**lane** — A named work stream within a queue worker family, recorded on a claim so the broker can tell whose claim a holder verb acts on. All scheduled lanes share one worker id, so the lane — resolved from `--lane`, else the `REPO_QUEUE_LANE` environment variable, else null — is the discriminator.

**leg** — One of `fleet agent doctor`'s four independent probes: bridge, queue, github, shell. There is no secrets leg; secrets is folded into the bridge leg's stated need. From the legs, doctor derives the rungs map.

**outbox** — The creation outbox: the broker-side queue for GitHub object creation (issues, pull requests, comments) that must survive crashes and rate limits. Producers enqueue; a single drainer claims, creates, verifies by read-back, and resolves, pacing creates and backing off on rate limits.

**placement** — The component that chooses where a dispatched job runs, from the surfaces registry, by live capacity and session history: a tiered cascade (free GitHub-hosted Actions, then budgeted Blacksmith, then the remaining surface kinds) with a hard-pin override and follow-up stickiness.

**rung** — One step of the `fleet agent next` intake ladder, tried in order: push (a bridge command addressed to the agent), queue (a queue claim), shortlist (the local file), priority (GitHub). A rung that is _down_ is skipped and recorded; a rung that is _empty_ is a different fact.

**seat** — One enrolled agent installation: a sandbox, a laptop, or another host running a fleet binary with its own identity, config, and token files. Rollout is verified per seat, because seats run whatever binary they were last given.

**shortlist** — The local file of candidate work an agent seat keeps for itself, managed by `fleet agent shortlist`. It is the one intake rung that is always available, because it needs no network, broker, or token.

**stack** — Merge-intent sense: a recorded, ordered set of merge intents (finished branches with their authoring lane, worker, and session) in the broker's `merge_intents` table, managed by `fleet queue stack`. Not a call stack and not the pm stack.

**stub** — The three-line stderr block a domain answers with when it is not usable in an invocation, always exit 69 (`EX_UNAVAILABLE`). Its wording names the reason class: not in this build, narrowed out by the invocation name, disabled by config, or included but not implemented.

**surface** — Two related senses. (1) _Execution surface_: an entry in `fleet-surfaces.json` naming a place fleet work can run (a GitHub Actions pool, Blacksmith, a codespace pool, the laptop), with its owner, targets, status, and capacity facts. (2) _Invocation surface_: the set of capabilities a single fleet invocation can use, after build, profile, and config narrowing.

**session proxy** — The fleet proxy domain's broker-and-client pair that carries a sandbox seat's traffic to the laptop's brokers over one channel, with a length-prefixed JSON wire protocol and a hello/capability handshake. `proxy serve` is the broker; `proxy connect` and `proxy init` are the client side.

**tombstone** — A terminal marker that records a task, claim, or session as deliberately dead, so later folds and replays skip it instead of resurrecting it. A tombstone is a record, not a deletion: the history stays readable.

**variant** — One of the three shipped capability profiles of the one binary — Full (`fleet`), Serve (`fleet-serve`), Connect (`fleet-connect`) — selected at build time by Cargo features and re-derivable at runtime from the compiled capability set. Hyphenated variant names double as argv0 profiles that narrow, never widen, an invocation's surface.

**worker** — The identity that claims and holds queue tasks (`--worker`, default `repo-worker-code`, with named exceptions such as `cancel`'s `fleet-monitor`). Holder verbs are checked against the claiming worker and lane; a mismatch is exit 5.
