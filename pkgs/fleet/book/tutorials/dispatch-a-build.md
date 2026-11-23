# Tutorial: dispatch a build remotely

This tutorial runs a build (or any command) in a target repository on a
fleet runner instead of on your own machine: first as a dry run, so you
read the placement decision before anything runs; then for real, so you
get a session handle; then polling that handle to a terminal result. It
also covers chaining a follow-up run onto the same host, pushing the Nix
cache, and the raw `gh` fallback for when no fleet binary is available.

It assumes the `fleet` binary is installed (see
[Installation and bootstrap](../getting-started/installation.md)). The
placement model behind this tutorial — surfaces, tiers and the registry
as data — is covered in [Placement and surfaces as data](../library/placement.md).

The verb is `fleet queue dispatch`. Despite living under `queue`, it is
not a broker verb: it is the placement engine absorbed into the queue
client. It decides **where** a job runs across the fleet's GitHub Actions
copies and runner providers, and dispatches the `fleet-command.yml`
workflow there. It is unrelated to the top-level `fleet dispatch` domain.

## Step 1: Dry-run first, and read the placement decision

Without `--dispatch`, the verb is a dry run: it loads the surface
registry, computes live capacity, picks a surface, prints the decision —
and writes nothing, dispatches nothing.

```bash
fleet queue dispatch --repo <owner/name> --command '<cmd>'
```

For example, a test run in this repository:

```bash
fleet queue dispatch --repo developing-today/code --command 'cargo test --workspace'
```

`--repo` and `--command` are both required, and `--repo` must be a valid
`owner/name` (exactly one `/`, characters `[A-Za-z0-9._-]`); anything
else is a fatal usage error, exit `2`. Every error in this verb family
prints `repo-queue: <message>` on stderr and exits `2`.

What the output means (human form):

```text
decision: <surface-id | queued>
  - <reasoning line>
  - …
dry-run (no session written; pass --dispatch to submit)
```

Read the reasoning lines — they are the decision, not decoration. The
cascade that produced them, in order:

0. A hard pin (`--surface ID`), if you gave one: that surface or a fatal
   error, never a fall-through.
1. Follow-up affinity, if you gave `--followup` (Step 5).
2. **Tier 1 — free GitHub-hosted Actions**: every `github-actions`
   surface that is eligible and has free slots, sorted least-loaded
   owner first, then least-recently-used owner, then the registry's
   `selection_priority`, then per-surface recency, then surface id.
3. **Tier 2 — Blacksmith**, in registry order, within its minute budget.
4. **Tier 3 — Codespaces**, only when the laptop is marked busy or the
   surface was explicitly selected.

If every tier is exhausted, the decision is `queued` with the reasoning
"all tiers exhausted — job stays queued (recorded, never dropped;
repo-queue remains its system of record)".

For machine-readable output, add `--json`. A dry run's object carries
`job`, `decision`, `reasoning` and `"mode": "dry-run (no session written;
pass --dispatch to submit)"`, plus a `note` when a testing override was
used, `load_read_errors` when any load read failed, and
`owner_load_coverage` — per owner: whether its load was measured,
partial, unknown, or simulated, the known subtotal, the cap, and which
reads are missing. Load failures are soft, but an owner whose load could
not be read is reported as unknown capacity, never as an idle zero.

What can go wrong, and the fix:

- `pinned surface <id> is ineligible: <why>`, exit `2`: a `--surface` pin
  is a placement choice, not an override of eligibility. Read the
  `<why>` — a common one is pinning the `org-run` surface, whose registry
  status is `standby`, not `active`, so it can never be selected or
  pinned. Drop the pin or pick an eligible surface.
- `--surface <id>: no such surface in the registry`: the id is not in
  the registry (including surfaces whose `kind` has no adapter). List
  the registry with `fleet queue surfaces` and copy an `id` exactly.
- A macOS job (`--platform macos`) is ineligible on surfaces without
  `platform_input: true`, and counts against the owner's macOS sub-cap
  (5 for every shipped owner). If the decision surprises you, the
  reasoning lines name the surface that failed and why.

## Step 2: Dispatch for real, and read the `fp_` handle

Add `--dispatch` (and `--json`, so the handle is unambiguous):

```bash
fleet queue dispatch --repo developing-today/code --command 'cargo test --workspace' --dispatch --json
```

What the output means: the job object, the `decision`, the full
`reasoning` array, and then the session fields — `handle`, `run_id`,
`run_url`, and (when known) `runner_repo` and `portable_id`. The handle
has the form `fp_` followed by 32 lowercase hex characters, for example:

