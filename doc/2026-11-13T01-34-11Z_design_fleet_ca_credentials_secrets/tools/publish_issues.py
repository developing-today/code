#!/usr/bin/env python3
"""Publish a reviewed Fleet CA issue manifest through fleet gh + repo-queue.

The complete argument vector and the entire manifest are validated before
subprocesses, network calls, or receipt/temp-file writes. No writes occur by
default; --publish is the explicit write gate. Never call gh directly here:
fleet gh owns the token rotation and write retry-safety contract.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


MARKER_PREFIX = "fleet-ca-v03-proposal:"
REPO = "developing-today/code"
URGENCY = {
    # Canonical dispatch mapping in pm-stack/dispatch/README.md.
    "critical": (10, "priority/critical"),
    "high": (20, "priority/high"),
    "normal": (40, "priority/normal"),
    "low": (70, "priority/low"),
}
KIND_LABEL = {
    "design": "kind/design", "research": None,
    "implementation": "kind/feature", "test": "kind/test",
    "migration": "kind/chore", "operations": "kind/chore",
    "security review": "kind/security", "documentation": "kind/docs",
}


class PublishError(Exception):
    pass


def parse_args(argv: list[str]) -> argparse.Namespace:
    parser = argparse.ArgumentParser(allow_abbrev=False)
    parser.add_argument("--manifest", required=True)
    parser.add_argument("--queue", required=True)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument("--publish", action="store_true", help="perform writes; default is --dry-run")
    mode.add_argument("--dry-run", action="store_true", help="print the plan only (default)")
    parser.add_argument("--receipt")
    parser.add_argument("--fleet", default="fleet")
    parser.add_argument("--repo-queue", default="repo-queue")
    args = parser.parse_args(argv)
    if args.publish and not args.receipt:
        parser.error("--receipt is required with --publish")
    if not args.publish and args.receipt:
        parser.error("--receipt is only valid with --publish")
    if not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._-]*", args.queue) or args.queue.endswith("-shadow"):
        parser.error("--queue must be a named non-shadow pilot queue")
    return args


def _str(value: Any) -> bool:
    return isinstance(value, str) and bool(value.strip())


def validate_manifest(doc: Any) -> list[dict[str, Any]]:
    """Validate the shared proposals array and resolve its dependency DAG."""
    if not isinstance(doc, list) or not doc:
        raise PublishError("manifest must be a non-empty JSON array of common proposal objects")
    ids: set[str] = set()
    result: list[dict[str, Any]] = []
    allowed_kinds = set(KIND_LABEL)
    for i, source in enumerate(doc):
        where = f"proposals[{i}]"
        if not isinstance(source, dict):
            raise PublishError(f"{where} must be an object")
        p = dict(source)
        pid = p.get("id")
        if not _str(pid) or not re.fullmatch(r"[a-z0-9][a-z0-9._-]{2,100}", pid):
            raise PublishError(f"{where}.id must be a stable lowercase id")
        if pid in ids:
            raise PublishError(f"duplicate proposal id: {pid}")
        ids.add(pid)
        for field in ("title", "body", "kind", "stage", "lifecycle", "urgency"):
            if not _str(p.get(field)):
                raise PublishError(f"{where}.{field} must be a non-empty string")
        if p["kind"] not in allowed_kinds:
            raise PublishError(f"{where}.kind is not in the coordinator issue-kind contract")
        if p["stage"] not in {"design", "research", "implementation", "test", "migration", "operations", "security review", "documentation"}:
            raise PublishError(f"{where}.stage is not a recognized delivery stage")
        if p["lifecycle"] not in {"Ready", "Backlog", "Blocked", "Parked"}:
            raise PublishError(f"{where}.lifecycle must be Ready|Backlog|Blocked|Parked")
        urgency = p["urgency"].lower()
        if urgency not in URGENCY:
            raise PublishError(f"{where}.urgency must be critical|high|normal|low")
        if type(p.get("parent")) is not int or p["parent"] not in {9156, 9157, 9158, 9159, 9160, 9162, 9163, 9164, 9165, 9166, 9167}:
            raise PublishError(f"{where}.parent must be one of the existing domain starters (not #9161)")
        for field in ("source_ranges", "requirement_ids", "depends_on"):
            if not isinstance(p.get(field), list) or any(not _str(x) for x in p[field]):
                raise PublishError(f"{where}.{field} must be a list of non-empty strings")
        if not p["source_ranges"] or not p["requirement_ids"]:
            raise PublishError(f"{where}: source_ranges and requirement_ids must not be empty")
        p["urgency"] = urgency
        p["priority"], p["priority_label"] = URGENCY[urgency]
        p["labels"] = [label for label in (KIND_LABEL[p["kind"]], "app/fleet", p["priority_label"]) if label]
        lifecycle_label = {"Ready": "status/ready", "Blocked": "status/blocked", "Parked": "status/parked"}.get(p["lifecycle"])
        if lifecycle_label:
            p["labels"].append(lifecycle_label)
        p["queue_kind"] = "triage"
        p["problem"] = f"Initial triage for {p['title']} ({p['stage']}; lifecycle {p['lifecycle']})."
        p["priority_reason"] = f"Urgency {urgency}; mapped to implemented queue priority {p['priority']}."
        p["evidence"] = [*p["requirement_ids"], *p["source_ranges"]]
        p["acceptance"] = [f"Triage and route the independently scoped deliverable in issue {p['id']}; preserve its stated acceptance criteria."]
        p["tests"] = [f"Use the issue's stated verification/tests and record evidence before dispositioning {p['id']}."]
        p["scope"] = {"include": [p["title"]], "exclude": ["Production changes forbidden by the #9155 safety gates"]}
        p["risk"] = {"level": "medium", "flags": ["design-triage"]}
        p["timebox_minutes"] = 20
        p["max_attempts"] = 6
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
    return ordered


def marker(proposal: dict[str, Any]) -> str:
    return f"<!-- {MARKER_PREFIX}{proposal['id']} -->"


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
            "dedupe": {"checked_at": datetime.now(timezone.utc).isoformat(),
                       "basis": f"Stable issue proposal {p['id']}; issue marker searched before creation."},
        },
    }


def run(argv: list[str], *, capture: bool = True) -> subprocess.CompletedProcess[str]:
    try:
        return subprocess.run(argv, capture_output=capture, text=True, check=False, timeout=120)
    except (OSError, subprocess.TimeoutExpired) as exc:
        raise PublishError(f"command failed to start or timed out: {argv[0]}: {exc}") from exc


def checked(argv: list[str]) -> str:
    proc = run(argv)
    if proc.returncode:
        detail = (proc.stderr or proc.stdout).strip()[:500]
        raise PublishError(f"command returned {proc.returncode}: {argv[:4]}: {detail}")
    return proc.stdout.strip()


def json_output(argv: list[str]) -> Any:
    output = checked(argv)
    try:
        return json.loads(output)
    except json.JSONDecodeError as exc:
        raise PublishError(f"expected JSON from {argv[:4]}") from exc


def issue_by_marker(fleet: str, key: str) -> dict[str, Any] | None:
    hits = json_output([fleet, "gh", "issue", "list", "--repo", REPO, "--state", "all",
                        "--search", f'"{key}" in:body', "--json", "number,title,body,url,labels"])
    if not isinstance(hits, list):
        raise PublishError("issue search returned a non-list result")
    exact = [i for i in hits if key in (i.get("body") or "")]
    if len(exact) > 1:
        raise PublishError(f"marker {key} matches multiple issues; refusing to choose")
    return exact[0] if exact else None


def api_json(fleet: str, method: str, endpoint: str, *fields: str) -> Any:
    args = [fleet, "gh", "api", "--method", method, endpoint]
    for field in fields:
        args.extend(["-F", field])
    return json_output(args)


def atomic_receipt(path: Path, value: dict[str, Any]) -> None:
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, temp_name = tempfile.mkstemp(prefix=f".{path.name}.", suffix=".tmp", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as stream:
            json.dump(value, stream, indent=2, sort_keys=True)
            stream.write("\n")
            stream.flush()
            os.fsync(stream.fileno())
        os.replace(temp_name, path)
    finally:
        if os.path.exists(temp_name):
            os.unlink(temp_name)


def validate_queue_packets(proposals: list[dict[str, Any]], repo_queue: str, queue: str) -> None:
    """Run the sanctioned read-only packet validator for every proposal pre-write."""
    for p in proposals:
        preview_issue = {"number": 1, "title": p["title"],
                         "url": f"https://github.com/{REPO}/issues/1"}
        packet = queue_packet(p, queue, preview_issue)
        with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False) as stream:
            packet_path = Path(stream.name)
            json.dump(packet, stream)
        try:
            checked([repo_queue, "validate", str(packet_path)])
        finally:
            packet_path.unlink(missing_ok=True)


def publish_one(p: dict[str, Any], fleet: str, repo_queue: str, queue: str,
                receipt: dict[str, Any], receipt_path: Path,
                published: dict[str, dict[str, Any]]) -> None:
    key = marker(p)
    blockers = [published[pid]["number"] for pid in p["depends_on"]]
    existing = issue_by_marker(fleet, key)
    if existing:
        issue = existing
    else:
        body = (p["body"].rstrip() + f"\n\nKind: {p['kind']}\nStage: {p['stage']}\n"
                f"Lifecycle: {p['lifecycle']}\nUrgency: {p['urgency']}\n"
                f"Requirement IDs: {', '.join(p['requirement_ids'])}\n"
                f"Source ranges: {', '.join(p['source_ranges'])}\n" + key)
        cmd = [fleet, "gh", "issue", "create", "--repo", REPO, "--title", p["title"],
               "--body", body, "--parent", str(p["parent"])]
        for label in dict.fromkeys(p["labels"]):
            cmd += ["--label", label]
        if blockers:
            cmd += ["--blocked-by", ",".join(str(n) for n in blockers)]
        out = checked(cmd)
        match = re.search(r"https://github\.com/([^/]+/[^/]+)/issues/(\d+)", out)
        if not match or match.group(1).lower() != REPO:
            # Do not retry an ambiguous write. The marker search on a later run recovers it.
            raise PublishError(f"create returned no recognizable {REPO} issue URL; rerun to reconcile marker {key}")
        number = int(match.group(2))
        issue = json_output([fleet, "gh", "issue", "view", str(number), "--repo", REPO,
                             "--json", "number,title,body,url,labels"])
    if key not in (issue.get("body") or ""):
        raise PublishError(f"issue #{issue.get('number')} failed marker read-back")
    number = int(issue["number"])
    issue_labels = {label.get("name") for label in issue.get("labels", []) if isinstance(label, dict)}
    missing_labels = sorted(set(p["labels"]) - issue_labels)
    if missing_labels:
        checked([fleet, "gh", "issue", "edit", str(number), "--repo", REPO,
                 "--add-label", ",".join(missing_labels)])
        issue = json_output([fleet, "gh", "issue", "view", str(number), "--repo", REPO,
                             "--json", "number,title,body,url,labels"])
        issue_labels = {label.get("name") for label in issue.get("labels", []) if isinstance(label, dict)}
        if not set(p["labels"]).issubset(issue_labels):
            raise PublishError(f"label read-back failed for issue #{number}: {missing_labels}")
    issue_id = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}").get("id")
    if type(issue_id) is not int or issue_id < 1:
        raise PublishError(f"issue #{number} read-back lacks numeric database id")

    # Native sub-issue hierarchy is a different relationship from blockedBy.
    children = api_json(fleet, "GET", f"repos/{REPO}/issues/{p['parent']}/sub_issues?per_page=100")
    if not isinstance(children, list):
        raise PublishError(f"parent #{p['parent']} sub-issues read-back was not a list")
    if not any(int(c.get("number", -1)) == number for c in children):
        api_json(fleet, "POST", f"repos/{REPO}/issues/{p['parent']}/sub_issues",
                 f"sub_issue_id={issue_id}")
    children = api_json(fleet, "GET", f"repos/{REPO}/issues/{p['parent']}/sub_issues?per_page=100")
    if not any(int(c.get("number", -1)) == number for c in children):
        raise PublishError(f"native parent link read-back failed for #{number} under #{p['parent']}")

    # Native dependency API uses the blocker's database id, not issue number.
    for blocker in blockers:
        blocker_obj = api_json(fleet, "GET", f"repos/{REPO}/issues/{blocker}")
        blocker_id = blocker_obj.get("id")
        if type(blocker_id) is not int or blocker_id < 1:
            raise PublishError(f"blocker #{blocker} has no numeric database id")
        current = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}/dependencies/blocked_by")
        if not isinstance(current, list):
            raise PublishError(f"blocked_by read-back was not a list for #{number}")
        if not any(int(x.get("id", -1)) == blocker_id for x in current):
            api_json(fleet, "POST", f"repos/{REPO}/issues/{number}/dependencies/blocked_by",
                     f"issue_id={blocker_id}")
    current = api_json(fleet, "GET", f"repos/{REPO}/issues/{number}/dependencies/blocked_by")
    current_ids = {int(x["number"]) for x in current}
    if not set(blockers).issubset(current_ids):
        raise PublishError(f"native dependency read-back failed for issue #{number}")

    receipt["published"][p["id"]] = {
        "issue": {"number": number, "url": issue["url"], "database_id": issue_id,
                  "marker": key, "parent": p["parent"], "blocked_by": blockers,
                  "proposal_dependencies": p["depends_on"], "labels": sorted(issue_labels)},
        "state": "issue-and-links-verified; queue-pending",
    }
    atomic_receipt(receipt_path, receipt)
    packet = queue_packet(p, queue, issue)
    # Validate with the sanctioned client before mutation; this is read/validate-only.
    with tempfile.NamedTemporaryFile("w", encoding="utf-8", suffix=".json", delete=False) as stream:
        packet_path = Path(stream.name)
        json.dump(packet, stream)
    try:
        checked([repo_queue, "validate", str(packet_path)])
        enqueue_out = checked([repo_queue, "enqueue", str(packet_path), "--queue", queue])
    finally:
        packet_path.unlink(missing_ok=True)
    # The enqueue command returns its canonical row; follow with a queue-list read-back.
    row = None
    for line in reversed(enqueue_out.splitlines()):
        try:
            candidate = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(candidate, dict) and "id" in candidate:
            row = candidate
            break
    if row is None or row.get("dedupe_key") != packet["dedupe_key"]:
        raise PublishError(f"queue enqueue did not return matching dedupe row for {p['id']}")
    task_id = row.get("id")
    listing = checked([repo_queue, "list", "--queue", queue, "--task", str(task_id)])
    if packet["dedupe_key"] not in listing or f"task {task_id}" not in listing:
        raise PublishError(f"queue task {task_id} read-back did not match dedupe key")

    receipt["published"][p["id"]] = {
        "issue": {"number": number, "url": issue["url"], "database_id": issue_id,
                  "marker": key, "parent": p["parent"], "blocked_by": blockers,
                  "proposal_dependencies": p["depends_on"]},
        "queue": {"id": task_id, "dedupe_key": packet["dedupe_key"], "queue": queue,
                  "readback": listing.strip()},
        "state": "verified",
    }
    published[p["id"]] = {"number": number, "database_id": issue_id, "url": issue["url"]}
    atomic_receipt(receipt_path, receipt)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(sys.argv[1:] if argv is None else argv)
    manifest_path = Path(args.manifest)
    try:
        raw = manifest_path.read_text(encoding="utf-8")
        manifest = json.loads(raw)
        proposals = validate_manifest(manifest)
    except (OSError, json.JSONDecodeError, PublishError) as exc:
        print(f"publish_issues: {exc}", file=sys.stderr)
        return 2
    plan = {"mode": "publish" if args.publish else "dry-run", "repo": REPO, "queue": args.queue,
            "proposals": [{"id": p["id"], "marker": marker(p), "title": p["title"],
                           "parent": p["parent"],
                           "depends_on": p["depends_on"], "lifecycle": p["lifecycle"],
                           "urgency": p["urgency"], "labels": p["labels"],
                           "queue_dedupe_key": f"repo-health:v1:triage:{REPO}:issue:<resolved-number>"} for p in proposals]}
    if not args.publish:
        print(json.dumps(plan, indent=2))
        return 0
    receipt_path = Path(args.receipt)
    receipt: dict[str, Any] = {**plan, "published": {}, "started_at": datetime.now(timezone.utc).isoformat()}
    try:
        if receipt_path.exists():
            try:
                prior = json.loads(receipt_path.read_text(encoding="utf-8"))
            except (OSError, json.JSONDecodeError) as exc:
                raise PublishError(f"cannot read prior receipt safely: {exc}") from exc
            if (not isinstance(prior, dict) or prior.get("repo") != REPO or
                    prior.get("queue") != args.queue or
                    prior.get("proposals") != plan["proposals"] or
                    not isinstance(prior.get("published", {}), dict)):
                raise PublishError("existing receipt does not match this repo, queue, and complete proposal plan")
            receipt = prior
            receipt.pop("failure", None)
        published: dict[str, dict[str, Any]] = {}
        for pid, item in receipt.get("published", {}).items():
            issue_ref = item.get("issue", {})
            if type(issue_ref.get("number")) is int and type(issue_ref.get("database_id")) is int:
                published[pid] = {"number": issue_ref["number"],
                                  "database_id": issue_ref["database_id"], "url": issue_ref.get("url")}
        # Up-front, read-only compatibility gate: refuse missing mapped labels before the first write.
        label_rows = json_output([args.fleet, "gh", "label", "list", "--repo", REPO,
                                  "--limit", "500", "--json", "name"])
        existing_labels = {row.get("name") for row in label_rows if isinstance(row, dict)}
        missing = sorted({label for p in proposals for label in p["labels"]} - existing_labels)
        if missing:
            raise PublishError(f"mapped labels are not present in the repository; no writes made: {missing}")
        validate_queue_packets(proposals, args.repo_queue, args.queue)
        # Serial writes deliberately respect both fleet gh token rotation and the queue's own pacing.
        atomic_receipt(receipt_path, receipt)
        for proposal in proposals:
            publish_one(proposal, args.fleet, args.repo_queue, args.queue, receipt, receipt_path, published)
    except PublishError as exc:
        receipt["failure"] = str(exc)
        atomic_receipt(receipt_path, receipt)
        print(f"publish_issues: {exc}; see receipt {receipt_path}", file=sys.stderr)
        return 3
    receipt["finished_at"] = datetime.now(timezone.utc).isoformat()
    atomic_receipt(receipt_path, receipt)
    print(json.dumps({"published": len(proposals), "receipt": str(receipt_path)}, indent=2))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
