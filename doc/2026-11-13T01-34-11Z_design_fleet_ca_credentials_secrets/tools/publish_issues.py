#!/usr/bin/env python3
"""Publish a reviewed Fleet CA issue manifest through fleet gh + repo-queue.

The complete argument vector and the entire manifest are validated before
subprocesses, network calls, or receipt/temp-file writes. No writes occur by
default; --publish is the explicit write gate. Never call gh directly here:
fleet gh owns the token rotation and write retry-safety contract.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import stat
import subprocess
import sys
import tempfile
import time
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MARKER_PREFIX = "fleet-ca-v03-proposal:"
REPO = "developing-today/code"
QUEUE = "repo-health-code-pilot"
FLEET = "fleet"
REPO_QUEUE = "repo-queue"
REPO_ROOT = Path(__file__).resolve().parents[3]
DESIGN_DIR = REPO_ROOT / "doc" / "2026-11-13T01-34-11Z_design_fleet_ca_credentials_secrets"
PUBLICATION_DIR = DESIGN_DIR / "evidence" / "publication"
TEMP_ROOT = Path.home() / ".local/share/opencode/tmp/opencode"
MAX_ISSUE_BODY_CHARS = 65_536
MAX_ISSUE_BODY_BYTES = 65_536
LIVE_TASK_STATUSES = {"queued", "claimed", "parked", "blocked"}
COMMON_PROPOSAL_FIELDS = {"id", "title", "parent", "kind", "source_ranges", "requirement_ids",
                         "stage", "lifecycle", "urgency", "depends_on", "body"}
URGENCY = {
    # Intrinsic score bands (lower = sooner); each maps separately to the
    # established GitHub label and repo-queue scale.
    "emergency": (0, 99, "priority/critical", 10),
    "supercritical": (100, 199, "priority/critical", 10),
    "critical": (200, 399, "priority/critical", 10),
    "high": (400, 799, "priority/high", 20),
    "standard": (800, 1199, "priority/normal", 40),
    "low": (1200, 1599, "priority/low", 70),
    "someday": (1600, 1999, "priority/low", 70),
    "cold": (2000, None, "priority/low", 70),
}
KIND_LABEL = {
    "design": "kind/design", "research": None,
    "implementation": "kind/feature", "test": "kind/test",
    "migration": None, "operations": None,
    "security review": "kind/security", "documentation": "kind/docs",
}


class PublishError(Exception):
    pass


class QueueReadbackError(PublishError):
    def __init__(self, message: str, scans: dict[str, dict[str, Any]]):
        super().__init__(message)
        self.scans = scans


def reject_duplicate_json_keys(pairs: list[tuple[str, Any]]) -> dict[str, Any]:
    result: dict[str, Any] = {}
    for key, value in pairs:
        if key in result:
            raise PublishError(f"duplicate JSON key: {key}")
        result[key] = value
    return result


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--queue", required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--publish", action="store_true", help="perform writes; default is --dry-run")
    mode.add_argument("--dry-run", action="store_true", help="print the plan only (default)")
    parser.add_argument("--receipt")
    args = parser.parse_args(argv)
    for option in ("manifest", "queue", "receipt"):
        value = getattr(args, option)
        if value is None:
            continue
        if "\x00" in value:
            parser.error(f"--{option} must not contain U+0000")
        try:
            value.encode("utf-8", errors="strict")
        except UnicodeError:
            parser.error(f"--{option} must be strictly UTF-8 encodable")
    if args.publish and not args.receipt:
        parser.error("--receipt is required with --publish")
    if not args.publish and args.receipt:
        parser.error("--receipt is only valid with --publish")
    if args.queue != QUEUE:
        parser.error(f"--queue must be the verified code pilot queue {QUEUE!r}")
    return args


def _str(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def validate_text(value: Any, path: str = "$") -> None:
    """Reject NUL and non-UTF-8 scalar values recursively without echoing contents."""
    if isinstance(value, str):
        if "\x00" in value:
            raise PublishError(f"{path} contains U+0000")
        try:
            value.encode("utf-8", errors="strict")
        except UnicodeError as exc:
            raise PublishError(f"{path} is not strictly UTF-8 encodable") from exc
        return
    if isinstance(value, list):
        for index, item in enumerate(value):
            validate_text(item, f"{path}[{index}]")
        return
    if isinstance(value, dict):
        for index, (key, item) in enumerate(value.items()):
            validate_text(key, f"{path}.<key[{index}]>")
            child_path = f"{path}.{key}" if isinstance(key, str) and key in COMMON_PROPOSAL_FIELDS else f"{path}.value[{index}]"
            validate_text(item, child_path)


def validate_manifest(doc: Any) -> list[dict[str, Any]]:
    """Validate the shared proposals array and resolve its dependency DAG."""
    validate_text(doc)
    if not isinstance(doc, list) or not doc:
        raise PublishError("manifest must be a non-empty JSON array of common proposal objects")
    ids: set[str] = set()
    fields = COMMON_PROPOSAL_FIELDS
    result: list[dict[str, Any]] = []
    allowed_kinds = set(KIND_LABEL)
    for i, source in enumerate(doc):
        where = f"proposals[{i}]"
        if not isinstance(source, dict):
            raise PublishError(f"{where} must be an object")
        if set(source) != fields:
            missing, extra = sorted(fields - set(source)), sorted(set(source) - fields)
            raise PublishError(f"{where} schema mismatch; missing={missing}, extra={extra}")
        p = dict(source)
        pid = p.get("id")
        if not _str(pid) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]{0,100}", pid):
            raise PublishError(f"{where}.id must be a stable alphanumeric id")
        if pid in ids:
            raise PublishError(f"duplicate proposal id: {pid}")
        ids.add(pid)
        for field in ("title", "body", "kind", "stage", "lifecycle"):
            if not _str(p.get(field)):
                raise PublishError(f"{where}.{field} must be a non-empty string")
        if len(p["title"]) > 256 or "\n" in p["title"] or "\r" in p["title"]:
            raise PublishError(f"{where}.title must be one line of at most 256 characters")
        if MARKER_PREFIX in p["body"]:
            raise PublishError(f"{where}.body must not contain publisher marker text")
        if p["kind"] not in allowed_kinds:
            raise PublishError(f"{where}.kind is not in the coordinator issue-kind contract")
        if p["lifecycle"] not in {"Ready", "Backlog", "Blocked", "Parked"}:
            raise PublishError(f"{where}.lifecycle must be Ready|Backlog|Blocked|Parked")
        if type(p["urgency"]) is not int or p["urgency"] < 0:
            raise PublishError(f"{where}.urgency must be a non-negative intrinsic numeric score")
        raw_parent = p.get("parent")
        if type(raw_parent) is int:
            parent = raw_parent
        elif isinstance(raw_parent, str) and re.fullmatch(r"#?\d+", raw_parent):
            parent = int(raw_parent.removeprefix("#"))
        else:
            parent = -1
        if parent not in {9156, 9157, 9158, 9159, 9160, 9162, 9163, 9164, 9165, 9166, 9167}:
            raise PublishError(f"{where}.parent must be one of the existing domain starters (not #9161)")
        p["parent"] = parent
        for field in ("source_ranges", "requirement_ids", "depends_on"):
            if not isinstance(p.get(field), list) or any(not _str(x) for x in p[field]):
                raise PublishError(f"{where}.{field} must be a list of non-empty strings")
        if not p["source_ranges"] or not p["requirement_ids"]:
            raise PublishError(f"{where}: source_ranges and requirement_ids must not be empty")
        urgency_band = next((band for band, (low, high, _, _) in URGENCY.items()
                             if p["urgency"] >= low and (high is None or p["urgency"] <= high)), "low")
        p["urgency_band"] = urgency_band
        p["priority_label"] = URGENCY[urgency_band][2]
        p["priority"] = URGENCY[urgency_band][3]
        p["labels"] = [label for label in (
            KIND_LABEL[p["kind"]], "app/fleet", "epic/fleet-ca-secrets", p["priority_label"]
        ) if label]
        # Do not label a new issue status/ready before its initial triage:
        # that label specifically means triaged and ready for implementation.
        lifecycle_label = {"Blocked": "status/blocked", "Parked": "status/parked"}.get(p["lifecycle"])
        if lifecycle_label:
            p["labels"].append(lifecycle_label)
        p["queue_kind"] = "triage"
        blockers = ", ".join(p["depends_on"]) if p["depends_on"] else "none declared"
        p["problem"] = (
            f"Initial triage only for {p['title']} ({p['stage']}; lifecycle {p['lifecycle']}; "
            f"proposal blockers: {blockers}). Read the issue's native links. Do not begin implementation "
            "or production changes from this triage task; #9155 safety gates remain in force."
        )
        p["priority_reason"] = (
            f"Initial triage queue priority {p['priority']} is the defined projection of the "
            f"separate intrinsic urgency score {p['urgency']} ({urgency_band}); it is not that score."
        )
        p["evidence"] = [*p["requirement_ids"], *p["source_ranges"]]
        p["acceptance"] = [f"Triage and route the independently scoped deliverable in issue {p['id']}; preserve its stated acceptance criteria."]
        p["tests"] = [f"Use the issue's stated verification/tests and record evidence before dispositioning {p['id']}."]
        p["scope"] = {"include": [p["title"]], "exclude": ["Production changes forbidden by the #9155 safety gates"]}
        p["risk"] = {"level": "medium", "flags": ["design-triage"]}
        p["timebox_minutes"] = 20
        p["max_attempts"] = 6
        check_content_contract(p["body"], p["id"])
        result.append(p)
    by_id = {p["id"]: p for p in result}
    for p in result:
        if len(set(p["depends_on"])) != len(p["depends_on"]):
            raise PublishError(f"{p['id']}.depends_on contains duplicates")
        unknown = set(p["depends_on"]) - set(by_id)
        if unknown:
            raise PublishError(f"{p['id']}.depends_on references unknown proposal IDs: {sorted(unknown)}")
        if p["id"] in p["depends_on"]:
            raise PublishError(f"{p['id']} cannot depend on itself")
    # Stable topological order means every blocker has a numeric issue id before linking.
    ordered: list[dict[str, Any]] = []
    pending = list(result)
    done: set[str] = set()
    while pending:
        ready = [p for p in pending if set(p["depends_on"]).issubset(done)]
        if not ready:
            raise PublishError("depends_on contains a cycle")
        for p in ready:
            ordered.append(p)
            done.add(p["id"])
            pending.remove(p)
    # Validate the actual body that will be sent, including publisher metadata,
    # before any subprocess/network/temp/receipt write for the batch.
    for p in ordered:
        rendered = issue_body(p)
        encoded = rendered.encode("utf-8")
        if len(rendered) > MAX_ISSUE_BODY_CHARS or len(encoded) > MAX_ISSUE_BODY_BYTES:
            raise PublishError(f"{p['id']}.rendered_body exceeds GitHub's 65,536-character/byte issue limit")
        p["rendered_body"] = rendered
        p["rendered_body_sha256"] = hashlib.sha256(encoded).hexdigest()
    return ordered


def marker(proposal: dict[str, Any]) -> str:
    return f"<!-- {MARKER_PREFIX}{proposal['id']} -->"


def issue_body(proposal: dict[str, Any]) -> str:
    return (proposal["body"].rstrip() + f"\n\nKind: {proposal['kind']}\nStage: {proposal['stage']}\n"
            f"Lifecycle: {proposal['lifecycle']}\nIntrinsic urgency: {proposal['urgency']} "
            f"({proposal['urgency_band']}; lower is sooner)\n"
            f"Requirement IDs: {', '.join(proposal['requirement_ids'])}\n"
            f"Source ranges: {', '.join(proposal['source_ranges'])}\n" + marker(proposal))


def check_content_contract(body: str, proposal_id: str) -> None:
    headings = {re.sub(r"[^a-z0-9]+", " ", value.lower()).strip()
                for value in re.findall(r"(?m)^#{1,6}\s+(.+?)\s*$", body)}
    required_groups = {
        "outcome/why": {"outcome", "why", "why user outcome", "outcome and why"},
        "current repo evidence": {"current evidence", "repo evidence", "verified evidence"},
        "technical boundaries": {"technical boundaries", "boundaries", "scope", "proposed files interfaces"},
        "acceptance": {"acceptance", "acceptance criteria", "acceptance oracle", "definition of done"},
        "tests": {"tests", "negative tests", "test verification"},
        "failure modes": {"failure modes", "risk failure modes", "risks and mitigations", "risk and mitigation"},
        "safety": {"safety", "safety and rollback"},
        "rollback": {"rollback", "safety and rollback"},
        "estimate": {"estimate", "size complexity", "size and complexity"},
        "open decisions": {"open decisions", "open questions"},
    }
    missing = [name for name, aliases in required_groups.items()
               if not any(heading == alias or heading.startswith(alias + " ")
                          for heading in headings for alias in aliases)]
    if missing:
        raise PublishError(f"{proposal_id}.body lacks coordinator §4.2 sections: {', '.join(missing)}")


def queue_packet(p: dict[str, Any], queue: str, issue: dict[str, Any]) -> dict[str, Any]:
    url = issue["url"]
    title = issue["title"]
    return {
        "title": f"Triage {REPO}#{issue['number']}: {title}"[:180],
        "body": f"Creator filing needs initial triage: {url}",
        "priority": p["priority"],
        "dedupe_key": f"repo-health:v1:triage:{REPO}:issue:{issue['number']}",
        "payload": {
            "schema": "repo-task/v1", "kind": p["queue_kind"], "repo": REPO, "queue": queue,
            "issue": {"number": issue["number"], "url": url, "title": title},
            "problem": p["problem"], "priority_reason": p["priority_reason"],
            "evidence": p["evidence"], "acceptance": p["acceptance"], "tests": p["tests"],
            "scope": p["scope"], "risk": p["risk"], "timebox_minutes": p["timebox_minutes"],
            "max_attempts": p["max_attempts"], "context_refs": [url],
            "dedupe": {"checked_at": issue["created_at"],
                       "basis": f"Stable issue proposal {p['id']}; issue marker searched before creation."},
        },
    }


def run(argv: list[str], *, capture: bool = True,
        cwd: Path | None = None) -> subprocess.CompletedProcess[str]:
    try:
        env = os.environ.copy()
        # FLEET_GH_BIN is an upstream test/control hook that can replace the
        # real gh binary. Do not let it bypass this publisher's rotation path.
        env.pop("FLEET_GH_BIN", None)
        return subprocess.run(argv, capture_output=capture, text=True, check=False,
                              timeout=120, env=env, cwd=cwd)
    except (OSError, ValueError, subprocess.TimeoutExpired) as exc:
        raise PublishError(f"command failed to start or timed out: {argv[0]}: {exc}") from exc


def is_remote_mutation(argv: list[str]) -> bool:
    if argv[:3] == [FLEET, "gh", "issue"] and len(argv) > 3 and argv[3] in {"create", "edit"}:
        return True
    if argv[:3] == [FLEET, "gh", "api"]:
        if "--method" in argv:
            index = argv.index("--method")
            if index + 1 < len(argv) and argv[index + 1].upper() not in {"GET", "HEAD"}:
                return True
        elif "-F" in argv or "-f" in argv:
            return True  # gh api defaults parameter-bearing requests to POST.
    return argv[:2] == [REPO_QUEUE, "enqueue"]


def checked(argv: list[str]) -> str:
    for attempt in range(3):
        if is_remote_mutation(argv):
            # Recheck for each remote mutation attempt, including a classified
            # rate-limit retry after fleet gh has exhausted its own rotation.
            require_epic_open(FLEET)
        proc = run(argv)
        if proc.returncode == 0:
            return proc.stdout.strip()
        detail = (proc.stderr or proc.stdout).strip()
        # fleet gh has already exhausted its sanctioned token rotation. Retry
        # only a visible GitHub rate-limit refusal (pre-execution safe); never
        # retry an ambiguous write or a permission/validation failure.
        is_rate_limit = bool(re.search(r"(?:HTTP\s+429|API rate limit exceeded|secondary rate limit|You have exceeded.*rate limit)", detail, re.I))
        if not is_rate_limit or attempt == 2:
            raise PublishError(f"command returned {proc.returncode}: {argv[:4]}: {detail[:500]}")
        retry_after = re.search(r"Retry-After:\s*(\d+)", detail, re.I)
        delay = int(retry_after.group(1)) if retry_after else min(2 ** attempt, 4)
        time.sleep(delay)
    raise AssertionError("unreachable")


def json_output(argv: list[str]) -> Any:
    output = checked(argv)
    try:
        return json.loads(output)
    except json.JSONDecodeError as exc:
        raise PublishError(f"expected JSON from {argv[:4]}") from exc


def issue_by_marker(fleet: str, key: str) -> dict[str, Any] | None:
    search_token = key.removeprefix("<!-- ").removesuffix(" -->")
    hits = json_output([fleet, "gh", "issue", "list", "--repo", REPO, "--state", "all",
                        "--limit", "500", "--search", f'"{search_token}" in:body',
                        "--json", "number,title,body,url,labels,state"])
    if not isinstance(hits, list):
        raise PublishError("issue search returned a non-list result")
    if any(not isinstance(item, dict) for item in hits):
        raise PublishError("issue search returned a malformed issue record")
    if any(type(item.get("number")) is not int or item["number"] < 1
           or not isinstance(item.get("body"), str) or not isinstance(item.get("title"), str)
           or not isinstance(item.get("url"), str) for item in hits):
        raise PublishError("issue search returned incomplete number/title/body/URL fields")
    exact = [i for i in hits if key in (i.get("body") or "")]
    if len(exact) > 1:
        raise PublishError(f"marker {key} matches multiple issues; refusing to choose")
    return exact[0] if exact else None


def api_json(fleet: str, method: str, endpoint: str, *fields: str) -> Any:
    args = [fleet, "gh", "api", "--method", method, endpoint]
    for field in fields:
        args.extend(["-F", field])
    return json_output(args)


def issue_close_reopen_history(fleet: str, number: int) -> dict[str, Any]:
    """Read a bounded complete issue-event history; incomplete history fails closed."""
    events: list[dict[str, Any]] = []
    page_size = 100
    max_pages = 20
    for page in range(1, max_pages + 1):
        page_events = api_json(fleet, "GET",
                               f"repos/{REPO}/issues/{number}/events?per_page={page_size}&page={page}")
        if not isinstance(page_events, list) or any(not isinstance(event, dict) for event in page_events):
            raise PublishError(f"issue #{number} lifecycle history was malformed")
        events.extend(page_events)
        if len(page_events) < page_size:
            break
    else:
        raise PublishError(f"issue #{number} lifecycle history exceeded the bounded {max_pages * page_size}-event scan")

    transitions = []
    for event in events:
        kind = event.get("event")
        if not isinstance(kind, str):
            raise PublishError(f"issue #{number} lifecycle history has a malformed event kind")
        if kind in {"closed", "reopened"}:
            created = event.get("created_at")
            if not _str(created):
                raise PublishError(f"issue #{number} close/reopen history lacks a timestamp")
            try:
                when = datetime.fromisoformat(created.replace("Z", "+00:00"))
            except ValueError as exc:
                raise PublishError(f"issue #{number} close/reopen timestamp was invalid") from exc
            if when.tzinfo is None:
                raise PublishError(f"issue #{number} close/reopen timestamp lacked a timezone")
            transitions.append((kind, when.astimezone(timezone.utc)))
    closed = any(kind == "closed" for kind, _ in transitions)
    reopened = any(kind == "reopened" for kind, _ in transitions)
    closed_then_reopened = any(
        close_time < reopen_time
        for close_kind, close_time in transitions if close_kind == "closed"
        for reopen_kind, reopen_time in transitions if reopen_kind == "reopened"
    )
    return {"checked": True, "closed": closed, "reopened": reopened,
            "closed_then_reopened": closed_then_reopened, "event_count": len(events)}


def require_fresh_issue_history(fleet: str, number: int) -> dict[str, bool]:
    history = issue_close_reopen_history(fleet, number)
    if history["closed"] and history["reopened"] and history["closed_then_reopened"]:
        raise PublishError(f"issue #{number} was previously closed then reopened; refusing to reuse its marker")
    if history["closed"] or history["reopened"]:
        raise PublishError(f"issue #{number} has inconsistent/incomplete close-reopen history; manual reconciliation required")
    return {"checked": True, "closed": False, "reopened": False,
            "closed_then_reopened": False, "event_count": history["event_count"]}


def require_epic_open(fleet: str) -> None:
    epic = json_output([fleet, "gh", "issue", "view", "9155", "--repo", REPO, "--json", "number,state"])
    if not isinstance(epic, dict) or epic.get("number") != 9155 or str(epic.get("state", "")).upper() != "OPEN":
        raise PublishError("program epic #9155 is not OPEN; refusing issue creation")


def parent_children(fleet: str, parent: int) -> list[dict[str, Any]]:
    children = api_json(fleet, "GET", f"repos/{REPO}/issues/{parent}/sub_issues?per_page=100")
    if (not isinstance(children, list) or any(not isinstance(child, dict) for child in children)
            or any(type(child.get("number")) is not int or child["number"] < 1 for child in children)):
        raise PublishError(f"parent #{parent} sub-issues read-back was malformed")
    numbers = [child["number"] for child in children]
    if len(numbers) != len(set(numbers)):
        raise PublishError(f"parent #{parent} sub-issues read-back contained duplicate children")
    return children


def validate_dependency_rows(rows: Any) -> dict[int, dict[str, Any]]:
    """Validate every native blocker row; never filter malformed/foreign edges out."""
    if not isinstance(rows, list) or any(not isinstance(row, dict) for row in rows):
        raise PublishError("native blocked_by response was not a list of issue objects")
    by_number: dict[int, dict[str, Any]] = {}
    database_ids: set[int] = set()
    api_repo_url = f"https://api.github.com/repos/{REPO}"
    for row in rows:
        number, database_id = row.get("number"), row.get("id")
        if type(number) is not int or number < 1 or type(database_id) is not int or database_id < 1:
            raise PublishError("native blocker row has a missing/non-positive issue or database ID")
        expected_api_url = f"{api_repo_url}/issues/{number}"
        expected_html_url = f"https://github.com/{REPO}/issues/{number}"
        if "url" in row and row["url"] != expected_api_url:
            raise PublishError(f"native blocker #{number} API URL is noncanonical/foreign")
        if "html_url" in row and row["html_url"] != expected_html_url:
            raise PublishError(f"native blocker #{number} HTML URL is noncanonical/foreign")
        if "repository_url" in row and row["repository_url"] != api_repo_url:
            raise PublishError(f"native blocker #{number} belongs to another repository")
        if number in by_number or database_id in database_ids:
            raise PublishError("native blocked_by response contains duplicate issue/database IDs")
        by_number[number] = row
        database_ids.add(database_id)
    return by_number


def validate_receipt_target(
    candidate: str | Path,
    digest: str,
    *,
    repo_root: Path = REPO_ROOT,
    publication_dir: Path = PUBLICATION_DIR,
) -> Path:
    """Only permit this manifest's dedicated, untracked append-receipt path."""
    expected = publication_dir / f"{digest}.jsonl"
    target = Path(os.path.abspath(candidate))
    if target != Path(os.path.abspath(expected)):
        raise PublishError(f"--receipt must be the manifest-specific path {expected}")

    dirs = [repo_root, repo_root / "doc", publication_dir.parents[1],
            publication_dir.parent, publication_dir]
    for directory in dirs:
        try:
            info = directory.lstat()
        except FileNotFoundError:
            if directory == publication_dir:
                continue
            raise PublishError(f"receipt path component is missing: {directory}")
        if stat.S_ISLNK(info.st_mode) or not stat.S_ISDIR(info.st_mode):
            raise PublishError(f"receipt path component must be a real directory, not a symlink: {directory}")

    try:
        info = target.lstat()
    except FileNotFoundError:
        info = None
    if info is not None:
        if stat.S_ISLNK(info.st_mode):
            raise PublishError("receipt target must not be a symlink (including a dangling symlink)")
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise PublishError("existing receipt must be a regular single-link file")

    relative = target.relative_to(repo_root)
    tracked = run(["git", "ls-files", "--error-unmatch", "--", str(relative)], cwd=repo_root)
    if tracked.returncode == 0:
        raise PublishError("receipt target is tracked; refusing to append to repository source")
    if tracked.returncode != 1:
        raise PublishError(f"could not establish receipt target is untracked: {tracked.stderr.strip()[:300]}")
    return target


