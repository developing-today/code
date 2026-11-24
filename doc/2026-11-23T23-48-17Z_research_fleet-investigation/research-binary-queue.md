# Fleet program investigation — Research: the fleet binary, the fleet book, and the queue broker

- **Scope:** (1) the installed `fleet` binary's full command surface, (2) a census of
  the fleet book in `pkgs/fleet/book/` and a coverage comparison against the binary,
  (3) queue-broker internals in `pkgs/fleet/crates/fleet-queue/`, (4) verification of
  the known queue defects code#13091, code#13191, code#13146 and the malformed-packet
  class.
- **Method / sources:** first-hand `--help` dumps from the sandbox-seat binary;
  read-only reads of the laptop checkout `~/git/code` (developing-today/code) via
  `~/tools/laptop-ssh.sh`; GitHub state via `gh api` REST only (the GitHub connector
  was not used). No repo was modified, no worktree created, no comment posted.
- **Run date:** 2026-11-23 (America/Chicago morning; GitHub timestamps are UTC).
- **Grounding:** every version, count and status below names its source inline.
  Anything not verified first-hand is labelled **[unverified]**.

---

## 1. The fleet binary's full surface

### 1.1 Where the binary lives — three different builds in play

| Location                                                                                                                                  | What it is                                                                                                                                                                                                                                                                     | Version evidence                                                                                                                                                                   |
| ----------------------------------------------------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `~/.local/share/hatch-recovery/bin/fleet` (sandbox seat; `~/tools/bin/fleet` symlinks here; also on PATH as `fleet`)                      | The seat's working binary. 9,678,656 bytes, static-pie ELF, sha256 `c689a9006ea57e66fcaa7e5b283974e7a29f29e7a026e4522a67fab661035b04` (matches the sibling `fleet.sha256`). Byte-identical in size/timestamp to the sibling file `fleet-static-3cb241a` in the same directory. | `fleet version`: `fleet 0.1.0`, derived `0.1.0-dev+g044a6bd`, **source rev `044a6bda94bdd2fead70566816a3e4cfcc2e3d5a`**                                                            |
| `~/.local/share/deploy/profiles/queue-broker/current/bin/fleet` on the laptop → `/nix/store/0ymw7b3zl2hxij9npmzal6h5fq2g4cp3-fleet-0.1.0` | The deployed queue-broker build (see §3.5).                                                                                                                                                                                                                                    | `fleet version` on the laptop: derived `0.1.0-dev+gb8bbcc0`, **source rev `b8bbcc0c49564ab31db8d58b026b67532db9491e`** — this is the "b8bbcc0 binary" the book's surface describes |
| `~/tools/fleet` (sandbox)                                                                                                                 | An older 7,632,520-byte copy (2026-11-22 restore). **It does not execute from the sandbox**: `cannot execute: required file not found` (its interpreter is absent — the known `~/tools/fleet` exec defect recorded in MEMORY.md). Do not cite it as a working binary.          | n/a (not runnable)                                                                                                                                                                 |
| Laptop checkout source, `~/git/code` HEAD                                                                                                 | The source of truth the next rollout will build. HEAD moved during this pass: `a66148f7393f86658cbcb75a9237924dd2c3d33f` at first read, `cafdfdc5c9c33839dc8e5e126f62b6487d358a98` at last `git log` read.                                                                     | `git rev-parse HEAD` via laptop-ssh                                                                                                                                                |

Other staged builds in `~/.local/share/hatch-recovery/bin/`: `fleet-src-d26fa16`,
`fleet-static-7bdd04a`, `fleet-static-b8bbcc0` (the last cannot be invoked under its
own filename — the binary rejects an unknown argv0 with exit 2 and lists the expected
names: `fleet`, `fleet-serve`, `fleet-connect`, or an alias).

**Headline divergence:** the sandbox binary (044a6bd), the deployed broker binary
(b8bbcc0) and the laptop `main` source are three different revisions. In particular,
**neither shipped binary contains the `fleet queue repair` verb** (§4.2), although the
verb is merged on `main`.

### 1.2 Identity, capabilities, invocation shape

From `fleet version` / `fleet capabilities` (sandbox binary, verbatim facts):

- Version `0.1.0`; derived version `0.1.0-dev+g044a6bd`; build name `fleet`.
- 15 capabilities, all enabled in this build (usable set = compiled set):
  `proxy-connect, proxy-serve, proxy-raw-splice, queue, queue-broker, secrets-broker,
search, search-serve, dispatch, dispatch-test, laptop-agent, secrets, gh, bridge,
agent`. There is no `shell` capability atom — the `shell` domain rides on `agent`
  (also stated in book `getting-started/shape.md`).
- Multicall shape: behaviour is selected by argv0, then by the first argument.
  Aliases: `queue`←`repo-queue`; `serve`←`queue-broker`, `secrets-broker`;
  `proxy`←`laptop-agent-broker`; `search`←`atlas-search`; `laptop-agent`←`laptop-agent`.
  Variant names `fleet-serve` / `fleet-connect` act as capability-ceiling profiles.
  A domain a build cannot serve answers with a stub and **exit code 69**; an unknown
  argv0 or domain is a usage error, exit 2.
- Built-in (non-domain) commands: `init`, `session bind`, `version`, `capabilities`,
  `help [domain]`. Note the help-routing quirk, verified first-hand: `fleet help init`,
  `fleet help session`, `fleet help version` and `fleet help capabilities` print
  `error: no domain named '<x>'` first (they are not domains) and then still print the
  built-in's own usage; `fleet init --help` and `fleet session bind --help` work directly.

### 1.3 The complete command tree (sandbox binary 044a6bd, dumped recursively)

Purposes are the binary's own one-liners / usage text, condensed. Flags are listed
only where they carry meaning beyond the obvious; full flag tables are in the book's
reference chapters (§2).

**`fleet queue`** — _Repo work queue: enqueue, claim, and complete tasks through the
queue broker._ Alias argv0: `repo-queue`. 31 top-level verbs in the domain help
listing, three of which are groups (`fallback`, `outbox`, `stack`) and one a combined
listing entry (`dispatch|session|surfaces`):

