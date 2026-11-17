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
    issue_id: int | None = None


def task_id_from(value: str) -> int | None:
    match = TASK_ID_RE.search(value)
    return int(match.group(1)) if match else None


# Fallback-era and broker-era names carry the issue number as well as
# (for broker tasks) the task id: branches queue/<task>-<issue>-<slug>
# and queue/fallback-<issue>-<slug> / queue/fb-<issue>-<slug>; fleet
# dirs <task>-<issue>-<slug>, <task>-issue-<issue>-<slug>, and
# fb-<issue>-<slug> / fallback-<issue>-<slug>. Fallback claims are
# keyed by issue (there are no task ids in fallback mode, #6619), so
# protection needs the issue identity too (#6812).
BRANCH_TASK_ISSUE_RE = re.compile(r"^queue/(\d+)-(\d+)(?:-|$)")
BRANCH_FALLBACK_RE = re.compile(r"^queue/(?:fallback|fb)-(\d+)(?:-|$)")
DIR_TASK_ISSUE_RE = re.compile(r"^(\d+)-(\d+)(?:-|$)")
DIR_TASK_ISSUE_ALT_RE = re.compile(r"^(\d+)-issue-(\d+)(?:-|$)")
DIR_FALLBACK_RE = re.compile(r"^(?:fallback|fb)-(\d+)(?:-|$)")


def issue_id_from(value: str) -> int | None:
    match = BRANCH_FALLBACK_RE.search(value)
    if match:
        return int(match.group(1))
    for pattern in (BRANCH_TASK_ISSUE_RE, DIR_TASK_ISSUE_ALT_RE, DIR_TASK_ISSUE_RE):
        match = pattern.search(value)
        if match:
            return int(match.group(2))
    match = DIR_FALLBACK_RE.search(value)
    if match:
        return int(match.group(1))
    return None


def dir_task_id(value: str) -> int | None:
    # First numeric segment of a <task>-<issue>-<slug> fleet dir name;
    # task_id_from only recognises the queue/task-/target- prefixes.
    for pattern in (DIR_TASK_ISSUE_RE, DIR_TASK_ISSUE_ALT_RE):
        match = pattern.search(value)
        if match:
            return int(match.group(1))
    return None


def is_adhoc_target(entry: Path, registered: set[Path]) -> bool:
    return (
        ADHOC_TARGET_RE.fullmatch(entry.name) is not None
        and entry.resolve() not in registered
    )


def worktree_root_for_target(path: Path) -> Path | None:
    # The checkout owning a <worktree>/pkgs/id/target path, or a
    # repo-root <worktree>/target path (#6812: root targets are real
    # cargo output when a build runs at the checkout root or with
    # CARGO_TARGET_DIR=<worktree>/target). An ad-hoc target-<id> dir
    # is its own target, never a checkout root.
    if (
        path.name == "target"
        and path.parent.name == "id"
        and path.parent.parent.name == "pkgs"
    ):
        return path.parent.parent.parent
    if path.name == "target" and ADHOC_TARGET_RE.fullmatch(path.parent.name) is None:
        return path.parent
    return None


def branch_for_unregistered(path: Path, cache: dict) -> str | None:
    # Current branch of a checkout the primary does not register
    # (e.g. the fleet/queue/* worktrees), read from the checkout
    # itself. None when unreadable — the caller treats the worktree
    # as unmapped, which protects its targets.
    resolved = path.resolve()
    if resolved in cache:
        return cache[resolved]
    branch = None
    if (path / ".git").exists():
        try:
            proc = subprocess.run(
                ["git", "-C", str(path), "rev-parse", "--abbrev-ref", "HEAD"],
                capture_output=True,
                text=True,
                timeout=30,
            )
            name = proc.stdout.strip()
            if proc.returncode == 0 and name and name != "HEAD":
                branch = name
        except (OSError, subprocess.SubprocessError):
            branch = None
    cache[resolved] = branch
    return branch


def looks_like_cargo_target(path: Path) -> bool:
    # A repo-root target/ is cargo output only when it holds build
    # products; a stray dir named target is not the janitor's concern.
    return any(
        (path / marker).is_dir() for marker in ("debug", "release", ".fingerprint")
    )


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
    current_task = (
        task_id_from(current_branch or "")
        or task_id_from(root.name)
        or dir_task_id(root.name)
    )
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