def append_receipt(path: Path, value: dict[str, Any]) -> None:
    """Append one fsynced JSONL event without following a receipt symlink."""
    dir_fd = None
    fd = None
    try:
        absolute_parent = Path(os.path.abspath(path.parent))
        flags = os.O_RDONLY | os.O_DIRECTORY | os.O_NOFOLLOW
        dir_fd = os.open(absolute_parent.anchor, flags)
        components = absolute_parent.parts[1:]
        for index, component in enumerate(components):
            try:
                next_fd = os.open(component, flags, dir_fd=dir_fd)
            except FileNotFoundError:
                if index != len(components) - 1:
                    raise PublishError(f"receipt parent component disappeared: {component}")
                os.mkdir(component, mode=0o700, dir_fd=dir_fd)
                os.fsync(dir_fd)
                next_fd = os.open(component, flags, dir_fd=dir_fd)
            os.close(dir_fd)
            dir_fd = next_fd
        fd = os.open(path.name, os.O_CREAT | os.O_APPEND | os.O_WRONLY | os.O_NOFOLLOW,
                     0o600, dir_fd=dir_fd)
        info = os.fstat(fd)
        if not stat.S_ISREG(info.st_mode) or info.st_nlink != 1:
            raise PublishError("receipt target changed to a non-regular or multiply-linked file")
        data = (json.dumps(value, sort_keys=True, separators=(",", ":")) + "\n").encode("utf-8")
        view = memoryview(data)
        while view:
            written = os.write(fd, view)
            if written <= 0:
                raise OSError("short append made no progress")
            view = view[written:]
        os.fsync(fd)
        os.fsync(dir_fd)
    except OSError as exc:
        raise PublishError(f"cannot append durable receipt {path}: {exc}") from exc
    finally:
        if fd is not None:
            os.close(fd)
        if dir_fd is not None:
            os.close(dir_fd)