```text
fp_d0a842c9bd33aa2999f5a12393397ab9
```

Keep it. The handle is how you poll (Step 3), how a follow-up finds this
run (Step 5), and how the session is recorded in the local store at
`~/.local/state/fleet-place/sessions.json` (override the store directory
with `$FLEET_PLACE_STATE`). That store is a labelled stopgap with the
reference implementation's record shape, kept so records migrate when
the designed backend table lands.

How dispatch actually happens, so the output makes sense: for the chosen
surface's host repo, the client snapshots existing workflow runs, POSTs
the workflow dispatch (anything but HTTP `204` is fatal), then polls the
run list for up to about 40 seconds to correlate the new run — exactly,
when the surface declares a `dispatch_id` input, by matching a fresh
`fp_…` dispatch id placed in the run title.

What can go wrong, and the fix:

- The dispatch succeeds but no run correlates in time: the session is
  still written, with `run_id: null`, state `dispatched`, and the note
  "dispatch accepted (HTTP 204) but the run did not appear within ~40s;
  resolve it from the workflow's run list". This is not a failed
  dispatch — do not re-dispatch. Resolve the run from the host repo's
  workflow run list, then refer to it by its portable reference
  `<host-owner>/<host-repo>#<run-id>` (Step 4).
- The all-tiers-exhausted case **with** `--dispatch`: a session record is
  still written — `surface`, `run_id` and `run_url` all null, state
  `queued` — and its handle is printed (the JSON gains `handle` and
  `state`, but no `run_id`/`run_url` keys). The job is recorded, never
  dropped; nothing is running yet.

## Step 3: Poll the session to a terminal result

```bash
fleet queue session status <handle>
```

What the output means: the stored record is refreshed from GitHub first
(if it has a run and a registered host surface), then printed in the
form:

```text
<handle>: state=<state> surface=<surface-id> run=<run-url>
```

States are `queued`, `dispatched`, `running`, `succeeded`, `failed` and
`cancelled`, mapped from the GitHub run's status and conclusion. Add
`--json` to print the whole refreshed record instead of the one line.

When you need the verdict rather than the state:

```bash
fleet queue session result <handle>
```

That prints `<handle>: <state> (terminal=<bool>) run=<run-url>`, where
`terminal` is true exactly when the state is `succeeded`, `failed` or
`cancelled`. While it is not terminal, the record carries the note
"not terminal yet — reconnect with the same handle". Poll `result` on
your own cadence; there is no streaming form, and the run's own output
is not streamed either — it lives on the run page (`gh run view --log`).

Two neighbouring forms worth knowing:

```bash
fleet queue session list
fleet queue session list --mine
```

`list` prints every record in the local store, newest first — from the
stored state, **without** refreshing. `--mine` keeps only records whose
`submitted_by` is the current user on this host.

What can go wrong, and the fix:

- `repo-queue: no session with handle <h> in <store path>`, exit `2`:
  the handle is not in this seat's local store. Handles are local —
  another seat's store does not have your record. Use the portable
  reference instead (Step 4), which resolves live without the store.
- Passing `--help` to `session status` or `session result` is safe:
  a help flag after the subcommand prints the session usage and exits 0
  — it is never taken as a handle.

## Step 4: Refer to a run portably

Anywhere a handle is accepted, a portable reference
`OWNER/REPO#RUN_ID` is accepted too — where `OWNER/REPO` is the
**runner-host** repo of a registered `github-actions` or `blacksmith`
surface (for example `developing-today/fleet`), not the target repo:

```bash
fleet queue session result developing-today/fleet#45146933211
```

What the output means: the reference is resolved directly against the
GitHub API with that surface's token, producing a synthetic record with
the same state mapping as Step 3. This is the form that survives moving
between seats and workspaces, and the form a follow-up takes when you do
not have the original handle.

## Step 5: Chain a follow-up that sticks to the host

A follow-up downloads the previous run's `fleet-output` artifact and, by
preference, stays on the same host surface — which also keeps it in the
same Actions cache scope, since caches are per host repo.

```bash
fleet queue dispatch --repo developing-today/code --command '<cmd>' --followup <handle-or-portable-ref> --dispatch
```

