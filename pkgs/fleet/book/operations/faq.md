# FAQ

Short answers, each pointing at the chapter that owns the long one. For symptom-driven diagnosis, see [Troubleshooting](troubleshooting.md); for definitions, see [Glossary](glossary.md).

**Why is fleet one binary instead of a tool per job?**

One multicall binary means one artefact to build, pin, publish, and roll out to every seat, and one registry that every build knows in full — so `--help` and `fleet capabilities` are complete even on a narrowed build, and an unavailable domain answers with an informative stub instead of "command not found". Domains are compiled in per variant by Cargo features; hyphenated names (`fleet-serve`, `fleet-connect`) and aliases (`repo-queue`) are argv0 personalities of the same binary. See [Register a domain or capability](../development/register-a-domain.md).

**Why did my command print a stub and exit 69?**

The domain is not usable in that invocation: not compiled into the build, narrowed out by the invocation name, or disabled by config on that host. The stub's first line says which; `fleet capabilities` says it in JSON. Exit 69 is `EX_UNAVAILABLE` — the binary working as designed. See the stub entry in [Troubleshooting](troubleshooting.md).

**The broker is down — what do I do?**

First, find out *which* broker and *which layer*: `fleet agent doctor` for the seat, `fleet queue health` for the queue, `GET /health` for the secrets-broker. On a sandbox seat, the heal chain rebuilds the stack in order (tailscaled → relay → session broker → secrets broker → gh) on a roughly five-minute cycle. On the laptop, systemd owns the brokers — check the unit, and redeploy through the profile if the generation is bad. If the *queue*-broker is unreachable for a work verb, the client switches to the GitHub fallback with exit 6 rather than failing outright. See [Health checks and the heal stack](health-and-healing.md).

**Where do secrets come from?**

By name, never by value in files or commands. The resolution path is the secrets-broker protocol first (a Bitwarden Secrets Manager cache that refreshes on a TTL), then the `bws` CLI as fallback, then `op` where a vault is named. Deploy definitions, bootstrap scripts, and this book name secrets; they never carry values. A secret minted moments ago is visible immediately: a cache miss reads through to Bitwarden for that one name.

**Why did placement pick Blacksmith?**

Placement is a tiered cascade over the surfaces in `fleet-surfaces.json`: free GitHub-hosted Actions capacity first (Tier 1, least-loaded owner first), then Blacksmith (Tier 2) within its monthly budget — Blacksmith is a budgeted fallback, not a default, and a dispatch can require explicit allowance for paid capacity. A `--followup` dispatch sticks to the previous session's surface when that surface still has capacity and its owner's queued count is below the sticky threshold (2 on every shipped surface). See also: why did placement pick *anything* — it picks from live capacity and local session history, per owner, not per repository whim.

**What is an `fp_` handle?**

The handle of a placement dispatch session: `fp_` plus a 128-bit random value, printed when `fleet queue dispatch` records the session. It names the dispatch session record (whose local records live in `~/.local/state/fleet-place/sessions.json`) and is what `fleet queue session status <handle>` and `fleet queue session result <handle>` take. It is distinct from the workflow's `dispatch_id` input, which is a separate fresh correlation id generated per dispatch when the surface's workflow declares one. A portable alternative form for run correlation is `OWNER/REPO#RUN_ID`.

**Can I use fleet as a library?**

The workspace's library crates (`fleet-registry`, `fleet-config`, `fleet-secrets`, `fleet-queue`, and the rest) are real crates with real APIs, and fleet's own domains consume them that way. But they are an internal workspace on the atlas-search pattern — versioned together at 0.1.0, consumed by path, with no published-crate compatibility promise. Inside the repository, write against them freely; outside it, shell out to the binary, whose command contract is the stable surface.

**How do I add a verb?**

Follow [Add a subcommand](../development/add-a-subcommand.md): registry entry, both usage strings, dispatch arm, handler, broker verb if it mutates state, fallback twin if it must work broker-down, oracle-named tests, and `docs/verbs.md` plus this book in the same change. For a flag on an existing verb, see [Add a flag](../development/add-a-flag.md).

**Which queue am I on?**

There is one fleet queue; the `--queue` flag scopes within it, it does not select a different system. The subtleties: `claim` and `list` are *unscoped* when neither flag nor the `REPO_QUEUE_DEFAULT_QUEUE` environment variable is set (the broker works across the one queue); `enqueue` has no default queue at all and must name one; most other verbs default to `repo-health-code-shadow` via `--queue`'s default. If a verb behaved as if it could not see your task, check which of those three cases you were in before suspecting the broker.

**Why exit 5?**

Exit 5 is the queue client's "the verb ran, but there is no matching claimed row / no claimable task" code. It covers: `claim` finding nothing claimable (`{"task": null}`), the no-double-serve guard holding a fresh claim (`fallback_held`), and every holder verb (`heartbeat`, `checkpoint`, `park`, `release`, `block`, `fail`, `complete`) whose worker/lane does not match the claimed row or whose row is already terminal. `fleet agent next` also exits 5 when no intake rung had work. See the lane/worker mismatch and lease-expiry entries in [Troubleshooting](troubleshooting.md).

**Why did `complete` reject me?**

Because completion of a fix-class task is evidence-gated: it must carry a verified disposition record from `fleet queue verify-disposition`, read back live from GitHub, and the broker's gate found a defect — a stale record, a verdict the readback does not support, a linked PR missing from the readback, or a follow-up that fails its checks. The rejection leaves the task claimed, deliberately. Iterate with `verify-disposition --json` standalone (exit 0 means the record would pass) and complete promptly with a fresh record. See [Troubleshooting](troubleshooting.md).

**Where is this book's source?**

In the repository, at `pkgs/fleet/book/` (chapters) with `pkgs/fleet/book.toml` beside it and the build in `pkgs/fleet/docs.nix`, published as the `.#fleet-docs` package and deployed under `/fleet/` on the combined Pages site. Build or preview it with the steps in [Deploying this book](deploying-this-book.md). The book's maintenance rule — it changes in the same pull request as the behaviour it describes — is [Keeping this book current](../development/keeping-this-book-current.md).

**Why does the same secret name resolve differently on the laptop and the sandbox?**

Because "the secrets-broker" is location-relative: on the laptop it is the tailnet-bound broker; on a sandbox seat, `fleet init` runs a local loopback copy on the same port. Both are caches of the same Bitwarden Secrets Manager source, resolved by the same name — what differs is which cache answers, and therefore the value's age (`age_seconds` on a cache hit tells you). Check `/health`'s `oldest_secret_age_seconds` before assuming a rotation did not propagate.
