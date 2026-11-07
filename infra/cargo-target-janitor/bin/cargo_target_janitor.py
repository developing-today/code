#!/usr/bin/env python3
"""Dry-run-first, liveness-aware cargo target cleanup for ~/git/fleet."""

from __future__ import annotations

import argparse
import importlib.util
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import time
from dataclasses import dataclass
from datetime import datetime, timezone
from pathlib import Path

GIB = 1024**3
QUEUE = "repo-health-code-pilot"
REPO = "developing-today/code"
TASK_ID_RE = re.compile(r"(?:queue/|task-|target-)(\d+)(?:-|$)")
# Ad-hoc targets are exactly "target-<task-id>" (README). Anything else
# starting with "target-" (e.g. a registered worktree named
# "target-42-worktree") is a checkout, never a disposable target.
ADHOC_TARGET_RE = re.compile(r"target-\d+")


class SafetyError(RuntimeError):
    pass


@dataclass
class Target:
    path: Path
    bytes: int
    newest_mtime: float
    branch: str | None
    task_id: int | None
    task_status: str | None = None
    protected: str | None = None
    reason: str | None = None


def task_id_from(value: str) -> int | None:
    match = TASK_ID_RE.search(value)
    return int(match.group(1)) if match else None


def is_adhoc_target(entry: Path, registered: set[Path]) -> bool:
    return (
        ADHOC_TARGET_RE.fullmatch(entry.name) is not None
        and entry.resolve() not in registered
    )


def worktree_root_for_target(path: Path) -> Path | None:
    """The checkout owning a <worktree>/pkgs/id/target path, else None."""
    if (
        path.name == "target"
        and path.parent.name == "id"
        and path.parent.parent.name == "pkgs"
    ):
        return path.parent.parent.parent
    return None


def owner_changed(target: Target, branches_now: dict[Path, str | None]) -> bool:
    """Whether the checkout owning a worktree target has moved to a
    different branch/task since discovery. A switch between plan and
    removal invalidates the plan's queue/PR checks, which are keyed to
    the recorded branch/task (#4175 Codex P2): the apply path must
    re-derive the owner and stop if it changed."""
    root = worktree_root_for_target(target.path)
    if root is None:
        return False
    current_branch = branches_now.get(root.resolve())
    current_task = task_id_from(current_branch or "") or task_id_from(root.name)
    return current_branch != target.branch or current_task != target.task_id


def cwd_inside(cwds: list[Path], base: Path) -> bool:
    return any(cwd == base or cwd.is_relative_to(base) for cwd in cwds)


def target_stats(path: Path) -> tuple[int, float]:
    """Count regular files without following symlinks; fail closed on I/O errors."""
    total = 0
    newest = path.lstat().st_mtime
    seen_inodes: set[tuple[int, int]] = set()
    for root, dirs, files in os.walk(path, followlinks=False):
        dirs[:] = [name for name in dirs if not (Path(root) / name).is_symlink()]
        for name in dirs + files:
            entry = Path(root) / name
            try:
                st = entry.lstat()
            except FileNotFoundError:
                continue  # Cargo may be creating/removing files during the scan.
            newest = max(newest, st.st_mtime)
            if stat.S_ISREG(st.st_mode):
                # Cargo hard-links artifacts (target/debug/<bin> and
                # target/debug/deps/<bin>-<hash> share an inode): count
                # each inode once, at allocated size, so the cap and the
                # reclaimed reporting track real disk usage.
                key = (st.st_dev, st.st_ino)
                if key in seen_inodes:
                    continue
                seen_inodes.add(key)
                blocks = getattr(st, "st_blocks", 0)
                total += blocks * 512 if blocks else st.st_size
            else:
                total += st.st_size
    return total, newest


def active_process_cwds(proc_root: Path = Path("/proc")) -> list[Path]:
    """Return current-user process working directories; unreadable own cwd is unsafe."""
    try:
        pids = list(proc_root.iterdir())
    except OSError as exc:
        raise SafetyError(f"cannot inspect process table: {exc}") from exc
    result = []
    uid = os.getuid()
    for entry in pids:
        if not entry.name.isdigit():
            continue
        try:
            comm = (entry / "comm").read_text(encoding="utf-8").strip().strip("()")
        except FileNotFoundError:
            continue
        except OSError:
            raise SafetyError(f"cannot inspect process {entry.name} identity")
        if not (
            comm in {"cargo", "rustc", "rust-analyzer"}
            or comm.startswith("build-script")
        ):
            continue
        try:
            if entry.stat().st_uid != uid:
                continue
            result.append(Path(os.readlink(entry / "cwd")))
        except FileNotFoundError:
            continue  # process exited during the scan
        except PermissionError as exc:
            raise SafetyError(f"cannot verify build process {entry.name} cwd") from exc
        except OSError:
            continue
    return result


