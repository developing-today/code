# Placement and remote dispatch

*Placement* is the decision of where a build job runs. The live engine is
Rust, inside the queue crate, exposed as `fleet queue dispatch`,
`fleet queue session …`, and `fleet queue surfaces`. (A Go directory at
`pkgs/fleet/placement/` is an unwired proof of concept for a different
placement-record design; it is not this engine, and its own README says so.)

Source: https://github.com/developing-today/code/blob/main/pkgs/fleet/crates/fleet-queue/src/placement.rs

## Surfaces as data: the registry

No surface is hardcoded. A *surface* is one entry in the registry file
`fleet-surfaces.json` (version 2), and adding a location is adding an entry.
Resolution order: `--registry <path>` or the `FLEET_PLACE_REGISTRY`
environment variable (strict — a bad path is fatal, never a silent fallback);
else `$HOME/git/code/pkgs/fleet/fleet-surfaces.json` when it exists; else the
cwd-relative `pkgs/fleet/fleet-surfaces.json`; else the copy embedded in the
binary. `fleet queue surfaces` prints the resolved registry verbatim as
pretty JSON.

The top level is `{comment, version, laptop: {busy, busy_source, note},
owners: {…}, surfaces: […]}`. Each surface entry carries:

| Field | Meaning |
|---|---|
| `id` | Surface identity; the `--surface` pin, session records, and tie-breaks key on it. |
| `kind` | Adapter selector: `github-actions`, `blacksmith`, `codespace`, or `laptop`. Any other value has no adapter and is invisible to the cascade (and cannot be pinned). |
| `status` | `base_eligible` requires exactly `active` for Actions/Blacksmith surfaces. Shipped values: `active`, `standby`, `pending-scope`, `handoff-only`. |
| `owner` | The account whose cap, load, and budget the surface draws on. Load is aggregated per owner, not per surface. |
| `repo` | The *runner-host* repo the workflow is dispatched in (not the target repo); null for codespace and laptop. |
| `workflow` | The workflow file dispatched; `fleet-command.yml` for all Actions/Blacksmith surfaces. |
| `targets` | Target-repo patterns: exact `owner/repo`, `owner/*`, or `*`. |
| `dispatch_as` | `user` = the signed-in user's token; absent/`app` = a GitHub App installation token. Any other value fails eligibility. |
| `dispatch_id_input` | The workflow declares a `dispatch_id` input (used for exact run correlation). |
| `platform_input` | The workflow declares a `platform` input; gates macOS eligibility and sending the input. |
| `previous_run_id_input` | The workflow accepts the follow-up output-download inputs. |
| `container_input` | The workflow accepts `container_image`; `--container` is only sent when this is true. |
| `sticky_queue_threshold` | Follow-up stickiness threshold; code default 2, minimum 1. All shipped surfaces: 2. |
| `selection_priority` | Tier-1 sort key; code default 50. Shipped: 10 everywhere except `org-run` at 20. Lower sorts first. |
| `public_targets_only` | Only `fleet-forks` (true): eligibility does a live repo read and requires `"private": false`. |
| `budget` | Blacksmith: `{free_minutes_monthly, paid_cap_minutes, rate_note}`. Codespace: `{core_hours_monthly, note}`. |
| `max_live` | Only `codespace-overflow`: 3. Printed in the capacity summary; **never enforced** (source behaviour). |

The seven shipped surfaces: `fleet-personal` (github-actions, dezren39/fleet,
App dispatch, active), `fleet-org` (github-actions, developing-today/fleet,
user dispatch, active; also the Blacksmith host), `fleet-forks`
(github-actions, developing-today-forks/fleet, user dispatch, active,
public targets only), `org-run` (standby — because eligibility requires
`active`, it can never be selected or pinned; retained for manual use),
`fleet-org-blacksmith` (blacksmith, Linux-only fallback on the fleet-org
host), `codespace-overflow` (codespace, `pending-scope`), and `laptop`
(`handoff-only` — a placement record only; its adapter never runs shell
commands on the laptop). Actions/Blacksmith surfaces target
`dezren39/*`, `developing-today/*`, and `developing-today-forks/*`, except
`org-run`, which targets only `developing-today/*`.

