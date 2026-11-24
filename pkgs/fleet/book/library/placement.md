# Placement and surfaces as data

Placement — deciding _where_ a build job runs — is data, not code. The
surface registry is a JSON file; adding a location is adding an entry;
no surface is hardcoded in the engine. This chapter documents that file
field by field, how the registry is resolved and extended, the decision
types the engine works with, the local session store format, and how
another crate uses placement.

The live engine is Rust: the `placement` module of `fleet-queue`
(gated behind the crate's `client` feature), exposed as the
`fleet queue dispatch`, `fleet queue session` and `fleet queue surfaces`
verbs. It is the absorption of the `fleet-place` reference implementation
into the queue client. The operator's walkthrough is
[Tutorial: dispatch a build remotely](../tutorials/dispatch-a-build.md).

> Do not confuse this engine with `pkgs/fleet/placement/`. That
> directory is a different, unrelated Go proof-of-concept (a
> placement-_record_ package for a policy framework) that is
> deliberately not wired into the live queue client or any dispatcher.
> Two placement systems share one name; only the Rust module over
> `fleet-surfaces.json` dispatches anything.

## The surface registry: `fleet-surfaces.json`

The registry lives at `pkgs/fleet/fleet-surfaces.json` and is **compiled
into the binary** (`include_str!`). Resolution at runtime, in order:

1. An explicit override — the `--registry PATH` flag or the
   `FLEET_PLACE_REGISTRY` environment variable — is **strict**: a bad
   path is a fatal error, never a silent fallback. The file must parse
   as JSON and contain a `surfaces` array, or the error is
   `registry {path} has no surfaces array`.
2. Without an override, `$HOME/git/code/pkgs/fleet/fleet-surfaces.json`
   wins if it exists (a live checkout can carry a newer registry).
3. Then the cwd-relative `pkgs/fleet/fleet-surfaces.json`, if it exists.
4. Finally, the embedded copy.

Top-level shape:

```json
{
  "comment": "Fleet placement registry. …",
  "version": 2,
  "laptop": { "busy": false, "busy_source": "manual-flag", "note": "…" },
  "owners": { "<owner>": { "…": "…" } },
  "surfaces": [{ "…": "…" }]
}
```

- `comment`: human documentation of the format; never read by code.
- `version`: `2` in the shipped registry.
- `laptop.busy` (bool), `laptop.busy_source` (string, shipped value
  `"manual-flag"`), `laptop.note` (string): the saturation signal for
  the codespace tier. It is set from queue depth where a caller can read
  it, else flipped by hand or overridden per-dispatch with
  `--laptop-busy`. Deliberately not scraped telemetry.
- `owners`: per-owner caps, below.
- `surfaces`: the surface entries, below. `fleet queue surfaces` prints
  the resolved registry verbatim as pretty JSON — that output _is_ this
  file, which is the quickest way to read the registry a given binary
  will use.

### Every field of a surface entry

The union of fields across the shipped entries, with the code that
consumes each:

| Field                    | Type              | Consumed by / meaning                                                                                                                                                                                                                                                                                           |
| ------------------------ | ----------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `id`                     | string            | Surface identity: the `--surface ID` pin, the session record's `surface`, and the sort tie-break                                                                                                                                                                                                                |
| `kind`                   | string            | Adapter selector: `github-actions`, `blacksmith`, `codespace` or `laptop`. Any other value gets **no adapter** — the surface is invisible to the cascade and cannot be pinned either                                                                                                                            |
| `status`                 | string            | Eligibility requires exactly `"active"` for `github-actions`/`blacksmith` surfaces. Shipped values: `active`, `standby` (`org-run`), `pending-scope` (`codespace-overflow`), `handoff-only` (`laptop`). The codespace and laptop adapters do not apply this check                                               |
| `owner`                  | string            | The account whose cap, load and budget the surface draws on; load is aggregated per owner, not per surface                                                                                                                                                                                                      |
| `repo`                   | string or null    | The _runner-host_ repo the workflow is dispatched in (not the target repo). `null` for `codespace-overflow` and `laptop`. Used for token minting, load counting, run correlation, and the session record's `runner_repo` / `portable_id`                                                                        |
| `workflow`               | string or null    | The workflow file dispatched; `"fleet-command.yml"` for all Actions/Blacksmith surfaces, `null` for codespace/laptop                                                                                                                                                                                            |
| `targets`                | string array      | Target-repo patterns: exact `owner/repo`, `owner/*`, or `*` (see matching rules below)                                                                                                                                                                                                                          |
| `notes`                  | string            | Human documentation only; never read by code                                                                                                                                                                                                                                                                    |
| `app_installation`       | integer or null   | GitHub App installation id for that host repo. Documentation/record only in the Rust code — nothing in the engine reads it; token minting goes through a separate helper                                                                                                                                        |
| `dispatch_as`            | string, optional  | `"user"` dispatches with the signed-in user's token; absent or `"app"` uses an App installation token. Any other value fails eligibility                                                                                                                                                                        |
| `dispatch_id_input`      | bool              | The workflow declares a `dispatch_id` input. When true, dispatch mints a fresh `fp_…` handle (separate from the session handle) and sends it as that input; the workflow puts it in the run title, enabling exact run correlation. When absent/false, correlation falls back to pre-dispatch snapshot exclusion |
| `platform_input`         | bool              | The workflow declares a `platform` input. Gates macOS: a `platform=macos` job is ineligible on a surface where this is not `true`                                                                                                                                                                               |
| `previous_run_id_input`  | bool              | The workflow accepts `previous_run_id` / `previous_run_repo` inputs (follow-up output download)                                                                                                                                                                                                                 |
| `container_input`        | bool              | The workflow accepts `container_image`; a `--container` value is only sent when this is `true`                                                                                                                                                                                                                  |
| `sticky_queue_threshold` | integer           | Follow-up stickiness threshold. Code default if absent: `2`, clamped to at least 1. All shipped surfaces: `2`                                                                                                                                                                                                   |
| `selection_priority`     | integer           | Tier-1 sort key (third position). Code default if absent: `50`. Shipped: `10` everywhere except `org-run` at `20`. Lower sorts first                                                                                                                                                                            |
| `public_targets_only`    | bool, optional    | Only `fleet-forks` (`true`). Eligibility does a live `GET /repos/{target}` and requires `"private": false`                                                                                                                                                                                                      |
| `host_surface`           | string, optional  | Only `fleet-org-blacksmith` (`"fleet-org"`): records which github-actions surface hosts this Blacksmith variant. Documentation-level linkage; the Blacksmith adapter dispatches its own surface's repo/workflow directly                                                                                        |
| `budget`                 | object, optional  | Blacksmith: `{free_minutes_monthly, paid_cap_minutes, rate_note}`. Codespace: `{core_hours_monthly, note}`                                                                                                                                                                                                      |
| `max_live`               | integer, optional | Only `codespace-overflow` (`3`). Printed in the capacity summary only; **never enforced**                                                                                                                                                                                                                       |

### The seven shipped surfaces

| `id`                   | `kind`         | `status`      | Owner / host repo                                     | `dispatch_as` | Notable                                                                                                                                                                                             |
| ---------------------- | -------------- | ------------- | ----------------------------------------------------- | ------------- | --------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| `fleet-personal`       | github-actions | active        | dezren39 / dezren39/fleet                             | (app)         | Its repo secrets include `GHCR_ORG_PAT` and `NIX_CACHE_*` (names only) for private checkouts, cross-run artifacts and Nix cache pushes                                                              |
| `fleet-org`            | github-actions | active        | developing-today / developing-today/fleet             | user          | Canonical org runner; also the Blacksmith host                                                                                                                                                      |
| `fleet-forks`          | github-actions | active        | developing-today-forks / developing-today-forks/fleet | user          | `public_targets_only: true` — private targets are rejected because this host has no `GHCR_ORG_PAT` secret                                                                                           |
| `org-run`              | github-actions | standby       | developing-today / developing-today/code              | (app)         | `platform_input`, `previous_run_id_input`, `container_input` all false; priority 20. Because eligibility requires `"active"`, it can never be selected or pinned — retained for manual workflow use |
| `fleet-org-blacksmith` | blacksmith     | active        | developing-today / developing-today/fleet             | user          | `host_surface: fleet-org`; budget below                                                                                                                                                             |
| `codespace-overflow`   | codespace      | pending-scope | dezren39 / no repo                                    | —             | Budget 180 core-hours/month; `max_live: 3`; never creates codespaces                                                                                                                                |
| `laptop`               | laptop         | handoff-only  | dezren39 / no repo                                    | —             | Targets `["*"]`; a placement record only — this adapter never runs shell commands on the laptop                                                                                                     |

All Actions/Blacksmith surfaces target
`["dezren39/*", "developing-today/*", "developing-today-forks/*"]` except
`org-run`, which targets only `["developing-today/*"]`.

### Per-owner caps

From the shipped `owners` object:

| Owner                    | `concurrent_jobs_cap` | `concurrent_macos_cap` | `plan`                         | `private_minutes_monthly` |
| ------------------------ | --------------------- | ---------------------- | ------------------------------ | ------------------------- |
| `developing-today`       | 20                    | 5                      | `free-org`                     | 2000                      |
| `developing-today-forks` | 20                    | 5                      | `free-org (assumed - confirm)` | 2000                      |
| `dezren39`               | 40                    | 5                      | `pro`                          | 3000                      |

Only `concurrent_jobs_cap` is read as the cap: an owner missing from
`owners`, or a missing field, yields cap **0** — zero free slots, so the
surface is unusable in Tier 1. `plan` and `private_minutes_monthly` are
never read by the placement code; they are registry documentation.
For a macOS job the effective cap is
`min(concurrent_jobs_cap, concurrent_macos_cap)` — 5 for every shipped
owner — and the adapter deliberately counts _all_ of the owner's active
host runs against that minimum, so macOS headroom is
`max(0, 5 − (running + queued on that owner's hosts))`.

### Target matching

A repo string is valid (`valid_owner_repo`) when it has exactly one `/`,
both parts non-empty and not `.`/`..`, with characters
`[A-Za-z0-9._-]` only; a `--repo` failing this is a fatal usage error
(exit `2`). A pattern matches a repo when: `"*"` matches any valid
`owner/name`; `"owner/*"` matches when the owner is exactly equal and
the name is a single non-empty segment; anything else is exact string
equality. A surface targets a job iff any of its `targets` matches the
job's repo.

### Budgets that are, and are not, enforced

Blacksmith's registry budget is real: `free_minutes_monthly: 3000`,
`paid_cap_minutes: 0`, with a `rate_note` recording that Blacksmith is
orgs-only, free for 3,000 minutes a month, then paid per minute — and
that `paid_cap_minutes` is the knob to raise. Eligibility computes
`remaining = free + paid_cap + this job's --allow-paid-blacksmith −
used`, where `used` is reconciled from the local store's ledger of
Blacksmith dispatch durations for the current UTC month. The
`--allow-paid-blacksmith MINUTES` value (a float, default `0.0`) is an
eligibility allowance for this one decision only — not a per-run
spending cap, not a workflow input, not persisted.

By contrast, the codespace budget numbers (`core_hours_monthly: 180`,
`max_live: 3`) appear only in a capacity _summary string_. Neither is
measured or enforced anywhere; dispatch does not count live codespaces.

## Reading and extending the registry

Read the registry the way the binary does:

```bash
fleet queue surfaces
```

That prints the resolved registry as pretty JSON (a `--registry PATH`
flag reads a different file). To extend placement:

1. Add a surface entry to `fleet-surfaces.json` with a unique `id`, a
   `kind` that has an adapter, the `owner` whose caps it draws on (add
   the owner to `owners` first if it is new — a missing owner means cap
   0), its host `repo` and `workflow`, and its `targets`.
2. Add a compatible workflow to the host repo: the entry's capability
   flags (`dispatch_id_input`, `platform_input`,
   `previous_run_id_input`, `container_input`) are promises about that
   workflow's declared inputs. Declaring a flag the workflow does not
   implement sends inputs into the void; omitting one the workflow
   supports disables the feature silently.
3. Set `status: "active"` only when the surface should really be picked.
   `standby` is a supported, deliberate state: registered, documented,
   and unpickable.

No code change is needed for a new surface of an existing `kind`. A new
`kind` is a code change: one adapter implementing the trait below.

## Placement decision types

The engine's vocabulary, as researched from the module:

- **The job**: `{repo, command, ref, runner, platform, container,
surface_pin, preferred_surface, previous_run_id, previous_run_repo,
push_nix_cache, actions_cache, actions_cache_restore_keys,
allow_paid_blacksmith}` — exactly the fields a session record stores
  under `job` (below). `runner` is `auto | github | blacksmith`;
  `platform` is `linux | macos`. A job pinning `runner=blacksmith`
  makes every github-actions adapter ineligible, and vice versa for
  `runner=github`.
- **The adapter trait**: each registry `kind` is served by one adapter
  implementing `eligible(job, ctx, gh) -> (bool, reason)`,
  `capacity(job, ctx) -> {summary, free_slots, remaining_total_minutes}`,
  `dispatch(job, gh) -> DispatchResult`, and `status(session, gh)`
  (defaulting to the stored state). `free_slots` and
  `remaining_total_minutes` are both optional in a capacity report: a
  summary-only adapter (codespace) reports neither. For the GitHub
  Actions adapter, `free_slots` is present only when the owner's load
  coverage is measured or simulated — or when a partial known subtotal
  already proves saturation, yielding a proven `0`; an unreadable owner
  reports no estimate rather than a false zero.
- **`DispatchResult`**: `{run_id, run_url, note, codespace,
terminal_state}` — what a dispatch attempt hands back to be recorded
  as a session.
- **The decision**: `decide(registry, job, ctx, gh)` returns the chosen
  adapter (or none, for the all-tiers-exhausted case) together with the
  `reasoning` array — one human string per skip, candidate and pick,
  which is part of the dispatch verb's output, not a debug log.
- **The context (`ctx`)**: built per decision from the GitHub API and
  the local store — per-owner load `(running, queued)` counted only over
  that owner's fleet runner-host repos (two `total_count` reads per host
  repo, `in_progress` and `queued`; failures are soft and recorded in
  `load_read_errors`), per-owner load coverage (status
  measured/partial/unknown/simulated, known subtotal, cap, read time,
  source, and missing reads — also emitted as the decision output's
  `owner_load_coverage`), per-owner and per-surface last-dispatch epochs
  from the session store, the month's Blacksmith minutes used, and the
  laptop-busy flag (`--laptop-busy` or the registry's `laptop.busy`).

The cascade itself — hard pin, follow-up affinity (kept only while the
preferred surface is eligible, has capacity, and its owner's queued
count is strictly below its `sticky_queue_threshold`), Tier 1 free
Actions sorted by `(load, owner_last_dispatch, selection_priority,
surface_last_dispatch, surface_id)` — with a deterministic
unknown-capacity fallback, named in the reasoning, when no eligible
surface has measured headroom — Tier 2 Blacksmith in registry
order, Tier 3 Codespaces, then exhaustion — is walked step by step in
[Tutorial: dispatch a build remotely](../tutorials/dispatch-a-build.md).

## The session store format

The local store at `$FLEET_PLACE_STATE/sessions.json` defaults to
`~/.local/state/fleet-place/sessions.json`. Its existing record shape
remains readable. Dispatch commits locally before its best-effort mirror
write to the broker’s `placement_sessions` table. Listing merges local
records with up to 200 broker records and attempts to insert missing local
records into the mirror. Status, result and follow-up prefer the record
with the newer `updated_at` and select the broker copy on ties.

Store shape:

```json
{
  "sessions": { "<fp_handle>": { "…": "…" } },
  "blacksmith_dispatches": [{ "…": "…" }]
}
```

A missing or unparseable store loads as
`{"sessions": {}, "blacksmith_dispatches": []}`. Writes are
pretty-printed JSON with recursively sorted keys, written to a
per-process temp file and renamed over the store, under a lock
directory (`sessions.lockdir` beside the store; waiters poll every
50 ms up to a 15-second deadline; a lock older than 120 seconds is
stale and is broken). New sessions are committed under the lock by
re-loading the on-disk store and merging, so concurrent processes do
not drop each other's records.

A session record (dispatched case):

```json
{
  "handle": "fp_…",
  "job": { "repo": "…", "command": "…", "ref": "…", "runner": "…", "platform": "…", "container": "…", "surface_pin": "…", "preferred_surface": "…", "previous_run_id": "…", "previous_run_repo": "…", "push_nix_cache": false, "actions_cache": "…", "actions_cache_restore_keys": [], "allow_paid_blacksmith": 0.0 },
  "surface": "fleet-org",
  "surface_kind": "github-actions",
  "runner_repo": "developing-today/fleet",
  "portable_id": "developing-today/fleet#45146933211",
  "decision_reasoning": ["…"],
  "run_id": 45146933211,
  "run_url": "https://github.com/developing-today/fleet/actions/runs/45146933211",
  "state": "queued|dispatched|running|succeeded|failed|cancelled",
  "submitted_by": "<USER>@<hostname>",
  "actor": "developing-today[bot] (GitHub App dispatch)",
  "created_at": "YYYY-MM-DDTHH:MM:SSZ",
  "updated_at": "YYYY-MM-DDTHH:MM:SSZ",
  "note": "…",
  "codespace": "…"
}
```

Field notes an embedder must know:

- `handle` is `fp_` plus 16 random bytes from `/dev/urandom`,
  hex-encoded — 35 characters total, 128 bits.
- `surface_kind` is omitted in the all-tiers-exhausted record, which has
  `surface` / `run_id` / `run_url` null, `state: "queued"`, and no
  `runner_repo` / `portable_id`. `runner_repo` appears only when the
  surface has a repo; `portable_id` only when a run id is known;
  `note` only when the adapter supplied one; `codespace` only on
  codespace dispatches.
- Initial state: the adapter's `terminal_state` if set (codespace
  dispatches are created already-terminal), else `queued` if a run was
  correlated, else `dispatched`.