def write_temp_json(value: dict[str, Any]) -> Path:
    if not TEMP_ROOT.is_dir():
        raise PublishError(f"approved temporary directory is unavailable: {TEMP_ROOT}")
    path: Path | None = None
    try:
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json",
                                         dir=TEMP_ROOT, delete=False) as stream:
            path = Path(stream.name)
            json.dump(value, stream)
        return path
    except OSError as exc:
        if path is not None:
            path.unlink(missing_ok=True)
        raise PublishError(f"could not stage metadata-only JSON in the approved temporary directory: {exc}") from exc


def empty_receipt_state() -> dict[str, Any]:
    return {
        "records": [], "create_intents": {}, "create_results": {},
        "issue_verified": {}, "queue_intents": {}, "queue_acks": {},
        "task_verified": {}, "queue_unverified": [],
    }


def read_receipt(path: Path, plan: dict[str, Any], proposals: list[dict[str, Any]]) -> dict[str, Any]:
    state = empty_receipt_state()
    if not path.exists():
        return state
    try:
        records = [json.loads(line, object_pairs_hook=reject_duplicate_json_keys)
                   for line in path.read_text(encoding="utf-8").splitlines() if line.strip()]
    except (OSError, UnicodeError, json.JSONDecodeError, PublishError) as exc:
        raise PublishError(f"cannot read append receipt safely: {exc}") from exc
    if any(not isinstance(record, dict) for record in records):
        raise PublishError("receipt contains a non-object record")
    header_keys = {"event", "repo", "queue", "proposal_digest", "proposals", "started_at"}
    if not records or set(records[0]) != header_keys:
        raise PublishError("receipt must start with exactly one complete plan header")
    header = records[0]
    expected = {"event": "plan", "repo": REPO, "queue": plan["queue"],
                "proposal_digest": plan["proposal_digest"], "proposals": plan["proposals"]}
    if any(header.get(k) != v for k, v in expected.items()) or not _str(header.get("started_at")):
        raise PublishError("existing receipt does not match this repo, queue, and complete proposal plan")

    proposals_by_id = {p["id"]: p for p in proposals}
    expected_body_hash = {p["id"]: p["rendered_body_sha256"] for p in proposals}
    for record in records[1:]:
        event = record.get("event")
        pid = record.get("proposal_id")
        if not isinstance(event, str):
            raise PublishError("receipt event name must be a string")
        per_proposal_events = {"create_intent", "create_result", "issue_verified", "queue_intent",
                               "queue_ack", "task_verified", "queue_unverified"}
        if event in per_proposal_events and (not isinstance(pid, str) or pid not in proposals_by_id):
            raise PublishError(f"receipt {event} references an unknown proposal")
        if event == "create_intent":
            if set(record) != {"event", "proposal_id", "marker", "body_sha256", "parent", "title", "at"}:
                raise PublishError("malformed create_intent receipt event")
            if pid not in proposals_by_id or pid in state["create_intents"] or pid in state["create_results"]:
                raise PublishError("duplicate or unknown create_intent proposal")
            p = proposals_by_id[pid]
            if (record["marker"] != marker(p) or record["body_sha256"] != expected_body_hash[pid]
                    or record["parent"] != p["parent"] or record["title"] != p["title"] or not _str(record["at"])):
                raise PublishError(f"create_intent does not match proposal {pid}")
            state["create_intents"][pid] = record
        elif event == "create_result":
            if set(record) != {"event", "proposal_id", "number", "url", "body_sha256", "source", "at"}:
                raise PublishError("malformed create_result receipt event")
            if pid not in proposals_by_id or pid in state["create_results"]:
                raise PublishError("duplicate or unknown create_result proposal")
            if not isinstance(record["source"], str) or record["source"] not in {"create_response", "marker_search"}:
                raise PublishError("create_result has an unrecognized reconciliation source")
            if record["source"] == "create_response" and pid not in state["create_intents"]:
                raise PublishError("create_result response has no preceding durable create_intent")
            if (type(record["number"]) is not int or record["number"] < 1
                    or not _str(record["url"]) or record["body_sha256"] != expected_body_hash[pid]
                    or not _str(record["at"])):
                raise PublishError(f"create_result fields do not match proposal {pid}")
            match = re.fullmatch(rf"https://github\.com/{re.escape(REPO)}/issues/(\d+)", record["url"])
            if not match or int(match.group(1)) != record["number"]:
                raise PublishError(f"create_result URL/number mismatch for proposal {pid}")
            state["create_results"][pid] = record
        elif event == "issue_verified":
            if set(record) != {"event", "proposal_id", "issue", "at"}:
                raise PublishError("malformed issue_verified receipt event")
            if pid not in proposals_by_id or pid in state["issue_verified"] or pid not in state["create_results"]:
                raise PublishError("duplicate, unknown, or out-of-order issue_verified event")
            issue = record["issue"]
            required = {"number", "url", "database_id", "marker", "body_sha256", "parent",
                        "parent_issue_url", "blocked_by", "labels", "lifecycle_history"}
            if not isinstance(issue, dict) or set(issue) != required:
                raise PublishError("malformed issue_verified issue receipt")
            result = state["create_results"][pid]
            p = proposals_by_id[pid]
            expected_parent_url = f"https://api.github.com/repos/{REPO}/issues/{p['parent']}"
            history = issue["lifecycle_history"]
            if (not isinstance(history, dict)
                    or set(history) != {"checked", "closed", "reopened", "closed_then_reopened", "event_count"}
                    or history["checked"] is not True
                    or any(type(history[k]) is not bool for k in ("closed", "reopened", "closed_then_reopened"))
                    or type(history["event_count"]) is not int or history["event_count"] < 0):
                raise PublishError("issue_verified lifecycle history flags were malformed")
            expected_dependencies = []
            for dependency_id in p["depends_on"]:
                dependency_record = state["issue_verified"].get(dependency_id)
                if dependency_record is None:
                    raise PublishError(f"issue_verified {pid} precedes native blocker {dependency_id}")
                dependency_issue = dependency_record["issue"]
                expected_dependencies.append((dependency_issue["number"], dependency_issue["database_id"]))
            actual_dependencies = issue["blocked_by"]
            if (not isinstance(actual_dependencies, list)
                    or any(not isinstance(edge, dict) or set(edge) != {"number", "database_id"}
                           or type(edge.get("number")) is not int or edge["number"] < 1
                           or type(edge.get("database_id")) is not int or edge["database_id"] < 1
                           for edge in actual_dependencies)):
                raise PublishError("issue_verified native blocker set is malformed")
            actual_dependency_pairs = sorted((edge["number"], edge["database_id"]) for edge in actual_dependencies)
            if actual_dependency_pairs != sorted(expected_dependencies):
                raise PublishError(f"issue_verified native blocker set is not exact for {pid}")
            if (issue["number"] != result["number"] or issue["url"] != result["url"]
                    or type(issue["database_id"]) is not int or issue["database_id"] < 1
                    or issue["marker"] != marker(p) or issue["body_sha256"] != expected_body_hash[pid]
                    or issue["parent"] != p["parent"] or issue["parent_issue_url"] != expected_parent_url
                    or not isinstance(issue["labels"], list)
                    or any(not _str(label) for label in issue["labels"])
                    or not _str(record["at"])):
                raise PublishError(f"issue_verified receipt fields do not match proposal {pid}")
            state["issue_verified"][pid] = record
        elif event == "task_verified":
            if set(record) != {"event", "proposal_id", "task", "readback_scope", "scans", "at"}:
                raise PublishError("malformed task_verified receipt event")
            if (pid not in proposals_by_id or pid in state["task_verified"]
                    or pid not in state["issue_verified"] or pid not in state["queue_intents"]):
                raise PublishError("duplicate, unknown, or out-of-order task_verified event")
            task = record["task"]
            required = {"id", "status", "queue", "dedupe_key", "issue_number", "priority"}
            if not isinstance(task, dict) or set(task) != required:
                raise PublishError("malformed task_verified task receipt")
            scope = record["readback_scope"]
            if (not isinstance(scope, dict) or set(scope) != {"queue", "status_filters", "completed_status_filters",
                                                            "limit_per_status", "matched_row"}
                    or scope["queue"] != plan["queue"]
                    or scope["status_filters"] != sorted(LIVE_TASK_STATUSES)
                    or not isinstance(scope["completed_status_filters"], list)
                    or any(s not in LIVE_TASK_STATUSES for s in scope["completed_status_filters"])
                    or not isinstance(record["scans"], dict)
                    or set(record["scans"]) != set(scope["completed_status_filters"])
                    or any(not isinstance(scan, dict)
                           or set(scan) != {"count", "ids", "limit", "truncated"}
                           or type(scan["count"]) is not int or scan["count"] < 0
                           or type(scan["limit"]) is not int or scan["limit"] != 500
                           or type(scan["truncated"]) is not bool
                           or not isinstance(scan["ids"], list)
                           or scan["count"] != len(scan["ids"])
                           or scan["truncated"] != (scan["count"] >= scan["limit"])
                           or any(type(task_id) is not int or task_id < 1 for task_id in scan["ids"])
                           for scan in record["scans"].values())
                    or scope["limit_per_status"] != 500 or not isinstance(scope["matched_row"], dict)):
                raise PublishError("task_verified read-back scope is incomplete")
            issue_number = state["create_results"][pid]["number"]
            expected_key = f"repo-health:v1:triage:{REPO}:issue:{issue_number}"
            if (type(task["id"]) is not int or task["id"] < 1
                    or not isinstance(task["status"], str) or task["status"] not in LIVE_TASK_STATUSES
                    or task["queue"] != plan["queue"] or task["dedupe_key"] != expected_key
                    or task["issue_number"] != issue_number
                    or task["priority"] != proposals_by_id[pid]["priority"] or not _str(record["at"])):
                raise PublishError(f"task_verified receipt fields do not match proposal {pid}")
            matched_row = scope["matched_row"]
            if (set(matched_row) != {"id", "priority", "status", "age", "dedupe_key", "title"}
                    or matched_row.get("id") != task["id"] or matched_row.get("status") != task["status"]
                    or matched_row.get("dedupe_key") != task["dedupe_key"]
                    or matched_row.get("priority") != task["priority"]):
                raise PublishError("task_verified row is not the exact same ID/key/status observation")
            state["task_verified"][pid] = record
        elif event == "failure":
            if set(record) not in ({"event", "error", "at"}, {"event", "proposal_id", "phase", "error", "at"}):
                raise PublishError("malformed failure receipt event")
            if "proposal_id" in record and (not isinstance(record["proposal_id"], str)
                                             or record["proposal_id"] not in proposals_by_id):
                raise PublishError("failure receipt references unknown proposal")
            if "phase" in record and not _str(record["phase"]):
                raise PublishError("failure receipt phase is empty")
            if not _str(record.get("error")) or not _str(record.get("at")):
                raise PublishError("failure receipt lacks error/time")
        elif event == "queue_unverified":
            required = {"event", "proposal_id", "ack", "scans", "readback_scope", "reason", "at"}
            if (set(record) != required or pid not in proposals_by_id
                    or pid not in state["issue_verified"] or pid not in state["queue_intents"]):
                raise PublishError("malformed queue_unverified receipt event")
            if (not isinstance(record["ack"], dict) or not isinstance(record["scans"], dict)
                    or not _str(record["reason"]) or not _str(record["at"])):
                raise PublishError("queue_unverified receipt lacks evidence")
            ack = record["ack"]
            issue_number = state["create_results"][pid]["number"]
            scope = record["readback_scope"]
            expected_key = f"repo-health:v1:triage:{REPO}:issue:{issue_number}"
            if (set(ack) != {"task_id", "queue", "received_queue", "expected_dedupe_key", "received_dedupe_key",
                             "expected_priority", "received_priority", "output_excerpt"}
                    or (ack["task_id"] is not None and (type(ack["task_id"]) is not int or ack["task_id"] < 1))
                    or ack["queue"] != plan["queue"] or ack["expected_dedupe_key"] != expected_key
                    or ack["expected_priority"] != proposals_by_id[pid]["priority"]
                    or (ack["received_priority"] is not None and type(ack["received_priority"]) is not int)
                    or (ack["received_queue"] is not None and not isinstance(ack["received_queue"], str))
                    or (ack["received_dedupe_key"] is not None and not isinstance(ack["received_dedupe_key"], str))
                    or not isinstance(ack["output_excerpt"], str)
                    or not isinstance(scope, dict)
                    or set(scope) != {"queue", "status_filters", "completed_status_filters", "limit_per_status"}
                    or scope["queue"] != plan["queue"] or scope["status_filters"] != sorted(LIVE_TASK_STATUSES)
                    or not isinstance(scope["completed_status_filters"], list)
                    or any(s not in LIVE_TASK_STATUSES for s in scope["completed_status_filters"])
                    or scope["limit_per_status"] != 500
                    or set(record["scans"]) != set(scope["completed_status_filters"])
                    or any(not isinstance(scan, dict)
                           or set(scan) != {"count", "ids", "limit", "truncated"}
                           or type(scan["count"]) is not int or scan["count"] < 0
                           or type(scan["limit"]) is not int or scan["limit"] != 500
                           or type(scan["truncated"]) is not bool
                           or not isinstance(scan["ids"], list)
                           or scan["count"] != len(scan["ids"])
                           or scan["truncated"] != (scan["count"] >= scan["limit"])
                           or any(type(task_id) is not int or task_id < 1 for task_id in scan["ids"])
                           for scan in record["scans"].values())):
                raise PublishError("queue_unverified evidence does not match the proposal's queue identity")
            prior_ack = state["queue_acks"].get(pid)
            if prior_ack and ack != prior_ack["ack"]:
                raise PublishError("queue_unverified ACK differs from the durable queue_ack event")
            state["queue_unverified"].append(record)
        elif event == "queue_intent":
            if set(record) != {"event", "proposal_id", "queue", "dedupe_key", "issue_number", "priority", "at"}:
                raise PublishError("malformed queue_intent event")
            if (pid not in proposals_by_id or pid in state["queue_intents"] or pid not in state["issue_verified"]
                    or not _str(record["at"])):
                raise PublishError("duplicate, unknown, or out-of-order queue_intent")
            issue_number = state["create_results"][pid]["number"]
            expected_priority = proposals_by_id[pid]["priority"]
            if (record["queue"] != plan["queue"] or record["issue_number"] != issue_number
                    or record["dedupe_key"] != f"repo-health:v1:triage:{REPO}:issue:{issue_number}"
                    or record["priority"] != expected_priority):
                raise PublishError(f"queue_intent identity mismatch for {pid}")
            state["queue_intents"][pid] = record
        elif event == "queue_ack":
            if set(record) != {"event", "proposal_id", "ack", "at"}:
                raise PublishError("malformed queue_ack event")
            if (pid not in proposals_by_id or pid in state["queue_acks"] or pid not in state["queue_intents"]
                    or not _str(record["at"])):
                raise PublishError("duplicate, unknown, or out-of-order queue_ack")
            ack = record["ack"]
            intent = state["queue_intents"][pid]
            if (not isinstance(ack, dict)
                    or set(ack) != {"task_id", "queue", "received_queue", "expected_dedupe_key", "received_dedupe_key",
                                    "expected_priority", "received_priority", "output_excerpt"}
                    or (ack["task_id"] is not None and (type(ack["task_id"]) is not int or ack["task_id"] < 1))
                    or ack["queue"] != plan["queue"] or ack["expected_dedupe_key"] != intent["dedupe_key"]
                    or ack["expected_priority"] != intent["priority"]
                    or (ack["received_priority"] is not None and type(ack["received_priority"]) is not int)
                    or (ack["received_queue"] is not None and not isinstance(ack["received_queue"], str))
                    or (ack["received_dedupe_key"] is not None and not isinstance(ack["received_dedupe_key"], str))
                    or not isinstance(ack["output_excerpt"], str)):
                raise PublishError(f"queue_ack identity mismatch for {pid}")
            state["queue_acks"][pid] = record
        else:
            raise PublishError(f"unknown receipt event: {event!r}")
    state["records"] = records
    return state