def _shape_int(reader: str, field: str, value) -> int:
    # int() alone accepts bools and truncates floats, and raises raw
    # TypeError/ValueError on the shapes a malformed row can carry --
    # neither is acceptable in a reader (#6844): validate the shape
    # and name the reader and the field in the SafetyError.
    if isinstance(value, bool) or not isinstance(value, (int, str)):
        raise SafetyError(
            f"{reader}: invalid shape: {field} is "
            f"{type(value).__name__}, expected an integer"
        )
    try:
        return int(value)
    except ValueError as exc:
        raise SafetyError(
            f"{reader}: invalid shape: {field}={value!r} is not an integer"
        ) from exc


def _shape_row(reader: str, row) -> dict:
    if not isinstance(row, dict):
        raise SafetyError(
            f"{reader}: invalid shape: row is {type(row).__name__}, "
            "expected an object"
        )
    return row


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
        if not isinstance(page, list):
            raise SafetyError(
                f"queue_claims: invalid shape: page is "
                f"{type(page).__name__}, expected a list of rows"
            )
        for row in page:
            _shape_row("queue_claims", row)
            _shape_int("queue_claims", "id", row.get("id"))
            payload = row.get("payload")
            if payload is not None and not isinstance(payload, dict):
                raise SafetyError(
                    f"queue_claims: invalid shape: payload is "
                    f"{type(payload).__name__}, expected an object"
                )
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
        if not isinstance(rows, list):
            raise SafetyError(
                f"queue_task_states: invalid shape: page is "
                f"{type(rows).__name__}, expected a list of rows"
            )
        for row in rows:
            _shape_row("queue_task_states", row)
            _shape_int("queue_task_states", "id", row.get("id"))
            if not isinstance(row.get("status"), str):
                raise SafetyError(
                    f"queue_task_states: invalid shape: status is "
                    f"{type(row.get('status')).__name__}, expected a string"
                )
        states.update({int(row["id"]): str(row["status"]) for row in rows})
    return states


class ClaimsSource:
    # One readable source of queue liveness for a janitor run.
    # claims() returns (claimed_task_ids, claimed_issue_numbers):
    # the broker source keys protection by task id, the GitHub
    # fallback source by issue number, because fallback claims live
    # on issues (there are no task ids in fallback mode, #6619).
    # states() returns broker task states, or {} when the source
    # has none — a source with no states never makes a target
    # eligible via the terminal-task reason; it never weakens a
    # protection.
    name = "abstract"

    def claims(self):
        raise NotImplementedError

    def states(self, task_ids):
        return {}


class HelperClaimsSource(ClaimsSource):
    name = "repo-queue-helper"

    def __init__(self, helper: Path):
        self._client = load_queue_client(helper)

    def claims(self):
        return queue_claims(self._client), set()

    def states(self, task_ids):
        return queue_task_states(self._client, task_ids)


def parse_fallback_scan(scan: dict) -> set[int]:
    # Issue numbers with a live fallback claim in a scan document.
    # Live and contested both protect: a contested issue has two
    # workers who believe they hold it, and the #6619 protocol says
    # neither proceeds — but the work is claimed, so never delete
    # under it. Stale/released/free do not protect here; the
    # queue/claimed label union in FallbackClaimsSource covers the
    # ambiguous broker-era residue separately.
    if not isinstance(scan, dict):
        raise SafetyError(
            f"parse_fallback_scan: invalid shape: scan is "
            f"{type(scan).__name__}, expected an object"
        )
    items = scan.get("items", [])
    if not isinstance(items, list):
        raise SafetyError(
            f"parse_fallback_scan: invalid shape: items is "
            f"{type(items).__name__}, expected a list"
        )
    issues = set()
    for item in items:
        if not isinstance(item, dict):
            raise SafetyError(
                f"parse_fallback_scan: invalid shape: item is "
                f"{type(item).__name__}, expected an object"
            )
        claim = item.get("claim") or {}
        if not isinstance(claim, dict):
            raise SafetyError(
                f"parse_fallback_scan: invalid shape: claim is "
                f"{type(claim).__name__}, expected an object"
            )
        number = _shape_int("parse_fallback_scan", "number", item.get("number"))
        if claim.get("state") in {"live", "contested"}:
            issues.add(number)
    return issues


