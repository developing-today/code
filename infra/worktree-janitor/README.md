# Worktree janitor

Recurring, policy-driven janitor for aging fleet worktrees under
`~/git/fleet` on the laptop (code issue #6884). It replaces the one-off
manual passes (2026-11-07 sweep; 2026-11-10/11 cleanup pass) that only
ran after disk-full events. The `target/`-dir half of #6884's policy is
the cargo-target-janitor's job (`infra/cargo-target-janitor`, hourly)
and is out of scope here.

## Policy (the user, 2026-11-07, as corrected twice on #6884)

The corrections on the issue thread are binding and supersede the
issue body's original text:

- **Age gate: 4 hours.** Nothing younger than 4 hours is ever
  touched — the user's correction supersedes the body's 48h figure.
- **Clean worktrees** are removed only when the provably-safe gate
  from the 2026-11-10/11 pass holds in full, re-verified immediately
  before removal: clean (`git status` empty incl. untracked), HEAD an
  ancestor of `origin/main`, zero commits absent from every origin
  ref, no live process with cwd inside, last commit and tree mtime
  both ≥ 4h old. Removal is plain `git worktree remove` — never
  `--force`, never `rm`. Vanished registrations are cleared with
  `git worktree prune`.
- **Aging dirty worktrees** follow the user's mechanism: commit ALL
  changes, push upstream under the worktree's **own branch name**,
  verify the push landed (`ls-remote` == HEAD), only then remove.
  There are **no `archive/*` branches** (an assistant invention,
  revoked on the issue) and **no local-preservation tier** (a copy on
  the same laptop is not permanence). If the push cannot happen or
  cannot be verified, the worktree is not removed.
- **Detached dirty worktrees are kept**: they have no branch to push
  under, and the janitor invents no branch.
- Locked worktrees, worktrees containing submodules, primaries, and
  anything outside `~/git/fleet` are always kept. Every keep carries
  its reason in the report.

## Usage

```bash
worktree-janitor                 # dry-run report (default): decides, removes nothing
worktree-janitor --apply         # execute the decisions
worktree-janitor --json          # machine-readable report
worktree-janitor --fleet-root DIR --min-age-hours 4 --no-fetch
```

Primaries are discovered from the `.git` files of the fleet root's
entries; each primary is fetched once per run (unless `--no-fetch`)
so the ancestor / on-origin checks read fresh refs.

## Schedule

`systemd/worktree-janitor.timer` (user systemd, every 6 hours) via
`systemd/install-units.sh`, which installs the script as
`~/.local/bin/worktree-janitor` and enables the timer — the same
shape as the cargo-target-janitor. Registered in
`doc/fleet-schedule-registry.md`.

## Tests

```bash
python3 infra/worktree-janitor/tests/test_worktree_janitor.py
```

Gate-decision tests pin every clause above: dirty, unpushed,
non-ancestor, live-cwd, too-young (commit age and tree mtime),
locked, submodule, primary, outside-root, detached-dirty, and the
fully-gated removal case.
