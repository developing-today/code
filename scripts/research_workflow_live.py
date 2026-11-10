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

import hashlib
import importlib.util
import json
import os
import re
import subprocess
import sys
import time
from datetime import datetime, timezone
from pathlib import Path
from urllib.parse import unquote, urlsplit


STAMP = "%Y-%m-%dT%H:%M:%SZ"
GUARD_STALE = 60  # seconds before an ownerless lock is stale
CLAIMED = {"claimed", "running"}
# A lease on C003, concept-C003 or a facet such as C003.2 marks C003 as taken.
CONCEPT_LEASE = re.compile(r"(?:concept-)?(C\d{3,}(?:\.\d+)*)")
DISPOSITIONS = ("incorporated", "acknowledged", "declined", "deferred", "superseded")
RECEIPT_KEYS = {"at", "by", "disposition", "note", "reason", "scanned", "target"}
RECEIPT_BLOCK = re.compile(r"^(`{3,})workflow-receipt\n(.*?)\n\1$", re.M | re.S)
# Everything str.splitlines() breaks at, so a value always stays on one line.
LINE_BREAK = re.compile(r"[\x00-\x1f\x7f-\x9f  ]")


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


def single_line(value, name):
    if not isinstance(value, str) or not value.strip() or LINE_BREAK.search(value):
        fail(f"{name} must be one nonblank line")
    return value


def plain(value):
    """Escape free text so prettier leaves the Markdown line byte-for-byte alone."""
    return re.sub(r"([\\`*_\[\]<>~])", r"\\\1", value)


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


# ----------------------------------------------------------------- receipts


def note_path(package, value):
    """Resolve --note to a file directly in `<package>/notes/`; refuse anything else.

    `notes/<id>.md` is read from the package; any other path from the working
    directory, so `<package>/notes/<id>.md` works too.
    """
    given = Path(value)
    if not given.is_absolute() and given.parts[:1] == ("notes",):
        path = Path(os.path.normpath(package / given))
    else:
        path = Path(os.path.abspath(given))
    folder = package / "notes"
    if (
        folder.is_symlink()
        or path.parent.resolve() != folder
        or path.suffix != ".md"
        or path.name.startswith(".")
    ):
        fail(f"--note must name a notes/<note-id>.md file of the package: {value}")
    if path.is_symlink() or not path.is_file():
        fail(f"Note is not a regular file: {value}")
    return path


def read_note(path, ids):
    """The note's header, once its marker, id, participants and body hash hold."""
    c = core()
    content = path.read_bytes().decode("utf-8")
    found = c.NOTE.search(content)
    if not found or c.NOTE.search(content, found.end()):
        fail(f"{path}: expected exactly one note marker")
    note = json.loads(found[1])
    body = content[found.end() :]
    if not body.startswith("\n\n") or hashlib.sha256(
        body[2:].encode()
    ).hexdigest() != note.get("body_sha256"):
        fail(f"{path}: note body hash mismatch")
    if note.get("id") != path.stem:
        fail(f"{path}: note id does not match its file name")
    if note.get("from") not in ids or note.get("to") not in ids:
        fail(f"{path}: note sender and receiver must be concepts of the package")
    if note["from"] == note["to"]:
        fail(f"{path}: a note goes to another concept")
    return note


def check_target(package, root, target):
    """A target is a link to a file inside the package, with a valid anchor."""
    single_line(target, "Receipt target")
    parsed = urlsplit(target)
    if (
        parsed.scheme
        or parsed.netloc
        or not parsed.path
        or re.search(r"[\s()<>]", target)
    ):
        fail(
            "Receipt target must be a relative link to a file in the package, without "
            f"spaces, parentheses or angle brackets: {target}"
        )
    c = core()
    if not c.contained(package, unquote(parsed.path)).is_file():
        fail(f"Receipt target is not a file in the package: {target}")
    c.check_link(package / "main.md", root, target)


def check_receipt(package, root, entry):
    if not isinstance(entry, dict) or set(entry) != RECEIPT_KEYS:
        fail(f"A receipt has exactly the keys {', '.join(sorted(RECEIPT_KEYS))}")
    parse_stamp(entry["at"])
    single_line(entry["by"], "Receipt by")
    if entry["disposition"] not in DISPOSITIONS:
        fail(f"Receipt disposition must be one of {', '.join(DISPOSITIONS)}")
    if not isinstance(entry["note"], str) or not re.fullmatch(
        r"notes/[^/.][^/]*\.md", entry["note"]
    ):
        fail(f"Receipt note must be notes/<note-id>.md: {entry['note']}")
    single_line(entry["reason"], "Receipt reason")
    if entry["disposition"] == "deferred" and "Revisit when:" not in entry["reason"]:
        fail("A deferred receipt's reason must carry a 'Revisit when: <event>' trigger")
    if not isinstance(entry["scanned"], list) or not all(
        isinstance(x, dict)
        and set(x) == {"ref", "sha"}
        and single_line(x["ref"], "Scanned ref")
        and re.fullmatch(r"[0-9a-f]{7,64}", str(x["sha"]))
        for x in entry["scanned"]
    ):
        fail("Receipt scanned must list {ref, sha} entries")
    check_target(package, root, entry["target"])