def labelled_claimed_issues() -> set[int]:
    # Open issues carrying queue/claimed, read completely via gh.
    # A broker-era claim leaves this label but no fallback marker;
    # the scan fold calls such an issue free, but its claim state
    # is genuinely unknown (a parked broker task can resume), so
    # the label alone protects. Paginated like open_pr_branches:
    # complete or fail closed.
    issues = set()
    for page_no in range(1, MAX_PR_PAGES + 1):
        proc = subprocess.run(
            [
                "gh",
                "api",
                f"repos/{REPO}/issues?state=open&labels=queue%2Fclaimed"
                f"&per_page={PR_PAGE}&page={page_no}",
            ],
            check=True,
            capture_output=True,
            text=True,
        )
        try:
            rows = json.loads(proc.stdout)
            if not isinstance(rows, list):
                raise TypeError("issues response is not a list")
            for row in rows:
                if "pull_request" not in row:
                    issues.add(int(row["number"]))
        except (ValueError, KeyError, TypeError) as exc:
            raise SafetyError(f"invalid gh issues response: {exc}") from exc
        if len(rows) < PR_PAGE:
            return issues
    raise SafetyError(
        f"claimed-issue read exceeded {MAX_PR_PAGES} pages of {PR_PAGE}; "
        "refusing to trust a possibly-truncated set"
    )


def fleet_binary() -> str:
    found = shutil.which("fleet")
    if found:
        return found
    local = Path.home() / ".local" / "bin" / "fleet"
    if local.is_file():
        return str(local)
    raise SafetyError("fleet binary not found on PATH or in ~/.local/bin")


class FallbackClaimsSource(ClaimsSource):
    name = "github-fallback"

    def claims(self):
        proc = subprocess.run(
            [fleet_binary(), "queue", "fallback", "scan", "--repo", REPO],
            capture_output=True,
            text=True,
            timeout=300,
        )
        if proc.returncode != 0:
            raise SafetyError(
                f"fallback scan failed (exit {proc.returncode}): "
                f"{proc.stderr.strip()[:200]}"
            )
        try:
            scan = json.loads(proc.stdout)
        except ValueError as exc:
            raise SafetyError(f"invalid fallback scan output: {exc}") from exc
        return set(), parse_fallback_scan(scan) | labelled_claimed_issues()


def load_claims_source(helper: Path) -> ClaimsSource:
    # The repo-queue helper stays the first choice where it still
    # exists, but its absence is no longer fatal (#6812): the
    # helper was retired in the fleet-binary migration, and a
    # janitor that refuses every run enforces nothing. The GitHub
    # fallback source needs only gh + the fleet binary and works
    # while the queue backend is down. If neither source can be
    # read, claims() raises and the run fails closed as before.
    if helper.is_file():
        return HelperClaimsSource(helper)
    return FallbackClaimsSource()


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


def scan_entries(root: Path):
    # (entry, is_top_level) for every dir that can own targets: the
    # fleet children, plus the queue/ sub-worktrees one level down
    # (#6812: queue/ alone held ~90 GiB the old top-level-only scan
    # never saw).
    for entry in sorted(root.iterdir()):
        yield entry, True
    queue_dir = root / "queue"
    if queue_dir.is_dir() and not queue_dir.is_symlink():
        for entry in sorted(queue_dir.iterdir()):
            yield entry, False


def target_paths_for_entry(entry: Path, registered: set, top_level: bool):
    # (target_path, owning_worktree_or_None) pairs owned by one
    # fleet entry: its pkgs/id target, its repo-root cargo target
    # (#6812), or the entry itself when it is an ad-hoc target-<id>.
    if top_level and is_adhoc_target(entry, registered):
        return [(entry, None)]
    paths = []
    pkgs_target = entry / "pkgs" / "id" / "target"
    if not pkgs_target.is_symlink() and pkgs_target.is_dir():
        paths.append((pkgs_target, entry.resolve()))
    root_target = entry / "target"
    if (
        not root_target.is_symlink()
        and root_target.is_dir()
        and looks_like_cargo_target(root_target)
    ):
        # A checkout's root target belongs to the checkout. A
        # non-checkout dir's root target (lane probe/scratch trees —
        # ~113 GiB of them, #6812) belongs to no checkout: it is
        # returned with worktree None and handled like an ad-hoc
        # target, except that its owning dir still exists, so the
        # worktree-gone reason must not fire for it (see discover).
        checkout = (entry / ".git").exists() or entry.resolve() in registered
        paths.append((root_target, entry.resolve() if checkout else None))
    return paths