Dry-run it first (drop `--dispatch`) and read the affinity line in the
reasoning. The preferred surface is kept only while **all** of these
hold: its adapter exists and is eligible; it has capacity; and the
owner's queued count is strictly below the surface's
`sticky_queue_threshold` — `2` on every shipped surface. With two or
more runs already queued on that owner, stickiness is released, and the
reasoning records exactly why: `followup affinity released from <id>:
eligible=<…>, capacity=<…>, queued=<n> (threshold 2); …`. On a keep, the
line reads `followup affinity keeps <id>: <capacity summary>`.

What can go wrong, and the fix:

- With `--dispatch`, a previous run that is not yet terminal is fatal:
  "previous run is still &lt;state&gt;; wait for a terminal result before
  downloading its output". Wait for Step 3's `result` to report
  `terminal=true` first.
- A previous dispatch with no correlated run yet is likewise fatal:
  "previous dispatch has no correlated run yet; resolve its status
  before continuing". Resolve it via Step 4, then chain from the
  portable reference.
- In dry-run mode neither guard fires, and a local previous record is
  read without refreshing — so a dry run can show an affinity a real
  dispatch would refuse. Treat the dry run as a placement preview, not a
  promise.

## Step 6: The `--push-nix-cache` variant

```bash
fleet queue dispatch --repo <owner/name> --command 'nix build' --push-nix-cache --dispatch
```

What changes: placement sends the workflow input `push_nix_cache: true`.
On success, the workflow's action signs `./result` and copies it to the
shared Fleet R2 cache — resolving `./result`, signing with the key from
the host repo's secret named `NIX_CACHE_SIGNING_KEY`, and uploading to
the URL in the secret named `NIX_CACHE_UPLOAD_URL` (names only; the
endpoint is secret-configured, not hardcoded).

What can go wrong, and the fix:

- "cache push requested but signing/upload secrets are not configured;
  skipping": the run still succeeds; only the push is skipped. The host
  surface you landed on does not carry the `NIX_CACHE_*` secrets — the
  `fleet-personal` registry note names them for that surface. Pin or
  follow up onto a surface whose host repo has them, or accept the skip.
- No `./result` in the target checkout: also a notice-and-skip. Build
  something that leaves a `./result` link, or drop the flag.
- A related variant: `--actions-cache KEY:PATH` binds one extra Actions
  cache path under the key prefix `KEY` (default namespace prefix
  `fleet-deps-v1`), with `--actions-cache-restore-key PREFIX` (repeatable,
  and it requires `--actions-cache`) adding restore prefixes.

## Step 7: The raw `gh workflow run` fallback

When no fleet binary is available — a foreign host, a broken install —
the same workflow can be dispatched directly. You give up placement (you
choose the host repo yourself), session records, and correlation; you
keep the identical workflow contract.

```bash
gh workflow run fleet-command.yml --repo developing-today/fleet --field repo=developing-today/code --field command='cargo test --workspace'
```

The workflow is `fleet-command.yml` in a fleet host repo
(`dezren39/fleet`, `developing-today/fleet` or
`developing-today-forks/fleet`). Its inputs include `repo` and `command`
(both required), plus optional `ref`, `runner` (`github` or
`blacksmith`), `platform` (`linux` or `macos`), `container_image`,
`actions_cache_key` (default `fleet-deps-v1`), `actions_cache_path`,
`actions_cache_restore_keys`, `previous_run_id`, `previous_run_repo`,
`push_nix_cache` and `timeout_minutes` (5–120, default 60 — an input
placement itself never sends, so placement dispatches always get the
60-minute default). Follow the run with `gh run list --repo <host-repo>
--workflow fleet-command.yml` and `gh run view --log`.

What can go wrong, and the fix:

- Dispatching in the target repo instead of a host repo: there is no
  `fleet-command.yml` in `developing-today/code` — it lives only in the
  fleet host repos. Re-run against a host repo, with the target named in
  the `repo` field.
- `runner=blacksmith` from any host repo other than
  `developing-today/fleet`, or with `platform=macos`: the workflow's
  normalize job rejects both. Use the `developing-today/fleet` host for
  Blacksmith, on Linux.

## You are done when

- A dry run printed a `decision` whose reasoning lines you can explain:
  which tier picked the surface, or why the pin/affinity held or was
  released.
- The real dispatch printed an `fp_…` handle (or a portable
  `OWNER/REPO#RUN_ID` reference you resolved from the run list).
- `fleet queue session result <handle>` reports `terminal=true` with
  the state you expected — `succeeded` for a good build — and you know
  the run's own log lives on its run page, not in the session record.