def validate_queue_packets(proposals: list[dict[str, Any]], repo_queue: str, queue: str) -> None:
    """Run the sanctioned read-only packet validator for every proposal pre-write."""
    for p in proposals:
        preview_issue = {"number": 1, "title": p["title"],
                         "url": f"https://github.com/{REPO}/issues/1",
                         "created_at": "1970-01-01T00:00:00Z"}
        packet = queue_packet(p, queue, preview_issue)
        packet_path = write_temp_json(packet)
        try:
            checked([repo_queue, "validate", str(packet_path)])
        finally:
            packet_path.unlink(missing_ok=True)


QUEUE_COLUMNS = ("task", "priority", "status", "age", "dedupe_key", "title")


def queue_table_rows(listing: str) -> list[dict[str, Any]]:
    """Parse repo-queue's fixed-width table without cross-column substring matches."""
    lines = [line for line in listing.splitlines() if line.strip()]
    if lines == ["(no tasks)"]:
        return []
    if len(lines) < 2 or tuple(re.split(r"\s{2,}", lines[0].strip())) != QUEUE_COLUMNS:
        raise PublishError("repo-queue list output did not match the documented table schema")
    if tuple(re.split(r"\s{2,}", lines[1].strip())) != tuple("-" * len(x) for x in QUEUE_COLUMNS):
        raise PublishError("repo-queue list output lacked its table separator")
    result = []
    for line in lines[2:]:
        cells = re.split(r"\s{2,}", line.strip())
        if len(cells) != len(QUEUE_COLUMNS):
            raise PublishError("repo-queue list returned a malformed task row")
        match = re.fullmatch(r"task (\d+)", cells[0])
        if not match or not cells[1].isdigit() or not cells[2] or not cells[4]:
            raise PublishError("repo-queue list returned a malformed task identity/status")
        result.append({"id": int(match.group(1)), "priority": int(cells[1]),
                       "status": cells[2], "age": cells[3],
                       "dedupe_key": cells[4], "title": cells[5]})
    return result


