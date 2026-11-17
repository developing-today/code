#!/usr/bin/env python3
"""Recurring worktree janitor for the fleet worktree tree (code issue #6884).

Policy (Drew, 2026-11-07, as corrected twice on issue #6884 — the
corrections are binding and supersede the issue body's original text):

- Nothing younger than 4 hours is ever touched. That is the only
  age protection; there is no 48h tier.
- Reproducible build artifacts are the cargo-target-janitor's job
  (infra/cargo-target-janitor); this janitor never deletes file
  content directly — it removes whole git worktrees, and only
  through `git worktree remove` (never --force, never rm).
- A clean worktree is removed only when the provably-safe gate from
  the 2026-11-10/11 cleanup pass (recorded on #6884) holds in full,
  re-verified immediately before removal:
    * clean (`git status` empty, including untracked files),
    * HEAD is an ancestor of origin/main,
    * zero commits absent from every origin ref,
    * no live process has its cwd inside the worktree,
    * last commit and tree mtime are both at least 4 hours old.
- An aging DIRTY worktree follows Drew's mechanism, not an archive
  scheme: commit ALL changes, push upstream under the worktree's
  OWN branch name, verify the push landed, only then remove.
  There are NO archive/* branches (an assistant invention, revoked)
  and NO local-preservation tier (a copy on the same laptop is not
  permanence). If any step fails, the worktree is kept untouched.
  A dirty worktree on a detached HEAD is kept: it has no branch to
  push under, and the janitor invents no branch.
- Anything failing a gate is kept, with the reason in the report.

Default mode is a dry-run report; --apply executes the decisions.
"""

from __future__ import annotations

import argparse
import concurrent.futures
import json
import os
import subprocess
import sys
import time
from dataclasses import dataclass, field

MIN_AGE_SECONDS = 4 * 3600  # Drew, 2026-11-07: "nothing younger than 4 hours"
HEAVY_DIRS = {".git", "target", "node_modules"}
MTIME_WALK_FILE_CAP = 20000


@dataclass
class WorktreeFacts:
    path: str
    primary: str
    branch: str | None = None  # None = detached HEAD
    is_primary: bool = False
    under_fleet: bool = True
    locked: bool = False
    prunable: bool = False  # registration whose path is already gone
    dirty: bool = False
    has_submodules: bool = False
    head_ancestor_of_origin_main: bool = False
    commits_not_on_origin: int = 0
    live_cwd: bool = False
    last_commit_epoch: int = 0
    tree_mtime_epoch: int = 0


@dataclass
class Decision:
    action: str  # "remove" | "preserve_then_remove" | "prune_registration" | "keep"
    reasons: list[str] = field(default_factory=list)


def decide(f: WorktreeFacts, now: int, min_age: int = MIN_AGE_SECONDS) -> Decision:
    """Pure gate: facts -> decision. The oracle is Drew's policy on #6884
    plus the 2026-11-10/11 cleanup-pass gate, both quoted in the README."""
    if f.is_primary:
        return Decision("keep", ["primary worktree"])
    if not f.under_fleet:
        return Decision("keep", ["outside the fleet worktree root"])
    if f.prunable:
        return Decision("prune_registration", ["registration path already gone"])
    if f.locked:
        return Decision("keep", ["worktree is locked"])
    if f.has_submodules:
        return Decision("keep", ["contains submodules; plain git worktree remove refuses"])
    if f.live_cwd:
        return Decision("keep", ["live process cwd inside worktree"])
    commit_age = now - f.last_commit_epoch
    mtime_age = now - f.tree_mtime_epoch
    if commit_age < min_age or mtime_age < min_age:
        return Decision(
            "keep",
            [f"younger than 4h (last-commit age {commit_age}s, tree-mtime age {mtime_age}s)"],
        )
    if f.dirty:
        if f.branch is None:
            return Decision(
                "keep",
                ["dirty on a detached HEAD: no branch to push upstream under, "
                 "and the janitor invents no branch"],
            )
        return Decision(
            "preserve_then_remove",
            [f"dirty and aging: commit all, push origin {f.branch}, verify, then remove"],
        )
    if not f.head_ancestor_of_origin_main:
        return Decision("keep", ["HEAD is not an ancestor of origin/main"])
    if f.commits_not_on_origin > 0:
        return Decision(
            "keep",
            [f"{f.commits_not_on_origin} commit(s) absent from every origin ref"],
        )
    return Decision("remove", ["clean, merged upstream, idle, aging: full gate passes"])


