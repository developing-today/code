# Fleet placement: where build jobs run

`dezren39/fleet` now exists in more than one home, and the fleet has
more than one kind of runner. This document is the contract for how a
job finds its surface — and the frozen interface agents use, so the
backends can multiply without the clients ever changing.

Companion pieces, all in this repo:

- `tools/fleet-surfaces.json` — the **surface registry** (data).
- `tools/fleet-place` — the **placement dispatcher** (reference
  implementation; destined to be absorbed into `repo-queue`, see
  "Absorption" below).
- `.github/workflows/fleet-run.yml` — the generic runner the Actions
  surfaces host (this fork carries two deltas over upstream:
  the `runner` input and per-entry cache options).

## The surfaces

| Surface id             | Kind           | Host                           | What it is                                                                                                       |
| ---------------------- | -------------- | ------------------------------ | ---------------------------------------------------------------------------------------------------------------- |
| `fleet-personal`       | github-actions | `dezren39/fleet`               | The original personal copy of the umbrella.                                                                      |
| `fleet-org`            | github-actions | `developing-today/fleet`       | This fork, in the org.                                                                                           |
| `fleet-forks`          | github-actions | `developing-today-forks/fleet` | The forks-org copy — absorbed as registry data only, no dispatcher change.                                       |
| `org-run`              | github-actions | `developing-today/code`        | The org-level generic runner in the code repo (pre-registered; activates when `org-run.yml` lands on code main). |
| `fleet-org-blacksmith` | blacksmith     | `developing-today/fleet`       | The org fork's fleet-run with `runner: blacksmith` — a variant, not a separate workflow.                         |
| `codespace-overflow`   | codespace      | —                              | Start/reuse a codespace on the target repo; hand the job to a session inside it.                                 |
| `laptop`               | laptop         | —                              | Recorded handoff only; laptop execution belongs to pm-dispatch / repo-queue lanes.                               |

Every dispatch goes through the **developing-today GitHub App** —
both installations (org `174792676`, personal `175747567`, which
covers `dezren39/fleet`, so even the personal copy dispatches as the
app). Tokens are minted by the existing fleet-paperwork machinery
(`infra/fleet-paperwork/fleet_paperwork.py token --repo <host>` in
the code repo). There is no user-token dispatch path; a surface the
app cannot reach is _ineligible_ until it can, and the dispatcher
says so in its decision output rather than falling back.

Status at writing (2026-11-04): `fleet-personal` is fully active.
`fleet-org` and `fleet-org-blacksmith` wait on one org-owner grant
(add this repo to the app installation — the API refuses non-owners)
plus repo secrets (below). `org-run` is live (its workflow landed on code main, with the `runner` input added for parity).
`codespace-overflow` waits on the `codespace` OAuth scope for `gh`. `fleet-forks` waits on the app being _installed_ on `developing-today-forks` at all.

## The registry is the system

A location exists exactly when it has an entry in
`tools/fleet-surfaces.json`:

```json
{ "id": "fleet-org", "kind": "github-actions", "owner": "developing-today", "repo": "developing-today/fleet", "workflow": "fleet-run.yml", "app_installation": 174792676, "targets": ["developing-today/code", "..."], "status": "active", "notes": "..." }
```

Owner capacity posture lives beside the surfaces under `owners`
(plan, concurrency cap, private-minute budget). **Adding a location
is adding an entry** — the dispatcher core does not change.

Owners move, too. When a repo changes owners — the expected case is
pure-clone forks moving to an archive org — nothing here changes
shape: update the entry's `owner` / `repo` / `app_installation` (and
the `owners` block when the owner is new). The client interface, the
cascade, and existing session handles are untouched.

Each `kind` is served by one small **adapter** in `fleet-place`
implementing four operations:

```
eligible(job)  -> (bool, reason)
capacity(job)  -> {summary, ...}
dispatch(job)  -> {run_id, run_url}
status(session)-> mapped state
```

**Adding a placement type** is a new adapter plus a new `kind`
value in the registry. Dispatcher logic, the cascade, and session
handling stay untouched. The two checklists are at the end of this
document.

## Client interface (frozen, v1)

Agents — lane workers, Muse clients, humans — see exactly this,
regardless of how many surfaces or kinds exist behind it:

```bash
# Submit (dry-run unless --dispatch). One target repo + one command.
tools/fleet-place submit \
  --repo developing-today/code \
  --command 'nix build .#mcpx' \
  [--ref SHA] [--runner auto|github|blacksmith] [--surface ID] \
  [--push-nix-cache] [--actions-cache KEY:PATH] \
  [--allow-paid-blacksmith MINUTES] [--laptop-busy] \
  --dispatch

# Reconnect — from any client, any time:
tools/fleet-place status HANDLE
tools/fleet-place result HANDLE
```

A dispatch returns a **session handle** (`fp_` + 128-bit random).
The session record — handle, job payload, surface chosen and why,
Actions run id/URL, submitter, state, timestamps — is stored
server-side, not in the client. Reconnect = present the handle;
`status` maps the backing run's live state onto the session
(`queued | running | succeeded | failed | cancelled`), `result`
reports terminal state. Clients are stateless; if a client dies, the
job and its session do not.