def verify_queue_list_filters(repo_queue: str, queue: str) -> dict[str, Any]:
    """Prove status-filtered bounded scans work; record but don't rely on --task."""
    baseline = queue_table_rows(checked([repo_queue, "list", "--queue", queue, "--limit", "2"]))
    if not baseline:
        raise PublishError("queue read-back capability is unverified on an empty queue")
    probe_queue = f"{queue}-readback-probe"
    probe_rows = queue_table_rows(checked([repo_queue, "list", "--queue", probe_queue, "--limit", "2"]))
    if probe_rows:
        raise PublishError("repo-queue --queue filter returned rows for the distinct read-back probe stream")
    scan_ids: dict[str, list[int]] = {}
    observed: set[str] = set()
    for status in sorted(LIVE_TASK_STATUSES):
        rows = queue_table_rows(checked([repo_queue, "list", "--queue", queue,
                                         "--status", status, "--limit", "500"]))
        if any(row["status"] != status for row in rows):
            raise PublishError(f"repo-queue --status {status} filter returned a row with another status")
        scan_ids[status] = [row["id"] for row in rows[:3]]
        if rows:
            observed.add(status)
    if len(observed) < 2:
        raise PublishError("bounded live-status scans did not positively demonstrate status filtering")

    sentinels = (2_147_483_647, 2_147_483_646)
    sentinel = next((n for n in sentinels if n not in {row["id"] for row in baseline}), None)
    if sentinel is None:
        raise PublishError("could not choose an unused task ID for the negative --task probe")
    task_filter_rows = queue_table_rows(checked([repo_queue, "list", "--queue", queue,
                                                  "--task", str(sentinel), "--limit", "2"]))
    # The installed broker may ignore --task. This is diagnostic only; exact
    # post-enqueue verification uses the bounded status scans below.
    task_probe = ("sentinel-returned" if any(row["id"] == sentinel for row in task_filter_rows)
                  else "nonmatching-rows-returned" if task_filter_rows else "no-rows-returned")
    return {"queue_filter_probe": "empty", "task_filter_probe": task_probe,
            "sentinel": sentinel, "status_scan_ids": scan_ids}