| Verb                                                                                                                                                                                                                                                                              | Purpose (from `--help`)                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `validate <packet-path>`                                                                                                                                                                                                                                                          | Validates a task packet without enqueuing it.                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `enqueue <packet-path> [--queue] [--platform] [--dry-run] [--shadow-intent]`                                                                                                                                                                                                      | Enqueues a task packet.                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `list [--queue] [--status] [--limit] [--not-before] [--lane] [--worker] [--task]`                                                                                                                                                                                                 | Lists tasks; bare `list` = broker default ordering (newest terminal rows first); `--status queued\|parked\|claimed\|blocked` for actionable work.                                                                                                                                                                                                                                                                                                               |
| `claim [--queue] [--worker] [--lease-seconds] [--lane] [--task] [--count] [--from-session] [--platform] [--session-id] [--model-provider/--model-id/--model-variant] [--filter] [--pool] [--launched-by] [--remaining-budget-minutes] [--handoff] [--bind]`                       | Claims the highest-priority eligible task.                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `release <task-id> [--worker] [--lane] [--reason]`                                                                                                                                                                                                                                | Releases a held claim. (Domain help listing omits `--lane`; the verb's own `--help` includes it — a listing/help skew in the binary itself.)                                                                                                                                                                                                                                                                                                                    |
| `lane <task-id> [--worker] [--exclusion-reason] [--opencode-reason] [--limit-platform/--limit-reason/--clear-platform-limit]`                                                                                                                                                     | Reports lane progress on a task.                                                                                                                                                                                                                                                                                                                                                                                                                                |
| `bind <task-id> --session-id <id> [--worker] [--launched-by] [--model-*]`                                                                                                                                                                                                         | Binds a worker/session to a task.                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `heartbeat <task-id> [--extend-seconds] [--progress/--progress-file]`                                                                                                                                                                                                             | Refreshes a claim lease.                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `checkpoint <task-id> [--stage] [--progress/--progress-file]`                                                                                                                                                                                                                     | Records a progress checkpoint.                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `park <task-id> --reason <text> [--delay-seconds] [--not-before] [--ready-condition] [--execution] [--required-budget-minutes] [--hands <kind> --action <text>]`                                                                                                                  | Parks a task. A park that waits on a person is refused unless it is a hands item (FDR-0013): `--hands physical\|interactive-login\|payment\|signature` with a five-minute `--action`.                                                                                                                                                                                                                                                                           |
| `wake <task-id> [--reason]`                                                                                                                                                                                                                                                       | Wakes a parked task.                                                                                                                                                                                                                                                                                                                                                                                                                                            |
| `block <task-id> --reason-class <c> --blocker <b> [--blocked-by-pr/--blocked-by-issue/--blocked-by-task/--blocked-by-ci] [--dep-recheck-seconds] [--dep-max-ttl-seconds] [--hands …]`                                                                                             | Marks a task blocked; same FDR-0013 hands rule as `park`.                                                                                                                                                                                                                                                                                                                                                                                                       |
| `unblock <task-id> [--reason] [--drop-dep <list>]`                                                                                                                                                                                                                                | Clears blocked state; `--drop-dep` drops only named `kind:ref` dep entries (`issue:`/`pr:`/`ci:`/`task:`), releasing the task only when the last dep is dropped.                                                                                                                                                                                                                                                                                                |
| `dep-recheck [--task] [--limit]`                                                                                                                                                                                                                                                  | Rechecks task dependencies.                                                                                                                                                                                                                                                                                                                                                                                                                                     |
| `quarantine-recheck [--task] [--limit]`                                                                                                                                                                                                                                           | Rechecks quarantined tasks (diagnose/release only — it cannot fix a still-malformed packet; see §3.4, §4.2).                                                                                                                                                                                                                                                                                                                                                    |
| `cancel <task-id> [--reason]`                                                                                                                                                                                                                                                     | Cancels a task.                                                                                                                                                                                                                                                                                                                                                                                                                                                 |
| `revive <task-id> [--reason] [--not-before]`                                                                                                                                                                                                                                      | Revives a terminal task.                                                                                                                                                                                                                                                                                                                                                                                                                                        |
| `fail <task-id> [--error] [--reason-class] [--execution]`                                                                                                                                                                                                                         | Fails a task.                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `complete <task-id> [--result/--result-file] [--telemetry] [--execution]`                                                                                                                                                                                                         | Completes a task.                                                                                                                                                                                                                                                                                                                                                                                                                                               |
| `verify-disposition <issue> --repo <owner/name> [--pr …] [--follow-up-task/--follow-up-issue/--follow-up-pr] [--staleness-seconds] [--json]`                                                                                                                                      | Reads the issue and linked PRs back from GitHub and prints the disposition a terminal completion must carry (#10611); exits 1 when the broker's gate would reject it.                                                                                                                                                                                                                                                                                           |
| `timeline <task-id> [--json]` / `history` (alias)                                                                                                                                                                                                                                 | Shows a task's event timeline.                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `show <task-id> [--limit] [--json]`                                                                                                                                                                                                                                               | Shows one task's current details and recent history.                                                                                                                                                                                                                                                                                                                                                                                                            |
| `models [--days] [--json]`                                                                                                                                                                                                                                                        | Rolls up serving-model stamps.                                                                                                                                                                                                                                                                                                                                                                                                                                  |
| `health [--queue]`                                                                                                                                                                                                                                                                | Broker health (live output observed in this pass: per-queue counts incl. `queued/claimed/parked/malformed_parked/handed_off/blocked/dep_blocked/done/cancelled/failed`).                                                                                                                                                                                                                                                                                        |
| `signal [--queue] [--reason]`                                                                                                                                                                                                                                                     | Sends a queue signal.                                                                                                                                                                                                                                                                                                                                                                                                                                           |
| `fallback <scan\|status\|claim\|heartbeat\|release\|complete\|unblock> …`                                                                                                                                                                                                         | GitHub-backed queue used when the primary broker is down; issue comments carry the claims. `scan` lists live/stale claims across fallback repos; `claim` posts a claim-marker comment (races detected on read-back). Note: the domain listing says `fallback <scan\|status\|claim\|heartbeat\|release\|unblock>` while the verb's own help adds `complete` — another listing/help skew.                                                                         |
| `dispatch --repo <owner/name> --command '<cmd>' [--ref] [--runner auto\|github\|blacksmith] [--platform linux\|macos] [--container] [--surface] [--followup] [--push-nix-cache] [--actions-cache …] [--allow-paid-blacksmith] [--laptop-busy] [--dispatch] [--json] [--registry]` | Placement dispatch (absorbed `fleet-place`): dry-run unless `--dispatch`; prints the placement decision / session handle.                                                                                                                                                                                                                                                                                                                                       |
| `session <status\|result\|list> [HANDLE\|OWNER/REPO#RUN_ID] [--mine] [--json] [--registry]`                                                                                                                                                                                       | Placement-session inspection. Verified live in this pass: `session list` returned 6 local records (`fp_…` handles in states dispatched/queued/succeeded/running).                                                                                                                                                                                                                                                                                               |
| `surfaces [--registry]`                                                                                                                                                                                                                                                           | Prints the resolved placement registry verbatim as JSON. Verified live: registry version 2, owners `developing-today` (cap 20, macOS 5, free-org), `developing-today-forks` (20/5), `dezren39` (40/5, pro); surfaces `fleet-personal`, `fleet-org`, `fleet-forks` (active, github-actions), `org-run` (standby), `fleet-org-blacksmith` (blacksmith, 3,000 free min/mo, paid cap 0), `codespace-overflow` (pending-scope, max 3 live), `laptop` (handoff-only). |
| `outbox <enqueue\|list\|metrics\|claim\|resolve\|drain>`                                                                                                                                                                                                                          | Creation-outbox (GitHub issue/PR creation requests): `enqueue [--repo] [--kind] [--dedup-key] [--producer] [--payload/--payload-file]`; `resolve <id> [--outcome done\|rate_limited\|uncertain\|failed] [--github-number/--github-url] [--retry-after] [--error]`; `drain [--owner] [--max]`.                                                                                                                                                                   |
| `stack <record\|list\|transition\|restack\|replay>`                                                                                                                                                                                                                               | PR-stack intents: `record --stack --repo --branch [--base-branch] [--head-sha] [--issue] [--pr] --lane`; `transition --stack --position --to [--pr] [--conflict-detail]`; `restack --stack --position --base`; `replay [--stack] [--repo-dir] [--dry-run]`.                                                                                                                                                                                                     |
| **`repair` — ABSENT**                                                                                                                                                                                                                                                             | `fleet queue repair --help` on both the sandbox binary (044a6bd) and the deployed laptop binary (b8bbcc0) answers `repo-queue: unknown command "repair"` (exit 2) and prints the verb list without it. The verb exists only in `main` source (§3.4, §4.2).                                                                                                                                                                                                      |

**`fleet serve`** — _Run a fleet daemon role in the foreground._ Aliases argv0:
`queue-broker`, `secrets-broker`. Verbs: `queue-broker`, `secrets-broker`, `search`,
`proxy`. Only `serve search` and `serve proxy` have their own `--help` text
(`serve search [--config] [--listen] [--job-ttl-ms] [--hard-ttl-ms]` — the Atlas
search service, loopback HTTP job protocol over a shared Snowflake embedder and a
read-only Turso backend, rehomed in #5604; `serve proxy` = `fleet proxy serve`).
`serve queue-broker --help` / `serve secrets-broker --help` fall back to the domain
help (verified first-hand).

**`fleet proxy`** — _The session proxy._ Alias argv0: `laptop-agent-broker`. Verbs:
`serve` (bind the broker socket from fleet config), `connect [--print-path] [--stdio]
[--request METHOD PATH …]` (ensure a broker answers, sync PATH shims, optionally
issue one request), `init` (same as `fleet init`), `ssh -- [ssh args…]` (system
OpenSSH through the proxy as a recovery path; destination pinned by fleet),
`splice` (listen on 127.0.0.1:21118, relay through the broker to the laptop's
RustDesk direct-access service), `get [--mode] [--no-resume] <laptop-path>
<local-path|->` and `put [-r] …` (chunked file transfer over the agent channel,
#5759 v1; SHA-256 verified before atomic rename; exit codes 2 usage / 3 transport /
69 capability-unavailable).

**`fleet dispatch`** — _Thin client over pm-stack dispatch._ Verbs: `status`
(GET /health on pm-dispatch), `launch --repo --prompt [--prompt-file] [--title]
[--model] [--directory] [--priority 10-90|critical|high|normal|low] [--issue/--no-issue]
[--wait/--no-wait]` (POST /v1/sessions: creates tracking issue + queue task, starts
an OpenCode session on the laptop, assigns the task), `enqueue --repo --title
--prompt [--kind triage|small_fix|pr_shepherd|ci_investigation] [--dedupe-key] …`
(POST /v1/tasks, no session).

**`fleet dispatch-test`** — _Explicit-dispatch CI test._ Verbs: `scan [--threshold N]
[--json]` (count active Actions jobs per discovered fleet-repo copy — the fleet repo
plus its fork network — and name the first eligible copy), `submit --submodule NAME
[--repo] [--ref] [--name] [--commands/--commands-file]` (store a job in the Turso
jobs store, dispatch `fleet-dispatch-test.yml`, mark dispatched only on success,
#6598), `status <job-id>` (print the stored row; unknown id exits 4).

**`fleet secrets`** — _Resolve secrets by name: laptop secrets-broker first,
Bitwarden as fallback._ Verbs: `get <name>` (value to stdout and nowhere else),
`list` (names in the broker's cache, never values).

**`fleet gh`** — _The real `gh` CLI passed through under a rotating fleet token_
(personal, then the developing.today App's installations; rate-limit/permission
failures fail over). Domain help lists four shadow subcommands — `tiers`,
`cache-serve`, `cache-status`, `closures` — and notes the implementation "lands in
a later step of epic #5412"; every other argument vector passes through to real
`gh`. Verified: `gh cache-serve [--bind <addr>]` (credential/token cache daemon,
code#10742; default bind 127.0.0.1:4802; memory-only, single-flight minting) and
`gh cache-status` (probe daemons, counts only, exits 3 when none answers) have real
help; `gh tiers --help` errors `unexpected argument '--help'` and `gh closures
--help` prints nothing — the passthrough/shadow boundary is uneven in this build.

**`fleet bridge`** — _Fleet messaging through the message bridge._ Verbs: `send`,
`agents`, `log`, `hello`, `inbox`, `post`, `ack`, `health`. All verbs share one
usage text (verified: every `bridge <verb> --help` prints the same page). Central
(owner token): `send --to <agent:<name>|group:<name>|all> [--command] [--sender]
<text|->`, `agents`, `log [--agent] [--direction] [--kind] [--since-id] [--limit]
[--order]`. Agent (fleet token): `hello [--agent] [--groups]` (register/refresh),
`inbox [--commands|--messages] [--all] [--take] [--wait SECS]`, `post [--reply-to]
[--to] [--instruction|--kind]`, `ack (--up-to ID | ID…)`. `health` = the bridge's
`/healthz`, no token. Bridge URL: `$FLEET_BRIDGE_URL`, else `[bridge] url`, else
`https://bridge.security.cab`. Exits: 0 ok, 2 usage, 3 transport/credentials,
4 bridge refused.

**`fleet agent`** — _The agent's front door._ Verbs: `kinds`, `enroll`, `doctor`,
`next`, `shortlist`. All verbs share one usage page. `enroll --kind K --name N
[--groups] [--dry-run] [--profile]` registers an agent and saves its profile;
`doctor [--agent N]` reports which legs answer from this seat; `next [--agent]
[--lane] [--repo] [--low-water] [--shortlist] [--platform] [--pool]
[--remaining-budget-minutes] [--filter] [--queue] [--no-push] [--no-queue]
[--no-shortlist] [--no-priority]` takes the next work down the intake ladder
(pushed command → queue → local shortlist → highest priority); `shortlist
status|take|clear`, `shortlist add --issue N --title T …`, `shortlist fill
[--repo] [--target N]`. Every verb prints one JSON object. Exits: 0 ok, 2 usage,
3 a needed leg is down, 5 no work on any rung. There is **no** top-level
`fleet doctor` or `fleet enroll` — both answer `error: unrecognized subcommand`
(verified); they exist only under `fleet agent`.

**`fleet shell`** — _Run a command, or open a shell, where builds belong._
Only subcommand in the registry listing is `targets`, but the domain's own usage
is flag-driven: `fleet shell [--target auto|laptop|codespace|actions|local]
[--codespace NAME] [--repo] [--cwd] [--build] [--timeout SECS] [--detach]
[--print] [-- COMMAND…]`. `auto` resolves: `FLEET_SHELL_TARGET`, then here if this
seat is the laptop, then the laptop, then a codespace, then here — never actions.
The actions target dispatches through placement (`fleet queue dispatch`) and waits
for the session result.

**`fleet search`** — _Atlas corpus search over the session history (Snowflake
hybrid, via atlas-search)._ Alias argv0: `atlas-search`. Verbs: `search` (also the
default: `fleet search <query> [options]`), `get-chunk <chunk-id> [--neighbors N]
[--text]`, `serve`. Search options include `--k` (default 10), `--pool` (1..=400,
default 100), `--mode hybrid|fts|vector`, `--kind/--topic/--project/--client`
filters, `--since/--until`, `--max-sensitivity shareable|personal|local-only|
secret-bearing` (default personal; server clamps to the seat's ceiling),
`--group-by-session`, `--expand`, `--with-message`, `--no-cache`, `--no-wait`,
`--hydrate`, `--raw-rows[=branches]`, `--text`, `--model` (default Snowflake
arctic-embed-m-v2.0), `--embedder auto|fastembed|st`, `--trunc`, `--vector-backend
scan|index-fill|index-fixed`, `--via-service/--standalone`, `--service-url`
(default `http://127.0.0.1:18760`). Routing: the sandbox seat prefers the service
with a loud standalone fallback; the laptop seat defaults to standalone;
`get-chunk` always runs standalone.

**`fleet laptop-agent`** — _Client for the laptop-agent daemon._ Alias argv0:
`laptop-agent`. Verbs per domain help: `health`, `jobs`, `kill`, `exec`, `run`,
`poll`, `fetch`. Caveat verified first-hand: in this build the domain is a partial
surface — `laptop-agent health --help` and `jobs --help` answer
`laptop-agent: unknown command "health"` / `"jobs"` and then print the verb list
(`health`, `jobs`, `kill JOB`, `exec [--cwd] [--timeout] [--shell] -- CMD…`,
`run …`, `poll JOB [--wait SECS]`, `fetch REMOTE_PATH LOCAL_PATH`); the domain
help itself says the implementation "lands in a later step of epic #5412". The
working laptop path in practice is `~/tools/laptop-agent.py` via
`~/tools/laptop-ssh.sh` (whose header records the same stub finding).

**Built-ins:** `fleet init` (autostart this host's roles per the `[host]` manifest;
on the sandbox: session proxy + standing secrets broker; strict no-op when already
running; unknown location exits 69), `fleet session bind [--session-id ID]`
(validate and persist this worktree's session identity), `fleet version`,
`fleet capabilities` (registry + capabilities as JSON), `fleet help [domain]`,
`fleet --help` / `--version` / `-V`.

**Counts:** 12 domains, 15 capabilities, 5 built-ins. Queue alone: 31 listed
top-level entries expanding to 31 + 6 (`fallback`) + 6 (`outbox`) + 5 (`stack`)

- 3 (`session`) leaf verbs.

---

## 2. The fleet book census

### 2.1 Location and build config

Source: laptop checkout `~/git/code/pkgs/fleet/`. The book root is `pkgs/fleet`
itself; sources are plain Markdown under `pkgs/fleet/book/` (readable on GitHub as
written), rendered via the `.#fleet-docs` flake output (`pkgs/fleet/docs.nix`).
`pkgs/fleet/book.toml` (mdBook config, #13165):

```toml
[book]
title = "fleet"
authors = ["developing-today"]
language = "en"
src = "book"
[build]
build-dir = "rendered"
create-missing = false   # every chapter in SUMMARY.md must exist on disk
[output.html]
git-repository-url = "https://github.com/developing-today/code"
```

There is no `book/src/` directory — `SUMMARY.md` is at `pkgs/fleet/book/SUMMARY.md`.
The book is live at `/code/fleet/` (per the task brief; the coordinator's census
records the URL as https://developing-today.github.io/code/fleet/ and the book's
tracking issue #13165 as CLOSED).

### 2.2 Census: 55 chapters, 98,894 words

`grep -c '^- \[' book/SUMMARY.md` = **55** chapter entries; `find book -name '*.md'`
= **56 files** (55 chapters + `SUMMARY.md` itself). Total word count
(`find book -name '*.md' -exec cat {} + | wc -w`) = **98,894 words**, of which
`SUMMARY.md` is 243 — so **98,651 words of chapter text**. Per-file counts from
`wc -w` on the laptop:

| Section         | Chapters                                                                                                                                                                                                                                                                                                                                                                    | Words  | What each covers                                                                                                                                                                                                                                                                                                              |
| --------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| (root)          | `index.md` — 1,290                                                                                                                                                                                                                                                                                                                                                          | 1,290  | "fleet": introduction and the domain map.                                                                                                                                                                                                                                                                                     |
| Getting started | `installation.md` 1,253; `quickstart.md` 1,155; `shape.md` 1,370                                                                                                                                                                                                                                                                                                            | 3,778  | Installation/bootstrap (incl. the b8bbcc0 release pin, §2.3); quickstart; the shape of the binary (argv0 personalities, registry, capabilities, build variants).                                                                                                                                                              |
| Concepts (10)   | `architecture.md` 1,875; `queue.md` 3,164; `placement.md` 2,281; `secrets.md` 1,532; `bridge.md` 943; `agents.md` 1,948; `proxy.md` 1,709; `search.md` 1,161; `gh.md` 1,274; `dispatch.md` 1,760                                                                                                                                                                            | 16,647 | One concept chapter per subsystem: architecture; the queue (tasks, lanes, leases — the largest concept chapter); placement and remote dispatch; secrets and the secrets broker; the bridge; agents (enrolment, doctor, intake ladder); proxy/sessions/recovery; Atlas search; the gh passthrough; dispatch and dispatch-test. |
| Reference (17)  | `builtins.md` 2,118; `queue-core.md` 4,522; `queue-lifecycle.md` 5,718; `queue-fallback.md` 2,588; `queue-dispatch.md` 1,565; `queue-admin.md` 3,182; `serve.md` 1,849; `proxy.md` 2,109; `dispatch.md` 1,208; `dispatch-test.md` 1,301; `secrets.md` 930; `gh.md` 1,592; `bridge.md` 1,790; `agent.md` 3,047; `shell.md` 1,536; `search.md` 1,194; `laptop-agent.md` 1,621 | 32,870 | Per-domain verb references with flag tables, behaviour and exit codes. Queue gets five chapters (core / lifecycle / fallback / dispatch-and-sessions / inspection-and-admin); `queue-lifecycle.md` is the book's largest chapter.                                                                                             |
| Tutorials (6)   | `enrol-a-seat.md` 2,085; `claim-and-complete.md` 2,250; `dispatch-a-build.md` 2,133; `resolve-a-secret.md` 1,782; `bridge-messaging.md` 1,606; `run-the-brokers.md` 1,841                                                                                                                                                                                                   | 11,697 | Task tutorials: enrol a seat; claim and complete a queue task; dispatch a build remotely; resolve a secret; bridge messaging; run the brokers.                                                                                                                                                                                |
| Library (4)     | `crates.md` 1,922; `queue-client.md` 1,960; `placement.md` 2,778; `registry-and-config.md` 2,088                                                                                                                                                                                                                                                                            | 8,748  | The crates; the queue client; placement and surfaces as data; registry, config and capabilities.                                                                                                                                                                                                                              |
| Development (8) | `layout.md` 1,174; `building.md` 1,269; `testing.md` 1,391; `add-a-subcommand.md` 1,234; `add-a-flag.md` 1,137; `register-a-domain.md` 1,634; `release-and-rollout.md` 1,719; `keeping-this-book-current.md` 1,017                                                                                                                                                          | 10,575 | Repository layout; building; testing; how to add a subcommand / a flag / register a domain or capability; release and rollout; keeping the book current.                                                                                                                                                                      |
| Operations (6)  | `services.md` 1,093; `health-and-healing.md` 1,275; `troubleshooting.md` 1,790; `deploying-this-book.md` 875; `faq.md` 1,110; `glossary.md` 903                                                                                                                                                                                                                             | 7,046  | Running the brokers as services; health checks and the heal stack; troubleshooting; deploying the book; FAQ; glossary.                                                                                                                                                                                                        |

(Section word sums are from the per-file `wc -w` output; they total 98,894 with
`index.md` and `SUMMARY.md` included.)

### 2.3 Coverage: book vs binary

**Against the b8bbcc0 binary the book was written for, coverage is essentially
complete.** Every domain has a reference chapter; every queue verb in the b8bbcc0
listing is documented, verified by chapter headings:

- `queue-core.md`: validate, enqueue, list, claim, release, show, health.
- `queue-lifecycle.md`: lane, bind, heartbeat, checkpoint, park, wake, block,
  unblock, dep-recheck, quarantine-recheck, cancel, revive, fail, complete,
  verify-disposition.
- `queue-admin.md`: timeline, history, signal, models, outbox (all 6 sub-verbs),
  stack (all 5 sub-verbs).
- `queue-fallback.md`: the fallback family. `queue-dispatch.md`: dispatch, session
  status/result/list, surfaces.
- Spot-checks that passed: the book **does cover search** in depth
  (`reference/search.md` + `concepts/search.md`: hybrid modes, sensitivity ceilings
  and the per-seat client config, routing defaults); it **does cover `fleet gh`'s
  shadow verbs** including `closures` and the closure-reason taxonomy
  (`reference/gh.md` §§ cache-serve / cache-status / closures) — fuller than the
  binary's own `gh --help`, which still calls the domain a later-step stub; it
  covers `agent shortlist`, `shell targets` (including the note that the registry
  help names only `targets` while the flags live in the shell usage), and the
  placement surfaces registry in full.

**Where the book is now stale or missing material (all relative to `main` source,
not to b8bbcc0):**

1. **`fleet queue repair` — not in the book at all.** `grep -rn "repair" book/`
   returns exactly one hit, an unrelated use of the word "repair" in
   `operations/health-and-healing.md` ("not a separate repair implementation").
   There is no repair section in any queue reference chapter. By contrast the
   non-book operator doc `pkgs/fleet/docs/verbs.md` **does** list `repair`
   (line 51: "Repair a quarantined item's malformed packet in place (dedupe stamp
   re-derived live; `--dry-run` reports only)") — PR #13218's docs-sync updated
   `docs/verbs.md`, the top-level queue help listing and the registry inventory,
   but no book chapter. So: book misses repair; the binary misses repair; only
   `main` source + `docs/verbs.md` have it.
2. **Placement sessions are described as a stopgap that has since landed.**
   `reference/queue-dispatch.md` §"fleet queue session status", Behaviour:
   _"Session records are a labeled **STOPGAP**: a local JSON store at
   `~/.local/state/fleet-place/sessions.json` … so records migrate when the
   designed placement-sessions table lands."_ The table **has** landed on `main`
   (#13146 slice 1, PR #13169: `placement_sessions` in the Turso backend + client
   mirror, §3.4/§4.3). The chapter's session list/status behaviour ("Lists all
   session records in the local store") likewise predates the backend union/mirror
   behaviour. `tutorials/run-the-brokers.md` mentions "The placement session
   store" once in passing; no chapter describes `/placement-put|get|list|sync`,
   the `FLEET_PLACE_REMOTE=off` escape hatch, or cross-seat reads.
3. **The installation pin is b8bbcc0.** `getting-started/installation.md` line 47
   pins `FLEET_URL` to `…/releases/download/fleet-static-b8bbcc0/fleet-x86_64-linux-musl`
   — accurate for the book's stated surface, and it means a reader following the
   book installs a binary without `repair` and without the placement-session
   client mirror.
4. **Minor:** `reference/search.md` says the search domain has "Two verbs:
   `search` (the default) and `get-chunk`" and assigns the service role to
   `fleet serve search`; the binary's domain listing also names a third verb,
   `serve`, under `fleet search` itself. The book's routing of readers to
   `fleet serve search` is correct in substance; the verb inventory is incomplete
   by one entry.

No other gaps were found in the spot-check set (queue repair ✗, placement
sessions ✗-stale, search ✓-with-minor-note, gh closures ✓, outbox/stack/fallback/
surfaces ✓). A full flag-by-flag diff of all 55 chapters against the 044a6bd help
dump was **not** performed — **[unverified]** beyond the spot-checks above.

---

## 3. Queue broker internals

Source: laptop checkout `~/git/code/pkgs/fleet/crates/fleet-queue/` (read at HEAD
`a66148f…`/`cafdfdc…`; the queue files did not change between those two reads in
anything cited here).

### 3.1 Crate shape

`fleet-queue` (Cargo.toml): _"The fleet queue domain: the repo-queue client surface
plus the queue-broker and secrets-broker server roles (epic #5412, step 2 #5430)."_
Three Cargo features select the surfaces: `client`, `queue-broker` (adds
`tiny_http`, `rusqlite` bundled), `secrets-broker`. It also builds the standalone
rollback artifacts under their historical names: `[[bin]] repo-queue` (client),
`queue-broker`, `secrets-broker`. The crate is one of 13 fleet crates in
`pkgs/fleet/crates/` (fleet, fleet-agent, fleet-bridge, fleet-config,
fleet-dispatch, fleet-dispatch-test, fleet-laptop-agent, fleet-proxy,
fleet-proxy-proto, fleet-queue, fleet-registry, fleet-search, fleet-secrets) —
there is **no separate queue-broker crate**; the broker is the `queue-broker`
feature of `fleet-queue`, served by the multicall binary as `fleet serve
queue-broker`.

Key modules (`crates/fleet-queue/src/`): `queue_broker.rs` (HTTP/UDS broker shell:
routing + auth; the only unauthenticated route is the health probe),
`turso_queue.rs` + `turso_queue/` (the backend, §3.2), `client/` (`transport.rs`,
`commands_lifecycle.rs`, `commands_query.rs`, `fallback_stack.rs`,
`verify_disposition.rs`, `support.rs`, `mode_report.rs`), `validate.rs` (the packet
validator), `placement.rs` + `placement/` (placement engine + `remote.rs` mirror),
`fallback.rs` (GitHub-issue fallback queue), `creation_outbox.rs`, `disposition.rs`,
`merge_intent.rs`, `quarantine_transition.sql`, `person_wait.rs` / `needs_drew.rs`
(FDR-0013 hands/person-wait handling), `backoff.rs`, `home.rs`, `token_file.rs`,
`uds_http.rs`, `secrets_broker.rs`, `log.rs`, `work/`, `bin/` + `bin_args.rs`.

Backend submodules (`src/turso_queue/`): `core.rs` (wire types, schema, reads,
events, sweep), `claim.rs` (enqueue/revive/claim SQL and lane decision),
`lifecycle.rs`, `admin.rs`, `stacks_outbox.rs` (route dispatch incl. `/repair` and
the placement routes), `placement_sessions.rs`, `repair.rs`, `staleness.rs`,
`work_identity.rs`, `label_enrich.rs`, `closed_bound.rs`.

### 3.2 The Turso backend shape

From the header of `src/turso_queue.rs` (verbatim facts):

- The queue core (tasks / leases / lanes / workers / checkpoints /
  repo_task_lane_ledger / events) lives on a **dedicated Turso database
  `fleet-queue`, org `dezren39`, group `great-star-lord`**; the read-write token
  is held in BWS under the name `turso.fleet-queue-rw-token` (name only; no value
  read in this pass). It is **the queue broker's only backend**.
- URL: env `QUEUE_BROKER_TURSO_URL`, default the fleet-queue HTTPS URL — the
  running broker logged it as `https://fleet-queue-dezren39.aws-us-east-1.turso.io`
  (§3.5). Token: env `QUEUE_BROKER_TURSO_TOKEN_FILE`, else fetched from the
  secrets broker by the BWS name above. The client reaches the broker over the
  broker protocol (direct broker URL / broker Unix socket, else the session
  proxy); the broker talks libSQL-over-HTTPS (ureq) to Turso — there is no local
  SQLite replica of the queue itself (rusqlite is used for the #8583 local
  coordination replica on another surface, per Cargo.toml's comment).
- Schema and semantics were reconstructed from the recovered Postgres function
  bodies (pm-stack migration-0021…0024: `claim_repo_task`, `fail_repo_task`,
  `park_task`, `expire_leases`, lane assign/bind) plus the broker/client contract;
  deviations are listed in the cutover runbook (`doc/fleet/queue-turso-cutover.md`,
  cutover executed 2026-11-07T05:10Z — cited from the designs research file, not
  re-read in this pass).
- **Atomicity:** libSQL is single-writer. A claim is **one conditional UPDATE**
  whose WHERE clause re-checks claimability (status in queued/parked with a due
  `not_before`, attempts left) against the row a subquery picks — the analogue of
  PG's `FOR UPDATE SKIP LOCKED`. A competing claim that loses matches no row and
  returns `{row: null}` (client exit 5).
- **agent.events (#5690):** queue-transition writers are the backend's `event()`
  calls in the same dispatch/database as the task write; non-queue writers and
  consumers use broker verbs `/events-append`, `/events-seen`, `/events-count`,
  `/events-prune`.

Schema: `SCHEMA: [&str; 19]` in `turso_queue/core.rs`, applied with
`CREATE TABLE IF NOT EXISTS` by `ensure_schema` on broker start. Tables:

| Table                                         | Shape (columns of note)                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                |
| --------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `tasks`                                       | `id` PK autoincrement; `queue, title, body, payload (TEXT JSON, default '{}'), dedupe_key, priority, status (default 'queued'), not_before, worker_id, claimed_at, lease_until, lane_id, platform, attempts (default 0), max_attempts (default 1000), claim_provider/claim_model/claim_variant/claim_session_id, progress (JSON), result (JSON), last_error, park_kind, wait_kind, ready_condition, required_budget_minutes, last_checked_at, check_outcome, created_at, updated_at, completed_at, blocked_deps, dep_next_recheck_at, dep_blocked_until, dep_recheck_seconds`. Indexes: `tasks_queue_dedupe` on `(queue, dedupe_key)` — deliberately **not unique** (pre-enforcement PG data carries duplicate pairs; enqueue dedupe is a read-then-insert check) — and `tasks_claim_scan` on `(queue, status, priority, created_at)`. |
| `leases`                                      | `task_id` PK, `worker_id`, `lease_until`, `heartbeat_at`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                              |
| `lanes`                                       | `lane_id` PK, `worker_id`, `queue`, `enabled`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         |
| `workers`                                     | `worker_id` PK, `role`, `display_name`, `status`, `last_heartbeat_at`, `created_at`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                   |
| `checkpoints`                                 | `(worker_id, key)` PK, `value`, `updated_at`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `repo_task_lane_ledger`                       | one row per task (`task_id` UNIQUE): assigned/executed lane, exclusion/opencode reasons, platform limit, session_id, fallback reason, timestamps.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `events`                                      | `id` PK, `worker_id, task_id, kind, detail (JSON), created_at` — the timeline store (§3.3); indexes on `(task_id, id)`, `(kind, created_at)`, `(created_at)`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| `creation_outbox` + `creation_outbox_circuit` | GitHub creation outbox: `id` PK, `repo, kind, payload, dedup_key, payload_hash, producer, state (default 'pending'), available_at, attempt_count (max 8), last_error_class, lease_owner/lease_until, github_number/github_url, completed_at`; unique index on `(repo, dedup_key)`; circuit-breaker singleton row for consecutive rate limits.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                          |
| merge-intent table                            | DDL of record in `merge_intent.rs` (`MERGE_INTENT_DDL`, #8582 increment 2) — backs `queue stack`.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                      |
| `placement_sessions`                          | §3.4.                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                  |

### 3.3 Task states, lifecycle, timeline

From `turso_queue/core.rs` (verbatim constants):

```rust
pub const LIVE_STATUSES: [&str; 5] = ["queued", "claimed", "parked", "handed_off", "blocked"];
pub const TERMINAL_STATUSES: [&str; 3] = ["done", "cancelled", "failed"];
```

So the state machine has **8 stored statuses**: live `queued → claimed` (with
`handed_off` as the claimant-sink return state), waits `parked` (timer/condition/
person waits, discriminated by `park_kind`/`wait_kind`/`ready_condition`) and
`blocked` (dependency/person blocks, with `blocked_deps` + recheck scheduling),
and terminal `done` / `cancelled` / `failed`. Quarantine is **not** a status: a
malformed packet discovered at claim time is forced to `blocked` with
`last_error = "quarantined by claim path: packet failed validation (#5444): …"`
plus a `blocked` event (see `claim_guard_reason` / quarantine path in
`turso_queue/claim.rs`). The broker's `health` verb additionally reports derived
buckets `malformed_parked` and `dep_blocked` (observed in live `fleet queue
health` output in this pass).

Claim-path facts worth recording (from `claim.rs` comments/tests): the claim
lease default is 2,700 s (#7617 — `lease_until` is stored identically on the
`tasks` row and the `leases` row); the claiming session's model is stamped on the
row (`claim_provider/model/variant/session_id`, #8176/#9046) and appended as an
event; an expired claim returns to `queued` or to the `handed_off` sink depending
on checkpoint progress (#8184); session-to-session transfer claims a task directly
with no release-and-reclaim gap (#8210).

**Timeline:** the `events` table is append-only (core.rs: "this module exposes no
update or delete" for the append path; pruning is a separate verb). Every verb
writes its transition through `event()` in the same dispatch as the task write.
`verb_events_list` (#9046) reads one task's events oldest-first via the
`events_task` index — that is what `fleet queue timeline`/`history` and `show`
render. Retention: `prune_events` deletes events older than `keep_days`
(default 30, clamped 1..=3650) and self-logs an `events_pruned` row; it runs from
both `/events-prune` and the broker sweep, alongside lease expiry and the
dep/quarantine recheckers.

### 3.4 Dedupe attestation and the repair module; placement_sessions

**Dedupe attestation.** `validate.rs` defines the one repairable defect as a named
constant (verbatim):

```rust
pub const DEDUPE_STAMP_ERROR: &str =
    "payload.dedupe: must have checked_at and basis as non-empty strings";
```

`validate_packet` (the single oracle shared by `/enqueue`, the claim guard and
repair) requires `payload.dedupe.checked_at` and `payload.dedupe.basis` to be
non-empty strings, alongside the dedupe-key family rules: `packet.dedupe_key` must
match a design pattern and agree with the payload (repo, issue/PR numbers, kind
family, slice/pass ordinals — `dedupe_errors` in `validate.rs`, ~20 distinct error
strings; canonical-spelling rules reject leading-zero numbers as a second dedupe
identity, #5510). Work identity — the `(repo, issue, kind)` triple extracted by
`turso_queue/work_identity.rs` — is the second dedupe axis: enqueue and claim
refuse a second live task for the same work (#10627), with the #13091 amendment
that a **completed** holder also absorbs re-filings (§4.1).

**Repair (`turso_queue/repair.rs`, #13191 — on `main` only).** Module header,
condensed verbatim: repair runs `validate_packet` and fixes only defect classes
whose correct value can be re-derived by performing the check the field attests
to — today exactly one class, the dedupe stamp. It re-performs the dedupe check
live (no other live, claim-guard-clean task may hold the same work identity
#10627 or the same dedupe key; a quarantined row never counts as a holder), and
on success stamps:

```rust
pub(crate) const REPAIR_DEDUPE_BASIS: &str = "fleet queue repair: dedupe re-checked live at repair time; no other live task holds this work identity or dedupe key";
```

with `checked_at` = repair time. A live duplicate **refuses** the repair
("stamping anyway would be a false attestation"). Anything else is unrepairable:
the verb fails, lists the defects, mutates nothing. Repairs are computed and
re-validated in memory first; the write is one status-guarded UPDATE plus a
`repaired` timeline event naming verb, operator, defects and repairs. State
rules: eligible `blocked | queued | parked | handed_off`; `claimed` is refused
(never mutate under a worker); `cancelled | failed` point at `revive`; `done` is
refused. On success a blocked task returns to `queued`, or stays `blocked` with
its dep record intact. Wiring: broker route `"/repair" => verb_repair` in
`turso_queue/stacks_outbox.rs`; client `cmd_repair` in
`client/commands_lifecycle.rs` calling `/repair`, usage
`repo-queue repair <task-id> [--worker <id>] [--reason <text>] [--dry-run]`
(`client/support.rs`). The same PR made `unblock`'s full-override path refuse a
still-quarantined row and point at `fleet queue repair <id>` (`turso_queue/admin.rs`).

**placement_sessions (PR #13169, #13146 slice 1).** DDL in `core.rs` SCHEMA
(verbatim):

```sql
CREATE TABLE IF NOT EXISTS placement_sessions (
    handle TEXT PRIMARY KEY, record TEXT NOT NULL DEFAULT '{}',
    state TEXT, surface TEXT, submitted_by TEXT, run_id INTEGER,
    created_at TEXT NOT NULL, updated_at TEXT NOT NULL);
CREATE INDEX IF NOT EXISTS placement_sessions_created ON placement_sessions(created_at);
```

`record` holds the full client session JSON verbatim — "the client record shape
is the contract; the scalar columns are extractions for list/filter queries".
Module `turso_queue/placement_sessions.rs` implements four broker verbs, routed
in `stacks_outbox.rs`: `/placement-put` (upsert by handle — a seat refreshing
state is the authority), `/placement-get`, `/placement-list` (newest first,
limit clamped 1..=500, optional `submitted_by` filter), `/placement-sync`
(`INSERT OR IGNORE` batch, max 500 records/call — the migration path; it can
never overwrite a backend row). Handles must be `fp_`-prefixed; records require
`created_at`/`updated_at`. Client side: `placement/remote.rs` (`SessionRemote`
trait, `BrokerRemote` at a 5 s budget, `NullRemote`, `FLEET_PLACE_REMOTE=off`
escape hatch) over a bounded transport helper (`broker_roundtrip_bounded` in
`client/transport.rs` — caller-set budget on every leg, no broker auto-spawn,
errors returned, never a process exit). The local JSON store stays the **only**
input to placement decisions; the backend is a best-effort mirror (local commit
first, then put; reads take the newer of local vs backend by `updated_at`).

### 3.5 Deployment: the queue-broker profile on the laptop

Verified via laptop-ssh in this pass:

- Profile: `~/.local/share/deploy/profiles/queue-broker/current` →
  `/nix/store/0ymw7b3zl2hxij9npmzal6h5fq2g4cp3-fleet-0.1.0` (symlink dated
  2026-11-23 03:08 local). The profile directory contains only `current` — no
  generations log was present at that path in this pass **[the append-only
  generation log described in book `development/release-and-rollout.md` was not
  located under the profile dir; unverified where it lives]**.
- The profile's binary reports source rev **b8bbcc0c49564ab31db8d58b026b67532db9491e**
  (derived `0.1.0-dev+gb8bbcc0`) — i.e. the running broker predates both the
  #13091 fix merge (10:04Z) and the #13191/#13146 merges (11:04–11:14Z) in
  _client_ terms; note however §4.1: the #13091 fix is broker-side code, and the
  coordinator's census records this generation as "gen 5 / b8bbcc0" carrying the
  `unblock` / evidence-gated `complete` verbs. Whether the b8bbcc0 broker build
  contains the #13091 broker-side fix is determined by its commit date relative
  to the fix commit `f7f45d67925e` — **[unverified in this pass; the fix's own
  merge was 2026-11-23T10:04:34Z and the profile symlink is dated 03:08 CST =
  09:08Z, which predates the merge — so on timestamps the running broker does
  NOT yet carry the #13091 fix]**.
- Service: `queue-broker.service` (user unit, "deploy target queue-broker (issue
  #5766)"), active/running since Mon 2026-11-23 05:38:40 CST at last check,
  main PID running `…/current/bin/fleet serve queue-broker`. Startup log lines:
  `queue-broker: listening on 100.99.113.103:4801` (tailnet address),
  `queue-broker: listening on uds /run/user/1337/fleet/queue-broker.sock`,
  `queue-broker: queue backend: turso (https://fleet-queue-dezren39.aws-us-east-1.turso.io)`.

---

## 4. Known queue defects — verified via `gh api` REST

All statuses below were read from `gh api repos/developing-today/code/…` in this
pass (2026-11-23, after 12:35 CST / 18:35Z the coordinator's census used a
~12:40Z read; the GitHub timestamps quoted are UTC as returned).

### 4.1 code#13091 — double-serve defect + fix PR #13143

- **Issue:** #13091 _"fix(queue): broker re-serves parked and completed tasks
  (double-serve)"_ — **state: closed**, labels `[status:done]`, created
  2026-11-23T08:05:53Z, updated 2026-11-23T10:12:41Z.
- **Defect as filed:** at least five re-serves on 2026-11-23 — task 21099 served
  to two lanes in parallel (#10246); task 21120 served after PR #12899 merged;
  task 20671 re-served after the #10639 slice's PR #12781 merged; task 21123
  (#10359) re-served after a recorded park + stand-down; task 21296 (#10283)
  served as a duplicate of completed task 17123 (PR #13075 landed, issue closed)
  and completed a second time through the disposition gate. Two later occurrence
  comments add: task 21302 served by `fleet agent next` in the same window the
  dispatcher recorded it **cancelled** (the lane's park attempt was answered
  "task 21302 is not claimed by archer: it is cancelled"), and task 20671 served
  again while its issue carried `queue/parked` + `status/parked`.
- **Fix PR #13143:** _"fix(queue): broker never re-serves condition parks or
  completed work (#13091)"_ — **state: closed, merged: true, merged_at
  2026-11-23T10:04:34Z**, head `xaltha/13091-broker-double-serve` @
  **`f7f45d67925e37149416b4a04d72be01475a80d5`**, base `main`.
- **Root cause / fix shape** (from the fix comment on #13091, 2026-11-23T09:09:38Z,
  and the merged code's comments in `turso_queue/claim.rs` / `work_identity.rs`):
  (1) `verb_park` stamps a default `not_before` (+1200 s) on every park and the
  default pick released _every_ park on that timer — including condition parks
  owned by the park-revalidation sweeper (`task_terminal`/`task_status`/
  `event_seen`) and person parks owned by `/wake` (`person_answer`); task 21123's
  ~23-minute re-serve cycle matches the timer. Fix: a `PARKED_TERM_EXCLUSION`
  ANDed into the pick predicate's parked branch (mirrored in the diagnostics).
  Genuine waits keep timer release (FDR-0008). (2) A `done` row is unservable but
  re-filing manufactured a fresh servable row: revive-on-enqueue fired for any
  terminal predecessor and the #10627 identity rule ignored terminal holders.
  Fix: enqueue treats a completed holder as absorbing the re-filing (in code:
  `completed_identity_holder` in `work_identity.rs`; tests
  `enqueue_after_completion_on_the_same_key_files_no_new_task_13091` and
  `…_under_another_key_…` in `turso_queue.rs`).
- **Deployment caveat:** per §3.5, the running laptop broker is still the
  b8bbcc0 generation whose profile symlink (09:08Z) predates the fix merge
  (10:04Z) — so the merged fix was **not yet the running broker build** at the
  time of this pass **[timestamp inference, flagged in §3.5]**. This matches the
  coordinator's census note that lanes still observe stale serves and that
  broker-generation lag is the open question.

### 4.2 code#13191 — the packet-repair verb: NOT still in flight; merged, not deployed

- **Issue:** #13191 _"fleet queue: add a repair verb for quarantined/malformed
  task packets (in-place payload fix)"_ — **state: closed**, labels
  `[status:done, kind/feature, app/fleet]`, created 2026-11-23T10:30:57Z,
  updated 2026-11-23T11:14:32Z. (The task brief asked whether it is "still in
  flight" — it is not; it closed the same morning it was filed.)
- **Origin:** queue task 18533 (for #7709) was quarantined because its
  `payload.dedupe` lacked `checked_at`/`basis` (validation #5444); every claim
  re-quarantined it, `quarantine-recheck` could only diagnose it
  (`still_quarantined`), and no verb could repair a payload in place — the only
  disposition was cancel-with-disposition (done 2026-11-23, recorded on #13091).
  Task 18533's cancelled state was explicitly left standing by the issue.
- **PR:** search `repo:developing-today/code 13191 type:pr` returns exactly one:
  **PR #13218** _"fleet queue: add repair verb for quarantined/malformed task
  packets"_ — **state: closed, merged: true, merged_at 2026-11-23T11:14:22Z**,
  head `lane-repair-13191` @ `b8abe089efbad7ce2608ab1ead7fdf057b92576a`; merge
  commit on main `ab10437c6b7` ("Merge pull request #13218"), implementation
  commit `2aaadb82aa6` (both from laptop `git log`). The PR body records the
  contract (§3.4), seven named-oracle tests (all passing), the `unblock` audit
  fix, and one pre-existing unrelated failure on main filed as #13215
  (`sweep_wiring_runs_the_detective_beside_the_recheckers`, whitespace-sensitive
  source assertion; #13215 is itself now closed per `gh api`).
- **Rollout gap (the live finding):** the verb is merged but **absent from every
  shipped binary checked**: sandbox seat binary 044a6bd — `unknown command
"repair"` (first-hand, this pass; the coordinator's census independently found
  the same on `fleet-static-3cb241a`); deployed laptop broker binary b8bbcc0 —
  `unknown command "repair"` (first-hand via a correctly-named symlink to the
  profile binary, this pass). The book also lacks it (§2.3). The pending step is
  the publish-fleet + bootstrap-pin rollout that puts a post-#13218 build on the
  broker profile and the seats.

### 4.3 code#13146 — placement improvement: criterion 1 still pending

- **Issue:** #13146 _"impl(fleet): placement sessions to the queue backend
  (placement_sessions) — centralize decision state, keep placement
  decentralized"_ — **state: open**, labels `[status:in-review]`, created
  2026-11-23T09:14:24Z, updated 2026-11-23T11:05:27Z. Drew's directive as filed:
  dispatch stays decentralized (each seat's binary decides locally); only the
  decision state (the `fp_…` session records, today only in per-seat
  `~/.local/state/fleet-place/sessions.json`) is centralized into the Turso
  queue backend.
- **Slice 1 landed:** PR #13169 _"feat(fleet): mirror placement sessions to the
  queue backend (placement_sessions) — #13146 slice 1"_ — **closed, merged:
  true, merged_at 2026-11-23T11:04:50Z**, head `lane-placement-sessions` @
  `eca57d1b5ea7471930e1ffd687548398fd797a73`, merge commit `15b2ac834b0d`.
  Shipped: the `placement_sessions` table + four broker verbs, the bounded
  transport helper, the client mirror with local fallback and opportunistic
  migration, and the runbook update (§3.4). Verification recorded on the issue:
  full fleet-queue suite 556 passed / 0 failed on the laptop worktree; the
  remote dispatch route was unusable for Rust (fleet-build image has no cargo)
  — filed as #13170.
- **Criterion 1 status — PENDING (the reason the issue stays open).** Acceptance
  criterion 1 reads: _"Dispatch on seat A; `fleet queue session status <fp_
handle>` on seat B (empty local store) returns the session."_ The slice-1
  comment on #13146 (2026-11-23T11:05:27Z) records it verbatim as: _"Cross-seat
  status by handle — PENDING the normal binary rollout (broker gains the verbs
  and seats gain the client from main …). This issue stays open for that
  record."_ Criteria 2 (broker down = today's behaviour), 3 (migration never
  overwrites), 4 (decisions unchanged) and 5 (named-oracle tests) are recorded
  as covered by tests in the same comment. Next slices, in order: live
  cross-seat verification once the rollout lands; then the conditional
  Blacksmith minute ledger; then hard cap enforcement. Note the rollout this
  waits on is the same one §4.2 waits on.

### 4.4 The packet-defect class — the "~30 malformed-packet issues" list: not found as a single list

The brief asked for the list of ~30 malformed-packet issues "if recorded on an
issue", with a citation. **No such single list was found** in this pass —
labelled **[unverified / not located]**. Searches performed (all `gh api`
search/issues on developing-today/code): `malformed packet`, `quarantined
packet`, `"dedupe" "checked_at"`, `"quarantined by claim path"`, `packet
validator`. None returns an issue enumerating ~30 malformed-packet tasks/issues.
What _is_ recorded, and should not be conflated with the missing list:

1. **code#9609 (closed)** — the largest documented quarantine census: _"279 tasks
   quarantined at claim"_ (fresh DB read 2026-11-13 ~23:10Z), split **270 rows**
   `packet.max_attempts: must be an integer between 1 and 100` (every row
   carrying `max_attempts=1000` from the #8973 bulk raise, against the #6448
   validator cap of 100) and **9 rows** `payload.schema: must be 'repo-task/v1'`
   / missing `payload.problem`. Follow-ups: #9621 (validator ceiling raised
   100 → 1000, closed), #10648/#10652 (max_attempts single source of truth +
   quarantine recheck, closed), #10886 (separate quarantine budget / recheck
   window rotation, closed).
2. **code#13191 (closed)** — the dedupe-stamp subclass: the worked example is
   task 18533 (#7709), and the issue states only qualitatively that _"the
   blocked pool likely contains more rows in this class"_ — no enumeration.
3. **code#13091 cleanup comment (2026-11-23T10:10:41Z)** — a disposition table of
   **ten** stale blocked tasks (15977/#9451, 16048/#9501, 17396 + 17397/#10412,
   18533/#7709, plus 16081, 19158, 19174, 19207, 20502, 21101, 21143 named in the
   correction comment) — a cleanup batch, not a malformed-packet census; of
   these only 18533 is recorded as a packet defect (the dedupe stamp).
4. **Live queue state (this pass):** `fleet queue health` reported
   `malformed_parked = 0` on every queue, while `repo-health-code-pilot` alone
   carried 384 `blocked` + 165 `dep_blocked` of 13,407 total tasks — i.e. at
   read time there was no standing malformed-parked population for a ~30-item
   list to describe. **[The ~30 figure may come from a lane record, a queue
   snapshot, or a comment thread outside the searched set; it is not cited
   here because it was not found.]**

Related packet-defect history for orientation (all closed unless noted):
#5444 (claim could hand out a payload-null task — the quarantine mechanism's
origin), #7673 (claim never hands out a payload-less/invalid packet),
#6430 (packet string fields never type-checked), #10714/#10766 (lane blocks vs
the quarantine rechecker), #11802 (no second live task for the same work,
#10627), PR #12743 (**open** — "make coverage merge-watch packets
validator-safe": a live dry-run packet rejected for an unrecognized
`merge-watch` dedupe key, then for missing `payload.prs`).

---

## 5. Headline facts and open items

1. **Three revisions in play:** sandbox seat binary `044a6bd`, deployed laptop
   queue-broker `b8bbcc0`, laptop `main` source at `cafdfdc…`. Any statement
   about "the fleet binary" must name which one it means.
2. **The book is complete for b8bbcc0 and stale for main:** 55 chapters /
   98,651 words of chapter text; it misses `fleet queue repair` entirely and
   still describes placement sessions as a local-JSON stopgap.
3. **`fleet queue repair` (#13191) is merged (PR #13218, 2026-11-23T11:14:22Z)
   but deployed nowhere** — both shipped binaries answer `unknown command
"repair"`. Same rollout gates #13146 criterion 1 (cross-seat session
   status), which is the stated reason #13146 remains open
   (`status:in-review`).
4. **#13091 is closed with PR #13143 merged** (head `f7f45d67925e`), but on
   timestamps the running broker profile (symlink 09:08Z) predates the merge
   (10:04Z) — broker-generation lag is the plausible explanation for any
   post-merge double-serve observations **[timestamp inference]**.
5. **Queue backend:** one Turso database (`fleet-queue`, group
   `great-star-lord`), 19-statement schema, 8 stored task statuses
   (5 live + 3 terminal), quarantine as `blocked` + `last_error` rather than a
   status, append-only `events` timeline, claim as a single conditional UPDATE.
6. **Not located:** a single recorded list of ~30 malformed-packet issues
   (§4.4); the documented censuses are #9609's 279-row split and #13191's
   unenumerated dedupe-stamp class.