- `actor` is hardcoded to the App string even for `dispatch_as: "user"`
  surfaces — a known inaccuracy in the record for the majority of
  shipped surfaces. Do not build attribution on it.
- `blacksmith_dispatches` entries are `{handle, month, minutes,
run_id, note, reconciled}`: appended with `minutes: 0.0` at dispatch,
  then reconciled to the run's real duration (rounded to 2 dp) when the
  session reaches a terminal state, or before the next real dispatch
  decision.
- Refresh: `session status` / `session result` (and follow-up
  resolution, and Blacksmith reconciliation) re-read the GitHub run and
  update `state` + `updated_at` on change. `session list` does **not**
  refresh — it prints stored state as-is. A portable reference
  `OWNER/REPO#RUN_ID` bypasses the store entirely and resolves live, as
  described in the tutorial.

## Using placement from another crate

Placement is a module of `fleet-queue` behind the `client` feature, and
its researched public surface is the verb entry points the queue client
dispatches to: `cmd_dispatch`, `cmd_session` and `cmd_surfaces` in
`fleet_queue::placement`. The supported embedding today is therefore
the same one the binary uses — depend on `fleet-queue` with the
`client` feature and drive the verbs, either through
`fleet_queue::client::run` with a dispatch-shaped argument vector, or
through the placement entry points directly.