def find_queue_task(repo_queue: str, queue: str, task_id: int | None,
                    dedupe_key: str) -> tuple[dict[str, Any] | None, dict[str, dict[str, Any]]]:
    """One exact same-row live observation suffices; absence is not proof."""
    scanned: dict[str, dict[str, Any]] = {}
    for status in sorted(LIVE_TASK_STATUSES):
        try:
            rows = queue_table_rows(checked([repo_queue, "list", "--queue", queue,
                                             "--status", status, "--limit", "500"]))
        except PublishError as exc:
            raise QueueReadbackError(str(exc), scanned) from exc
        if any(row["status"] != status for row in rows):
            raise QueueReadbackError(f"repo-queue --status {status} filter drifted during task verification", scanned)
        scanned[status] = {"count": len(rows), "ids": [row["id"] for row in rows],
                           "limit": 500, "truncated": len(rows) >= 500}
        same_id = [row for row in rows if task_id is not None and row["id"] == task_id]
        same_key = [row for row in rows if row["dedupe_key"] == dedupe_key]
        if len(rows) >= 500 and (same_id or same_key):
            raise QueueReadbackError(
                f"task {task_id if task_id is not None else dedupe_key} appeared in a capped/truncated {status} scan; observation is indeterminate",
                scanned,
            )
        if same_id and (len(same_id) != 1 or same_id[0]["dedupe_key"] != dedupe_key
                        or same_id[0]["status"] != status):
            raise QueueReadbackError(f"task {task_id} matched, but its same-row dedupe/status did not", scanned)
        if same_key and (len(same_key) != 1 or same_key[0]["status"] != status
                         or (task_id is not None and same_key[0]["id"] != task_id)):
            raise QueueReadbackError(f"dedupe key matched a different task ID/status than expected", scanned)
        matches = same_key
        if matches:
            return matches[0], scanned
    return None, scanned


def issue_by_number(fleet: str, number: int) -> dict[str, Any]:
    issue = json_output([fleet, "gh", "issue", "view", str(number), "--repo", REPO,
                         "--json", "number,title,body,url,labels,state"])
    if not isinstance(issue, dict):
        raise PublishError(f"issue #{number} read-back was not an object")
    return issue


def record_create_result(path: Path, state: dict[str, Any], p: dict[str, Any],
                         issue: dict[str, Any], source: str) -> None:
    number = issue.get("number")
    url = issue.get("url")
    if type(number) is not int or number < 1 or not _str(url):
        raise PublishError(f"proposal {p['id']} has no authoritative issue number/URL")
    match = re.fullmatch(rf"https://github\.com/{re.escape(REPO)}/issues/(\d+)", url)
    if not match or int(match.group(1)) != number:
        raise PublishError(f"proposal {p['id']} has an issue URL/repo/number mismatch")
    expected = {"event": "create_result", "proposal_id": p["id"], "number": number,
                "url": url, "body_sha256": p["rendered_body_sha256"], "source": source}
    previous = state["create_results"].get(p["id"])
    if previous:
        if any(previous.get(k) != v for k, v in expected.items() if k != "event"):
            raise PublishError(f"receipt create_result conflicts with live proposal {p['id']}")
        return
    if source == "create_response" and p["id"] not in state["create_intents"]:
        raise PublishError(f"proposal {p['id']} create response has no durable intent")
    append_receipt(path, {**expected, "at": datetime.now(timezone.utc).isoformat()})
    state["create_results"][p["id"]] = expected


