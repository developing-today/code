#!/usr/bin/env python3
# Copyright developing.today LLC & contributors to the project.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Live coordination for research packages (Python stdlib only).

Loaded by research_workflow.py through EXTENSIONS. `ready` plans waves from the
committed state plus live leases. Leases are never committed: they live in
`$(git rev-parse --git-common-dir)/agent-work/locks/`, shared by every worktree,
or in the profile's `agent_work_dir` when that is not writable (collaboration
rule 5). RESEARCH_WORKFLOW_NOW (YYYY-MM-DDTHH:MM:SSZ) pins the clock for tests.
"""

import importlib.util
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path


STAMP = "%Y-%m-%dT%H:%M:%SZ"
GUARD_STALE = 60  # seconds before an ownerless lock is stale
CLAIMED = {"claimed", "running"}
# A lease on C003, concept-C003 or a facet such as C003.2 marks C003 as taken.
CONCEPT_LEASE = re.compile(r"(?:concept-)?(C\d{3,}(?:\.\d+)*)")


def core():
    """Return research_workflow, loading it by path when imported on its own."""
    module = sys.modules.get("research_workflow")
    if module is None:
        spec = importlib.util.spec_from_file_location(
            "research_workflow", Path(__file__).with_name("research_workflow.py")
        )
        module = importlib.util.module_from_spec(spec)
        sys.modules["research_workflow"] = module
        spec.loader.exec_module(module)
    return module


def fail(message):
    raise ValueError(message)


# -------------------------------------------------------------------- clock


def now():
    pinned = os.environ.get("RESEARCH_WORKFLOW_NOW")
    if pinned:
        return parse_stamp(pinned)
    return datetime.now(timezone.utc).replace(microsecond=0)


def parse_stamp(value):
    try:
        return datetime.strptime(value, STAMP).replace(tzinfo=timezone.utc)
    except (TypeError, ValueError):
        fail(f"Invalid UTC timestamp (want YYYY-MM-DDTHH:MM:SSZ): {value}")


def stamp(moment):
    return moment.strftime(STAMP)


def age(path):
    """Seconds since path last changed; infinite when it is gone."""
    try:
        return time.time() - path.stat().st_mtime
    except OSError:
        return float("inf")


# ----------------------------------------------------------------- live dir


def git_common_dir(root):
    try:
        result = subprocess.run(
            ["git", "rev-parse", "--path-format=absolute", "--git-common-dir"],
            cwd=root,
            capture_output=True,
            text=True,
        )
    except OSError as error:
        fail(f"git is unavailable: {error}")
    if result.returncode:
        detail = (result.stderr.strip().splitlines() or ["git rev-parse failed"])[-1]
        fail(f"no git common dir: {detail}")
    return Path(result.stdout.strip())


def writable(path):
    """True when path is a writable directory or could be created as one."""
    while not path.exists() and path != path.parent:
        path = path.parent
    return path.is_dir() and os.access(path, os.W_OK | os.X_OK)


def live_dir(root, config, create=False):
    """`<git common dir>/agent-work` when writable, else the profile's `agent_work_dir`."""
    reasons = []
    try:
        path = git_common_dir(root) / "agent-work"
        if writable(path):
            if create:
                path.mkdir(exist_ok=True)
            return path
        reasons.append(f"{path} is not writable")
    except ValueError as error:
        reasons.append(str(error))
    configured = config.get("agent_work_dir")
    if configured is None:
        fail("; ".join(reasons + ["the profile sets no agent_work_dir"]))
    path = Path(configured).expanduser() if isinstance(configured, str) else None
    if path is None or not path.is_absolute():
        fail("agent_work_dir must be an absolute path, so every worktree shares it")
    if not writable(path):
        fail("; ".join(reasons + [f"agent_work_dir {path} is not writable"]))
    if create:
        path.mkdir(parents=True, exist_ok=True)
    return path


def read_owner(lock):
    """Parse `<name> <acquired> <expires> <purpose>`; None when missing or malformed."""
    try:
        raw = (lock / "owner").read_text(encoding="utf-8")
    except (OSError, UnicodeDecodeError):
        return None
    parts = raw.rstrip("\n").split(" ", 3)
    if len(parts) != 4:
        return None
    try:
        acquired, expires = parse_stamp(parts[1]), parse_stamp(parts[2])
    except ValueError:
        return None
    return {
        "name": parts[0],
        "acquired": acquired,
        "expires": expires,
        "purpose": parts[3],
    }


