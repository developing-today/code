# worktree-reaper

Retires dead fleet worktrees on the laptop, implementing Drew's
quiet-worktree rule (2026-11-01). Born from the 2026-11-01 reality check
(`worktree-reality-check-2026-11-01.md`): the fleet creates ~272
worktrees/day by template (one per dispatched task) and, until now,
nothing ever retired them automatically — the population grew 187 →
1,511 in five days and was cleaned by hand. Steady state under the
reaper should be ~100 standing trees instead.

**This directory is the canonical home of the whole system** — script,
config, systemd units, tests, and this runbook. Nothing load-bearing
lives only on the laptop.

## The rule

Scope: every **linked** worktree of every primary under `~/git` on the
laptop (primaries are discovered structurally — a checkout whose `.git`
is a directory; linked worktrees have `.git` as a file). Primaries
themselves are never reaped, and nothing under `~/backups` or `~/atlas`
is in scope.

A tree is reaped only when **all** gates hold at decision time:

| Gate | Proof |
|---|---|
| PUSHED | Branch tip is on origin: equal to `origin/<branch>`, an ancestor of it, contained in origin's default branch, or patch-equivalent to either (`git cherry`). Origin refs are refreshed first with a full-refspec fetch (`+refs/heads/*:refs/remotes/origin/* --prune`); if the fetch fails, that primary is skipped entirely (fail closed). Detached HEADs are **never** reaped in v1. |
| CLEAN | `git status` shows no tracked changes and no untracked files beyond the generated-dirt allowlist (below). |
| QUIET | Last activity ≥ `quiet_hours` (default 6) ago, absolute epoch. Activity = newest working-tree file mtime (the tree's root `.git` entry excluded) max()ed with the HEAD reflog timestamp. |
| UNOCCUPIED | A fresh `/proc` cwd scan shows no process inside the tree; re-scanned immediately before each removal. |
| NOT DENIED | Not on the config denylist, not locked, and not the tree the running script lives in (install-tree self-protection). |

Reap = `git worktree remove` (plain; `--force` **only** when the sole
blocker is allowlisted generated dirt, re-verified by a fresh `git
status` in the same run immediately before removal), then
`git worktree prune`, then `git branch -d` (plain `-d` only — a refusal
keeps the branch and is logged; nothing is ever `-D`'d). At most
`max_removals` (default 100) trees are reaped per run; the rest are
logged `deferred-cap`.

Dirty or unpushed trees are **counted in the run summary, never
auto-rescued and never auto-issued** — rescuing stays a human/agent
decision (the 2026-11-01 hygiene program's lesson: automatic issue
filing is how you hit a rate limit and lose track of what exists).

### Generated-dirt allowlist

Untracked files that do not block reaping (config keys
`generated_dirt_prefixes` / `generated_dirt_files`):

- anything under `pkgs/mcpx/internal/cli/logs/` (mcpx run logs),
- any file named `fleet-feed.json` (the pm aggregator's generated
  feed — observed at the tree root and at
  `pm-stack/aggregator/fleet-feed.json`).

Allowlisting applies to **untracked** entries only: a *modified tracked*
`fleet-feed.json` is real dirt and keeps the tree. Everything else
untracked — probe WIP, `.cloudflare/`, notes — keeps the tree.

## Logs and state

All state lives in `~/.local/state/worktree-reaper/` on the laptop
(pm-aggregator precedent):

- `runs/run-<ts>-<pid>.jsonl` — one JSON decision record per tree
  (tree, branch, tip, every gate's result, action taken) plus a summary
  line per run.
- `last-run.json` — the most recent run's summary (evaluated / reaped /
  kept-by-reason counts, bytes reclaimed).
- `config.local.json` — optional local config overlay (see Denylist).
- `DISABLED` — the kill switch (see below).

## Install (laptop)

Pre-merge bootstrap installs from a linked worktree of the laptop's
code primary on this branch; post-merge, reinstall from the primary
(the canonical home — a primary is structurally never reaped).

```sh
# on the laptop, from the checkout that holds infra/worktree-reaper:
$WR/systemd/install-units.sh        # WR = <checkout>/infra/worktree-reaper
systemctl --user list-timers 'worktree-reaper*'
```

`install-units.sh` substitutes the checkout path into the units,
copies them to `~/.config/systemd/user/`, adds the install tree to the
local overlay's `deny_paths` (belt-and-braces next to the script's
built-in install-tree refusal), daemon-reloads, and enables the hourly
timer (`OnCalendar=hourly`, `Persistent=true`, up to 10 min randomized
delay, so the first fire is within ~an hour of install).

### Reinstall / upgrade

Pull the checkout, re-run `install-units.sh`. The script itself is
always executed from the checkout, so a pull alone upgrades the code;
re-running the installer is only needed when the units change.

### Restore / uninstall

The reaper only removes worktrees whose content is proven on origin, so
"restore" of a reaped tree is just:

```sh
git -C ~/git/<primary> worktree add ~/git/fleet/<name> <branch>   # branch still exists locally or on origin
```

To stop the system: `systemctl --user disable --now worktree-reaper.timer`
(and remove the two unit files from `~/.config/systemd/user/` +
`daemon-reload` to uninstall fully). Logs in
`~/.local/state/worktree-reaper/` are plain files — keep or delete.

## Operating it

- **Dry run** (prints the full would-reap list, removes nothing;
  still fetches so the PUSHED proofs are real):
  `python3 bin/worktree_reaper.py --dry-run`
- **Kill switch**: `touch ~/.local/state/worktree-reaper/DISABLED` —
  the reaper exits 0 doing nothing. Remove the file to resume.
- **Denylist a tree**: add its absolute path to `deny_paths` (or a
  prefix to `deny_prefixes`, or its basename to `deny_names`) in
  `~/.local/state/worktree-reaper/config.local.json` — no repo change,
  no reinstall. For a permanent, fleet-wide exclusion, edit the shipped
  `config.json` in this directory via PR instead. The shipped config
  already denylists the 8 primaries' paths as belt-and-braces.
- **Config**: `config.json` next to this README (script searches
  `--config`, `$WORKTREE_REAPER_CONFIG`, then the repo copy). Keys:
  `git_root`, `state_dir`, `quiet_hours`, `max_removals`, `primaries`
  (empty = discover), `deny_paths/prefixes/names`,
  `generated_dirt_prefixes/files`, `fetch`. The state-dir overlay
  `config.local.json` merges on top (deny lists union, scalars win).

## Tests

Synthetic-repo gate tests (bare origin + clone + worktrees; every gate
exercised end-to-end, plus parser/allowlist/walk unit tests):

```sh
python3 tests/test_worktree_reaper.py
```

Run them on the laptop — the occupied gate needs a real `/proc`.

## Bootstrap note (2026-11-01)

Initial rollout ran: build + tests on the laptop → dry-run against the
live farm with hand re-verification of sampled candidates → one
supervised live run → timer install. Once the PR merges, reinstall from
`~/git/code` per the Install section and remove the bootstrap worktree.