def publish_one(p: dict[str, Any], fleet: str, repo_queue: str, queue: str,
                receipt_path: Path, receipt_state: dict[str, Any],
                published: dict[str, dict[str, Any]]) -> None:
    require_epic_open(fleet)
    key = marker(p)
    blockers = [published[pid]["number"] for pid in p["depends_on"]]
    body = p["rendered_body"]
    body_hash = p["rendered_body_sha256"]
    intent = receipt_state["create_intents"].get(p["id"])
    result = receipt_state["create_results"].get(p["id"])
    existing = issue_by_marker(fleet, key)

    if existing:
        issue = existing
        if issue.get("body") != body or issue.get("title") != p["title"]:
            raise PublishError(f"proposal marker {key} exists but body/title differs; refusing reuse")
        if result and (result["number"] != issue.get("number") or result["url"] != issue.get("url")):
            raise PublishError(f"marker search conflicts with prior create_result for {p['id']}")
        if not result:
            record_create_result(receipt_path, receipt_state, p, issue, "marker_search")
    elif result:
        # The create returned a URL previously. Search may lag; fetch that
        # known URL directly and verify its marker/body before any reuse.
        issue = issue_by_number(fleet, result["number"])
        if issue.get("url") != result["url"] or issue.get("body") != body or issue.get("title") != p["title"]:
            raise PublishError(f"known create URL does not resolve to the exact proposal {p['id']}")
    elif intent:
        # A POST may have committed even if the caller saw a timeout/reset.
        # A zero search result is not evidence of absence; never POST again.
        raise PublishError(f"proposal {p['id']} has unresolved create_intent and no marker result; manual reconciliation required")
    else:
        require_epic_open(fleet)
        if len(parent_children(fleet, p["parent"])) >= 100:
            raise PublishError(f"parent #{p['parent']} has reached GitHub's 100 direct sub-issue cap; refusing alternate hierarchy")
        create_payload = {"title": p["title"], "body": body, "labels": list(dict.fromkeys(p["labels"]))}
        create_path = write_temp_json(create_payload)
        intent_record = {"event": "create_intent", "proposal_id": p["id"], "marker": key,
                         "body_sha256": body_hash, "parent": p["parent"], "title": p["title"],
                         "at": datetime.now(timezone.utc).isoformat()}
        try:
            append_receipt(receipt_path, intent_record)  # fsync happens before the single create POST.
            receipt_state["create_intents"][p["id"]] = intent_record
            # One create API operation only; hierarchy and blocker edges are
            # explicit, separately gated/read-back operations below.
            out = checked([fleet, "gh", "api", "--method", "POST",
                           f"repos/{REPO}/issues", "--input", str(create_path)])
        finally:
            create_path.unlink(missing_ok=True)
        try:
            response_issue = json.loads(out, object_pairs_hook=reject_duplicate_json_keys)
        except (json.JSONDecodeError, PublishError) as exc:
            raise PublishError(f"issue create response is ambiguous; intent remains indeterminate: {exc}") from exc
        if not isinstance(response_issue, dict):
            raise PublishError(f"issue create response is not an object; intent remains indeterminate")
        number = response_issue.get("number")
        url = response_issue.get("html_url")
        if type(number) is not int or number < 1 or not _str(url):
            raise PublishError(f"issue create response lacks number/html_url; intent remains indeterminate")
        result_match = re.fullmatch(rf"https://github\.com/{re.escape(REPO)}/issues/(\d+)", url)
        if not result_match or int(result_match.group(1)) != number:
            raise PublishError(f"issue create response has a foreign or mismatched URL; intent remains indeterminate")
        response_pointer = {"number": number, "url": url}
        record_create_result(receipt_path, receipt_state, p, response_pointer, "create_response")
        issue = issue_by_number(fleet, number)
        if issue.get("url") != url:
            raise PublishError(f"issue view URL differs from create response for proposal {p['id']}")

    if not isinstance(issue, dict):
        raise PublishError("issue read-back was not an object")
    if type(issue.get("number")) is not int or issue["number"] < 1 or not _str(issue.get("url")):
        raise PublishError("issue read-back lacked a positive number or URL")
    if str(issue.get("state", "")).upper() != "OPEN":
        raise PublishError(f"proposal marker {key} resolves to a closed issue; refusing further writes")
    if issue.get("body") != body or issue.get("title") != p["title"]:
        raise PublishError(f"issue #{issue.get('number')} failed full-body marker read-back")
    number = issue["number"]
    lifecycle_history = require_fresh_issue_history(fleet, number)
    labels = issue.get("labels")
    if not isinstance(labels, list) or any(not isinstance(label, dict) or not _str(label.get("name")) for label in labels):
        raise PublishError(f"issue #{number} label read-back was malformed")
    issue_labels = {label["name"] for label in labels}
    if {label for label in issue_labels if label.startswith("priority/")} - {p["priority_label"]}:
        raise PublishError(f"issue #{number} has a conflicting priority label; refusing to normalize it")
    lifecycle_labels = issue_labels & {"status/blocked", "status/parked", "status/ready"}
    expected_lifecycle_labels = {label for label in p["labels"] if label in {"status/blocked", "status/parked"}}
    if lifecycle_labels - expected_lifecycle_labels:
        raise PublishError(f"issue #{number} has a conflicting lifecycle label; refusing to normalize it")
    missing_labels = sorted(set(p["labels"]) - issue_labels)
    if missing_labels:
        checked([fleet, "gh", "issue", "edit", str(number), "--repo", REPO,
                 "--add-label", ",".join(missing_labels)])
        issue = issue_by_number(fleet, number)
        labels = issue.get("labels")
        if not isinstance(labels, list) or any(not isinstance(label, dict) or not _str(label.get("name")) for label in labels):
            raise PublishError(f"issue #{number} label read-back was malformed")
        issue_labels = {label["name"] for label in labels}
        if (str(issue.get("state", "")).upper() != "OPEN" or issue.get("body") != body
                or issue.get("title") != p["title"] or not set(p["labels"]).issubset(issue_labels)
                or {label for label in issue_labels if label.startswith("priority/")} != {p["priority_label"]}):
            raise PublishError(f"label/body/state read-back failed for issue #{number}")

    issue_metadata = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}")
    if not isinstance(issue_metadata, dict):
        raise PublishError(f"issue #{number} API read-back was not an object")
    issue_id = issue_metadata.get("id")
    if type(issue_id) is not int or issue_id < 1:
        raise PublishError(f"issue #{number} read-back lacks numeric database id")
    if not _str(issue_metadata.get("created_at")):
        raise PublishError(f"issue #{number} read-back lacks creation time required for stable queue dedupe")
    expected_parent_url = f"https://api.github.com/repos/{REPO}/issues/{p['parent']}"
    current_parent_url = issue_metadata.get("parent_issue_url")
    if current_parent_url not in {None, expected_parent_url}:
        raise PublishError(f"issue #{number} already has another native parent: {current_parent_url}")
    issue["created_at"] = issue_metadata["created_at"]

    # Exact native hierarchy: the child API parent pointer and parent list
    # must agree; GitHub permits a single native parent per issue.
    children = parent_children(fleet, p["parent"])
    child_listed = any(c.get("number") == number for c in children)
    if current_parent_url is None:
        if child_listed:
            raise PublishError(f"issue #{number} appears under parent #{p['parent']} but its parent pointer is absent")
        if len(children) >= 100:
            raise PublishError(f"parent #{p['parent']} has reached GitHub's 100 direct sub-issue cap; refusing alternate hierarchy")
        api_json(fleet, "POST", f"repos/{REPO}/issues/{p['parent']}/sub_issues", f"sub_issue_id={issue_id}")
        issue_metadata = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}")
        if not isinstance(issue_metadata, dict) or issue_metadata.get("parent_issue_url") != expected_parent_url:
            raise PublishError(f"native parent pointer read-back failed for issue #{number}")
    elif not child_listed:
        raise PublishError(f"issue #{number} parent pointer is set but the parent sub-issues list omits it")
    children = parent_children(fleet, p["parent"])
    if not any(c.get("number") == number for c in children):
        raise PublishError(f"native parent-list read-back failed for #{number} under #{p['parent']}")

    # Resolve every blocker ID from GitHub now. Receipt numbers/IDs never
    # supply this map; it is built only from earlier live issue reads.
    blocker_ids: dict[int, int] = {}
    for blocker in blockers:
        blocker_obj = api_json(fleet, "GET", f"repos/{REPO}/issues/{blocker}")
        blocker_map = validate_dependency_rows([blocker_obj])
        if blocker not in blocker_map:
            raise PublishError(f"blocker read-back number mismatch for #{blocker}")
        blocker_ids[blocker] = blocker_map[blocker]["id"]
    current = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}/dependencies/blocked_by")
    current_map = validate_dependency_rows(current)
    current_ids = set(current_map)
    expected_blockers = set(blockers)
    extras = current_ids - expected_blockers
    if extras:
        raise PublishError(f"issue #{number} has unexpected native blockers {sorted(extras)}; refusing to remove them")
    for blocker, blocker_id in blocker_ids.items():
        if blocker not in current_ids:
            api_json(fleet, "POST", f"repos/{REPO}/issues/{number}/dependencies/blocked_by",
                     f"issue_id={blocker_id}")
    current = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}/dependencies/blocked_by")
    current_map = validate_dependency_rows(current)
    actual_blocker_pairs = sorted((edge_number, edge["id"]) for edge_number, edge in current_map.items())
    expected_blocker_pairs = sorted((blocker, blocker_ids[blocker]) for blocker in blockers)
    if actual_blocker_pairs != expected_blocker_pairs:
        raise PublishError(f"native blocked_by read-back mismatch for #{number}: expected {expected_blocker_pairs}, got {actual_blocker_pairs}")

    issue_record = {
        "number": number, "url": issue["url"], "database_id": issue_id,
        "marker": key, "body_sha256": body_hash, "parent": p["parent"],
        "parent_issue_url": expected_parent_url,
        "blocked_by": [{"number": n, "database_id": database_id}
                       for n, database_id in actual_blocker_pairs],
        "labels": sorted(issue_labels), "lifecycle_history": lifecycle_history,
    }
    prior_issue = receipt_state["issue_verified"].get(p["id"])
    if prior_issue:
        if prior_issue["issue"] != issue_record:
            raise PublishError(f"live issue verification differs from its receipt for {p['id']}")
    else:
        append_receipt(receipt_path, {"event": "issue_verified", "proposal_id": p["id"],
                                      "issue": issue_record, "at": datetime.now(timezone.utc).isoformat()})
        receipt_state["issue_verified"][p["id"]] = {"issue": issue_record}
    published[p["id"]] = {"number": number, "database_id": issue_id, "url": issue["url"]}

    packet = queue_packet(p, queue, issue)
    packet_path = write_temp_json(packet)
    try:
        checked([repo_queue, "validate", str(packet_path)])
    finally:
        packet_path.unlink(missing_ok=True)

    queue_intent = receipt_state["queue_intents"].get(p["id"])
    queue_ack_event = receipt_state["queue_acks"].get(p["id"])
    prior_task = receipt_state["task_verified"].get(p["id"])
    dedupe_key = packet["dedupe_key"]
    if queue_intent is None:
        queue_intent = {"event": "queue_intent", "proposal_id": p["id"], "queue": queue,
                        "dedupe_key": dedupe_key, "issue_number": number, "priority": p["priority"],
                        "at": datetime.now(timezone.utc).isoformat()}
        append_receipt(receipt_path, queue_intent)  # intent is durable before enqueue.
        receipt_state["queue_intents"][p["id"]] = queue_intent
        enqueue_path = write_temp_json(packet)
        try:
            enqueue_out = checked([repo_queue, "enqueue", str(enqueue_path), "--queue", queue])
        finally:
            enqueue_path.unlink(missing_ok=True)
        ack_row = None
        for line in reversed(enqueue_out.splitlines()):
            try:
                candidate = json.loads(line, object_pairs_hook=reject_duplicate_json_keys)
            except (json.JSONDecodeError, PublishError):
                continue
            if isinstance(candidate, dict) and "id" in candidate:
                ack_row = candidate
                break
        received_task_id = ack_row.get("id") if isinstance(ack_row, dict) else None
        ack = {
            "task_id": received_task_id if type(received_task_id) is int and received_task_id > 0 else None,
            "queue": queue,
            "received_queue": ack_row.get("queue") if isinstance(ack_row, dict) and isinstance(ack_row.get("queue"), str) else None,
            "expected_dedupe_key": dedupe_key,
            "received_dedupe_key": ack_row.get("dedupe_key") if isinstance(ack_row, dict) and isinstance(ack_row.get("dedupe_key"), str) else None,
            "expected_priority": p["priority"],
            "received_priority": ack_row.get("priority") if isinstance(ack_row, dict) and type(ack_row.get("priority")) is int else None,
            "output_excerpt": enqueue_out[:500],
        }
        queue_ack_event = {"event": "queue_ack", "proposal_id": p["id"], "ack": ack,
                           "at": datetime.now(timezone.utc).isoformat()}
        append_receipt(receipt_path, queue_ack_event)
        receipt_state["queue_acks"][p["id"]] = queue_ack_event
    else:
        ack = queue_ack_event["ack"] if queue_ack_event else {
            "task_id": None, "queue": queue, "received_queue": None,
            "expected_dedupe_key": dedupe_key, "received_dedupe_key": None,
            "expected_priority": p["priority"], "received_priority": None,
            "output_excerpt": "enqueue response was not durably recorded; read-only reconciliation only",
        }

    def record_unverified(scans: dict[str, dict[str, Any]], reason: str) -> None:
        scope = {"queue": queue, "status_filters": sorted(LIVE_TASK_STATUSES),
                 "completed_status_filters": sorted(scans), "limit_per_status": 500}
        event = {"event": "queue_unverified", "proposal_id": p["id"], "ack": ack,
                 "scans": scans, "readback_scope": scope, "reason": reason,
                 "at": datetime.now(timezone.utc).isoformat()}
        append_receipt(receipt_path, event)
        receipt_state["queue_unverified"].append(event)

    if (ack["received_queue"] not in {None, queue}
            or ack["received_dedupe_key"] not in {None, dedupe_key}
            or ack["received_priority"] not in {None, p["priority"]}):
        record_unverified({}, "enqueue ACK queue/key mismatch; no retry was attempted")
        raise PublishError(f"queue enqueue ACK did not identify expected queue/key for {p['id']}")
    expected_task_id = ack["task_id"]
    if prior_task is not None:
        if expected_task_id is not None and prior_task["task"]["id"] != expected_task_id:
            raise PublishError(f"queue ACK task id conflicts with prior receipt for {p['id']}")
        expected_task_id = prior_task["task"]["id"]
    try:
        live_row, scans = find_queue_task(repo_queue, queue, expected_task_id, dedupe_key)
    except QueueReadbackError as exc:
        record_unverified(exc.scans, f"queue read-back error: {exc}; no enqueue retry was attempted")
        raise PublishError(f"queue task read-back errored; supervisor reconciliation required") from exc
    except PublishError as exc:
        record_unverified({}, f"queue read-back error: {exc}; no enqueue retry was attempted")
        raise PublishError("queue task read-back errored; supervisor reconciliation required") from exc
    readback_scope = {"queue": queue, "status_filters": sorted(LIVE_TASK_STATUSES),
                      "completed_status_filters": sorted(scans), "limit_per_status": 500}
    if live_row is None:
        record_unverified(scans, "enqueue intent/ACK was not independently observed as the exact live task row in bounded status scans; no enqueue retry was attempted")
        raise PublishError("queue task live-row read-back is unverified; supervisor reconciliation required")

    task_id = live_row["id"]
    if live_row["priority"] != p["priority"]:
        record_unverified(scans, f"live queue row priority {live_row['priority']} differs from score-derived expected priority {p['priority']}")
        raise PublishError("queue task priority does not match the reviewed score-band projection; supervisor reconciliation required")

    task_record = {"id": task_id, "status": live_row["status"], "queue": queue,
                   "priority": live_row["priority"],
                   "dedupe_key": packet["dedupe_key"], "issue_number": number}
    readback_scope["matched_row"] = live_row
    prior_task = receipt_state["task_verified"].get(p["id"])
    if prior_task:
        if any(prior_task["task"].get(k) != v for k, v in task_record.items() if k != "status"):
            raise PublishError(f"live task row differs from its receipt for {p['id']}")
    else:
        append_receipt(receipt_path, {"event": "task_verified", "proposal_id": p["id"],
                                      "task": task_record, "readback_scope": readback_scope,
                                      "scans": scans,
                                      "at": datetime.now(timezone.utc).isoformat()})
        receipt_state["task_verified"][p["id"]] = {"task": task_record,
                                                     "readback_scope": readback_scope}