def worktree_branches(primary: Path) -> dict[Path, str | None]:
    proc = subprocess.run(
        ["git", "-C", str(primary), "worktree", "list", "--porcelain"],
        check=True,
        capture_output=True,
        text=True,
    )
    result: dict[Path, str | None] = {}
    current: Path | None = None
    for line in proc.stdout.splitlines():
        if line.startswith("worktree "):
            current = Path(line[9:]).resolve()
            result[current] = None
        elif current is not None and line.startswith("branch "):
            result[current] = line[7:].removeprefix("refs/heads/")
        elif not line:
            current = None
    return result


def load_queue_client(helper: Path):
    """Load the existing helper and its transient key once per run."""
    if not helper.is_file():
        raise SafetyError(f"queue helper unavailable: {helper}")
    spec = importlib.util.spec_from_file_location("repo_queue_helper", helper)
    if spec is None or spec.loader is None:
        raise SafetyError("cannot load repo-queue helper")
    module = importlib.util.module_from_spec(spec)
    sys.modules[spec.name] = module
    spec.loader.exec_module(module)
    key = module.get_service_key()  # never print or persist this value
    return module, key


CLAIMS_PAGE = 1000
# Fail-closed bound on pagination (#6618): a liveness read that cannot
# be completed inside the bound raises SafetyError and cleanup stops,
# rather than proceeding on a set that might be silently truncated.
MAX_CLAIM_PAGES = 100
STATES_CHUNK = 500


def queue_claims(client) -> set[int]:
    """Read ALL the pilot's claimed IDs through the existing helper.

    Paginated (#6618): a single page capped at 1000 rows silently
    dropped every claimed task past row 1000 from the never-delete
    set. Pages are followed until a short page arrives; needing more
    than MAX_CLAIM_PAGES pages fails closed.

    Keyset pagination on id (#6790): pages are ordered by id and each
    next page asks for id greater than the largest id seen. Offset
    paging over this LIVE set was not stable — PostgREST re-evaluates
    offset against the current filtered set on every request, so a
    task leaving the claimed set mid-read shifted every later row one
    position forward and the next offset page skipped a still-claimed
    task, dropping it from the never-delete set. Rows leaving behind
    the keyset cursor cannot shift rows ahead of it; rows ahead are
    read exactly once, in order.
    """
    module, key = client
    rows = []
    after = 0
    for _ in range(MAX_CLAIM_PAGES):
        query = (
            "tasks?select=id,queue,status,payload&"
            f"queue=eq.{QUEUE}&status=eq.claimed"
            f"&id=gt.{after}&order=id.asc&limit={CLAIMS_PAGE}"
        )
        page = module.http("GET", query, key) or []
        rows.extend(page)
        if len(page) < CLAIMS_PAGE:
            break
        after = max(int(row["id"]) for row in page)
    else:
        raise SafetyError(
            f"claimed-task read exceeded {MAX_CLAIM_PAGES} pages "
            f"of {CLAIMS_PAGE}; refusing to trust a possibly-truncated set"
        )
    return {
        int(row["id"])
        for row in rows
        if row.get("status") == "claimed"
        and row.get("queue") == QUEUE
        and (row.get("payload") or {}).get("repo") == REPO
    }


def queue_task_states(client, task_ids: set[int]) -> dict[int, str]:
    """States for a locally-built ID set, read in chunks (#6618).

    Disposition of the old single `limit=1000` read: the ID set comes
    from local fleet directories and can in principle exceed 1000, so
    it is chunked — each query names at most STATES_CHUNK specific
    IDs with limit=1000 > STATES_CHUNK, so no chunk's page can ever
    truncate its answer.
    """
    if not task_ids:
        return {}
    module, key = client
    states = {}
    ordered = sorted(task_ids)
    for start in range(0, len(ordered), STATES_CHUNK):
        chunk = ordered[start : start + STATES_CHUNK]
        ids = ",".join(str(task_id) for task_id in chunk)
        rows = (
            module.http(
                "GET",
                f"tasks?select=id,status&queue=eq.{QUEUE}&id=in.%28{ids}%29&limit=1000",
                key,
            )
            or []
        )
        states.update({int(row["id"]): str(row["status"]) for row in rows})
    return states


PR_PAGE = 100
MAX_PR_PAGES = 200