This example is illustrative: the `client::run` route uses the verified
entry point from [The queue client](queue-client.md); the exact
signatures of the individual `cmd_*` placement functions were not
verified in this pass, so they are not presented as checked signatures
here.

```toml
# Cargo.toml
[dependencies]
fleet-queue = { path = "../fleet-queue", features = ["client"] }
```

```rust
// Ask placement for a dry-run decision (no --dispatch flag) and let
// the placement module print its decision + reasoning, exactly as
// `fleet queue dispatch` would.
fn dry_run_decision() -> i32 {
    let args: Vec<String> = [
        "dispatch",
        "--repo", "developing-today/code",
        "--command", "cargo test --workspace",
        "--json",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    fleet_queue::client::run(&args)
}
```

The verb entry points share the standard signature, verified against
the source: `placement::cmd_dispatch(args: &[String]) -> i32`, and
likewise `cmd_session` and `cmd_surfaces`. The engine itself is
`pub fn decide(registry: &serde_json::Value, job: &Job, ctx: &Ctx, gh: &dyn Gh) -> R<(Option<Box<dyn Adapter>>, Vec<String>)>` —
it returns the chosen adapter (or `None` when the job stays queued)
plus the reasoning lines. `Job` is a public struct, and the adapter
types (`Adapter`, `Capacity`, `DispatchResult`, and the concrete
adapters) are re-exported publicly from `fleet_queue::placement`
(`pub use adapters::{...}`), so embedders can name them.