**Session store (stopgap, labeled):** records currently live in
`~/.local/state/fleet-place/sessions.json` on the dispatching host.
The queue backend (Supabase `muse-dev2`) has no fitting table today —
the `agent.*` tables are read mirrors of other systems, and the
queue's operational schema is mid-refactor in another line of work.
The designed home is a dedicated table; the absorption issue carries
this DDL as the starting point:

```sql
create table agent.placement_sessions (
  handle        text primary key,            -- 'fp_' || 32 hex chars
  job           jsonb not null,              -- the submit payload
  surface       text,                        -- registry surface id
  surface_kind  text,
  decision      jsonb not null,              -- reasoning lines
  run_id        bigint,
  run_url       text,
  state         text not null,               -- queued|running|succeeded|failed|cancelled|dispatched
  submitted_by  text not null,
  actor         text not null,               -- 'developing-today[bot]'
  created_at    timestamptz not null default now(),
  updated_at    timestamptz not null default now()
);
```

### Absorption into `repo-queue` (the binary clients use)

`fleet-place` is the reference implementation, not the destination.
The spec the binary inherits — same registry file, same adapter
architecture, same handle semantics (the shape deliberately mirrors
the queue's existing job-id + pickup model):

```bash
repo-queue dispatch --repo developing-today/code \
  --command 'nix build .#mcpx' [--ref SHA] [--runner auto|github|blacksmith] \
  [--surface ID] [--push-nix-cache] [--actions-cache KEY:PATH] \
  [--allow-paid-blacksmith MINUTES]
# -> prints the session handle (fp_...) and the chosen surface

repo-queue session status HANDLE    # mapped state + run URL
repo-queue session result HANDLE    # terminal result
repo-queue session list [--mine]    # recent sessions
```

Token flow inside the binary: resolve the target host repo's owner →
mint that installation's token through the same fleet-paperwork flow
(App private key from the sops secrets file; JWT; installation
token) → POST the workflow dispatch → record the session row → print
the handle. No user token anywhere in the path.

## The placement cascade

Unless a job pins a surface (`--surface`), placement walks:

1. **GitHub-hosted Actions, eligible owner copies.** Free capacity
   per owner = plan concurrency cap − (running + queued jobs on that
   owner's fleet runner hosts, counted live via the API — a proxy
   for account load; GitHub exposes no account-wide runs endpoint).
   Least-loaded eligible surface wins. Public-repo targets burn no
   minutes on any plan; concurrency is the binding limit.
2. **Blacksmith, org surfaces only, inside budget.** The provider is
   orgs-only and charges for public-repo minutes past its free tier,
   so this tier spends a _budget_, not spare capacity (below).
3. **Codespaces, last resort** — only when every Actions tier is
   tapped **and the laptop is saturated** (registry `laptop.busy` or
   `--laptop-busy`, fed from pm-dispatch/queue depth where a caller
   can read it, else set by hand — deliberately not scraped
   telemetry). Reuse-only: fleet-place never _creates_ a codespace.
4. **Otherwise the job stays queued.** The decision is recorded as a
   session with state `queued`; repo-queue remains the system of
   record for queued work. Nothing is ever dropped for lack of a
   surface.

A job may also pin a _runner_ (`--runner blacksmith` skips tier 1;
`--runner github` skips tier 2) or a _surface_ (`--surface`), which
bypasses the cascade but never eligibility: a pinned surface that is
ineligible fails loudly with the reason.

### Capacity accounting

| Owner / pool             | Plan | Concurrent jobs         | Private minutes/mo        | Public minutes            |
| ------------------------ | ---- | ----------------------- | ------------------------- | ------------------------- |
| `dezren39` (personal)    | Pro  | 40 (macOS 5)            | 3,000 included            | unlimited                 |
| `developing-today` (org) | Free | 20 (macOS 5)            | 2,000 included            | unlimited                 |
| Blacksmith (org)         | —    | provider pool           | 3,000 **free**, then paid | 3,000 free, then **paid** |
| Codespaces (`dezren39`)  | Pro  | ≤3 live (fleet posture) | 180 **core-hours**/mo     | —                         |

GitHub numbers: Actions limits reference
(docs.github.com/en/actions/reference/limits), verified 2026-11-04.
The private budgets bind only when a _target_ repo is private — all
five current fleet targets are public, so today the concurrency caps
and the two provider budgets are what placement actually spends.

**Blacksmith budget rule.** The free 3,000 min/mo is the surface's
budget by default: the dispatcher tracks consumption from the
durations of the Blacksmith runs it dispatched (the API exposes no
usage endpoint), marks the surface tapped when the free tier is
spent, and stops placing there. Spending past it is a deliberate
act: `--allow-paid-blacksmith MINUTES` per job, or raise
`paid_cap_minutes` in the surface's registry `budget` — Drew's knob,
default 0.

## Triggering each surface directly

The dispatcher is the front door, but every surface is just a
workflow dispatch away (as the app — mint a token first):

```bash
TOK=$(python3 ~/git/code/infra/fleet-paperwork/fleet_paperwork.py token --repo dezren39/fleet)
# Personal copy, single-shot:
curl -X POST -H "Authorization: Bearer $TOK" \
  https://api.github.com/repos/dezren39/fleet/actions/workflows/fleet-run.yml/dispatches \
  -d '{"ref":"main","inputs":{"repo":"developing-today/code","ref":"<sha>","command":"git rev-parse HEAD","runner":"github"}}'
# Org fork, Blacksmith legs (once the fork is app-covered):
#   same call against developing-today/fleet with "runner":"blacksmith"
# Fan-out / together / cache options travel in the `jobs` input as
# JSON — see the fleet-run.yml header for the full schema.
```

`gh workflow run fleet-run.yml --repo <host>` works too, but files
the dispatch under the _user_ token — fine for a human poking their
own copy, not the fleet's dispatch path.

## Caching: build once, test anywhere

The cache fields/steps live in this fork's revision of
`fleet-run.yml` (the FORK DELTAS). A surface hosting upstream's
revision accepts entries carrying `cache` and ignores the field —
the options take effect on surfaces running the fork revision.

Dispatched jobs run in the fleet-build container, which already
_reads_ the fleet's caches (R2 + `developing-today` cachix
substituters are baked into the image). Two opt-ins let a job also
_write_ for its follow-ups:

