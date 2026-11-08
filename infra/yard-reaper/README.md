# yard-reaper

Reaps the laptop's `~/git/fleet` scratch yard under Drew's disposition
rule (2026-11-07, code#7093): **KEEP text, scripts, and markdown** —
provenance beats extension. This directory is the canonical home of
the tool: script, systemd units, tests, and this runbook.

## Lineage

The yard reaper is the cargo-target-janitor (code#3448) widened to
the whole yard. The janitor already reaped every cargo `target/`
tree in the yard under a 150 GiB cap with fail-closed liveness
gates; #7093's audit showed the yard is far more than target trees
(probe sandboxes, nested clones, fixture floods, queue/ children),
so the same engine now also classifies **whole entries**. The
janitor's target plan runs unchanged inside every yard-reaper run;
its separate hourly service is retired once this one is installed
(both enforce the same cap with the same gates — running both would
double the liveness reads for nothing).

## What is reaped, and the proof each class needs

| Class | Proof before reap |
|---|---|
| `target/` trees (incl. ad-hoc `target-<id>`, scratch root targets, `queue/` children) | The janitor's gates: not claimed (task or issue), no open PR, terminal task or idle ≥ 48h, no build process inside, cap pressure. Re-verified fresh before each removal. Removal is `rmtree` of the exact planned path (build output is regenerable; this is the janitor's standing behavior). |
| Nested clones (`.git` is a directory, not a registered worktree) | `git status` completely clean AND HEAD equal to the **live** tip of the upstream branch (`git ls-remote`, never a stale tracking ref). Anything uncommitted or unpushed protects the clone. |
| Scratch trees (probe-*, lane scratch, fixture floods) | **Zero keep-set files** outside bulk subtrees, per the heuristics below. |
| Registered worktrees | Never touched here — the worktree-reaper owns them (their `target/` trees are still in the target plan). Registration is read from **every** primary under `~/git`, not just one: the yard holds worktrees of code, opencode, openchamber, nix, and more, and a worktree of any primary is registered. |
| Yard-root loose files | Never touched — the known-precious set. |

Whole entries are **moved to the recoverable trash**
(`~/.local/share/Trash/yard-reaper/<run-id>/`, `queue/` children
under `queue/`), named one by one — never deleted outright, never
by wildcard. A `reap-start` record is fsynced to the run log before
each move.

## Keep-set heuristics (stated per #7093)

- Keep suffixes: `.md .txt .sh .py .ts .js .json .yaml .yml .toml .log`.
- Provenance beats extension: files inside `target/`, `dist/`,
  `node_modules/`, or `.git/` never count as keep-set (generated
  bulk), and neither do files under a directory whose name marks it
  as bench-fixture bulk (`watcher-bench`, `bench-fixture`, `fixture`).
- An entry holding even one keep-set file outside those places is
  protected as `holds-keep-set` and reported by name — extraction
  of its keep set (review → PR, per Drew's rule: preservation =
  pushed or PR'd, no local preservation tier, no archive branches)
  is separate follow-up work, never improvised by the reaper.

## Safety gates (every reap, both plans)

- **Idle ≥ 4h** for whole entries (any file mtime; the janitor's
  48h threshold still governs target trees).
- **Liveness**: broker claimed task ids + GitHub fallback claimed
  issues, open-PR branches — all read completely, paginated,
  fail-closed (an unreadable liveness source refuses the run).
- **Occupancy**: a fresh `/proc` scan of ALL processes (not just
  builds) immediately before each removal; any cwd inside protects.
- **Examination holds**: `DENY_NAMES` in the script (currently
  `7780-1931-slice1` and `11081`, under separate examination).
- **Caps**: at most `--max-entry-removals` (default 100) whole
  entries per run; the rest are logged `deferred`.
- Every gate is re-checked against fresh state immediately before
  each individual removal; a gate tripping mid-run stops the run
  (exit 2, with a `refused` record in the run log) rather than
  skipping ahead.

## Runs and reports

State dir: `~/.local/state/yard-reaper/runs/yard-run-<ts>-<pid>.jsonl`
— one JSONL per run: `plan` (every target and entry with its class,
size, keep-file count, and protection/reason), `delete-start` /
`deleted` per target, `reap-start` / `reaped` per entry, and a
`summary` (bytes reclaimed per plan, entries reaped/deferred by
name, trash root). Dry-run is the default; `--apply` reaps.

Schedule: `yard-reaper.timer`, hourly with a 10-minute random delay
— the same cadence the retired janitor timer ran on.