def open_pr_branches(repo: str = REPO) -> set[str]:
    """Read ALL open PR branch names, paginated (#6618).

    The old single `gh pr list --limit 500` silently dropped every
    open PR past the first 500 from the never-delete set. This pages
    the REST pulls endpoint (per_page/page) until a short page; a
    read that still has not terminated after MAX_PR_PAGES pages
    fails closed instead of trusting a possibly-truncated set.
    Both call sites (plan-time and the apply-loop re-check) use this
    one function, so there is no second, weaker copy.
    """
    branches = set()
    for page_no in range(1, MAX_PR_PAGES + 1):
        proc = subprocess.run(
            [
                "gh",
                "api",
                f"repos/{repo}/pulls?state=open&per_page={PR_PAGE}&page={page_no}",
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        try:
            rows = json.loads(proc.stdout)
            if not isinstance(rows, list):
                raise TypeError("pulls response is not a list")
            for row in rows:
                branches.add(str(row["head"]["ref"]))
        except (ValueError, KeyError, TypeError) as exc:
            raise SafetyError(f"invalid gh PR response: {exc}") from exc
        if len(rows) < PR_PAGE:
            return branches
    raise SafetyError(
        f"open-PR read exceeded {MAX_PR_PAGES} pages of {PR_PAGE}; "
        "refusing to trust a possibly-truncated set"
    )


def discover_targets(
    fleet_root: Path,
    branches: dict[Path, str | None],
    claimed: set[int],
    statuses: dict[int, str],
    prs: set[str],
    process_cwds: list[Path],
    stale_before: float,
) -> list[Target]:
    root = fleet_root.resolve(strict=True)
    registered = {path.resolve() for path in branches}
    worktree_task_ids = {
        task_id
        for branch in branches.values()
        if (task_id := task_id_from(branch or "")) is not None
    }
    candidates: list[tuple[Path, Path | None, str | None, int | None]] = []
    for entry in sorted(root.iterdir()):
        if entry.is_symlink() or not entry.is_dir():
            continue
        if is_adhoc_target(entry, registered):
            candidates.append((entry, None, None, task_id_from(entry.name)))
            continue
        target = entry / "pkgs" / "id" / "target"
        if target.is_symlink() or not target.is_dir():
            continue
        worktree = entry.resolve()
        branch = branches.get(worktree)
        candidates.append(
            (
                target,
                worktree,
                branch,
                task_id_from(branch or "") or task_id_from(entry.name),
            )
        )

    result = []
    for path, worktree, branch, task_id in candidates:
        resolved = path.resolve(strict=True)
        if not resolved.is_relative_to(root) or resolved == root:
            continue
        try:
            size, newest = target_stats(resolved)
        except OSError as exc:
            result.append(
                Target(path, 0, 0, branch, task_id, protected=f"stat-failed: {exc}")
            )
            continue
        status = statuses.get(task_id) if task_id is not None else None
        if (
            worktree is None
            and task_id is not None
            and task_id not in worktree_task_ids
        ):
            reason = "worktree-gone"
        elif status in {"done", "failed", "cancelled"}:
            reason = "terminal-task"
        else:
            reason = "older-than-threshold" if newest <= stale_before else None
        protection = None
        if worktree is None and task_id is None:
            protection = "unmapped-ad-hoc-target"
        elif task_id is not None and task_id in claimed:
            protection = "live-claimed-task"
        elif branch and branch in prs:
            protection = "open-pr-worktree"
        elif task_id is not None and any(task_id_from(pr) == task_id for pr in prs):
            protection = "open-pr-worktree"
        elif worktree is not None and branch is None:
            protection = "unmapped-worktree"
        elif any(
            cwd == worktree or cwd.is_relative_to(worktree)
            for cwd in process_cwds
            if worktree is not None
        ):
            protection = "process-in-worktree"
        elif worktree is None and any(cwd.is_relative_to(root) for cwd in process_cwds):
            # target-* may be shared via CARGO_TARGET_DIR, so preserve it
            # while any fleet Cargo build is running.
            protection = "fleet-build-active"
        elif any(
            cwd == resolved or cwd.is_relative_to(resolved) for cwd in process_cwds
        ):
            protection = "process-in-target"
        elif reason is None:
            protection = "younger-than-48h"
        result.append(
            Target(
                resolved,
                size,
                newest,
                branch,
                task_id,
                status,
                protected=protection,
                reason=reason,
            )
        )
    return result


def measure_total(
    fleet_root: Path, branches: dict[Path, str | None] | None = None
) -> int:
    """Re-measure every in-scope target once, including ones created mid-run."""
    root = fleet_root.resolve(strict=True)
    registered = {path.resolve() for path in (branches or {})}
    total = 0
    for entry in root.iterdir():
        if entry.is_symlink() or not entry.is_dir():
            continue
        paths = (
            [entry]
            if is_adhoc_target(entry, registered)
            else [entry / "pkgs" / "id" / "target"]
        )
        for path in paths:
            if path.is_symlink() or not path.is_dir():
                continue
            resolved = path.resolve(strict=True)
            if resolved.is_relative_to(root):
                total += target_stats(resolved)[0]
    return total


def build_plan(targets: list[Target], cap_bytes: int) -> tuple[list[Target], int]:
    total = sum(target.bytes for target in targets)
    eligible = sorted(
        (t for t in targets if t.protected is None),
        key=lambda t: (t.newest_mtime, str(t.path)),
    )
    selected = []
    projected = total
    for target in eligible:
        if projected <= cap_bytes or len(selected) >= 20:
            break
        selected.append(target)
        projected -= target.bytes
    return selected, projected


def append_log(log: Path, record: dict) -> None:
    log.parent.mkdir(parents=True, exist_ok=True)
    with log.open("a", encoding="utf-8") as stream:
        stream.write(json.dumps(record, sort_keys=True) + "\n")
        stream.flush()
        os.fsync(stream.fileno())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--fleet-root", type=Path, default=Path.home() / "git/fleet")
    parser.add_argument("--primary", type=Path, default=Path.home() / "git/code")
    parser.add_argument(
        "--queue-helper", type=Path, default=Path.home() / "hatch/tools/repo-queue.py"
    )
    parser.add_argument(
        "--state-dir",
        type=Path,
        default=Path.home() / ".local/state/cargo-target-janitor",
    )
    parser.add_argument("--cap-gib", type=int, default=150)
    parser.add_argument("--stale-hours", type=int, default=48)
    parser.add_argument(
        "--apply",
        action="store_true",
        help="delete explicitly listed eligible targets; default is dry-run",
    )
    args = parser.parse_args()
    if args.cap_gib < 1 or args.stale_hours < 1:
        parser.error("cap and stale threshold must be positive")

    try:
        # Snapshot every independent liveness source before producing a plan.
        branches = worktree_branches(args.primary)
        queue_client = load_queue_client(args.queue_helper)
        claimed = queue_claims(queue_client)
        prs = open_pr_branches()
        cwds = active_process_cwds()
        task_ids = set()
        registered = {path.resolve() for path in branches}
        for entry in args.fleet_root.iterdir():
            if is_adhoc_target(entry, registered):
                task_id = task_id_from(entry.name)
                if task_id is not None:
                    task_ids.add(task_id)
            branch = branches.get(entry.resolve())
            task_id = task_id_from(branch or "") or task_id_from(entry.name)
            if task_id is not None and (entry / "pkgs/id/target").is_dir():
                task_ids.add(task_id)
        statuses = queue_task_states(queue_client, task_ids)
        now = time.time()
        targets = discover_targets(
            args.fleet_root,
            branches,
            claimed,
            statuses,
            prs,
            cwds,
            now - args.stale_hours * 3600,
        )
        total = sum(t.bytes for t in targets)
        selected, projected = build_plan(targets, args.cap_gib * GIB)
        run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        log = args.state_dir / "runs" / f"run-{run_id}-{os.getpid()}.jsonl"
        print(
            json.dumps(
                {
                    "mode": "apply" if args.apply else "dry-run",
                    "cap_gib": args.cap_gib,
                    "total_gib": round(total / GIB, 2),
                    "projected_gib": round(projected / GIB, 2),
                    "over_cap_after_this_run": projected > args.cap_gib * GIB,
                    "candidates": [
                        {
                            "path": str(t.path),
                            "gib": round(t.bytes / GIB, 2),
                            "reason": t.reason,
                            "task_id": t.task_id,
                            "task_status": t.task_status,
                            "branch": t.branch,
                        }
                        for t in selected
                    ],
                    "protected": [
                        {
                            "path": str(t.path),
                            "gib": round(t.bytes / GIB, 2),
                            "reason": t.protected,
                        }
                        for t in targets
                        if t.protected
                    ],
                    "log": str(log),
                },
                indent=2,
            )
        )
        append_log(
            log,
            {
                "event": "plan",
                "at": datetime.now(timezone.utc).isoformat(),
                "mode": "apply" if args.apply else "dry-run",
                "total_bytes": total,
                "cap_bytes": args.cap_gib * GIB,
                "selected": [
                    {
                        "path": str(t.path),
                        "bytes": t.bytes,
                        "reason": t.reason,
                        "task_id": t.task_id,
                        "branch": t.branch,
                    }
                    for t in selected
                ],
                "protected": {str(t.path): t.protected for t in targets if t.protected},
            },
        )
        if not args.apply:
            return 0

        reclaimed = 0
        remaining_estimate = total
        for target in selected:
            # Fresh queue, PR, and process state immediately before each removal.
            fresh_claimed = queue_claims(queue_client)
            fresh_prs = open_pr_branches()
            fresh_cwds = active_process_cwds()
            path = target.path
            if path.is_symlink() or not path.is_dir():
                raise SafetyError(f"target changed since plan: {path}")
            resolved = path.resolve(strict=True)
            if not resolved.is_relative_to(args.fleet_root.resolve(strict=True)):
                raise SafetyError(f"target escaped fleet root: {path}")
            if target.task_id in fresh_claimed:
                raise SafetyError(
                    f"task {target.task_id} became live; stopping before {path}"
                )
            if target.reason == "terminal-task":
                fresh_status = queue_task_states(queue_client, {target.task_id}).get(
                    target.task_id
                )
                if fresh_status not in {"done", "failed", "cancelled"}:
                    raise SafetyError(
                        f"task {target.task_id} is no longer terminal; stopping"
                    )
            if target.reason == "worktree-gone":
                current_branches = worktree_branches(args.primary)
                if target.task_id in {
                    task_id_from(branch or "") for branch in current_branches.values()
                }:
                    raise SafetyError(
                        f"task {target.task_id} gained a worktree; stopping"
                    )
            if target.branch and target.branch in fresh_prs:
                raise SafetyError(f"branch {target.branch} gained an open PR; stopping")
            if target.task_id is not None and any(
                task_id_from(pr) == target.task_id for pr in fresh_prs
            ):
                raise SafetyError(f"task {target.task_id} gained an open PR; stopping")
            worktree_root = worktree_root_for_target(resolved)
            if worktree_root is not None and owner_changed(
                target, worktree_branches(args.primary)
            ):
                raise SafetyError(
                    f"owning worktree changed branch/task since plan; stopping before {path}"
                )
            if worktree_root is not None and cwd_inside(fresh_cwds, worktree_root):
                raise SafetyError(
                    f"build process active in worktree; stopping before {path}"
                )
            if any(
                cwd == resolved or cwd.is_relative_to(resolved) for cwd in fresh_cwds
            ):
                raise SafetyError(f"process entered target; stopping before {path}")
            if target.branch is None and any(
                cwd.is_relative_to(args.fleet_root.resolve(strict=True))
                for cwd in fresh_cwds
            ):
                raise SafetyError(
                    f"fleet build active; stopping before ad-hoc target {path}"
                )
            if remaining_estimate <= args.cap_gib * GIB:
                break
            size, mtime = target_stats(resolved)
            if (
                target.reason == "older-than-threshold"
                and mtime > now - args.stale_hours * 3600
            ):
                raise SafetyError(f"target became recent; stopping before {path}")
            # Write-ahead record is fsynced before the bounded, exact-path removal.
            append_log(
                log,
                {
                    "event": "delete-start",
                    "at": datetime.now(timezone.utc).isoformat(),
                    "path": str(resolved),
                    "bytes": size,
                    "task_id": target.task_id,
                    "branch": target.branch,
                },
            )
            shutil.rmtree(resolved)
            reclaimed += size
            remaining_estimate -= size
            append_log(
                log,
                {
                    "event": "deleted",
                    "at": datetime.now(timezone.utc).isoformat(),
                    "path": str(resolved),
                    "bytes": size,
                },
            )
        remaining = measure_total(args.fleet_root, branches)
        append_log(
            log,
            {
                "event": "summary",
                "at": datetime.now(timezone.utc).isoformat(),
                "reclaimed_bytes": reclaimed,
                "remaining_bytes": remaining,
                "over_cap": remaining > args.cap_gib * GIB,
            },
        )
        print(
            json.dumps(
                {
                    "reclaimed_gib": round(reclaimed / GIB, 2),
                    "remaining_gib": round(remaining / GIB, 2),
                    "over_cap": remaining > args.cap_gib * GIB,
                    "log": str(log),
                },
                indent=2,
            )
        )
        return 0
    except (SafetyError, OSError, subprocess.CalledProcessError, SystemExit) as exc:
        print(f"cargo-target-janitor: refusing cleanup: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