def discover_targets(
    fleet_root: Path,
    branches: dict[Path, str | None],
    claimed: set[int],
    statuses: dict[int, str],
    prs: set[str],
    process_cwds: list[Path],
    stale_before: float,
    claimed_issues: set[int] | None = None,
) -> list[Target]:
    if claimed_issues is None:
        claimed_issues = set()
    root = fleet_root.resolve(strict=True)
    registered = {path.resolve() for path in branches}
    worktree_task_ids = {
        task_id
        for branch in branches.values()
        if (task_id := task_id_from(branch or "")) is not None
    }
    candidates = []
    branch_cache: dict = {}
    for entry, top_level in scan_entries(root):
        if entry.is_symlink() or not entry.is_dir():
            continue
        for target, worktree in target_paths_for_entry(entry, registered, top_level):
            if worktree is None and target == entry:
                # Ad-hoc target-<id>: the entry IS the target.
                candidates.append(
                    (target, None, None, task_id_from(entry.name), None, None, True)
                )
                continue
            if worktree is None:
                # Scratch root target: no checkout, but the owning
                # dir exists and a build inside it must protect it.
                task_id = task_id_from(entry.name) or dir_task_id(entry.name)
                issue_id = issue_id_from(entry.name)
                candidates.append(
                    (target, None, None, task_id, issue_id, entry.resolve(), False)
                )
                continue
            branch = branches.get(worktree)
            if branch is None:
                branch = branch_for_unregistered(entry, branch_cache)
            task_id = (
                task_id_from(branch or "")
                or task_id_from(entry.name)
                or dir_task_id(entry.name)
            )
            issue_id = issue_id_from(branch or "") or issue_id_from(entry.name)
            candidates.append((target, worktree, branch, task_id, issue_id, worktree, False))

    result = []
    for path, worktree, branch, task_id, issue_id, owner_dir, adhoc in candidates:
        resolved = path.resolve(strict=True)
        if not resolved.is_relative_to(root) or resolved == root:
            continue
        try:
            size, newest = target_stats(resolved)
        except OSError as exc:
            result.append(
                Target(
                    path, 0, 0, branch, task_id,
                    protected=f"stat-failed: {exc}", issue_id=issue_id,
                )
            )
            continue
        status = statuses.get(task_id) if task_id is not None else None
        if (
            worktree is None
            and adhoc
            and task_id is not None
            and task_id not in worktree_task_ids
        ):
            reason = "worktree-gone"
        elif status in {"done", "failed", "cancelled"}:
            reason = "terminal-task"
        else:
            reason = "older-than-threshold" if newest <= stale_before else None
        protection = None
        if worktree is None and adhoc and task_id is None:
            protection = "unmapped-ad-hoc-target"
        elif task_id is not None and task_id in claimed:
            protection = "live-claimed-task"
        elif issue_id is not None and issue_id in claimed_issues:
            protection = "live-claimed-issue"
        elif branch and branch in prs:
            protection = "open-pr-worktree"
        elif task_id is not None and any(task_id_from(pr) == task_id for pr in prs):
            protection = "open-pr-worktree"
        elif worktree is not None and branch is None:
            protection = "unmapped-worktree"
        elif owner_dir is not None and cwd_inside(process_cwds, owner_dir):
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
                issue_id=issue_id,
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
    for entry, top_level in scan_entries(root):
        if entry.is_symlink() or not entry.is_dir():
            continue
        for path, _worktree in target_paths_for_entry(entry, registered, top_level):
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

    run_id = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    log = args.state_dir / "runs" / f"run-{run_id}-{os.getpid()}.jsonl"
    try:
        # Snapshot every independent liveness source before producing a plan.
        branches = worktree_branches(args.primary)
        source = load_claims_source(args.queue_helper)
        claimed, claimed_issues = source.claims()
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
        statuses = source.states(task_ids)
        now = time.time()
        targets = discover_targets(
            args.fleet_root,
            branches,
            claimed,
            statuses,
            prs,
            cwds,
            now - args.stale_hours * 3600,
            claimed_issues=claimed_issues,
        )
        total = sum(t.bytes for t in targets)
        selected, projected = build_plan(targets, args.cap_gib * GIB)
        print(
            json.dumps(
                {
                    "mode": "apply" if args.apply else "dry-run",
                    "claims_source": source.name,
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
                "claims_source": source.name,
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
            fresh_claimed, fresh_issues = source.claims()
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
            if target.issue_id is not None and target.issue_id in fresh_issues:
                raise SafetyError(
                    f"issue {target.issue_id} became claimed; stopping before {path}"
                )
            if target.reason == "terminal-task":
                fresh_status = source.states({target.task_id}).get(
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
    except (
        SafetyError,
        OSError,
        subprocess.CalledProcessError,
        subprocess.TimeoutExpired,
        SystemExit,
    ) as exc:
        # A refusal is a run too (#6812): until it left a record,
        # an hourly-dead janitor was visible only in the systemd
        # journal while its state dir stayed empty.
        try:
            append_log(
                log,
                {
                    "event": "refused",
                    "at": datetime.now(timezone.utc).isoformat(),
                    "mode": "apply" if args.apply else "dry-run",
                    "reason": str(exc),
                },
            )
        except OSError:
            pass
        print(f"cargo-target-janitor: refusing cleanup: {exc}", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