Target matching: a repo must be a valid `owner/name` (one slash, both parts
non-empty, characters `[A-Za-z0-9._-]`); `*` matches any valid repo,
`owner/*` matches that owner exactly with a single-segment name, anything
else is exact equality. A `--repo` failing validation is a fatal usage
error, exit 2 — and every error in the dispatch/session family prints
`repo-queue: <msg>` and exits 2.

## Owners and caps

From the registry's `owners` table:

| Owner | `concurrent_jobs_cap` | `concurrent_macos_cap` | `plan` | `private_minutes_monthly` |
|---|---|---|---|---|
| `dezren39` | **40** | **5** | `pro` | 3000 |
| `developing-today` | **20** | **5** | `free-org` | 2000 |
| `developing-today-forks` | **20** | **5** | `free-org (assumed - confirm)` | 2000 |

So the working summary is caps of 40/20/20, and a macOS cap of 5 for every
owner. An owner missing from the table, or a missing cap field, yields cap 0
— zero free slots, unusable in Tier 1. `plan` and `private_minutes_monthly`
are registry documentation only; the placement code never reads them, and
GitHub private-repo minute billing is not modelled in the cascade.

For a macOS job the effective cap is `min(concurrent_jobs_cap,
concurrent_macos_cap)` — 5 for every owner — and the adapter counts *all* of
the owner's active host runs (Linux included) against that minimum, so
dispatch never oversubscribes the small macOS pool. There is no
macOS-specific load measurement anywhere.

## The tier cascade

`decide` evaluates, in order:

0. **Hard pin.** `--surface ID` looks the id up in the adapter map (unknown
   id is fatal) and runs that adapter's eligibility; ineligible is fatal —
   no fall-through. A pin is a placement choice, not a paid-usage
   authorisation: a pinned Blacksmith surface is still subject to its budget
   check.
1. **Follow-up affinity** — a soft preference (below); falls through on
   failure.
2. **Tier 1 — free GitHub-hosted Actions.** Every `github-actions` surface
   that is eligible with `free_slots > 0`, sorted ascending by
   `(load, owner_last_dispatch, selection_priority, surface_last_dispatch,
   surface_id)`: least-loaded owner first; among equals, the
   least-recently-used owner (an owner never used sorts before all used
   owners); then registry priority; then per-surface last dispatch; then the
   surface id alphabetically. The rotation is owner-first.
3. **Tier 2 — Blacksmith.** `blacksmith` surfaces in registry order; the
   first eligible one with `remaining_total_minutes > 0` wins.
4. **Tier 3 — Codespaces.** `codespace` surfaces in registry order; the
   first eligible wins. Eligibility itself enforces the last-resort
   condition.
5. **Exhaustion.** The decision is `queued`: with `--dispatch`, a session
   record with `state: "queued"` and no surface or run is still written and
   its handle printed — the job stays queued, recorded, never dropped; the
   repo queue remains its system of record.

Runner pinning interacts before sorting: `--runner blacksmith` makes every
github-actions adapter ineligible, `--runner github` makes the Blacksmith
adapter ineligible, and `--runner auto` (the default) leaves both open.

## Load measurement

There is no account-wide Actions endpoint, so load is a proxy counted only
over each owner's *fleet runner-host repos* (the distinct `repo` values of
that owner's github-actions/blacksmith surfaces). For each host repo, with
that surface's token, placement reads only `total_count` from
`GET /repos/{host}/actions/runs?status=in_progress&per_page=1` and the same
with `status=queued` — the run lists themselves are never enumerated. Tier-1
load per owner is `running + queued`, summed across its host repos. Failures
are soft: no token, a non-200, or a transport error contributes 0 for that
repo/status and adds a `load_read_errors` entry, which is surfaced in the
dispatch output when non-empty. Owner and surface last-dispatch times come
from the local session store, not the API. Two testing overrides exist
(`--assume-owner-load owner=N,…` and `--assume-blacksmith-used MINUTES`);
they are simulation, are labelled as such in the output, and do not appear
in the printed usage string.