def receipt_text(note, entry):
    payload = json.dumps(entry, ensure_ascii=False, indent=2, sort_keys=True)
    fence = "`" * max(
        3, max((len(x) + 1 for x in re.findall(r"`+", payload)), default=0)
    )
    return (
        f"# Receipt for {note['id']}\n\n"
        f"{fence}workflow-receipt\n{payload}\n{fence}\n\n"
        f"- Note: [{note['from']} to {note['to']}](../{entry['note']})\n"
        f"- Disposition: `{entry['disposition']}` — {plain(entry['reason'])}\n"
        f"- Landed in: [{plain(entry['target'])}](../{entry['target']})\n"
    )


def receipts(package, root):
    """Every receipt in the package by note path, each checked whole.

    Registered with research_workflow.DISPOSITION_READERS, so `check` counts a
    note with a receipt as resolved.
    """
    folder = package / "receipts"
    if folder.is_symlink():
        fail(f"{folder}: receipts/ must be a directory, not a symlink")
    found = {}
    for path in sorted(folder.glob("*.md")):
        if path.is_symlink() or not path.is_file():
            fail(f"{path}: a receipt must be a regular file")
        blocks = RECEIPT_BLOCK.findall(path.read_bytes().decode("utf-8"))
        if len(blocks) != 1:
            fail(f"{path}: expected exactly one workflow-receipt block")
        entry = json.loads(blocks[0][1])
        check_receipt(package, root, entry)
        if entry["note"] != f"notes/{path.stem}.md":
            fail(f"{path}: receipt names another note: {entry['note']}")
        found[entry["note"]] = entry
    return found


def resolve_command(args, root, config, package):
    """Write receipts/<note-id>.md once; everything is validated before the write."""
    c = core()
    state = c.read_state(package)
    path = note_path(package, args.note)
    note = read_note(path, {x["id"] for x in state["concepts"]})
    key = f"notes/{path.name}"
    entry = {
        "at": stamp(now()),
        "by": note["to"] if args.by is None else args.by,
        "disposition": args.disposition,
        "note": key,
        "reason": args.reason,
        "scanned": [],
        "target": args.target,
    }
    check_receipt(package, root, entry)
    receipt = package / "receipts" / path.name
    if receipt.exists() or receipt.is_symlink():
        fail(f"{key} already has a receipt: receipts/{path.name}")
    for concept in state["concepts"]:
        text = c.contained(package, concept["path"]).read_text(encoding="utf-8")
        for block in c.RESOLUTION.findall(c.unfenced(text)):
            if json.loads(block).get("note") == key:
                fail(f"{key} already has a disposition in {concept['path']}")
    content = receipt_text(note, entry).encode("utf-8")
    if receipt.parent.is_symlink():
        fail(f"{receipt.parent}: receipts/ must be a directory, not a symlink")
    receipt.parent.mkdir(exist_ok=True)
    try:
        # O_EXCL: of two writers resolving one note, exactly one creates the file.
        fd = os.open(receipt, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o644)
    except FileExistsError:
        fail(f"{key} already has a receipt: receipts/{path.name}")
    try:
        with os.fdopen(fd, "wb") as stream:
            stream.write(content)
    except BaseException:
        receipt.unlink(missing_ok=True)
        raise
    return {"receipt": f"receipts/{path.name}", **entry}


# ----------------------------------------------------------------- register


def register(sub):
    sub.choices["ready"].set_defaults(handler=ready_command)
    core().DISPOSITION_READERS["receipts"] = receipts
    resolve = sub.add_parser(
        "resolve-note", help="record how the receiver handled a note, once"
    )
    resolve.add_argument("--repo-root")
    resolve.add_argument("--package", required=True)
    resolve.add_argument("--note", required=True, help="notes/<note-id>.md")
    resolve.add_argument("--disposition", choices=DISPOSITIONS, required=True)
    resolve.add_argument(
        "--target",
        required=True,
        help="where it landed: <file>#<anchor> in the package",
    )
    resolve.add_argument("--reason", required=True)
    resolve.add_argument(
        "--by", help="who resolved it; defaults to the receiving concept"
    )
    resolve.set_defaults(handler=resolve_command)
