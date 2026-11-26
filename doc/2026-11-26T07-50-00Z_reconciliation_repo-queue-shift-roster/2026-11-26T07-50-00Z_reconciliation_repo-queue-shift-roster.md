# Repo-queue worker shift roster reconciliation — 22 vs 33

Reconciliation of record for developing-today/code#15341, the first
deliverable of the repo-queue worker shift migration to OpenChamber
scheduled lanes (#2174 wave 1). Compiled 2026-11-26 by a xaltha lane
(Muse sandbox seat). This record reconciles the two lane rosters the
#14222 inventory (PR #15351) left unreconciled; it retires nothing and
measures no parity — those are #15341's later steps, gated below.

## Verdict

The 22 and the 33 are **two generations of the same family, with zero
job-id overlap**. The difference is a naming-and-filter generation
change, not 11 net-new lanes appearing beside the old 22:

- **Registry generation (22)** keys lanes by _pilot project_: 16 code
  pilots (letters A–P), plus 2 lanes each for nix, openchamber and
  opencode. The pilot's prompt carried the scope.
- **Projection generation (33)** keys lanes by _claim filter_, the
  designated-lane model of #8097 (see `AGENTS.md`, "The one queue"):
  25 unfiltered `any-*` lanes, 5 `code-*` (code-only) lanes and 3
  `not-code-*` lanes. The filter expression carries the scope, so the
  per-project pilot suffixes disappear.

Because no id survives the rename, a name-based diff of the two
rosters reads as 22 deletions + 33 additions. The family's parallelism
went 22 → 33 (+11) in the same step.

## Generation 1 — registry roster (22, recorded 2026-11-03)

Source: `doc/fleet-schedule-registry.md`, Surface 1 rows for
`repo-queue-worker-shift*` (last full live census 2026-11-03).
Cadence `interval@40m` for every row; owner
`goal:keep-the-four-repos-healthy`. 20 enabled, 2 disabled.

| Group             | Job ids                                                                                                                                                                                                                                                                                                                                                                                                                                                                      | Count  | State                   |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | ----------------------- |
| code pilot A–P    | `repo-queue-worker-shift`, `repo-queue-worker-shift-b`, `repo-queue-worker-shift-c`, `repo-queue-worker-shift-d`, `repo-queue-worker-shift-e`, `repo-queue-worker-shift-f`, `repo-queue-worker-shift-g`, `repo-queue-worker-shift-h`, `repo-queue-worker-shift-i`, `repo-queue-worker-shift-j`, `repo-queue-worker-shift-k`, `repo-queue-worker-shift-l`, `repo-queue-worker-shift-m`, `repo-queue-worker-shift-n`, `repo-queue-worker-shift-o`, `repo-queue-worker-shift-p` | 16     | enabled                 |
| nix pilot         | `repo-queue-worker-shift-nix`                                                                                                                                                                                                                                                                                                                                                                                                                                                | 1      | enabled                 |
| nix overflow      | `repo-queue-worker-shift-nix-b`                                                                                                                                                                                                                                                                                                                                                                                                                                              | 1      | **disabled**            |
| openchamber pilot | `repo-queue-worker-shift-openchamber`, `repo-queue-worker-shift-openchamber-b`                                                                                                                                                                                                                                                                                                                                                                                               | 2      | enabled                 |
| opencode pilot    | `repo-queue-worker-shift-opencode`                                                                                                                                                                                                                                                                                                                                                                                                                                           | 1      | enabled                 |
| opencode overflow | `repo-queue-worker-shift-opencode-b`                                                                                                                                                                                                                                                                                                                                                                                                                                         | 1      | **disabled**            |
| **Total**         |                                                                                                                                                                                                                                                                                                                                                                                                                                                                              | **22** | 20 enabled / 2 disabled |

## Generation 2 — projection roster (33, recorded 2026-11-25)

Source: the #14222 inventory, rows 5–7, compiled from the cron
definition projections on the felix seat
(`~/workspace/goals/keep-the-four-repos-healthy/crons/` and
`~/workspace/cron.d/` copies there). Cadence 40m per the inventory.
Live enabled state was unverifiable at inventory time and remains
unverified (see "Live scheduler check").

| Group      | Job ids                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                 | Count  | Filter          |
| ---------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ------ | --------------- |
| unfiltered | `repo-queue-worker-shift-any-a`, `repo-queue-worker-shift-any-b`, `repo-queue-worker-shift-any-c`, `repo-queue-worker-shift-any-d`, `repo-queue-worker-shift-any-e`, `repo-queue-worker-shift-any-f`, `repo-queue-worker-shift-any-g`, `repo-queue-worker-shift-any-h`, `repo-queue-worker-shift-any-i`, `repo-queue-worker-shift-any-j`, `repo-queue-worker-shift-any-k`, `repo-queue-worker-shift-any-l`, `repo-queue-worker-shift-any-m`, `repo-queue-worker-shift-any-n`, `repo-queue-worker-shift-any-o`, `repo-queue-worker-shift-any-p`, `repo-queue-worker-shift-any-q`, `repo-queue-worker-shift-any-r`, `repo-queue-worker-shift-any-s`, `repo-queue-worker-shift-any-t`, `repo-queue-worker-shift-any-u`, `repo-queue-worker-shift-any-v`, `repo-queue-worker-shift-any-w`, `repo-queue-worker-shift-any-x`, `repo-queue-worker-shift-any-y` | 25     | none (any repo) |
| code-only  | `repo-queue-worker-shift-code-a`, `repo-queue-worker-shift-code-b`, `repo-queue-worker-shift-code-c`, `repo-queue-worker-shift-code-d`, `repo-queue-worker-shift-code-e`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                | 5      | `repo = code`   |
| not-code   | `repo-queue-worker-shift-not-code-a`, `repo-queue-worker-shift-not-code-b`, `repo-queue-worker-shift-not-code-c`                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                        | 3      | `repo != code`  |
| **Total**  |                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                                         | **33** |                 |

The 5 + 3 split matches `AGENTS.md`'s record of the designated
filtered lanes ("5 code-only, 3 not-code per surface", #8097); the 25
unfiltered lanes are the default unfiltered claim expressed as named
lanes rather than as pilot letters.

## Mapping between the generations

There is no 1:1 lane mapping, and inventing one would be a guess. The
honest mapping is by _role_:

| Registry generation role                          | Projection generation successor                                                      |
| ------------------------------------------------- | ------------------------------------------------------------------------------------ |
| code pilots A–P (16, scope in prompt)             | split across `any-a`..`any-y` (unfiltered) and `code-a`..`code-e` (code-only filter) |
| nix / openchamber / opencode pilots (4 enabled)   | covered by `any-*` (unfiltered claim) and `not-code-a`..`not-code-c`                 |
| disabled overflow lanes (`-nix-b`, `-opencode-b`) | no successor ids; overflow capacity is the extra `any-*` letters (25 vs 16)          |

## Live scheduler check (2026-11-26, this seat)

Attempted, with the limits stated rather than smoothed over:

- This seat's live `cron.list` (xaltha sandbox seat, 2026-11-26) holds
  **zero** `repo-queue-worker-shift*` jobs — the family lives on the
  primary seat, whose live scheduler no lane seat can read.
- The repo-queue broker was unreachable in the same run
  (`fleet agent next`, queue rung: tunnel to the laptop broker,
  `Resource temporarily unavailable (os error 11)`; incident #15131),
  so the laptop's OpenChamber stores were also unreadable.
- This seat's own projection copy under
  `~/workspace/goals/keep-the-four-repos-healthy/crons/minutely/`
  contains no `repo-queue-worker-shift*` files; the 33-file projection
  set was on the felix seat.

**Conclusion:** neither generation's _live_ enabled state on the
primary seat is verified by this record. What is reconciled is the
rosters' identity, counts, naming model and role mapping. The
remaining live step is named, not hidden: a primary-seat `cron.list`
read, after which every surviving projection-generation lane gets a
registry row in the same pass.

**Registry defect recorded:** the 33 projection-generation lanes have
no rows in `doc/fleet-schedule-registry.md`. Under the registry's own
maintenance rule ("a schedule that exists on a surface but not here
is a defect"), that absence is now on record against #15341; rows are
deliberately _not_ added here, because their live states are
unverified and the registry does not carry guesses.

## The native side (migration target, for scale)

Registry Surface 2a (recorded 2026-11-03, amended 2026-11-23): 15
enabled OpenChamber queue lanes across the code / opencode /
openchamber / nix projects (`*/40 * * * *`, America/Chicago,
workhorse route; code-project lanes take `fleet agent next` as their
intake front door) plus the #3026 claimant/executor — 16 scheduled
tasks in total. Parity is therefore 15 native lanes against a Muse
family of 33 current-generation lanes, not against the stale 22.

## #2174 gating — verified state (2026-11-26)

Per #15341's acceptance criteria, gaps 2 and 4 were re-checked:

- Gap 1 (per-run history, PR #8437): merged, per #14222's record.
- Gap 3 (failure watchdog, PR #11158): merged, per #14222's record.
- Gap 2 (token accounting, #2072 exporter work): **still open** —
  #2072 is OPEN as of this check (2026-11-26, `gh issue view 2072`).
- Gap 4 (server residency — laptop OpenChamber always-on):
  **unverified** — the laptop was unreachable from this seat
  (incident #15131).

So the gate is _not_ confirmed closed, and no lane retirement may
start from this record. Next slice for #15341, in the same
"after this PR" form: after a primary-seat read is available,
`cron.list` on the primary seat shows each of the 33 projection ids
as enabled or absent, and the registry gains one row per surviving
lane; in parallel, gap 2 closes or the parity A/B proceeds with
token accounting explicitly marked unmeasured.

## Verification

Counts and ids in this record are checked by
`scripts/test_fleet_schedule_registry_roster.py` (oracle: this
record's sources — the registry's Surface 1 rows, the #14222
inventory's rows 5–7, and the Surface 2a header): the registry yields
exactly the 22 generation-1 ids with exactly the 2 named disabled
lanes, and this document enumerates exactly the 22 + 33 ids above,
with no overlap between the generations.