## LRU rotation

The Tier-1 sort's second and fourth keys are the rotation: `owner_last_dispatch`
and `surface_last_dispatch` are the epoch of the most recent stored session
for that owner/surface, or 0 if never. Because load dominates, rotation only
decides among equally loaded owners — which is the common case at zero load,
where it spreads work across owners instead of always picking the first.

## Follow-up affinity

`--followup HANDLE|OWNER/REPO#RUN_ID` chains a dispatch after a previous
run: it sets the preferred surface to the previous session's surface and
carries the previous run id and runner-host repo for the output-download
inputs. Before Tier 1, the preferred surface is kept if and only if its
adapter exists and is eligible, it has capacity, **and the owner's queued
count is strictly below the surface's `sticky_queue_threshold` — shipped
value 2 on every surface**. With 2 or more runs already queued on that
owner, stickiness is released and the cascade proceeds; the reasoning line
records exactly why. Two pre-dispatch guards apply to real dispatches: a
previous run that is not yet terminal is fatal ("wait for a terminal result
before downloading its output"), and one still in `dispatched` state (no run
correlated yet) is fatal. In dry-run mode neither guard fires.

Staying on the same host matters beyond queueing: GitHub Actions caches are
scoped to the repository the workflow runs in, so the same cache key string
in two host repos is two different caches.

## Blacksmith budget rules

Blacksmith is orgs-only: **3,000 free minutes per month**, then paid
per-minute (it charges for public-repo minutes past the free tier). The
registry budget on `fleet-org-blacksmith` is `free_minutes_monthly: 3000`,
`paid_cap_minutes: 0` — paid overage is disabled unless a dispatch
explicitly supplies `--allow-paid-blacksmith`. Code defaults if the budget
object is absent: free 3000, paid cap 0.

Eligibility, in order: the owner must not be `dezren39` ("Blacksmith is
orgs-only; never the personal copy" — a hardcoded owner check, not a
registry flag); Linux only; base eligibility (active, targets,
`dispatch_as`); not runner-pinned to `github`; a surface token must be
obtainable; and the budget must have minutes left:
`remaining = free_minutes_monthly + paid_cap_minutes +
job.allow_paid_blacksmith − blacksmith_used_minutes`.

`--allow-paid-blacksmith MINUTES` semantics, precisely: it is parsed as a
floating-point number (fractional minutes are accepted, default 0) and
**added to the paid cap for this one decision only**. It is an eligibility
allowance — not a per-run spending cap, not a workflow input, and not
persisted as an allowance anywhere. Once eligible, the run runs to
completion however long it takes; the actual minutes are reconciled into
the ledger afterwards and count against future decisions. `--runner
blacksmith` alone does not authorise paid minutes.

Usage accounting is local: `blacksmith_used_minutes` is the sum of `minutes`
over the session store's `blacksmith_dispatches` entries for the current UTC
month. A new dispatch appends an entry with `minutes: 0.0, reconciled:
false`; when the run reaches a terminal state, reconciliation replaces the
minutes with the run's real duration, rounded to 2 decimal places. Before
any real dispatch decision, outstanding Blacksmith handles are reconciled
first (unless the used-minutes override was given). The runner name on the
wire is `blacksmith-4vcpu-ubuntu-2404`; the workflow's normalize job also
errors if `runner=blacksmith` arrives with `platform=macos` or from any host
repo other than `developing-today/fleet`.

## The Actions cache and the Nix cache push

`--actions-cache KEY:PATH` (a single value; both sides must be non-empty)
becomes the workflow inputs `actions_cache_key` / `actions_cache_path`;
`--actions-cache-restore-key PREFIX` is repeatable, requires
`--actions-cache`, and travels newline-joined. Without `--actions-cache`,
placement sends no cache inputs and the workflow's own defaults apply:
key prefix **`fleet-deps-v1`**, empty path, empty restore keys.