- **Nix results → R2** (`--push-nix-cache`, or `"cache":{"push_nix":true}`
  in a jobs entry): the job leaves a `./result` symlink; after the
  command, the runner signs the result's closure with the existing
  fleet signing key and `nix copy`s it to the existing upload URL —
  the same sign+copy pattern as `containers/fleet-build/export.sh`,
  reusing the `NIX_CACHE_SIGNING_KEY` / `NIX_CACHE_UPLOAD_URL` repo
  secrets. No new keys. Any later job on any surface — including a
  test rerun — then fetches the closure instead of rebuilding.
  If the secrets are absent on the hosting repo the step notices and
  skips; it never fails the job.
- **Non-nix artifacts → actions/cache** (`--actions-cache KEY:PATH`,
  or `"actions_key"`/`"actions_path"` in an entry): a plain
  `actions/cache@v4` restore/save of a workspace-relative path.
  Per-repo cache (10 GB, 7-day eviction) — unlike R2 it does not
  cross surfaces; prefer R2 for anything a follow-up on _another_
  surface needs.

This fork still needs its secrets provisioned before either write
path does anything here: `GHCR_ORG_PAT` (cross-repo checkout),
`NIX_CACHE_SIGNING_KEY`, `NIX_CACHE_UPLOAD_URL`, `CACHIX_AUTH_TOKEN`
— the same four the personal copy carries.

## Keeping the copies in sync

Upstream is `dezren39/fleet`; this fork tracks it. Sync is a normal
git operation on a branch, PR'd and merged like any other change:

```bash
git fetch upstream            # dezren39/fleet
git merge upstream/main       # on a sync branch, then PR as the app
```

Two classes of divergence are deliberate and must survive syncs:
this fork's `fleet-run.yml` deltas (the `runner` input, the cache
fields/steps) and everything under `tools/` + this document, which
exist only here. Conflicts in `fleet-run.yml` resolve by re-applying
the deltas onto the new upstream file — they are small, commented
`FORK DELTAS`, and grep-able. The registry is fork-local data; a
third copy in another org starts from this file, not from upstream.

### Add a location (~3 steps)

1. **Copy**: fork `dezren39/fleet` into the new owner (or clone this
   fork's tooling into an existing copy).
2. **Install**: add the copy to the developing-today app's
   installation for that owner (org-owner GUI step if the API
   refuses), and provision the four repo secrets.
3. **Register**: add the surface entry (and the owner, if new) to
   `tools/fleet-surfaces.json`. No code changes. The dispatcher
   places there from the next invocation.

### Add a placement type

1. **Adapter**: implement `eligible / capacity / dispatch / status`
   for the new kind in `tools/fleet-place` (copy the smallest
   existing adapter; Blacksmith's is the model for a variant kind,
   Codespace's for a non-Actions kind).
2. **Register the kind**: use the new `kind` value in registry
   entries; add the adapter to the `ADAPTERS` map.
3. **Document the budget**: if the kind spends money or a quota,
   its registry entry carries a `budget` block and the adapter's
   `capacity()` reports against it — placement must never spend a
   budget it cannot see.

### org-run parity (done)

`org-run.yml` landed on code main on 2026-11-04 and the `runner`
input followed the same day (code PR #5417, merged as the app), so
the org-run surface is `active` in the registry and the Blacksmith
variant exists there too — a `fleet-org-blacksmith`-style registry
entry on the code host is all that remains if Blacksmith legs are
ever wanted on org-run itself.
