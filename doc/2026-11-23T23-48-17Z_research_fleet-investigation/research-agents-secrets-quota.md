# Fleet investigation — Agents, secrets, bridge, laptop-agent, quota plane

Research file for the fleet program investigation. Coordinator brief: agents / secrets / quota slice.

- **Checkout read:** laptop `~/git/code` on `main` at `a66148f7393` (Merge PR #13253) via `~/tools/laptop-ssh.sh` (which itself is now a redirect to the laptop-agent path — see §5). All Rust paths below are `pkgs/fleet/crates/…` in `developing-today/code` unless stated otherwise.
- **GitHub reads:** `gh` CLI only (GitHub connector banned per standing rule), REST via `gh api repos/developing-today/code/…`. Paced, read-only. No repo was modified, no worktree created, no comment posted.
- **Secrets:** by name only. No value is quoted anywhere in this file.
- **Labelling:** a claim marked **[unverified]** could not be confirmed from code, book, or a fresh `gh` read in this pass.

Headline shape: `fleet` is one multicall Rust binary (`crates/fleet`, `fleet-agent`, `fleet-bridge`, `fleet-secrets`, `fleet-queue`, `fleet-config`, `fleet-registry`, `fleet-laptop-agent`, `fleet-proxy`, `fleet-dispatch`, `fleet-search`). `fleet agent` adds no transport of its own — it composes the bridge, queue, GitHub and laptop-agent verbs (`fleet-agent/src/lib.rs` module doc). The book under `pkgs/fleet/book/` is a faithful secondary source and is cited where it states something more compactly than the code; where book and code could diverge, code wins.

---

## 1. Agent lifecycle & enrollment

### 1.1 Kinds — one table, 16 rows

Source: `fleet-agent/src/kinds.rs` (`pub const KINDS`), rendered by `fleet agent kinds`.

A kind says three things (file doc comment): which bridge groups the agent joins, how it reaches the fleet from where it runs (its *seat*), and what to tell it at enrolment. “Adding a kind is adding a row here; no verb changes.”

Seat shapes (`enum Seat` in the same file):

| Seat id | Meaning (from `kinds::table()`) |
|---|---|
| `binary` | Runs the fleet binary itself: `fleet agent enroll`, then `fleet agent next` |
| `http` | No shell: speaks the bridge's fleet routes over HTTPS; work arrives in its mailbox |
| `connector` | A chat assistant with the Message Bridge connector; addressed by channel, no mailbox |

The 16 kinds (name — groups — seat):

- `muse` — `[muse]` — binary. “A Muse lane or a persistent Muse agent.” Note: a fresh Muse gets a bridge-capable static build from `scripts/muse/bootstrap.sh`; where the installed binary predates `fleet agent`, run the same commands on the laptop install via `~/tools/laptop-ssh.sh 'fleet agent next --agent <lane>'`.
- `opencode` — `[opencode]` — binary. Runs on the laptop seat, so every leg is local.
- `openchamber` — `[opencode, openchamber]` — binary. Joins `opencode` too, so a send to `group:opencode` reaches both pipelines.
- `openclaw` — `[openclaw]` — binary.
- `claude-code` — `[claude-code]` — binary. A cloud session has no tailnet: messaging and the priority rung work over HTTPS; builds go through `fleet shell`.
- `codex` — `[codex]` — binary.
- `gemini` — `[gemini]` — binary.
- `antigravity` — `[antigravity]` — binary.
- `grok` — `[grok]` — binary. Note verbatim: “A bot with no shell enrols as kind `http` under the same name and groups.”
- `dot` — `[dot]` — http. `pkgs/dot-bridge` carries its Slack and ChatGPT transports; the mailbox is the same store.
- `duet` — `[duet]` — http.
- `claude` — `[]` — connector, fixed address `claude`. “Set a wake hook for channel:claude on the bridge page so a send starts a session.”
- `chatgpt` — `[]` — connector, fixed address `chatgpt`. “Woken by its MCP Events subscription.”
- `laptop` — `[laptop]` — binary. A job or service on the fleet laptop.
- `http` — `[]` — http. Anything that can make an HTTPS request and nothing else.
- `generic` — `[]` — binary. Any other agent with a shell; the fallback the enrolment error names.

Fleet targeting consequence (standing context, consistent with this table): the kind is the type/group (`muse`), the enrolment `--name` is the individual instance. A send to `group:muse` addresses the type; `agent:<name>` addresses one instance. See §4.

### 1.2 `fleet agent enroll` — what it actually does

Source: `fleet-agent/src/enroll.rs` (`pub fn enroll`, `pub fn doctor`, `pub fn probe`, `pub struct Profile`).

Enrolment is “three things, done in one command” (file doc):

1. **Register the mailbox** on the bridge under its name and groups — concretely a `fleet bridge hello --agent <name> [--groups a,b]` call (the *Register* probe mode).
2. **Probe which legs answer from this seat** (see below).
3. **Save a small profile** so later verbs know who is asking without being told.

Mechanics, verified in code:

- Required: `--kind` (must be in `KINDS`, else usage error listing all kinds, “use generic for anything with a shell”) and, for non-connector kinds, `--name` (validated by `check_name`).
- `--groups a,b` *adds* to the kind's built-in groups (deduped).
- Connector kinds short-circuit: no mailbox, no profile; the output is the fixed `address`, `registered: false`, and steps (“Attach the Message Bridge connector…”, the kind note, and the central `fleet bridge send --to <address>` lines).
- Profile path: `--profile`, else `$FLEET_AGENT_PROFILE`, else `$XDG_CONFIG_HOME/fleet/agent.json`, else `~/.config/fleet/agent.json`. Profile JSON: `{name, kind, groups, enrolled_at, gh?}` where `gh` is the GitHub route that answered at enrolment (`fleet` or `plain`). Only a `binary` seat on a non-dry-run saves a profile — “an http agent is enrolled from someone else's seat and must not take it over.”
- `--dry-run` probes in *Read* mode (`bridge inbox --agent <name> --limit 1`) instead of registering, saves nothing, and always exits 0.
- `registered` in the output is `!dry_run && bridge leg ok`. It is computed at enrol time from the bridge leg only and is **not** stored in `agent.json` (also stated in code#12898's body — a grok enrolment could exit 0 with `registered: false` and no bridge registry row).
- Exit code: 0 if dry-run **or any intake leg answers**; else 3 (`EXIT_DOWN`). “An agent is enrolled when at least one rung can bring it work.” A failed leg “narrows the intake ladder, it does not fail the enrolment.”

The four legs (`probe()` — issued together via `world.fleet_many`):

| Leg | Probe verb | “Needs” (verbatim string in code) |
|---|---|---|
| `bridge` | Register: `bridge hello`; Read: `bridge inbox --agent A --limit 1`; Service (no name): `bridge health` | “HTTPS to the message bridge, and the fleet token by name through fleet secrets” |
| `queue` | `queue health` | “the queue broker: its socket on the laptop, or the laptop transport from elsewhere” |
| `github` | `gh api rate_limit` via `fleet gh`; if that fails, plain signed-in `gh api rate_limit`; answer records `route: "fleet"\|"plain"` | “gh with a token: fleet gh's rotation, or a signed-in gh” |
| `shell` | `laptop-agent health` | “a route to the laptop: its broker, the Cloudflare tunnel, the tailnet or ssh” |

There is **no secrets leg** — secrets is folded into the bridge leg's stated need (book `concepts/agents.md` calls out that any text saying “bridge/queue/secrets/shell” is wrong; the source is explicit).

With no agent name, the bridge leg is reported **not ok** even if `bridge health` answers: “the service answers, but no agent name is known to read a mailbox with: enrol, or pass --agent.” Service health proves nothing about holding the fleet token; only a call made *as* an agent does (this is the #12243 fix referenced by code#12236 item 4).

Derived rungs (`rungs()` in `enroll.rs`): `push = bridge leg`, `queue = queue leg`, `shortlist = true` (always — it is a local file), `priority = github leg`.

Output extras by seat: binary seats get `run_loop` (`fleet agent next --agent <name>` / “do the task it returns, then run its `then` lines” / “repeat while the run has budget”) and a `builds` line (`fleet shell -- <command>` — “runs where builds belong; never build in a limited sandbox”); http seats get the token-by-name note (Bitwarden secret name `message-bridge.fleet-token`), the reference client `python3 scripts/fleet/bridge-mailbox.py inbox --agent <name> --commands --take`, and the four raw HTTPS routes (`POST /fleet/<token>/hello`, `GET /fleet/<token>/inbox`, `POST /fleet/<token>/send`, `POST /fleet/<token>/ack`).

`fleet agent doctor` is the same probe without registration: agent from `--agent`, else `$FLEET_AGENT`, else the saved profile; bridge probe in Read mode (or Service mode with no name); exit 0 iff any of bridge/queue/github answers; plus the `builds` line (“fleet shell reaches the laptop” vs “…will try a codespace, then run here”).

Tutorial form: `pkgs/fleet/book/tutorials/enrol-a-seat.md` (six steps: bootstrap → token files → bridge hello → enrol → doctor → first `next`).

### 1.3 `fleet agent next` — the intake ladder (rung order)

Source: `fleet-agent/src/next.rs`. This is the authoritative rung order; the file doc comment states it directly:

1. **push** — a command addressed to this agent on the bridge.
2. **queue** — a claim on the one fleet queue.
3. **shortlist** — the top of this host's local shortlist; or, when the list is low and nobody is refilling, a purpose-switch to rung **`refill`** instead of a task.
4. **priority** — the highest-priority open issue, straight from GitHub labels.

Every answer is one JSON object `{agent, rung, task, then, tried}`. `tried` is the per-rung log with result `skipped | empty | down | rejected` and a reason — down ≠ empty is a deliberate distinction. Exit 0 with a task; exit **5** (`EXIT_NO_WORK`, deliberately equal to the queue client's no-claim code) when no rung had work; 2 usage; 3 a needed leg down. Flags `--no-push/--no-queue/--no-shortlist/--no-priority` skip individual rungs (recorded as `skipped`). Agent identity resolves `--agent` → `$FLEET_AGENT` → saved profile, else a usage error (“who is asking?”).

**Rung 1 — push.** `fleet bridge inbox --agent <agent> --commands --take --limit 1`. `--take` marks what it returns read, so the command is consumed by exactly the hand-out that returns it. Task: `{command_id, instructions: <body>, message}`. `then`: `fleet bridge post --agent <agent> --reply-to <id> "done: …"` / `"could not: …"`.

**Rung 2 — queue.** Builds `fleet queue claim --worker <worker> --lane <lane> --lease-seconds 14400`, carrying through any of `--platform/--pool/--remaining-budget-minutes/--filter/--queue` the caller passed, and runs it *patiently* (`fleet_patient` — “a claim changes the queue: it is given time to finish”).

- **Lease:** `AGENT_CLAIM_LEASE_SECONDS = 14_400` (4 hours). The comment cites code#12618: agent work runs 1–3 hours between claim and completion with no heartbeat, so the queue's 45-minute default (2,700 s) lapsed mid-task, the claim evaporated, and the worker's `complete` was refused. 4 h “covers the observed range with headroom and stays inside the queue's one-day maximum”; the hand-out names the lease so the deadline is visible.
- **Lane defaulting (current code, post-#12544):** `default_lane(agent)` in `next.rs`. If the agent name is already lane-shaped — `<family>-<lowercase-alpha suffix>` with family in `{code, any, not-code, opencode, openchamber, nix, darwin-nix}` — it is preserved as the lane. Otherwise the agent gets an **unfiltered `any-*` lane**: `any-<lowercase slug of the name, ≤20 chars><128 digest bits>` where the digest is SHA-256 of the agent name, first 16 bytes, each byte rendered as two `a`–`p` letters. The comment explains why: “Lane rows are bound to their first worker, so two worker names that normalize to the same slug must still get different lanes.”
- **Worker mapping:** `registered_lane_worker(lane)` maps a known lane to its shared queue worker — `not-code-a|g → repo-worker-nix`, `not-code-b|e → repo-worker-opencode`, `not-code-d → repo-worker-openchamber`, and for single-letter-suffix lanes `code|any|not-code → repo-worker-code`, `opencode → repo-worker-opencode`, `openchamber → repo-worker-openchamber`, `nix → repo-worker-nix`. Anything else claims as `--worker <agent>` itself.
- Task: `{queue_task_id, title, row, lease_seconds, instructions}`; instructions say to checkpoint as you go and to complete or park before the lease lapses. `then`: `fleet queue checkpoint <id> --worker <w> --lane <l> …` and `fleet queue complete <id> --worker <w> --lane <l> …`.
- **Lane verdicts:** if the claim fails with a `claim_repo_task: lane …` error (the broker's spelling for every lane-about refusal — belongs to another worker, disabled, bound to another queue, registration did not land; marker constant in `next.rs`, citing #6853 criterion 2), the rung is recorded as **`rejected`**, not `down` — “so a reader does not mistake a healthy broker's refusal for a store outage and go fix the wrong thing.” The verdict is searched in both spellings: the JSON envelope's `error` field and the plain error text.

**Rung 3 — shortlist / refill.** See §2. Before hand-out, the taken entry is **rechecked** against GitHub (`recheck()`): closed, held, busy, tracker, or hands items are dropped (“a hold, a claim or a close since then must win”); up to `MAX_STALE_ENTRIES = 5` stale entries in a row are dropped before the rung gives way to priority. If GitHub does not answer, the entry is still handed out with `rechecked: false` and a `recheck_note` telling the worker to read the issue for a hold first.

**Rung 4 — priority.** `candidates(world, repo, 12)`: for each label in `RANKED_LABELS` (soonest first), page GitHub issues (`per_page=30`, up to `MAX_PAGES_PER_RANK = 5` pages, `sort=created&direction=asc` — oldest first within a rank). The pick is **reserved** into the shortlist file in one locked update (`shortlist::reserve`) so two runs on the same host at the same moment cannot both take the same issue; a host that cannot write the file still gets the first candidate.

Filtering — one rule (`kept_out()`) shared by ranking and recheck:

- `HOLD_LABELS` (FDR-0012, “a person set them on purpose”): `held`, `user/owned`, `do-not-auto-edit` — kept out.
- `BUSY_LABELS`: `status/in-progress`, `status/in-review`, `status/parked`, `status/waiting-dependency`, `status/deferred` — kept out.
- `NOT_WORK_LABELS`: `kind/tracker` — kept out (indexes other work).
- Hands items (body contains `<!-- hands:v1 `) — kept out (FDR-0013: no agent can do them).
- Pull requests — kept out.
- **Owner parks are NOT holds:** `user/escalated` and `status/blocked` (`OWNER_PARK_LABELS`) are handed out like any other work, with the task instructions extended verbatim in effect: state what was asked, decide it, record the decision on the issue, remove `user/escalated` and `status/blocked`, and continue (FDR-0013; `.agents/skills/decide-and-proceed`). The file doc: “Nothing here treats a person as a gate.”

Label ranks: `RANKED_LABELS` for the priority rung's queries is `[expedite, priority/critical, now, priority/high, next]`; `rank_of()` in `shortlist.rs` orders `[expedite, priority/critical, now, priority/high, next, priority/normal]` as ranks 0–5, unranked = 6. Default repo: `DEFAULT_REPO` = `developing-today/code` (in `fleet-agent/src/lib.rs`).

### 1.4 Claim semantics in the queue store (below `next`)

Source: `fleet-queue/src/turso_queue/lifecycle.rs` (`claim_preamble`), `fleet-queue/src/turso_queue/core.rs` (`lane_provision_sql`, `lane_seed_sql`, `lane_family_of`), `fleet-queue/src/turso_queue/claim.rs`.

- **One queue.** The registry's per-lane queue column no longer scopes what a lane may claim (one-queue model, #8097, 2026-11-10). `claim_preamble` enforces the lane↔stream binding **only** when the claim itself names a stream *and* the lane's registration binds one (`lane_queue_binds`: a registration of `"*"` binds nothing). Identity is recorded, never used to filter an unscoped claim.
- **Auto-register ruling — implemented, not just policy.** Since **code#12544** (Drew, 2026-11-22), a claim naming a lane with no registry row **registers it on the spot** and the claim proceeds:
  - SQL (`lane_provision_sql`): `INSERT OR IGNORE INTO lanes (lane_id, worker_id, queue, enabled) VALUES (?, ?, '*', 1)` — claiming worker as owner, one-queue binding `"*"`, enabled.
  - `INSERT OR IGNORE` means an existing row always wins: a lane disabled by hand stays disabled, a lane owned by another worker keeps its owner. Those two cases still refuse, with `claim_repo_task: lane <id> is disabled` / `… belongs to worker <w>, not <claimer>`.
  - Any name may register — the `lifecycle.rs` comment lists “an agent name (`archer`), a bare family name, a name in no known family at all.” This supersedes the 2026-11-16 known-family wildcard (#6853), which provisioned only `<family>-<suffix>` ids in the seven `LANE_FAMILIES` and refused everything else as “unknown lane_id”.
  - The canonical seed (`lane_seed_sql`, #6853) still exists: `INSERT OR IGNORE` of the canonical lanes, so it restores a missing canonical registration but can never resurrect or re-enable a hand-disabled lane. `lane_family_of` survives only for deriving an execution record's mode (#7598), not for claim gating.
- **History of the defect:** code#12544 (closed) — originally `next` defaulted the lane to the agent name itself; `archer` was not in the canonical universe (six families × a–z = 156 lanes at filing), so every muse-kind claim failed with `claim_repo_task: unknown lane_id archer` and fell back to GitHub, while `doctor` still reported the queue rung healthy (it checks reachability, not claimability). Measured at filing: full `next` 17.8 s warm, ~100 s cold/refill-heavy. Fix landed as **PR #12558** “fleet-queue: auto-register unknown lanes on claim (#12544)”, merged 2026-11-22T13:32:02Z (verified via `gh api …/pulls/12558`). Standing context records the live verification the same day: a queue claim as `archer` claimed task 19479 with no lane error, against the laptop queue-broker running from its deploy profile. The current `next.rs` `any-*` digest-lane defaulting (§1.3) is the client-side half of the same settlement.
- **“Any agent can claim any job” — precise scope.** The ruling as implemented is about *lane identity*, not task requirements: a task's own declared requirements still filter claimers. `claim.rs` documents the pick predicate: `requires.lane` (a specific lane id) and `requires.model` in a task payload exclude non-matching claimers (`task requires lane <l>`), and tasks carry an integer `priority`. The claim pick itself is one conditional UPDATE whose subquery picks “priority, then age; parked rows whose `not_before` has passed resume without an attempts increment” (`claim_pick_once` comment), with separate race-loss and quarantine budgets (#9609 — they shared one 32-cap until 2026-11-16, when a ~460-packet defective intake wave made every claim spend its whole budget quarantining). `claim_preamble` also sweeps expired leases and quarantines payload-less rows before any pick.
- **Batch claims** exist at the store level (#8217: absent/1 = single claim, 2..=cap = batch) sharing the same preamble. **[unverified]** whether any current caller uses batch claims routinely — no caller was traced in this pass.

### 1.5 Known seats — what is verified, what is not

- **`archer` (kind `muse`) — verified as an enrolled agent name in code and issues.** `fleet-agent/src/tests.rs` runs `next --agent archer` and pins the expected claim line `fleet queue claim --worker archer --lane {default_lane("archer")} --lease-seconds 14400`. Code#12544's body is entirely about the `archer` seat's dead queue rung; code#12898 verifies the three token files on “the archer seat” (65 bytes, mode 0600, hash-identical to the laptop server files). Archer is Drew's Muse instance (standing context: name Archer, “first in line”).
- **`grok` (kind `grok`) — verified as a kind and as a real seat episode.** Kind row in `kinds.rs` (§1.1). Code#12898 records the grok seat's 2026-11-22 enrolment: bridge/queue/shell legs failing for missing token files (§3.4), enrolment exiting 0 on the GitHub leg alone with `registered: false`, and no `grok` row in `fleet bridge agents` at verification time (2026-11-23). Standing context adds the seat id `muse-grok` (kind grok, enrolled 2026-11-22) with tailnet + GitHub legs passing and bridge/queue/secrets/shell legs failing from the single token-file root cause. Whether the grok seat has since fully enrolled is **[unverified]** in this pass (no fresh `fleet bridge agents` read was taken — that needs a live owner-token call).
- **`felix` — [unverified] as an enrolled fleet seat.** No occurrence in `fleet-agent` or `fleet-bridge` source. The name appears in the repo as a branch/persona prefix (e.g. the quota Stage-3 design PR branch `felix/1177-stage3-design` cited in code#12694) and in a cron-registration issue title (code#12769, “register felix-fleet-machine-watchdog cron”). That is consistent with another agent persona operating in the fleet, but no enrolment record, kind, or groups were found. Do not state felix's kind/groups as fact.
- **`xaltha` — [unverified] as an enrolled fleet seat.** No occurrence in fleet source or book. The only hit in this pass is code#12813's title (“primary clone left on xaltha branch”) — a branch name, not a seat record.
- **The 16-lane floor is an operating rule, not code.** Drew's agent work loop (2026-11-22, escalated through the day to “minimum 16 running on this machine”, ~23:44 CST) is an Archer orchestration instruction recorded in standing memory. Nothing in `fleet-agent`, `fleet-queue`, or the placement code enforces a concurrency floor of 16; the queue's own caps are per-claim (lease, batch cap) and the session cap of 30 unfinished root OpenChamber sessions on the laptop is a separate standing rule. Treat “16 lanes” as policy layered on top of the ladder, never as a fleet invariant.

### 1.6 Tracking: code#12236

Code#12236 — “fleet agent: put every lane on the intake ladder (follow-ups to #12227)” — **open**, labels include `status:in-review`, `priority/high`, `next`, `app/fleet`, `epic/fleet-tool` (fresh `gh` read this pass). It is the follow-up tracker for the front door itself (#12227 landed `fleet agent` + `fleet shell`), **not** a shortlist-specific issue — the brief's pointer to it for the shortlist resolves to this: the shortlist's remaining fleet-wide work lives here as item 1.

Open items in its body: (1) every scheduled lane (Muse, OpenCode, OpenChamber prompts) calls `fleet agent next` in place of hand-written mailbox/claim steps, with schedule changes registered in `doc/fleet-schedule-registry.md` in the same PR; (2) wake hook → work (a wake on `agent:<name>` starts a run that calls `next`, so a pushed command is worked within two minutes); (3) one bootstrap script per seat kind (`scripts/muse/bootstrap.sh` is the model); (6) `fleet shell --target actions`. Items 4 (enrolment checks the token, not only the bridge — done in #12243) and 5 (trackers are not handed out — done in #12227) are struck through as done. “Done when” includes a week of lane runs showing every task coming from `next` with the rung recorded.

---

## 2. Shortlist

Source: `fleet-agent/src/shortlist.rs` (560 lines) and the rung-3 logic in `next.rs`.

### 2.1 What it is

A small local JSON file of ready work **on this host** — rung 3 of the ladder. “It is what one host should do next, not a second backlog.” The queue verbs neither read nor write it (book `concepts/agents.md`).

- Path: `--shortlist`, else `$FLEET_SHORTLIST`, else `$XDG_STATE_HOME/fleet/shortlist.json`, else `~/.local/state/fleet/shortlist.json`.
- File shape (version 1): `{version, entries[], refill, filled_at, handed[]}`. An entry is `{repo, issue, title, rank, why, labels, added_by, added_at}`. A file that does not parse is an error naming the path — “the list is never silently reset.”
- Concurrency: written whole through a temp file + rename + `sync_all`, under a lock file (`.lock` sibling; stale after 30 s; 100 tries × 50 ms). All read-modify-write goes through `update()` as one locked step.
- Handed-out records (`handed[]`) are retained for `HANDED_TTL_SECS = 2 hours`: an issue handed out here is not handed out again here within the window, and `add()` refuses an issue already listed or recently handed.

### 2.2 Size, low-water, ranks

- **`DEFAULT_TARGET = 8`** — the fill target. `shortlist fill --target` defaults to it and is clamped to 1..=50.
- **`DEFAULT_LOW_WATER = 3`** — `is_low` is `entries.len() <= low_water` (so refill triggers at 3 or fewer, not below 3). `next --low-water N` overrides per call.
- **Rank:** `rank_of(labels)` — `expedite` 0, `priority/critical` 1, `now` 2, `priority/high` 3, `next` 4, `priority/normal` 5, unranked 6. `take()` hands out the soonest entry: lowest `(rank, added_at, issue)`.

### 2.3 Refill — the purpose switch

The defining behaviour (file doc + `needs_refill()`):

- A run that finds the list low (`<= low_water`), with **no refill marker** and **no fill finished within the window**, does *not* take an entry. `next` sets the refill marker `(agent, now)` in the same locked update and returns rung **`refill`** — that run's work *becomes* filling the list.
- The marker lasts **`REFILL_FRESH_SECS = 30 minutes`**, so one run refills and the rest keep working. `filled_at` is recorded when a fill completes; a list still low just after a fill is low “because little work is eligible, not because nobody filled it” — this is what stops a host with only one or two eligible issues from refilling forever instead of working them.
- The refill task handed out by `next` carries: `purpose: "refill-shortlist"`, `target: 8`, instructions (prefer the active tracks in `planning/plans/2026-11-21_results-first.md`, skip hold labels, add each with a one-line reason), up to **16 ranked candidates** fetched best-effort from GitHub (`candidates(world, repo, 16)`) with a `candidates_note` if GitHub did not answer, and any stale entries dropped on the way. `then`: `fleet agent shortlist fill --agent <a> --repo <r>`, `fleet agent shortlist add --agent <a> --repo <r> --issue <n> --title <t> --why <reason>`, then `fleet agent next …` again.

### 2.4 The verbs (`fleet agent shortlist <verb>`)

- `status` — `{path, count, low_water, low, refill, entries}` (after pruning expired handed/refill records).
- `add --issue N --title T [--repo R] [--rank N] [--why TEXT] [--agent A]` — adds unless already listed or handed within 2 h; the answer says which.
- `take [--agent A]` — the soonest entry, or `entry: null` with exit 5.
- `fill [--repo R] [--target N] [--agent A]` — fetches `target × 2` candidates from GitHub via the same `candidates()` ranking as the priority rung, adds them in rank order until the list reaches target (skipping listed/recently-handed), then clears the refill marker and stamps `filled_at`. Answer: `{added, count, target, candidates_seen}`.
- `clear` — empties entries and the refill marker; answer `{removed}`.

So “refill to 8” means: top the local file back up to 8 ranked, hold-filtered GitHub issues — it does not touch the fleet queue, and it is triggered by whichever agent's `next` run first notices the list at ≤3 with no fresh refill marker.

---

## 3. Secrets broker

Two different things share the name; keep them separate:

- **`fleet-secrets` (the client crate)** — `fleet-secrets/src/lib.rs`: the one secrets-by-name resolution path every fleet domain uses.
- **The secrets broker (the daemon)** — `fleet-queue/src/secrets_broker.rs`, served as `fleet serve secrets-broker`: a resident Bitwarden Secrets Manager (BWS) cache on the laptop. Design origin code#5362; epic code#5412.

Book chapter: `pkgs/fleet/book/concepts/secrets.md`; reference: `book/reference/secrets.md`.

### 3.1 The daemon: cache-first, full refresh on a TTL

Source: `fleet-queue/src/secrets_broker.rs` (1,397 lines; header doc is the spec).

- **Why it exists:** the per-call alternative — `bws secret list` + `bws secret get` per invocation — “made every repo-queue.py invocation pay 10–40 s.”
- **Bind:** laptop tailnet IP only, `SECRETS_BROKER_BIND` default `100.99.113.103:4800`, plus a Unix-socket dual-bind (#6216; default `~/.config/secrets-broker/broker.sock` via fleet-config) with identical routes/auth. On a sandbox seat, `fleet init` runs a local loopback copy and the bind is *forced* to `127.0.0.1:4800` when `FLEET_LOCATION=sandbox`, whatever the environment says (book `concepts/secrets.md`). Sandbox consumers otherwise reach the laptop broker through the session proxy (port 4800, §5).
- **Endpoints** (header `X-Agent-Token` required on everything except `/health`; an empty expected token never authenticates):
  - `GET /health` → `{ok, serving, secrets, last_refresh_unix, refreshing, oldest_secret_age_seconds}`; **503 while `serving` is false**.
  - `GET /secrets` → `{names: [...]}` — names only.
  - `GET /secret/<name>` → `{key, value, fetched_at_unix, age_seconds, source}` | 404 | 502 | 503.
  - `POST /refresh` → 202 `{ok: true, refresh: "started"}` (kicks a refresh on a new thread; body ignored).
- **Serving gate (#10868):** `serving` is true only once at least one refresh has *succeeded* (`last_refresh.is_some()`). Before that, `/health` is 503 and `/secret/<name>` is 503 “secrets not loaded yet”, with `refresh_error` attached when a refresh has failed (#10743) — “a live process whose refresh never loaded is not serving, and health says so.”
- **Refresh:** a background loop runs one full refresh **immediately at startup**, then sleeps the TTL and repeats — stale-while-revalidate: the cache keeps serving during a refresh, and a failed refresh keeps the old values. One refresh = `bws secret list` (parsed strictly — a malformed entry fails the whole parse, #6568, so a secret is never silently dropped and then evicted as “delisted”), then one `bws secret get` per secret, fetched **8-way parallel**. Each `bws` child has a 60 s deadline (#10871) so a hung child fails its fetch instead of wedging the `refreshing` guard forever. A second refresh while one is in progress is refused (“refresh already in progress”) and never overwrites the recorded refresh error.
- **TTL:** `SECRETS_BROKER_TTL_S`, **`DEFAULT_TTL_S = 900` (15 min)**, `MIN_TTL_S = 30`. Unset → 900 silently; unparseable → 900 with a warning naming the value; below 30 (including 0) → **startup refuses, exit 2**, naming the variable, value and minimum (#5860: TTL 0 spun the loop at ~110 full refreshes/sec, each costing a full vault fetch).
- **Per-secret install (#6767, preserving #5504's no-eviction rule per secret):** successes land immediately, each stamped with *its own* fetch time; a secret whose fetch fails keeps its previous value *and* its previous timestamp (a failure never evicts and never makes a stale value look fresh); a cached name absent from the authoritative list is removed; only a refresh in which **every** fetch fails installs nothing and keeps the whole previous cache. Net effect stated in the header: “A rotation therefore lands within one TTL of the edit, bounded and observable via the exposed age.”
- **Cache-first with read-through on miss (#6275):** a hit answers 200 immediately with `source: "cache"` and the value's age. A miss on a *loaded* cache is not final: the broker reads through to Bitwarden for that one name (single-flighted per name via a per-name `FetchGate` — concurrent missers wait on one gate instead of stampeding the vault; the cache is re-checked after acquiring the gate) and serves it with `source: "bitwarden-read-through"`, `age_seconds: 0`. Found → cached + 200; definitely absent → 404; source failure → loud **502**, never a silent 404-as-if-absent. Only a success is cached — a not-found or failure is returned but never installed, so the next miss retries the source.
- **Vault size:** ~221 secrets — verified from code#13081 (below): a completed refresh reported `secrets: 221` on 2026-11-23, and the heal troubleshooting entry describes “on the order of 220 `bws` fetches in a refresh cycle” (`book/operations/troubleshooting.md`). This is an observed count at that date, not a constant; the refresh cost is O(vault size) by construction (one `get` per secret).

Daemon token file: `~/.config/secrets-broker/token` (`SECRETS_BROKER_TOKEN_FILE`). Startup exit codes (book): 2 for a bad home or a refused TTL, 3 for a token-file or bind failure.

### 3.2 The client chain (`fleet-secrets`)

Source: `fleet-secrets/src/lib.rs` (2,167 lines; header doc is the spec). Platform rule: **secrets are resolved by name through this crate only** — no domain shells out to `bws` or reads a token file on its own.

Resolution order (`Resolver::default_chain`):

1. **Direct broker source (laptop only, #6218):** where fleet-config's probe selects the broker's own Unix socket, a direct `GET /secret/<name>` over that socket, reading the direct token file itself (`.config/secrets-broker/token`, `DIRECT_SECRETS_TOKEN_FILE`) into the request header. This leg owns the retry budget when present.
2. **Session-broker broker source:** one framed `fleet-proxy-proto` request/response through the session broker to port 4800, `GET /secret/<name>`. The request *names* the token file (`[secrets] token_file`, default `~/tools/secrets-broker-token`, `DEFAULT_SECRETS_TOKEN_FILE` in fleet-config); the session broker reads the file per request and sends `X-Agent-Token` — “fleet never opens it” on this leg, so the value never transits fleet's own memory.
3. **`bws` CLI fallback:** `bws secret list` + `get` — the same shape the broker itself uses. It carries its own credential explicitly (`BWS_ACCESS_TOKEN` env when non-empty, else the on-disk machine-account token file at `~/.config/bitwarden/access-token` or `~/.config/bws/access-token`), handed to the `bws` child via its environment. Before this fix the fallback inherited the ambient environment only and “was dead exactly when it was needed” (2026-11-15 incident, crate doc).
4. **`op` CLI (optional):** only where `FLEET_OP_VAULT` names a vault — `op read op://<vault>/<name>/<field>` — for seats keeping secrets in 1Password.

Discipline enforced by the types:

- Values live in `SecretValue`, whose `Debug` is redacted (`SecretValue(***)`) and which exposes the string only via `expose()`. Nothing in the crate persists a value: no cache, no file, no log line. Errors name the secret's *name*, the source and status codes — never a value, never a subprocess's stderr (which could quote vault material). The broker-side `BwsRunner` follows the same rule (exit status only).
- **Empty is failure (#6275):** an empty string is never a successful resolution anywhere in the chain (the 2026-11-05 incident was a caller exiting 0 with an empty value while its sources had actually failed).
- **Retry policy:** only `Unavailable` outcomes retry; a broker **404 is definitive** about the name and is never retried. Budget `BROKER_RETRY_BUDGET = 120 s` (“spans the observed restart windows — a full switch restarts the broker and its tunnel together; the broker itself is back in ~16 s”), first pause 2 s doubling to a 15 s cap, with up to +50% wall-clock jitter, and a **10 s per-attempt timeout** so an accept-but-silent peer cannot wedge the caller. Exactly one broker leg owns the budget (the direct leg when present; the session-broker leg then gets `RetryPolicy::NONE`). If the session-broker socket file does not exist at all, the leg does not burn the budget — no broker on this box is not a broker mid-restart. `list_names` never retries (fast-failing) but is still bounded by the per-attempt timeout.
- **`fleet secrets list` has no `bws` fallback** (book `reference/secrets.md`): a listing enumerates the broker's cache, not the vault, and is never silently substituted; a broker failure is loud (exit 3).

Verbs: `fleet secrets get <name>` (value to stdout and nowhere else; broken pipe on stdout is a quiet success) and `fleet secrets list` (names, one per line). Exits: 0 ok, 2 usage, 3 unresolved/unavailable.

### 3.3 The health-gate issue: code#13081

Code#13081 — “fix(heal): secrets-broker restart cadence starves its first refresh” — **open**, no labels (fresh `gh` read this pass). Body, in substance:

- Observed 2026-11-23 ~01:54–01:58 CST: the laptop-channel heal loop restarted `fleet serve secrets-broker` at 07:54:27Z, 07:54:32Z, 07:55:21Z and 07:56:24Z because health kept returning 503 with `refreshing: true, secrets: 0`. But a full refresh shells out to one `bws secret get` per secret (~221 secrets) and takes longer than the heal's restart patience — every restart killed the in-flight refresh, so the broker never reached `serving: true`. Left alone, the 07:56 instance completed its refresh and health went green (221 secrets) with no other intervention.
- Fix shape (from the issue): **the heal must not restart a secrets broker whose health reports `refreshing: true`** with refresh progress underway (or whose `bws` children are active); give the first refresh a patience window sized to the secret count; restart only when the process is absent, wedged with no `bws` activity, or still not serving after that window.

The book already states the general rule this issue instances: “Do not restart-loop a broker that is up but not serving” (`book/operations/services.md`, `health-and-healing.md`, `troubleshooting.md`). **[unverified]** whether a code fix for the heal's patience window has landed since filing — the issue is still open and no fix was traced in this pass.

Heal context (book `operations/health-and-healing.md`): the sandbox heal chain rebuilds in dependency order `tailscaled → relay :3130 → session broker → secrets broker → gh`, run whole by a ~5-minute heal cron and by `init-ensure` (which also fetches the secrets-broker token by name from BWS when the local token file is missing, so the broker can start authenticated).

### 3.4 Token distribution: the three `fleet.*` BWS names, and the trap

Source: code#12898 (closed) and its comments; book `concepts/secrets.md` §“The three token files and bootstrap provisioning”, `tutorials/enrol-a-seat.md` Step 2, `reference/secrets.md`, `getting-started/installation.md`.

The three shared service tokens a *client seat* needs, and where they live:

| BWS name (source of truth) | Client file on a seat (mode 0600) | Daemon-side counterpart on the laptop | Used by |
|---|---|---|---|
| `fleet.secrets-broker-token` | `~/tools/secrets-broker-token` | `~/.config/secrets-broker/token` | Session broker sends it as `X-Agent-Token` to the secrets-broker (port 4800); named in every `fleet secrets` broker request |
| `fleet.queue-broker-token` | `~/tools/queue-broker-token` | `~/.config/queue-broker/token` | Session-proxy queue requests name this file (fleet-queue client transport, port 4801) |
| `fleet.laptop-agent-token` | `~/tools/laptop-agent-token` | `~/tools/laptop-agent-token` (laptop) | `fleet laptop-agent` reads it directly; also the session broker's default token (port 4799) |

(Table's “used by” column is from code#12898's body table.)

History, verified from the issue record:

- **The gap:** a freshly enrolled non-`muse` seat had *no provisioning path at all* for these files. `scripts/muse/bootstrap.sh` never mentioned them; on the `muse` seat they existed only because the restic restore phase brought back the old sandbox's `~/tools` “by accident of the home backup, not by design”; the enrolment helper correctly skips `restore,persist,init` for non-`muse` kinds, removing the only accidental path. On the grok seat (2026-11-22) the consequences were: `fleet secrets list` → synthesised 502 (the session broker synthesises 502 when a request fails, and reading the named token file is the first step of its request path; `list` has no fallback); queue leg — “cannot read token file ~/tools/queue-broker-token”; shell leg — “laptop-agent-token unreadable”; and a bridge knock-on — resolving `message-bridge.fleet-token` went through the broken broker leg first, whose 120 s retry budget burned ~4 minutes over two names before the `bws` fallback could derive the token (the “bridge hello hangs” symptom).
- **The route decision (2026-11-22 ~23:55 CST, comment 7092823739 on #12898):** Drew relayed the grok seat's question “SSH key or BWS?” — **BWS chosen** (canonical store, no laptop `authorized_keys` change, fixes provisioning for every future seat). The three tokens were published in BWS under the `fleet.*` names above, created from the laptop's server files with values transferred entirely on the laptop, and verified byte-equal (trimmed) against the source files, 3 of 3.
- **The durable fix:** the bootstrap token phase in `scripts/muse/bootstrap.sh` now fetches the three `fleet.*` names automatically and writes them 0600 to the `~/tools/*-token` paths, whitespace-normalised to exactly one token line. **PR #12970** “feat(enroll): bootstrap token phase provisions the three broker token files (#12898)” — merged 2026-11-23T06:07:09Z, merge commit `353f24ad4b4`, head-SHA guarded at `446ea9a10ab` (verified via `gh api …/pulls/12970` and the issue's merged comment). A name the seat's BWS token cannot see **warns and continues** rather than failing enrolment — the check phase names the legs that stay down — so a seat whose BWS access does not yet include the fleet project is not blocked. New test `scripts/tests/test_muse_bootstrap_tokens.py` runs the real script (`--only token`) against a fake `bws`: 3 passed at landing. Code#12898 closed as completed (release comment 2026-11-23T06:14Z). Note the book's precision: the provisioning lives in the bootstrap (seat layer), **not** in the fleet crates — “no code under `pkgs/fleet/crates/` reads those three Bitwarden names to create the files.”
- **⚠️ The trap — `secrets-broker.token` is a DIFFERENT value.** The pre-existing BWS name `secrets-broker.token` does **not** hash-match the secrets-broker client token file; it is the broker's *own* token (the daemon-side value), was left untouched by the #12898 provisioning, and must never be used as the client token. Every current source repeats the warning: #12898's decision comment (“the new `fleet.*` names are the client-token source of truth”), `book/concepts/secrets.md` (“never used as the client token”), `book/tutorials/enrol-a-seat.md` (“do not reuse the older, different name `secrets-broker.token`, which is a different value”), `book/reference/secrets.md` (do not conflate client `~/tools/*-token` files with daemon-side `~/.config/*-broker/token` files). A second, related conflation to avoid: the sandbox heal/`init-ensure` path fetches a token *by the name `secrets-broker.token`* when the local file is missing (standing context, 2026-11-22 cold-test record) — that is the in-process broker's own startup fetch in the heal script, a different consumer from a client seat's `~/tools/secrets-broker-token`. **[unverified]** whether the heal script's BWS name has since been renamed to the `fleet.*` form — the heal script itself was not read in this pass; the two names' distinctness is the load-bearing fact either way.

Seat check (enrol tutorial Step 2): `ls -l ~/tools/secrets-broker-token ~/tools/queue-broker-token ~/tools/laptop-agent-token` — three regular files, `-rw-------`. A missing file does not block enrolment (the bridge leg is HTTPS + a by-name secret) but the queue, secrets and shell legs fail with synthesised transport errors — the classic “enrols fine, doctor shows three legs down” shape.

---

## 4. Bridge

Sources: client `fleet-bridge/src/lib.rs` (903 lines); server `pkgs/message-bridge/worker/src/fleet.js` (the bridge worker, live at `https://bridge.security.cab`); book `concepts/bridge.md`.

### 4.1 Model

Every fleet agent has a **mailbox** on the message bridge named after it, and registers with optional groups. Central — the laptop seat, or anyone holding the bridge's owner token — sends a message or a command to one agent, a group, or every agent; the agent reads its mailbox, does the work, and posts its answer back, which marks the command handled. There is no separate enrolment store on the bridge: **saying `hello` is registering** — name, groups, first-seen and last-seen all come from that call (worker `fleet.js`: `fleet_agent` table keyed by name, `touch()` upserts on hello/inbox/ack/post).

### 4.2 The two verb families (tokens, routes)

Auth is by secret name through fleet-secrets; values are never printed, logged, or put in argv. Tokens travel **in the URL path**, so every error string is scrubbed of token values (replaced with `<token>`) before printing, and plain `http` bridge URLs are accepted only to localhost — anything else must be `https` (`check_url` in the client).

**Central verbs — owner token** (secret name `message-bridge.access-token`):

| Verb | Route | Behaviour |
|---|---|---|
| `send --to <targets> [--command] [--sender NAME] <text \| ->` | `POST /api/<owner-token>/fleet/send` | Targets: `agent:<name>` (or a bare name), `group:<name>`, `all`; repeat `--to` or comma-separate. `--command` marks kind `command` (default: plain `message`). `-` reads text from stdin. |
| `agents` | `GET /api/<owner-token>/fleet/agents` | Registered agents, groups, unread counts (plus inbound/outbound totals and last id per agent, from `mailboxStats`). |
| `log [--agent A] [--direction inbound\|outbound] [--kind message\|command] [--since-id N] [--before-id N] [--limit N] [--order asc\|desc]` | `GET /api/<owner-token>/fleet/messages?…` | The central message log, filtered. |

**Agent verbs — fleet token** (secret name `message-bridge.fleet-token`; opens only the agent routes, cannot send commands):

| Verb | Route | Behaviour |
|---|---|---|
| `hello [--agent A] [--groups g1,g2]` | `POST /fleet/<fleet-token>/hello` `{agent, groups}` | Register / refresh groups. |
| `inbox [--agent A] [--groups g1,g2] [--commands \| --messages] [--all] [--since-id N] [--limit N] [--take] [--wait SECS] [--interval SECS]` | `GET /fleet/<fleet-token>/inbox?…` | Unread by default; `--all` includes read; `--take` marks what it returns read; `--wait` polls (default interval 15 s, wait capped at 24 h = `MAX_WAIT_SECS`) until something arrives or the seconds pass. |
| `post [--agent A] [--reply-to ID] [--to ADDR] [--instruction \| --kind K] <text \| ->` | `POST /fleet/<fleet-token>/send` | Message to central by default; `--reply-to` answers that row (and marks it read). `--to` sends to any address instead: `drew`, `claude`, `chatgpt`, `connectors`, `agent:<name>` (or bare name), `group:<name>`, `agents`, `all`. `--instruction`/`--kind instruction` makes it an instruction, which requires `--to`. |
| `ack [--agent A] (--up-to ID \| ID…)` | `POST /fleet/<fleet-token>/ack` | Mark messages read (≤1,000 ids at once, worker-validated). |
| `health` | `GET /healthz` | No token. Proves the service, not the seat's mailbox access. |

Defaults: agent from `--agent` → `$FLEET_AGENT` → `[bridge] agent`; groups from `--groups` → `$FLEET_AGENT_GROUPS` → `[bridge] groups`; URL from `$FLEET_BRIDGE_URL` → `[bridge] url` → `https://bridge.security.cab`. Exit codes: 0 ok, 2 usage, 3 transport/credentials, 4 the bridge refused (its JSON error goes to stderr; a 404 is reported as “a wrong token, an old bridge without fleet mailboxes, or a wrong URL”). Per-verb `--help` exists (#13197).

### 4.3 The HMAC-derived fleet token

Source: `fleet-bridge/src/lib.rs` — `FLEET_TOKEN_LABEL`, `hmac_sha256`, `b64url`, `derive_fleet_token`, `fleet_token()`.

When the fleet token's secret is missing but the owner token resolves (the laptop seat's shape), the client derives the fleet token **exactly as the bridge does**:

```
fleet_token = base64url( HMAC-SHA256( key = owner token bytes,
                                      message = "message-bridge/fleet-token/v1" ) )
```

Base64url without padding; HMAC is a hand-rolled RFC 2104 construction over SHA-256 in the crate (no external HMAC dependency). The derived value is used in-process only and never written anywhere. If neither secret resolves, agent verbs fail with exit 3 naming both secret names. This derivation is also why enrolment's bridge leg needs “the fleet token by name through fleet secrets” — and why a broken secrets-broker leg delayed it by minutes in the grok episode (§3.4): the chain tries the broker leg (120 s retry budget) before the `bws` fallback that can resolve the owner token.

### 4.4 Read / ack semantics

Reading is non-destructive by default. Exactly three operations change read state (book + worker code agree):

- `inbox --take` marks exactly what it returns as read — the “I have this now” claim. `fleet agent next`'s push rung uses `--commands --take`.
- `post --reply-to ID` answers a row and marks that row read. The worker refuses a `reply_to` that is not an inbound message *to that agent* (`reply_to <id> is not a message to <agent>`).
- `ack` marks explicit ids, or everything up to an id with `--up-to`.

Worker storage (`fleet.js`): `fleet_message` rows `{id (autoincrement), ts, agent, direction (inbound|outbound), kind (message|command), sender, body, batch, reply_to, read_at, from_addr}`; inbox returns rows `ORDER BY id ASC` with `next_since_id` for cursoring.

### 4.5 Batch ids

Verified in the worker (`fleet.js`), **not** exposed by the Rust CLI:

- A `send` operation accepts an optional `batch` (validated: string matching `^[A-Za-z0-9_-]{8,40}$`, else treated as null). If none is supplied, the worker generates one: `randomBatch()` = 12 random bytes, base64url-encoded (16 chars).
- One batch id is shared by **all** message rows a single multi-recipient send creates — one row per recipient agent, same `batch`. The send result is `{batch, messages, unregistered: [...]}` — recipients that were addressed by name but have never registered are still written as rows and reported back in `unregistered`, sorted.
- A `post --reply-to` **inherits the original row's batch** (the reply row carries the same `batch`), so a whole pushed task and its answers thread together by batch.
- The worker's `origin` operation returns `{id, from_addr, batch}` for an inbound row — how a relay/agent recovers who originally sent a message and which batch it belongs to.
- The `fleet bridge` CLI's `send` payload is only `{body, to, kind, sender?}` — it never sets `batch`, so every CLI send gets a fresh server-generated batch. **[unverified]** which non-CLI callers (relay, wake hooks, connector paths in `relay.js`/`wake.js`) supply explicit batch ids — those files were not read in this pass.

### 4.6 Who talks to whom

- **Central → agent(s):** `send` (owner token) to `agent:<name>`, `group:<name>`, or `all`. Group expansion is server-side against the registry: a group with no registered members is an error (`no registered agent is in group <g>`); `all` with no registered agents is an error unless the send is lenient (relay path). A send is capped by `MAX_RECIPIENTS` in the worker (constant not read in this pass — **[unverified]** value).
- **Agent → central:** `post` with no `--to` (an outbound row in the agent's own mailbox; central reads it via `log`/`agents`).
- **Agent → agent / anyone:** `post --to <addr>` — the same address grammar as `send`, plus the connector addresses `drew`, `claude`, `chatgpt`, `connectors`. An agent cannot send to itself via the worker's send path (`an agent can't send to itself`, enforced when an `exclude` matches). Agent-originated sends carry `from_addr`; the sending agent never receives its own message on the relay path.
- **Kinds gate the ladder, not the mailbox:** any enrolled agent can be *addressed*; what differs is the seat (§1.1) — http seats poll their mailbox over HTTPS as their only intake, connector seats (`claude`, `chatgpt`) have a channel address instead of a mailbox and are woken by a wake hook / MCP Events subscription respectively.

---

## 5. Laptop-agent

### 5.1 What it is

The laptop-agent is the fleet's single execution entry point to Drew's laptop (`amd`, Framework, NixOS; tailnet `100.99.113.103`): a resident daemon on the laptop that runs commands and detached jobs on request, so sandbox seats — which are orchestration-only by standing rule — execute everything on the laptop through it.

Three layers, do not conflate them:

1. **The daemon** (on the laptop, port **4799**): canonical source `pkgs/laptop-agent/src/laptop_agent.py`, packaged by `pkgs/laptop-agent/package.nix` as `bin/laptop-agent` (its own package, **not** the fleet binary). Header: “the reconciled single source (code#10131)” — the union of the live daemon lineage (timeout semantics, trip logging, `/exec` cancel, disconnect reap, `/file`) and the darwin-nix vendored lineage (persisted job index, fail-closed startup validation, request-framing 400s, job summaries, spool reaping). Stdlib-only Python; the Nix wrapper pins `LAPTOP_AGENT_BASH`. Older copies still exist at `pkgs/fleet/laptop-agent.py` (the client, see below) and `~/tools/laptop-agent.py` on seats — the package is the daemon's canonical source.
2. **The client** — two implementations of the same protocol:
   - Python: `pkgs/fleet/laptop-agent.py` (installed as `~/tools/laptop-agent.py`), the working client today.
   - Rust: `fleet-laptop-agent` crate (`fleet laptop-agent …`), epic code#5412 / issue code#10796, explicitly mirroring the Python client — “the same verbs, the same daemon protocol, the same trip-log record shape.” Its header notes what the binary adds: the transport ladder. The `~/tools/laptop-ssh.sh` shim's own header records the fleet binary's `laptop-agent` domain was a registry stub in build 0.1.0 (exit 69, “lands in a later step of epic #5412”, verified in `crates/fleet/src/main.rs` at shim-writing time); **[unverified]** which of the Python/Rust clients a given seat's shim resolves to today — the shim delegates to `~/tools/laptop-agent.py` per its current text (read this pass).
3. **The session broker / proxy** (sandbox side): `laptop-agent-broker.py` / `fleet serve proxy` — holds warm tunnel connections and serves local clients over `~/.cache/laptop-agent/broker.sock`. It is the single entry point for *all* laptop services, not just the daemon: its `port` field selects 4799 laptop-agent, 4800 secrets-broker, 4801 queue-broker (code#5362), and its `token_file` field names the file whose contents are sent as `X-Agent-Token` for that service (read per request, never stored; without one, the laptop-agent token is used).

### 5.2 Verbs and protocol

Client usage (identical in Python docstring, Rust `cli.rs` `USAGE`, and book `reference/laptop-agent.md`):

```
fleet laptop-agent health
fleet laptop-agent jobs
fleet laptop-agent kill JOB
fleet laptop-agent exec [--cwd DIR] [--timeout SECS] [--shell] -- CMD…
fleet laptop-agent run  [--cwd DIR] [--shell] -- CMD…        # detached job
fleet laptop-agent poll JOB [--wait SECS]
fleet laptop-agent fetch REMOTE_PATH LOCAL_PATH
```

Python client additionally documents `exec-batch FILE.json` (`{cmds: [...], timeout_s: N}`; exits with the worst per-command code). Daemon constants (both clients agree): `TARGET_HOST = 100.99.113.103`, `TARGET_PORT = 4799` (`PORT_LAPTOP_AGENT` in fleet-config), per-stream capture cap **1 MiB** (tail kept, truncation announced on stderr — it used to be dropped silently), token file `~/tools/laptop-agent-token` sent as a header only — never in argv, a log line, or an error message (an empty token file is refused by the Rust client).

Timeout/cancellation semantics (code#5343, fixed 2026-11-10; Python docstring is the fullest statement):

- `exec --timeout S` is enforced **in the daemon** (the remote process *group* is killed at S — children and grandchildren included) and locally: the client gives up at S + `TRANSPORT_GRACE_S` (15 s) and exits **124** (GNU timeout's code) even if the transport has wedged.
- An external `timeout N` (SIGTERM) or Ctrl-C no longer orphans the remote command: the client catches the signal, asks the daemon to cancel the in-flight exec (or kill the detached job) within a 3 s cancel budget (`CANCEL_BUDGET_S`), and exits 124. The `laptop-ssh.sh` shim cooperates: it detects a GNU `timeout` parent, exports `LAPTOP_AGENT_DEADLINE_S`, and the daemon reaps the remote group at the same deadline even if every signal path fails.
- SIGKILL cannot be caught; the daemon reaps an orphaned exec when its request connection dies, and a detached job keeps running by design (`kill JOB` ends it). The broker mirrors this: its watchdog hard-closes the tunnel of a client that hung up mid-flight, which both errors the blocked handler and signals the daemon to reap.
- A brokered reply that took >10 s beyond the daemon's own execution time is announced on stderr as a transport-layer wait, so a parked caller is distinguishable from a long remote command.

Trip logging: every request writes one v2 record to `~/.local/state/laptop-agent/trips.jsonl` on both sides — `{v: 2, ts, trip (12 hex chars), path, total_ms, status, transport (broker|cf|ts|ssh), t_sent, t_returned, t_received}`. The Rust crate pins the field names *and their order* to the Python client's, with a test, “so a log reader cannot tell which client wrote a line.”

### 5.3 Transport ladder

Rust crate (`fleet-laptop-agent/src/ladder.rs`, `legs.rs`; book `concepts/proxy.md` agrees): **broker socket → Cloudflare tunnel → Tailscale CONNECT → SSH**. “A dead leg degrades to the next; only a total failure errors, naming every leg that failed.”

Python client detail (docstring): primary transport since 2026-11-15 (#10356) is the named Cloudflare tunnel — HTTPS to `sandbox-link.security.cab` through the platform egress proxy, with Cloudflare Access service-token headers resolved from fleet secrets by name (`cloudflare.access-laptop-agent`, Bitwarden-backed; migrated from a 0600 file 2026-11-16). Fallback is the tailnet CONNECT proxy; `/exec` calls whose budget exceeds the Cloudflare edge's ~100 s response cap go direct to the tailnet path. `LAPTOP_AGENT_TRANSPORT=auto|cf|ts` pins the preference. The broker fast path skips the CONNECT handshake per call (cold ~1.8 s through the DERP hairpin vs ~0.25 s pooled, per `laptop-agent-broker.py`'s header; pool size 12 by default, idle exit after 1 h, keepalive 180 s).

**SSH fallback (#2879, 2026-11-16):** when *all* agent transports fail, `request()` retries the operation over real SSH (`~/tools/laptop-ssh-real.sh`, the preserved pre-agent transport): `/exec` and `/exec-batch` run over SSH with the same result shape, `/health` is synthesised from `/proc`, and detached jobs launched during the outage get `sshfb-` ids spooled under the laptop's `~/.local/state/laptop-agent/sshfb-jobs/` and stay on the SSH path for their whole life. The fallback fires only on transport failure (never on a daemon answer, including daemon HTTP errors), is announced once on stderr, honours the caller's deadline, and is disabled by a pinned transport or `LAPTOP_AGENT_NO_SSH_FALLBACK=1`.

`~/tools/laptop-ssh.sh` itself is **retired as an SSH transport** (2026-11-04, code#5343): exec-style invocations delegate to the laptop-agent client; only shapes the agent cannot express (forwards, TTY, stdin-carrying commands like `sudo -S` feeds) or `LAPTOP_SSH_REAL=1` fall back to real SSH. This research was conducted through that shim, i.e. through the laptop-agent path itself.

### 5.4 Deployment and generation

Source: `pkgs/deploy/services.nix` + `pkgs/deploy/deploy.py` (book `operations/services.md`, `development/release-and-rollout.md`).

- On the laptop, services are systemd (user) units rendered by `deploy.py` from per-service Nix profiles under the profile base. Each unit's `ExecStart` points at the profile's **`current`** link (`<base>/<name>/current/bin/laptop-agent`), never a bare store path — a generation flip is a profile repoint plus a restart of that one unit; the unit file does not change. Activation restarts only that unit, health-checks it, and flips the profile back to the previous generation on failure. Generation history is the append-only JSONL log per target; `deploy status` folds it.
- `laptop-agent` service entry (`services.nix`): name `laptopAgent`, entry point `bin/laptop-agent`, port 4799 (`LAPTOP_AGENT_PORT=4799`), **restart `always`** — deliberately: under `on-failure`, a clean SIGTERM/success exit leaves the daemon down, and “a past daemon-death episode showed that gap is real. Do not ‘fix’ that back.” Its daemon token is a local file (`~/.config/laptop-agent/token` per the services.nix comment). It names **no** broker-resolved secret names (“the laptop-agent shape” — it *is* a token holder, not a secrets consumer).
- Health: there is **no unauthenticated endpoint**; verification is through the authenticated client (`fleet laptop-agent health` / `laptop-agent.py health`) after activation. This is also the `shell` leg of `fleet agent doctor` (§1.2).
- The sibling laptop services deploy the same way: `queue-broker` (`fleet serve queue-broker`, port 4801, restart `on-failure`, unauthenticated `GET /healthz`) and `secrets-broker` (`fleet serve secrets-broker`, port 4800, restart `on-failure`, `GET /health`).
- `fleet init` never detached-spawns a daemon systemd owns: in systemd mode it reports/starts the `fleet-<role>.service` units; spawn mode exists only on sandbox seats, and only for the proxy and secrets-broker roles (§3.3).

**Role as single entry point:** standing rule (AGENTS.md / MEMORY, Drew 2026-11-04) — “laptop-agent is the single entry point for fleet laptop work.” In code terms that is realised twice over: the daemon is the only sanctioned remote-execution surface for seats, and the session broker multiplexes *all three* laptop services (execution, secrets, queue) over the one tunnel/broker path, with per-service token files.

---

## 6. Quota plane

The quota plane is **not** a `pkgs/fleet` crate. There is no `fleet-quota` crate in `pkgs/fleet/crates/` (verified by listing: fleet, fleet-agent, fleet-bridge, fleet-config, fleet-dispatch, fleet-dispatch-test, fleet-laptop-agent, fleet-proxy, fleet-proxy-proto, fleet-queue, fleet-registry, fleet-search, fleet-secrets). Its live code is in the **pm-stack aggregator** (`pm-stack/aggregator/src/quota-*.ts`, TypeScript/Bun, in `developing-today/code` at repo root — not under `pkgs/`), with an older placement-policy PoC in Go at `pkgs/fleet/placement/`. Its doctrine lives in Drew's standing rulings (AGENTS.md / MEMORY / the goal registry file below), and its program is tracked under epic code#1047 (usage & billing hub) and the quota-plane epics code#1149 (Stage 2) / code#12694 (Stage 3).

### 6.1 Where the pool registry actually lives

- **Not in the fleet repo tree.** The canonical pool registry is a *goal-workspace* file: `~/workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md` (“Provider quota registry — plans, models, burn order”, ~189 KB at read time). Its header: maintained from Drew's directives (2026-10-24), the deployed OpenCode config, and observed usage; facts labelled [known]/[observed]/[unknown]; snapshot tool `~/tools/provider-usage/check-usage.py`. It holds the POOL MAP (see §6.3), per-provider windows/resets/auth inventory, and the measurement-gate text. It is deliberately *not* a repo file — placement consumers read quota through the aggregator's `quota-current` contract, not by parsing the registry.
- In-repo analogues: `pkgs/fleet/placement/policy.go` defines a `Pool {id, name, drawn_by}` as part of the placement-policy PoC (#4986, consuming `placement-policy.yaml` from the #2024 framework) — explicitly “deliberately not wired into the live queue client or the OpenChamber dispatcher” (file doc). The deployed OpenCode config's `experimental.model_routes` is the section→door mapping source the aggregator's `src/aa.ts` reads (`routeLeafSets`) — cited by code#8882 as the join source between route sections and quota pools.

### 6.2 The gatherer and the measurement cadence

- **Gatherer:** `pm-stack/aggregator/scripts/gather-quota.ts` — the quota-plane Stage-2 service (code#1151, epic #1149): a **15-minute tick** on the laptop that fetches every configured provider's quota and writes `quota-current.json` plus append-on-change snapshot records to the aggregator snapshot store (`~/.local/state/pm-aggregator/snapshots/` on the laptop). Stage-3 precondition evidence (code#12694) records 20,635 usage records spanning 2026-10-20 → 2026-11-22 across every routing pool (antigravity doors, opencode-go workspaces, anthropic:claude-pro, meta:high-usage, openai:codex, cline-pass, command-code, opendesign), gatherer heartbeat live at filing.
- Gatherer correctness details verified from issue/packet records in this pass: provider fetching was unbounded `Promise.all` with a 15 s per-provider timeout; the #1268 follow-up (PR #3996, merged) bounded concurrency (default 6, matching the OpenChamber CLI's `QUOTA_FETCH_CONCURRENCY=6` / 10 s timeout from openchamber PR #529) and made timed-out/errored providers degrade to their **last-known entries marked stale** instead of being replaced by fresh error rows. Snapshot dedup keys by serviceId|window and had a credential-dimension defect (#1624: two credentials of one provider interleaved into one series) — triaged with a pinned series rule; **[unverified]** whether that fix has landed.
- **Freshness is a first-class field everywhere:** every quota observation carries `measuredAt`; consumers must not present a retained/stale reading as live (the aggregator's honesty gates — e.g. #1541 quota-current `measuredAt` freshness, #1564 `generatedAt` parse/future gates — exist because a NaN or future stamp once compared as “live” forever).

### 6.3 The doctrine, as ruled and as implemented

Drew's doctrine (standing rulings in AGENTS.md/MEMORY and the registry file's POOL MAP — these are the *operating* rules for placement/dispatch):

1. **Plan quotas only — NEVER FUNDS.** Pools are subscription/plan allowances. A 402/quota error is a wall, never a trigger for pay-as-you-go or balance fallback; at most **one** retry on the same pool, then rotate or stop and report; when every plan is walled, the session stops. The one named exception proves the rule: DeepSeek *direct* (`api.deepseek.com` balance) is a PAYG **reserve for when every plan is used up**, never a routine route or an agent default; routine DeepSeek comes from plan routes (ClinePass / Open Design / OpenCode Go).
2. **Measurement gate:** no placement, switch, or dispatch without a quota read **≤60 min old** for that pool; measure before *and during* burns; burns >1 h re-check hourly and at every failover. **A failed or stale quota read never idles a pool or stops dispatch** (Drew, 2026-10-30): unknown is not closed — probe inference directly, dispatch on last known state, retry the read. Near a reset, burn-by-expiry overrides the gate.
3. **Burn to 100%, burn-by-expiry:** pools should reach ~100% *at* their reset; soonest-expiring pool first; OpenAI is the final reserve. At a **known 100%** (fresh measurement) the pool is **closed** until its reset or a fresh read shows <100%. The old ≥85% caution zone was retired 2026-10-27 (“do not strand usable quota”); 85% survives only as the *near-wall notification* threshold (§6.4), not as a placement stop.
4. **Pool identity discipline:** one Meta pool (Muse API High Usage plan) with one sanctioned door (OpenCode `meta` provider); the `muse` *program* reaches the same plan via OAuth; the OpenCode `muse` provider is a disabled second door; the assistant Muse's Power subscription is **not a pool** and sessions never draw on it. Provider enable/disable is one flip in `disabled_providers`; definitions are never deleted to disable.
5. **The wall test for disabling a provider (Drew, 2026-11-10) — the brief's “wall” definition:** NEVER disable a provider for quota reasons on one signal. The wall signal is the **combination**: (a) binding window **<1% remaining** (a $0.05 remainder is walled — “nothing will run”) **AND** (b) failures whose error implies plan limits. One failed call alone “could be anything and must never bench a provider forever”; the opencode fork must wall the provider itself, or at minimum fully deprioritise it. Gap filed as opencode#2256 (per MEMORY; that repo was out of scope for this pass — **[unverified]** here).

As implemented in the aggregator's admission projection (`pm-stack/aggregator/src/quota-admission.ts`, #2732 packet A — “normalized, pure projection only… This module neither fetches nor persists”; callers supply exact billing scopes and *all* binding windows, and “dashboard pool/service labels are deliberately not accepted as identity”):

- Verdicts per window, worst-of per account: `eligible < unknown < pace_hold < walled < stopped` (rank map in code). A `Stop` record (explicit owner evidence) forces `stopped`; a stop is released only by explicit `releasedAt` evidence — “omission is NOT a release protocol.”
- **Walled (admission sense):** `latest.percent === 100` (`known_100`), or a **sustained ≥99.5%** — latest ≥99.5 *and* the adjacent previous observation ≥99.5 with the same denominator, separated by at least the window's collection interval (`sustained_99_5`). A prior wall for the *same period* (`priorWalls` with matching scope/kind/period/reset and a non-empty evidence id) also yields `walled` (`prior_wall_same_period`) — a wall, once evidenced for a period, sticks for that period.
- **Unknown, never guessed:** missing observation, invalid/corrupt observation (including conflicting duplicate ids or conflicting tied-latest reads — “a corrupt member cannot silently vanish and turn a bucket healthy”), an error-state or *retained* latest observation, or a stale observation (`now − measuredAt > min(freshnessMs, 1 h)`) all yield `unknown` — the code form of “a stale read never idles a pool”: unknown is a distinct verdict, not a wall and not eligibility.
- **Pace:** with two consistent adjacent observations, pace is computed in percentage-points/hour; `pace_hold` results from a configured `reservePct` threshold being reached, or from **projected exhaustion** — `measuredAt + (100 − percent)/rate < resetsAt` at the configured `pacePpHour` ceiling. This is the mechanism Stage-3 S4 (§6.5) proposes to feed with the forecast product.
- Note the deliberate difference from §6.3(5): admission's `walled` is a *measured-percent* verdict for placement; Drew's <1%-remaining-AND-plan-limit-failures test is the stricter *provider-disable* verdict for the OpenCode config. They answer different questions (stop placing vs. bench the door) and neither substitutes for the other.

Placement-policy PoC (`pkgs/fleet/placement/policy.go`, `evaluate.go`): the policy YAML carries `burn_rules` with a mechanical `quota_read_max_age_minutes` — the measurement gate expressed as data — plus pools with `drawn_by` options and a `pool_read_at` timestamp on every placement record. PoC scope is one dispatch path (the queue-worker handoff) and it is not wired live (see §6.1).

### 6.4 Section-risk notifications: code#8882

Code#8882 — “pm-stack notifications: section-risk push class” — **closed** (fresh `gh` read). It began as Drew's verbatim order of 2026-11-12 ~01:58 CST: phone push only when a whole route *section* fails or is at severe risk, showing the pools pending wall compactly.

What shipped (issue body + the done comment, task 15095 / PR #8889, merged 2026-11-12T08:27Z, squash `51121b4c`):

- A seventh notification class `section-risk` (“Section failure / severe risk”, default ON for new subscriptions) in `pm-stack/aggregator/src/notifications.ts`, alongside the existing per-pool `quota-wall` (pool walled well before reset, or sole-door pool walled) and `quota-near-wall` (sole-door pool ≥85%) classes; Drew's subscription was flipped to section-risk on, per-pool classes off, as acceptance item 4 — at deploy, not before.
- **Sections are exactly** `workhorse`, `speed`, `specialist`, `summit`, `small` (Drew's list; `scout` deliberately excluded; variants like `small-100k` roll up under `small` only if the config's leaf analysis says they share its doors). Section membership = quota pools whose provider door appears in the section's route leaf set (`aa.ts` `routeLeafSets` over the deployed OpenCode config, joined to pools on the same provider/credential keys as the quota view).
- Definitions (v1, measured windows only — no pace projection): **pool walled** = state `walled` or any measured window ≥100%; **pool pending wall** = not walled and any measured window ≥85%; pools in `error`/`stale`/`lapsed` state are skipped (never derived from retained windows) and do not count as serving. **Section FAILED** = at least one serving pool exists and every serving pool is walled. **Section SEVERE RISK** = not failed, and every non-walled serving pool is pending wall — no serving pool has all windows <85%.
- Delivery: one candidate per section per state (`objectRef section:<name>`) from `buildNotificationCandidates`, through the existing dedupe/cooldown/fingerprint machinery; the fingerprint changes when walled/pending membership or the soonest reset changes, and the compact payload shows which pools are walled, which are pending wall, and the soonest related resets.

### 6.5 Stage-3 slices: code#12698 (S4) and code#12701 (S7)

Epic **code#12694** — “epic(quota-plane): Stage 3 — predictive, self-tuning quota operations” — **open**. Design record `doc/2026-11-22T22-35-00Z_design_quota-plane-stage-3/` (design issue code#1177, also open; its design PR issue #12702 is closed). Gate stated on the epic and every child: implementation work starts after Drew's review of the Stage-3 design doc. Children in priority order: S1 forecast product, S2 forecast accuracy ledger + backtest harness, S3 shadow placement, S4 live forecast-driven admission, S5 anomaly detectors, S6 re-weighting proposal generator, S7 weekly forecast-accuracy review job.

Fresh `gh` status this pass:

- **S1 (code#12695) — closed** (via implementation issue code#12724, closed). The per-tick forecast product exists in code: `pm-stack/aggregator/src/quota-forecast.ts` (+ tests).
- **S2 (code#12696) — closed** via code#12733. Code present: `quota-forecast-accuracy.ts`, `quota-exhaustion.ts`, `quota-analytics.ts`.
- **S4 (code#12698) — open**, labels `kind/feature`, `priority/normal`, `app/pm-stack`; no status label. Scope: let the existing admission verdict path (`quota-admission` `pace_hold`) take the forecast as an input — a window projected to wall before its reset by more than the configured margin yields `pace_hold` for new placement on that pool, *before* the ≥85% caution zone is reached. One pool first (named in the implementation PR), behind a backend-style flag with current behaviour as default-and-rollback (same posture as `PM_AGG_STORE_BACKEND`). Acceptance: the named pool runs live on forecast admission ≥1 week with the flag on; realised outcomes scored via S2 and reported on the issue; rollback is the flag alone and is exercised once; **no placement decision is ever taken on a stale or honesty-flagged forecast** (stale forecast = current behaviour, per the stale-read doctrine). Supporting code present in tree: `quota-admission-shadow.ts`, `quota-shadow-placement.ts` (S3's shadow mode — S3's own tracking issue was not individually read; **[unverified]** its closure state).
- **S5 (code#12699) — open** (from the epic search listing; not read in full). Code present: `quota-anomaly.ts`.
- **S6 — [unverified] issue number/state** (not read individually). Code present: `quota-reweight.ts`, `quota-reweight-pr.ts` — the proposal mechanism is PRs, “never silent edits” (epic text).
- **S7 (code#12701) — open**, labels include `status:in-review`, `kind/feature`, `priority/normal`, `app/pm-stack`. Scope: a scheduled weekly job (registered in `doc/fleet-schedule-registry.md` in the same pass) posting the forecast-accuracy report to epic #12694 — per-pool projected-vs-realised, S5 detector findings, and any rule adjustments the accuracy record warrants (adjustments proposed through S6, never edited in place). Acceptance: first weekly report posted on #12694 *by the job*; the job in the fleet schedule registry; at least one rule consequence (an S6 proposal, or an explicit no-change finding with the accuracy numbers) in the first two reports. Note the search result code#13263 “feat(quota-plane): S7 weekly forecast-accuracy review job (#12701)” is **closed** — an implementation issue under the still-open S7 tracker, with code present (`quota-weekly-review.ts` + tests). Whether the weekly job has actually *run* and posted to #12694 is **[unverified]** in this pass (the epic's comments were not read).

Related quota-plane state from search (not read in full; titles/states only): Stage-2 epic code#1149 open; code#2490 (unify quota read path — server endpoint, CLI and gatherer must share the direct credential-store read) open; code#1270 (plan-store/tracker/measured reconciliation diff, daily) open; code#1173 (daily model + route audit) open.

### 6.6 Doctrine summary (one paragraph)

The quota plane as it actually operates today: a 15-minute laptop gatherer produces measured, timestamped per-pool readings; placement and dispatch may only consume a reading ≤60 min old, must treat stale/failed reads as *unknown* (never as a wall, never as a reason to idle a pool), burn each plan pool toward 100% at its reset in soonest-expiry order, treat a fresh measured 100% (or sustained 99.5% in admission code) as closed/walled for that period, never convert a quota wall into paid usage, and bench a provider door only on the combined <1%-remaining + plan-limit-failure signal. Everything predictive (forecast admission, anomaly detection, re-weighting, weekly accuracy review) is Stage 3: S1/S2 landed, S4/S7 still open and gated on Drew's design review, with the hard rule that no live decision is ever taken on a stale or honesty-flagged forecast.

---

## Appendix A — Source map (what was actually read)

Laptop checkout `~/git/code` @ `a66148f7393`, via `~/tools/laptop-ssh.sh`:

- `pkgs/fleet/crates/fleet-agent/src/{kinds.rs, enroll.rs, next.rs, shortlist.rs, lib.rs, tests.rs (grep)}`
- `pkgs/fleet/crates/fleet-secrets/src/lib.rs` (head + chain/retry sections)
- `pkgs/fleet/crates/fleet-queue/src/secrets_broker.rs`, `…/turso_queue/{claim.rs (grep), core.rs, lifecycle.rs}`, `…/client.rs (grep)`
- `pkgs/fleet/crates/fleet-bridge/src/lib.rs`
- `pkgs/fleet/crates/fleet-laptop-agent/src/{lib.rs, cli.rs}`; `pkgs/fleet/laptop-agent.py`, `pkgs/fleet/laptop-agent-broker.py` (headers); `pkgs/laptop-agent/package.nix`, `pkgs/laptop-agent/src/` (listing)
- `pkgs/fleet/crates/fleet-config/src/lib.rs` (port/token constants, grep); `fleet-registry/src/lib.rs` (head)
- `pkgs/message-bridge/worker/src/fleet.js` (validation + store ops)
- `pkgs/fleet/book/{concepts/secrets.md, concepts/bridge.md, concepts/agents.md, operations/services.md, operations/health-and-healing.md, tutorials/enrol-a-seat.md}`
- `pkgs/fleet/placement/policy.go` (head); `pm-stack/aggregator/src/quota-admission.ts`; `pm-stack/aggregator/src/` (quota file listing)

GitHub (`gh api`, developing-today/code): issues #12236, #13081, #12898 (+ comments), #8882 (+ comments), #12694, #12698, #12701, #12544; PRs #12970, #12558; search `quota-plane in:title`.

Workspace: `~/workspace/goals/keep-the-four-repos-healthy/hidden_files/provider-quota-registry.md` (header + pool map).

## Appendix B — Unverified / gaps (consolidated)

1. `felix` and `xaltha` as enrolled fleet seats (kind, groups, enrolment state) — no source or issue evidence found; only a branch prefix and a branch name respectively.
2. Whether the grok seat has completed enrolment since 2026-11-23 (no live `fleet bridge agents` read taken).
3. Whether a heal fix for code#13081's patience window has landed (issue still open; heal script not read).
4. Whether the heal/`init-ensure` BWS fetch still uses the name `secrets-broker.token` or was renamed to a `fleet.*` name (heal script not read).
5. The worker's `MAX_RECIPIENTS` value and which non-CLI callers supply explicit bridge `batch` ids (`relay.js`/`wake.js` not read).
6. Batch claims (#8217) — store support verified, routine callers not traced.
7. Quota: S3 and S6 tracking-issue numbers/states; whether the S7 weekly job has run and posted to #12694; whether the #1624 dedup fix landed; opencode#2256's state (out-of-scope repo).
8. Which laptop-agent client (Python vs Rust fleet binary) a given seat's `laptop-ssh.sh` resolves to today — the shim text delegates to the Python client; the Rust domain's stub status was recorded at shim-writing time and not re-verified live.