On the workflow side, the cache root is `$GITHUB_WORKSPACE/.fleet/cache`
with cargo, cargo-target, rustup, and Go subdirectories exported as the
matching environment variables. A dependency hash (the first 20 hex
characters of a SHA-256) is computed over every lockfile-shaped file in the
target checkout plus the `rustc`/`go`/`nix` version strings. The full cache
key is `{PREFIX}-{RUNNER_OS}-{dependency_hash}-{target_ref}`, per host repo
in effect. Restore keys, in order: the caller's prefixes, then
`{PREFIX}-{RUNNER_OS}-{dependency_hash}-`, then `{PREFIX}-{RUNNER_OS}-`.
Prior-output reuse is a separate channel: when a follow-up supplies a
previous run, the action downloads that run's `fleet-output` artifact
(14-day retention) into `.fleet/previous-output`.

`--push-nix-cache` sends `push_nix_cache: true`. The push step runs only on
success with the flag set; if either of the repo secrets named
`NIX_CACHE_SIGNING_KEY` / `NIX_CACHE_UPLOAD_URL` is unset, or `./result`
does not exist in the target checkout, it notices and skips. Otherwise it
signs the result's recursive path closure with the key (written to a 0600
temp file) and copies it to the upload URL — the shared Fleet R2 cache. The
endpoint is secret-configured, never hardcoded in the workflow.

Two workflow-contract facts placement users hit: placement never sends the
workflow's `timeout_minutes` input, so **all dispatches run under the
60-minute default** (the workflow validates 5–120); and the `dispatch_id`
input is a *fresh* correlation id minted per dispatch, not the session
handle, despite the workflow describing it as one.

## Sessions: `fp_` handles and portable references

A dispatched job gets a session handle: `fp_` plus 16 random bytes from
`/dev/urandom`, hex-encoded — `fp_` plus 32 lowercase hex characters, 128
bits. Records live in the local store at `$FLEET_PLACE_STATE/sessions.json`,
default `~/.local/state/fleet-place/sessions.json` — a labelled stopgap with
the reference implementation's path and record shape, so records migrate
when the designed queue-backend table lands. The store is guarded by a lock
directory (waiters poll every 50 ms up to 15 s; a lock older than 120 s is
stale and is broken) and written by rename of a temp file, with keys
recursively sorted.

A record carries the job, the surface (null in the queued case), the runner
repo, the decision reasoning, the run id and URL, the state
(`queued|dispatched|running|succeeded|failed|cancelled`), `submitted_by`,
timestamps, and — when the run is known — the **portable id**
`OWNER/REPO#RUN_ID` (for example `developing-today/fleet#45146933211`). One
recorded-field caveat, stated as the source behaves: `actor` is hardcoded
to `developing-today[bot] (GitHub App dispatch)` even for surfaces that
dispatch as the user.

`fleet queue session status|result|list` reads the store: `status` refreshes
the record from GitHub and prints its state, surface, and run URL; `result`
adds `terminal` (true exactly for `succeeded|failed|cancelled`) and, when
not terminal, the note "not terminal yet — reconnect with the same handle";
`list` prints stored records newest first without refreshing (`--mine`
filters to the current `USER@hostname`). Anywhere a handle is accepted, the
portable `OWNER/REPO#RUN_ID` is accepted too and is resolved **directly
against GitHub without the local store** — the repo must be a registered
Actions/Blacksmith surface's host repo. Session state mapping from GitHub:
`queued|waiting|pending|requested` → `queued`; `in_progress` → `running`;
completed with conclusion `success|skipped|neutral` → `succeeded`,
`cancelled` → `cancelled`, anything else → `failed`. Note that
`session status --help` does not print help — `--help` is taken as the
handle and errors; only bare `session --help` prints the usage.