# ---------------------------------------------------------------- git helpers

def run_git(args: list[str], cwd: str | None = None) -> subprocess.CompletedProcess:
    return subprocess.run(
        ["git", *args], cwd=cwd, capture_output=True, text=True, timeout=120
    )


def discover_primaries(fleet_root: str) -> list[str]:
    """Primaries that own worktrees under fleet_root, found by resolving
    each entry's .git file to its common dir's parent worktree."""
    primaries: set[str] = set()
    try:
        entries = sorted(os.listdir(fleet_root))
    except OSError:
        return []
    for name in entries:
        dotgit = os.path.join(fleet_root, name, ".git")
        if not os.path.isfile(dotgit):
            continue
        try:
            with open(dotgit) as fh:
                first = fh.readline().strip()
        except OSError:
            continue
        if not first.startswith("gitdir:"):
            continue
        gitdir = first.split(":", 1)[1].strip()
        # gitdir = <primary>/.git/worktrees/<name>; common dir is <primary>/.git
        common = os.path.dirname(os.path.dirname(gitdir))
        primary = os.path.dirname(common)
        if os.path.isdir(os.path.join(common, "objects")):
            primaries.add(os.path.realpath(primary))
    return sorted(primaries)


def list_worktrees(primary: str) -> list[dict]:
    out = run_git(["-C", primary, "worktree", "list", "--porcelain"])
    if out.returncode != 0:
        return []
    entries: list[dict] = []
    cur: dict = {}
    for line in out.stdout.splitlines():
        if not line.strip():
            if cur:
                entries.append(cur)
                cur = {}
            continue
        key, _, val = line.partition(" ")
        cur[key] = val
    if cur:
        entries.append(cur)
    return entries


def live_cwd_paths() -> set[str]:
    paths: set[str] = set()
    proc = "/proc"
    try:
        pids = [p for p in os.listdir(proc) if p.isdigit()]
    except OSError:
        return paths
    for pid in pids:
        try:
            paths.add(os.path.realpath(os.readlink(os.path.join(proc, pid, "cwd"))))
        except OSError:
            continue
    return paths


def newest_tree_mtime(root: str, young_epoch: int = 0) -> int:
    """Newest file mtime under root, skipping heavy dirs. Early-exit:
    the first file younger than `young_epoch` settles the age gate
    (young = keep), so active trees cost one stat, not a full walk.
    If the walk hits the file cap we return `now` (treat as young =
    keep): the janitor only ever errs toward keeping."""
    newest = 0
    seen = 0
    for dirpath, dirnames, filenames in os.walk(root):
        dirnames[:] = [d for d in dirnames if d not in HEAVY_DIRS]
        for name in filenames:
            seen += 1
            if seen > MTIME_WALK_FILE_CAP:
                return int(time.time())
            try:
                mt = int(os.stat(os.path.join(dirpath, name)).st_mtime)
            except OSError:
                continue
            if mt > young_epoch:
                return mt
            newest = max(newest, mt)
    try:
        newest = max(newest, int(os.stat(root).st_mtime))
    except OSError:
        pass
    return newest