def leases(base, moment=None):
    """Every lock under base/locks, expired ones included and flagged.

    A lock is `<resource>.lock`, or `<resource>@<package>.lock` when scoped to
    one package. A lock whose owner file is not written yet counts as live for
    GUARD_STALE seconds: its holder is still writing it.
    """
    moment = moment or now()
    found = []
    for lock in sorted((base / "locks").glob("*.lock")):
        if lock.name.startswith(".") or lock.is_symlink() or not lock.is_dir():
            continue
        resource, _, package = lock.name.removesuffix(".lock").partition("@")
        owner = read_owner(lock)
        found.append(
            {
                "resource": resource,
                "package": package or None,
                "name": owner["name"] if owner else None,
                "acquired": stamp(owner["acquired"]) if owner else None,
                "expires": stamp(owner["expires"]) if owner else None,
                "purpose": owner["purpose"] if owner else None,
                "expired": owner["expires"] <= moment
                if owner
                else age(lock) >= GUARD_STALE,
                "lock": lock.name,
            }
        )
    return found


# -------------------------------------------------------------------- ready


def covers(concept, other):
    """True when two ids name the same concept, or one is a facet of the other."""
    return (
        concept == other
        or concept.startswith(other + ".")
        or other.startswith(concept + ".")
    )


def waves(state, busy, hints):
    """Group open concepts into waves from hard `requires` edges.

    Wave 1 may start now. `busy` concepts are taken (running, claimed or leased):
    they stay out of every wave, and work that requires them waits for wave 2 or
    later. `hints` stay out too, with everything that requires them, until they
    are complete. Concepts linked by `shares-files-with` never share a wave, and
    wave 1 never holds a concept that shares files with busy work.
    """
    c = core()
    graph = c.dependencies(state)
    status = {x["id"]: x["status"] for x in state["concepts"]}
    done = {k for k, v in status.items() if v in c.COMPLETE}
    running = sorted(set(busy) - done)
    held = set(hints) - done - set(running)
    grown = True
    while grown:
        grown = False
        for node, prerequisites in graph.items():
            if node not in held | done | set(running) and held & set(prerequisites):
                held.add(node)
                grown = True
    shared = {
        frozenset((e["from"], e["to"]))
        for e in state["dependencies"]
        if e["kind"] == "shares-files-with"
    }
    placed = set(done)
    remaining = sorted(set(graph) - done - set(running) - held)
    result = []
    while remaining:
        candidates = [n for n in remaining if all(p in placed for p in graph[n])]
        chosen = []
        for node in candidates:
            others = chosen + (running if not result else [])
            if not any(frozenset((node, other)) in shared for other in others):
                chosen.append(node)
        if not result:
            placed |= set(running)
        elif not chosen:
            fail("Cannot order concepts into waves")
        result.append(chosen)
        placed |= set(chosen)
        remaining = [n for n in remaining if n not in chosen]
    return {
        "done": sorted(done),
        "running": running,
        "held": sorted(held),
        "waves": result,
    }


def ready_command(args, root, config, package):
    c = core()
    result = c.check_package(package, root)
    state = c.read_state(package)
    ids = [x["id"] for x in state["concepts"]]
    excluded, hints = {}, set()

    def exclude(target, reason, exact=False):
        for concept in ids:
            if concept == target or (not exact and covers(concept, target)):
                excluded.setdefault(concept, []).append(reason)

    claimed = CLAIMED | c.ACTIVE
    for concept in state["concepts"]:
        if concept["status"] in claimed:
            exclude(concept["id"], f"status {concept['status']}", exact=True)
        if concept.get("type") == "hint" and concept["status"] not in c.COMPLETE:
            hints.add(concept["id"])
            exclude(
                concept["id"], "type hint: waits for a note or a human answer", True
            )
    for task in state.get("tasks", []):
        if task.get("status") in claimed and isinstance(task.get("concept"), str):
            stage = f" at {task['stage']}" if task.get("stage") else ""
            exclude(task["concept"], f"task {task.get('id')} {task['status']}{stage}")
    try:
        base = live_dir(root, config)
        for lease in leases(base):
            found = CONCEPT_LEASE.fullmatch(lease["resource"])
            if (
                found
                and not lease["expired"]
                and lease["package"] in (None, package.name)
            ):
                holder = lease["name"] or "a new holder"
                until = f" until {lease['expires']}" if lease["expires"] else ""
                exclude(found[1], f"lease {lease['lock']} held by {holder}{until}")
        source = str(base)
    except ValueError as error:
        source = f"unavailable: {error}"
    plan = waves(state, set(excluded) - hints, hints)
    result.update(
        {
            "ready": plan["waves"][0] if plan["waves"] else [],
            **plan,
            "excluded": {k: excluded[k] for k in sorted(excluded)},
            "leases": source,
        }
    )
    return result


# ----------------------------------------------------------------- register


def register(sub):
    sub.choices["ready"].set_defaults(handler=ready_command)