def build_plan(manifest: list[dict[str, Any]], proposals: list[dict[str, Any]], mode: str) -> dict[str, Any]:
    return {
        "mode": mode, "repo": REPO, "queue": QUEUE,
        "proposal_digest": hashlib.sha256(json.dumps(
            manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest(),
        "proposals": [{
            "id": p["id"], "marker": marker(p), "title": p["title"], "parent": p["parent"],
            "kind": p["kind"], "stage": p["stage"], "source_ranges": p["source_ranges"],
            "requirement_ids": p["requirement_ids"], "depends_on": p["depends_on"],
            "lifecycle": p["lifecycle"], "urgency_score": p["urgency"],
            "urgency_band": p["urgency_band"], "priority_label": p["priority_label"],
            "queue_priority": p["priority"], "body": p["body"],
            "rendered_body": p["rendered_body"], "rendered_body_sha256": p["rendered_body_sha256"],
            "labels": p["labels"],
            "queue_dedupe_key": f"repo-health:v1:triage:{REPO}:issue:<resolved-number>",
        } for p in proposals],
    }


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    manifest_path = Path(args.manifest)
    try:
        raw = manifest_path.read_text(encoding="utf-8")
        manifest = json.loads(raw, object_pairs_hook=reject_duplicate_json_keys)
        proposals = validate_manifest(manifest)
        proposal_digest = hashlib.sha256(json.dumps(
            manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        if args.publish:
            receipt_target = validate_receipt_target(args.receipt, proposal_digest)
    except (OSError, json.JSONDecodeError, PublishError) as exc:
        print(f"publish_issues: {exc}", file=sys.stderr)
        return 2
    plan = build_plan(manifest, proposals, "publish" if args.publish else "dry-run")
    if not args.publish:
        print(json.dumps(plan, indent=2))
        return 0
    receipt_path = receipt_target
    receipt_started = False
    current_proposal: str | None = None
    try:
        receipt_state = read_receipt(receipt_path, plan, proposals)
        receipt_started = bool(receipt_state["records"])
        # Dependency issue numbers are populated only from fresh marker + API
        # verification during this run, never from local receipt contents.
        published: dict[str, dict[str, Any]] = {}
        # Up-front, read-only compatibility gate: refuse missing mapped labels before the first write.
        label_rows = json_output([FLEET, "gh", "label", "list", "--repo", REPO,
                                  "--limit", "500", "--json", "name"])
        if not isinstance(label_rows, list):
            raise PublishError("label list returned a non-list; refusing to publish")
        existing_labels = {row.get("name") for row in label_rows if isinstance(row, dict)}
        missing = sorted({label for p in proposals for label in p["labels"]} - existing_labels)
        if missing:
            raise PublishError(f"mapped labels are not present in the repository; no writes made: {missing}")
        validate_queue_packets(proposals, REPO_QUEUE, args.queue)
        queue_probe = verify_queue_list_filters(REPO_QUEUE, args.queue)
        require_epic_open(FLEET)
        # Serial writes deliberately respect both fleet gh token rotation and the queue's own pacing.
        if not receipt_state["records"]:
            append_receipt(receipt_path, {
                "event": "plan", "repo": REPO, "queue": args.queue,
                "proposal_digest": plan["proposal_digest"], "proposals": plan["proposals"],
                "started_at": datetime.now(timezone.utc).isoformat(),
            })
        receipt_started = True
        for proposal in proposals:
            current_proposal = proposal["id"]
            publish_one(proposal, FLEET, REPO_QUEUE, args.queue, receipt_path,
                        receipt_state, published)
    except (PublishError, OSError) as exc:
        if receipt_started:
            failure = {"event": "failure", "error": str(exc),
                       "at": datetime.now(timezone.utc).isoformat()}
            if current_proposal:
                failure["proposal_id"] = current_proposal
                if current_proposal in receipt_state["create_intents"] and current_proposal not in receipt_state["create_results"]:
                    failure["phase"] = "create-outcome-indeterminate"
                elif current_proposal in receipt_state["create_results"] and current_proposal not in receipt_state["task_verified"]:
                    failure["phase"] = "known-issue-incomplete-or-queue-unverified"
                else:
                    failure["phase"] = "publish"
            append_receipt(receipt_path, failure)
            print(f"publish_issues: {exc}; see receipt {receipt_path}", file=sys.stderr)
        else:
            print(f"publish_issues: {exc}; no receipt or issue write was made", file=sys.stderr)
        return 3
    print(json.dumps({"processed": len(proposals), "receipt_jsonl": str(receipt_path),
                      "task_filter_probe": queue_probe["task_filter_probe"]}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