def gather_facts(primary: str, entry: dict, fleet_root: str, live_cwds: set[str],
                 young_epoch: int = 0) -> WorktreeFacts:
    path = entry.get("worktree", "")
    real = os.path.realpath(path)
    f = WorktreeFacts(path=real, primary=primary)
    f.is_primary = real == os.path.realpath(primary)
    f.under_fleet = real == fleet_root or real.startswith(fleet_root.rstrip("/") + "/")
    f.locked = "locked" in entry
    f.prunable = "prunable" in entry or not os.path.isdir(real)
    branch_ref = entry.get("branch", "")
    f.branch = branch_ref.removeprefix("refs/heads/") if branch_ref else None
    if f.prunable or f.is_primary:
        return f
    # --untracked-files=normal: untracked directories collapse to one
    # entry, which is all a dirty verdict needs — `all` walks every
    # untracked file in build-output trees and makes a fleet-wide scan
    # take orders of magnitude longer for the same verdict.
    status = run_git(["-C", real, "status", "--porcelain", "--untracked-files=normal"])
    f.dirty = bool(status.stdout.strip())
    f.has_submodules = os.path.isfile(os.path.join(real, ".gitmodules"))
    anc = run_git(["-C", real, "merge-base", "--is-ancestor", "HEAD", "origin/main"])
    f.head_ancestor_of_origin_main = anc.returncode == 0
    cnt = run_git(["-C", real, "rev-list", "--count", "HEAD", "--not", "--remotes=origin"])
    try:
        f.commits_not_on_origin = int(cnt.stdout.strip() or "0")
    except ValueError:
        f.commits_not_on_origin = 1  # unreadable: the >0 gate keeps the worktree
    log = run_git(["-C", real, "log", "-1", "--format=%ct"])
    try:
        f.last_commit_epoch = int(log.stdout.strip() or "0")
    except ValueError:
        f.last_commit_epoch = int(time.time())  # unreadable: treat as young, keep
    f.tree_mtime_epoch = newest_tree_mtime(real, young_epoch)
    f.live_cwd = any(c == real or c.startswith(real + "/") for c in live_cwds)
    return f


# ------------------------------------------------------------------- executor

def remove_worktree(f: WorktreeFacts, now: int, min_age: int, fleet_root: str) -> tuple[bool, str]:
    """Re-verify the full clean gate immediately before removal, then
    plain `git worktree remove` (never --force)."""
    entries = list_worktrees(f.primary)
    entry = next((e for e in entries if os.path.realpath(e.get("worktree", "")) == f.path), None)
    if entry is None:
        return False, "registration vanished before removal"
    fresh = gather_facts(f.primary, entry, fleet_root, live_cwd_paths())
    d = decide(fresh, now, min_age)
    if d.action != "remove":
        return False, f"re-check gate failed: {'; '.join(d.reasons)}"
    out = run_git(["-C", f.primary, "worktree", "remove", f.path])
    if out.returncode != 0:
        return False, f"git worktree remove failed: {out.stderr.strip()[:200]}"
    return True, "removed"


def preserve_then_remove(f: WorktreeFacts, now: int, min_age: int, fleet_root: str) -> tuple[bool, str]:
    """Drew's mechanism for an aging dirty worktree: commit ALL changes,
    push upstream under the worktree's own branch, verify the push
    landed, only then remove. Any failure keeps the worktree."""
    path, branch = f.path, f.branch
    add = run_git(["-C", path, "add", "-A"])
    if add.returncode != 0:
        return False, f"git add failed: {add.stderr.strip()[:200]}"
    commit = run_git(
        ["-C", path, "commit", "-m", "janitor: preserve aging worktree state (#6884)"]
    )
    if commit.returncode != 0:
        return False, f"commit failed: {(commit.stderr or commit.stdout).strip()[:200]}"
    head = run_git(["-C", path, "rev-parse", "HEAD"]).stdout.strip()
    push = run_git(["-C", path, "push", "origin", f"HEAD:refs/heads/{branch}"])
    if push.returncode != 0:
        return False, f"push failed, worktree kept: {push.stderr.strip()[:200]}"
    remote = run_git(["-C", path, "ls-remote", "origin", f"refs/heads/{branch}"])
    landed = remote.stdout.split()[0] if remote.stdout.split() else ""
    if remote.returncode != 0 or landed != head:
        return False, f"push did not verify (ls-remote {landed!r} != HEAD {head!r}); kept"
    return remove_worktree(f, now, min_age, fleet_root)


