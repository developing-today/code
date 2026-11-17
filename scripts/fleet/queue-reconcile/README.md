# Queue coverage reconciliation (#7016)

The standing check that every open issue in a fleet repo holds a **live**
task in its own pilot queue.

## Why

Issues enter the fleet queue only when a creator-class lane files them
(file + enqueue in the same pass). Issues filed any other way — a work
lane filing a blocker mid-task, an audit lane, by hand, or predating the
queue (epics) — silently never get a queue item, so no lane ever
dispositions them. The 2026-11-07 reconciliation found 23 such issues
across the four repos; that day's import was a one-off. This is the
standing version of it.

## The rule (Drew's doctrine, 2026-11-07)

**Every open issue must hold a LIVE task — `queued`, `claimed`, `parked`
or `blocked` — in its own pilot queue.** A `done` task is not coverage
(done means triage or a landed slice, not a resolved issue), and neither
is a task in the `*-shadow` queue.

## How it works

Two halves, deliberately split:

| Half | Where | Why |
|---|---|---|
| The diff | `POST /coverage` on the queue broker | The broker owns the Turso connection; the diff is one query over live tasks, matched on the issue dedupe key. Read-only. |
| The import | `check_coverage.py` via `fleet queue enqueue` | Packets go through the sanctioned, validating, dedupe-keyed path like every other task. |

The dedupe key `repo-health:v1:triage:<repo>:issue:<n>` makes a re-run
idempotent by construction: an issue that already holds a live task is
never uncovered, and an enqueue that races another lane's import dedupes
onto the live row.

## Running it

```bash
# Report only (default) — writes a report, enqueues nothing
python3 scripts/fleet/queue-reconcile/check_coverage.py --observe

# Enqueue a triage packet for every uncovered issue
python3 scripts/fleet/queue-reconcile/check_coverage.py --import

# One repo, ad-hoc
python3 scripts/fleet/queue-reconcile/check_coverage.py --repo developing-today/code
```

Reports land in `~/.local/state/queue-reconcile/` (`latest.md` plus one
dated file per run).

Exit codes: `0` run completed, `2` configuration error, `3` a GitHub or
broker call failed. **A failure never reports as "0 uncovered"** — the
report names the failed repo in an ERRORS section.

## Schedule

`systemd/queue-coverage-reconcile.{service,timer}`: hourly at :41,
persistent. The service runs **observe-only** until `import` is written
to `~/.config/queue-reconcile/mode` — the same two-step rollout the
sweeper's revalidator uses, so the first cycles can be reviewed before
anything is enqueued automatically.

Install (on the laptop, once the units are reviewed):

```bash
mkdir -p ~/.config/systemd/user
cp scripts/fleet/queue-reconcile/systemd/queue-coverage-reconcile.* ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now queue-coverage-reconcile.timer
```

## Tests

```bash
python3 -I scripts/fleet/queue-reconcile/test_check_coverage.py
```

Offline: `--fixture` replaces both the GitHub enumeration and the broker
call, and a stub `fleet` binary records enqueues, so no live access and
no queue mutation are needed.

The broker half is covered by the `turso_queue` unit tests in
`pkgs/fleet/crates/fleet-queue/src/turso_queue.rs` (parser cases plus two
`proptest` properties: canonical keys round-trip for every class and
dispatch suffix, and a key for another repo never matches; plus the
legacy repo-name aliases — `developing-today/opencode`,
`developing-today/openchamber`, `dezren39/nix` — matching their renamed
counterparts, and not crossing into a different repo).