def main(argv: list[str] | None = None) -> int:
    ap = argparse.ArgumentParser(description=__doc__.splitlines()[0])
    ap.add_argument("--fleet-root", default=os.path.expanduser("~/git/fleet"))
    ap.add_argument("--min-age-hours", type=float, default=MIN_AGE_SECONDS / 3600)
    ap.add_argument("--apply", action="store_true", help="execute decisions (default: dry-run report)")
    ap.add_argument("--no-fetch", action="store_true", help="skip the per-primary fetch")
    ap.add_argument("--primary", action="append", default=[],
                    help="only consider this primary (repeatable); default: all discovered")
    ap.add_argument("--workers", type=int, default=8,
                    help="parallel fact-gathering threads (decisions and removals stay serial)")
    ap.add_argument("--json", action="store_true", help="emit the report as JSON")
    args = ap.parse_args(argv)

    fleet_root = os.path.realpath(os.path.expanduser(args.fleet_root))
    min_age = int(args.min_age_hours * 3600)
    now = int(time.time())
    live = live_cwd_paths()

    primaries = discover_primaries(fleet_root)
    if args.primary:
        wanted = {os.path.realpath(os.path.expanduser(p)) for p in args.primary}
        primaries = [p for p in primaries if p in wanted]
    report: list[dict] = []
    errors: list[str] = []
    jobs: list[tuple[str, dict]] = []
    for primary in primaries:
        if not args.no_fetch:
            run_git(["-C", primary, "fetch", "origin", "--quiet"])
        for entry in list_worktrees(primary):
            jobs.append((primary, entry))
    young_epoch = now - min_age

    def gather(job: tuple[str, dict]):
        primary, entry = job
        try:
            return primary, entry, gather_facts(primary, entry, fleet_root, live, young_epoch), None
        except Exception as exc:  # unreadable facts: keep, never guess
            return primary, entry, None, exc

    with concurrent.futures.ThreadPoolExecutor(max_workers=max(1, args.workers)) as pool:
        gathered = list(pool.map(gather, jobs))
    for primary, entry, facts, exc in gathered:
        if exc is not None:
            errors.append(f"{entry.get('worktree', '?')}: facts failed: {exc}")
            continue
        if not facts.under_fleet and not facts.is_primary:
            continue  # other roots are other janitors' scope
        decision = decide(facts, now, min_age)
        row = {
            "path": facts.path,
            "primary": primary,
            "branch": facts.branch,
            "action": decision.action,
            "reasons": decision.reasons,
            "applied": False,
            "result": "",
        }
        if args.apply and decision.action == "remove":
            ok, msg = remove_worktree(facts, now, min_age, fleet_root)
            row["applied"], row["result"] = ok, msg
        elif args.apply and decision.action == "preserve_then_remove":
            ok, msg = preserve_then_remove(facts, now, min_age, fleet_root)
            row["applied"], row["result"] = ok, msg
        elif args.apply and decision.action == "prune_registration":
            out = run_git(["-C", primary, "worktree", "prune"])
            row["applied"] = out.returncode == 0
            row["result"] = "pruned" if out.returncode == 0 else out.stderr.strip()[:200]
        report.append(row)

    if args.json:
        print(json.dumps({"fleet_root": fleet_root, "apply": args.apply,
                          "min_age_seconds": min_age, "rows": report,
                          "errors": errors}, indent=2))
    else:
        mode = "APPLY" if args.apply else "DRY-RUN"
        print(f"worktree-janitor {mode} root={fleet_root} min_age={min_age}s rows={len(report)}")
        for row in report:
            print(f"{row['action']:20s} {row['path']} [{row['branch'] or 'detached'}] "
                  f"— {'; '.join(row['reasons'])}"
                  + (f" => {row['result']}" if row["applied"] or row["result"] else ""))
        for err in errors:
            print(f"ERROR (kept): {err}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
