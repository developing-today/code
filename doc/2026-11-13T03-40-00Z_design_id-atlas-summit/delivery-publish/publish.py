#!/usr/bin/env python3
"""Fail-closed, resumable publisher for approved ID/Atlas delivery drafts.

Dry-run is the default. This script does not make external calls unless both
--execute and --go 9106 are present. It intentionally uses only the Python
standard library and the installed gh/fleet CLIs plus the read-only queue API.
"""
from __future__ import annotations

import argparse
import fcntl
import hashlib
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import unicodedata
import urllib.error
import urllib.request
from pathlib import Path
from urllib.parse import urlparse

REPO = "developing-today/code"
ROOT = Path(__file__).resolve().parents[3]
PACKAGE = ROOT / "doc/2026-11-13T03-40-00Z_design_id-atlas-summit"
DEFAULT_MANIFESTS = [
    "crosscut/implementation-slices.json", "part-a-addressing/implementation-slices.json",
    "part-a-graph/implementation-slices.json", "part-b/implementation-slices.json",
    "part-c/implementation-slices.json", "crosscut/research-slices.json",
    "delivery/research-slices.json",
]
QUEUE = "repo-health-code-pilot"
LIVE = {"queued", "claimed", "parked", "blocked"}
MARKER = "<!-- id-atlas-delivery: #9106:{id} -->"


class Refusal(Exception):
    pass


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def run(argv, *, timeout=90, input_text=None):
    try:
        p = subprocess.run(argv, text=True, capture_output=True, timeout=timeout, input=input_text)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Refusal(f"command failed safely: {argv[0]}: {e}") from e
    if p.returncode:
        raise Refusal(f"command failed ({p.returncode}): {' '.join(argv)}: {(p.stderr or p.stdout).strip()[:500]}")
    return p.stdout


def run_bytes(argv, *, timeout=90):
    try:
        p = subprocess.run(argv, capture_output=True, timeout=timeout)
    except (OSError, subprocess.TimeoutExpired) as e:
        raise Refusal(f"command failed safely: {argv[0]}: {e}") from e
    if p.returncode:
        message = p.stderr.decode("utf-8", errors="replace")
        raise Refusal(f"command failed ({p.returncode}): {' '.join(argv)}: {message.strip()[:500]}")
    return p.stdout


def strict_json(path):
    try:
        return json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as e:
        raise Refusal(f"cannot read JSON {path}: {e}") from e


def inside(path, base):
    p = Path(path)
    if p.is_absolute() or ".." in p.parts:
        raise Refusal(f"unsafe repo-relative path: {path}")
    resolved = (base / p).resolve()
    if not resolved.is_relative_to(base.resolve()):
        raise Refusal(f"path escapes approved root: {path}")
    return resolved


def github_anchor(heading):
    kept = []
    for char in heading.strip().lower():
        category = unicodedata.category(char)
        if char in {" ", "-", "_"} or category[0] in {"L", "N", "M"}:
            kept.append(char)
    return "".join(kept).replace(" ", "-")


def display_text(value):
    if isinstance(value, list): return "; ".join(display_text(v) for v in value)
    if isinstance(value, dict): return "; ".join(f"{k}={display_text(v)}" for k,v in sorted(value.items()))
    return str(value or "").strip()


def expected_draft(row, digest):
    """Exact publisher-facing draft renderer from the validated row contract."""
    hard = "; ".join(f"{e['id']} — {e['description']}" for e in row["hard"]) or "none"
    soft = "; ".join(f"{e['id']} — {e['description']}" for e in row["soft"]) or "none"
    owned = "\n".join(f"- `{f['path']}` ({f['status']}; owner: {f['owner']})" for f in row["owned"])
    design_url = f"https://github.com/{REPO}/blob/main/{row['design_source']}#{github_anchor(row['design_section'])}"
    oracle = row["oracle"]
    evidence = display_text(row.get("research_evidence"))
    return f'''## Problem and goal
{row['goal']}

## Version and design basis
Version: {row['version']}. Settled design: {design_url} (section: **{row['design_section']}**).
Tier: **{row['tier']}**.{f" Start trigger: {row['trigger']}" if row.get('trigger') else ""}
Statement IDs: {', '.join(row['statements']) or 'research provenance scope; no statement ownership asserted'}
Enables: {row['enables']}

## Scope and ownership
Parent: #{row['parent']} (native component/meta parent).
{owned}
Hard dependencies (native blockers): {hard}
Soft sequencing (not blockers): {soft}

## Test / evidence first
Create or validate `{row['test_file']}` first. Proposed command: `{row['command']}`
Oracle: **{oracle['kind']}** — expected `{oracle['expected']}`; verifier: `{oracle['verifier']}`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: {evidence or 'not applicable'}.
Rollback: {row['rollback']}

Manifest SHA-256 approval binding: `{digest}`.
{row['marker']}
'''


def parse_args(argv=None):
    # No filesystem/environment/subprocess access until argparse has rejected
    # the complete argv (abbreviations are deliberately disabled).
    p = argparse.ArgumentParser(allow_abbrev=False, description=__doc__)
    p.add_argument("--manifest", action="append", help="ordered repo-relative manifest path; repeat in approved order")
    p.add_argument("--draft-dir", default=str(PACKAGE / "delivery/drafts"))
    p.add_argument("--report", default=str(PACKAGE / "delivery/report.json"))
    p.add_argument("--approved-digest", required=True)
    p.add_argument("--approved-report-digest", help="SHA-256 of exact validator report bytes")
    p.add_argument("--design-sha", required=True)
    p.add_argument("--journal", required=True, help="external durable journal path")
    p.add_argument("--execute", action="store_true", help="permit external publication")
    p.add_argument("--go", choices=["9106"], help="explicit parent GO token; required with --execute")
    p.add_argument("--gh", default="gh")
    p.add_argument("--git", default="git")
    p.add_argument("--fleet", default="fleet")
    p.add_argument("--broker-url", help="queue broker base URL (or REPO_QUEUE_BROKER_URL / configured file)")
    p.add_argument("--token-file", help="queue token file (or REPO_QUEUE_TOKEN_FILE / default config path)")
    args = p.parse_args(argv)
    if args.execute != (args.go == "9106"):
        p.error("publication requires both --execute and --go 9106; dry-run accepts neither")
    if not re.fullmatch(r"[0-9a-fA-F]{40}", args.design_sha):
        p.error("--design-sha must be a full 40-hex commit SHA")
    if not re.fullmatch(r"[0-9a-f]{64}", args.approved_digest):
        p.error("--approved-digest must be a lowercase SHA-256")
    if args.approved_report_digest is not None and not re.fullmatch(r"[0-9a-f]{64}", args.approved_report_digest):
        p.error("--approved-report-digest must be a lowercase SHA-256")
    if args.execute and args.approved_report_digest is None:
        p.error("publication requires --approved-report-digest binding the exact report bytes")
    return args


def read_inputs(args):
    manifests = args.manifest or DEFAULT_MANIFESTS
    pairs = []
    entries = []
    source_by_id = {}
    for rel in manifests:
        path = inside(rel, PACKAGE)
        obj = strict_json(path)
        if not isinstance(obj, dict):
            raise Refusal(f"manifest must be a JSON object: {rel}")
        repo_rel = (PACKAGE.relative_to(ROOT) / Path(rel)).as_posix()
        pairs.append([repo_rel, obj])
        records = obj.get("slices", obj.get("research_slices"))
        if not isinstance(records, list):
            raise Refusal(f"manifest requires a slices/research_slices array: {rel}")
        for raw in records:
            if isinstance(raw, dict) and isinstance(raw.get("id"), str):
                if raw["id"] in source_by_id: raise Refusal(f"duplicate slice ID in source manifests: {raw['id']}")
                source_by_id[raw["id"]] = repo_rel
        entries.extend(records)
    digest = hashlib.sha256(canonical(pairs)).hexdigest()
    if digest != args.approved_digest:
        raise Refusal(f"manifest digest drift: expected {args.approved_digest}, computed {digest}")
    report_bytes = Path(args.report).read_bytes()
    report_digest = hashlib.sha256(report_bytes).hexdigest()
    if args.approved_report_digest is not None and report_digest != args.approved_report_digest:
        raise Refusal(f"validator report digest drift: expected {args.approved_report_digest}, computed {report_digest}")
    try:
        report = json.loads(report_bytes)
    except (UnicodeDecodeError, json.JSONDecodeError) as exc:
        raise Refusal(f"cannot parse validator report {args.report}: {exc}") from exc
    if not isinstance(report, dict): raise Refusal("validator report must be a JSON object")
    if report.get("manifest_sha256") != digest or report.get("valid") is not True or report.get("errors"):
        raise Refusal("validator report is invalid, has errors, or is for another manifest digest")
    normalized = report.get("entries")
    if not isinstance(normalized, list) or len(normalized) != len(entries):
        raise Refusal("validator report normalized entries do not match canonical manifests")
    if any(not isinstance(e, dict) for e in normalized):
        raise Refusal("validator report entries must all be objects")
    # IDs and ordering bind the normalized approval envelope to the exact raw
    # source snapshot; publishing consumes normalized records, never aliases.
    if [e.get("id") for e in normalized] != [e.get("id") for e in entries]:
        raise Refusal("validator report entry IDs/order differ from canonical manifests")
    if any(not isinstance(e, dict) or e.get("source") != source_by_id.get(e.get("id")) for e in normalized):
        raise Refusal("normalized report source paths do not match the ordered source manifests")
    entries = normalized
    if not entries or len({e.get("id") for e in entries}) != len(entries):
        raise Refusal("empty manifests or duplicate/missing slice IDs")
    drafts = Path(args.draft_dir).resolve()
    if not drafts.is_dir():
        raise Refusal(f"approved draft directory missing: {drafts}")
    for entry in entries:
        sid = entry.get("id")
        if not isinstance(sid, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", sid):
            raise Refusal(f"invalid slice ID {sid!r}")
        draft = (drafts / f"{sid}.md").resolve()
        if not draft.is_relative_to(drafts) or not draft.is_file():
            raise Refusal(f"missing or unsafe approved draft for {sid}")
        body = draft.read_text(encoding="utf-8")
        if body != expected_draft(entry, digest):
            raise Refusal(f"draft content drift from normalized approved row/digest for {sid}")
        entry["_body"] = body
    orchestration = report.get("orchestration")
    if not isinstance(orchestration, dict):
        raise Refusal("validator report lacks normalized orchestration record")
    if orchestration.get("parent") != 9112:
        raise Refusal("orchestration record must be natively parented to #9112")
    expected_meta_marker = f"<!-- id-atlas-delivery: #9106:{orchestration.get('id','')} -->"
    if orchestration.get("marker") != expected_meta_marker or not orchestration.get("id"):
        raise Refusal("orchestration stable marker does not match its normalized ID")
    return entries, digest, report_digest, orchestration, manifests


def verify_tracked_inputs(args, sha, inputs):
    """Require every approved local input to equal the selected commit tree."""
    for repo_path, local_path, is_json in inputs:
        source = inside(repo_path, ROOT)
        try:
            local = Path(local_path).read_bytes()
            committed = run_bytes([args.git, "show", f"{sha}:{repo_path}"])
        except OSError as exc:
            raise Refusal(f"cannot read approved tracked input {repo_path}: {exc}") from exc
        if is_json:
            try:
                same = canonical(json.loads(local)) == canonical(json.loads(committed))
            except (UnicodeDecodeError, json.JSONDecodeError) as exc:
                raise Refusal(f"approved JSON input is malformed: {repo_path}: {exc}") from exc
        else:
            same = local == committed
        if not same:
            raise Refusal(f"local input differs from approved merged commit {sha}: {repo_path}")
        if not source.is_file():
            raise Refusal(f"approved input is not a local file: {repo_path}")


def approved_tree_inputs(entries, orchestration, manifests):
    inputs = []
    package_rel = PACKAGE.relative_to(ROOT)
    for rel in manifests:
        path = inside(rel, PACKAGE)
        inputs.append(((package_rel / rel).as_posix(), path, True))
    for rel, is_json in [
        ("delivery/prepare_delivery.py", False),
        ("delivery/schema.md", False),
        ("inventory/statement-map.json", True),
        ("delivery/statement-exemptions.json", True),
        ("delivery/orchestration.json", True),
        ("delivery/validate_research_report.py", False),
    ]:
        path = inside((package_rel / rel).as_posix(), ROOT)
        inputs.append(((package_rel / rel).as_posix(), path, is_json))
    design_paths = {e.get("design_source") for e in entries}
    design_paths.add(orchestration.get("design_source"))
    for rel in sorted(x for x in design_paths if x):
        path = inside(rel, ROOT)
        inputs.append((rel, path, False))
    return inputs


def issue_number(value):
    if type(value) is int and value > 0: return value
    if isinstance(value, str):
        m = re.fullmatch(r"(?:developing-today/code#|#)([1-9][0-9]*)", value)
        if m: return int(m.group(1))
    raise Refusal(f"parent must resolve to a positive developing-today/code issue number: {value!r}")


def validate_dag(entries):
    byid = {e["id"]: e for e in entries}
    graph = {}
    for e in entries:
        deps = e.get("hard", e.get("requires", []))
        if not isinstance(deps, list):
            raise Refusal(f"{e['id']}: hard requires must be a list")
        ids = []
        for d in deps:
            if isinstance(d, str):
                if d not in byid: raise Refusal(f"{e['id']}: unknown hard slice prerequisite {d}")
                ids.append(d)
            elif isinstance(d, dict) and d.get("type") == "slice" and isinstance(d.get("id"), str):
                if d["id"] not in byid: raise Refusal(f"{e['id']}: unknown hard slice prerequisite {d['id']}")
                ids.append(d["id"])
            elif isinstance(d, dict) and d.get("type") == "issue" and type(d.get("id")) is int and d["id"] > 0:
                ids.append(d["id"])
            elif isinstance(d, dict) and type(d.get("issue")) is int and d["issue"] > 0:
                ids.append(d["issue"])
            else: raise Refusal(f"{e['id']}: malformed hard dependency {d!r}")
        graph[e["id"]] = ids
    visiting, done = set(), set()
    def visit(n):
        if n in visiting: raise Refusal(f"hard dependency cycle at {n}")
        if n in done: return
        visiting.add(n)
        for d in graph[n]:
            if isinstance(d, str): visit(d)
        visiting.remove(n); done.add(n)
    for n in graph: visit(n)
    ordered, remaining = [], set(byid)
    while remaining:
        ready = sorted(n for n in remaining if all(not isinstance(d, str) or d not in remaining for d in graph[n]))
        if not ready: raise Refusal("hard dependency graph cannot be topologically ordered")
        ordered.extend(ready); remaining.difference_update(ready)
    return byid, graph, ordered


def verify_main(args):
    # Use read-only GitHub API comparison; require exact commit readback too.
    sha = args.design_sha.lower()
    commit = run([args.gh, "api", f"repos/{REPO}/commits/{sha}", "--jq", ".sha"]).strip().strip('"')
    if commit.lower() != sha: raise Refusal("GitHub commit readback did not match design SHA")
    compare = json.loads(run([args.gh, "api", f"repos/{REPO}/compare/{sha}...main", "--jq", "{status:.status,base:.base_commit.sha,head:.head_commit.sha}"]))
    if compare.get("status") not in {"ahead", "identical"} or compare.get("base", "").lower() != sha:
        raise Refusal("design SHA is not proven on remote main (compare status must be ahead/identical)")


def atomic_save(path, data):
    path.parent.mkdir(parents=True, exist_ok=True)
    fd, name = tempfile.mkstemp(prefix=path.name + ".", dir=path.parent)
    try:
        with os.fdopen(fd, "w", encoding="utf-8") as f:
            json.dump(data, f, sort_keys=True, indent=2); f.write("\n"); f.flush(); os.fsync(f.fileno())
        os.replace(name, path)
        dfd = os.open(path.parent, os.O_RDONLY)
        try: os.fsync(dfd)
        finally: os.close(dfd)
    finally:
        if os.path.exists(name): os.unlink(name)


def gh_issue(args, number, fields="url,state,title,body,parent,blockedBy"):
    raw = run([args.gh, "issue", "view", str(number), "--repo", REPO, "--json", fields])
    issue = json.loads(raw)
    blockers = issue.get("blockedBy", [])
    if isinstance(blockers, dict):
        nodes = blockers.get("nodes")
        if not isinstance(nodes, list) or blockers.get("totalCount", len(nodes)) != len(nodes):
            raise Refusal("native blocker readback is malformed or incomplete")
        issue["blockedBy"] = nodes
    elif not isinstance(blockers, list):
        raise Refusal("native blocker readback has an unsupported shape")
    return issue


def graphql_marker_search(args, marker, since):
    # Paginated newest-open-issue scan provides an independent fallback when
    # search indexing lags. Never repeat a create merely because search misses.
    q = 'query($endCursor:String){repository(owner:"developing-today",name:"code"){issues(first:100,after:$endCursor,states:[OPEN,CLOSED],orderBy:{field:CREATED_AT,direction:DESC}){nodes{number url body createdAt} pageInfo{hasNextPage endCursor}}}}'
    out = run([args.gh, "api", "graphql", "--paginate", "--slurp", "-f", f"query={q}"])
    pages = json.loads(out)
    found = []
    for page in pages if isinstance(pages, list) else [pages]:
        nodes = page.get("data", {}).get("repository", {}).get("issues", {}).get("nodes", [])
        found.extend(x for x in nodes if marker in (x.get("body") or "") and x.get("createdAt", "") >= since)
    return found


def queue_packet(entry, issue, digest, timestamp, blockers):
    # Existing supported repo-task/v1 fields only; triage-family identity is
    # the created issue number and repo, with deterministic queue idempotency.
    number = int(issue)
    url = f"https://github.com/{REPO}/issues/{number}"
    sid = entry["id"]
    blocker_text = ", ".join(f"#{n}" for n in blockers) or "none"
    trigger = entry.get("trigger") or "none"
    self_block = (f"Run `repo-queue block <your-task-id> --worker <your-worker-id> --reason-class blocked --blocker 'waiting on native issue blockers' --blocked-by-issue {','.join(f'{REPO}#{n}' for n in blockers)}`. "
                  if blockers else "No hard native blockers are listed. ")
    preflight = (f"MANDATORY TRIAGE/PREFLIGHT ONLY for approved slice {sid}. Before implementation, inspect the linked GitHub issue's native blockers: {blocker_text}. "
        f"If any blocker remains open, do not implement; self-block your own claimed task. {self_block}Leave work paused until every native blocker is closed. "
        f"Check any contingent research trigger ({trigger}); do not start contingent research before its trigger. This is not production rollout authorization.")
    payload = {"schema":"repo-task/v1", "kind":"triage", "repo":REPO, "queue":QUEUE,
      "issue":{"number":number,"url":url,"title":entry["title"],"state_checked_at":timestamp},
      "problem":preflight,
      "evidence":[url, *[f"https://github.com/{REPO}/issues/{n}" for n in blockers], f"Approved manifest SHA-256: {digest}"],
      "acceptance":[f"Record the native-blocker preflight for {url}. If any blocker remains open, self-block your claimed task and perform no implementation. If all are closed, proceed only within the issue's approved scope and stated oracle."],
      "tests":[f"Run the exact future test command recorded in slice {sid}; attach output and changed-file summary."],
      "scope":{"include":[f"ID/Atlas slice {sid}"],"exclude":["Unrelated slices or unapproved design changes"]},
      "risk":{"level":"medium","flags":["approved-design-delivery"]},
      "priority_reason":"Approved design slice; queue item records the issue delivery and acceptance contract.",
      "timebox_minutes":45,"max_attempts":6,"context_refs":[url],
      "dedupe":{"checked_at":timestamp,"basis":f"Stable ID/Atlas delivery marker and journal for #9106:{sid}"}}
    return {"title":f"Preflight ID/Atlas {sid}: {entry['title']}"[:180],"body":f"Mandatory triage/preflight only for {sid}. Check native GitHub blockers before any implementation; if any remain open, self-block your claimed task and do no implementation. No production rollout is authorized. {url}",
      "priority":30,"dedupe_key":f"repo-health:v1:triage:{REPO}:issue:{number}","payload":payload}


def enqueue(args, packet):
    fd, filename = tempfile.mkstemp(prefix="id-atlas-", suffix=".json")
    try:
        with os.fdopen(fd, "w") as f: json.dump(packet, f, ensure_ascii=False)
        return run([args.fleet, "queue", "enqueue", filename, "--queue", QUEUE], timeout=120)
    finally:
        Path(filename).unlink(missing_ok=True)


def enqueue_row(stdout):
    # Current CLI prints human status followed by one JSON object row. Treat
    # anything else as unknown outcome and leave the journal intent resumable.
    row = None
    decoder = json.JSONDecoder()
    for match in re.finditer(r"(?m)^\s*\{", stdout):
        try:
            candidate, _ = decoder.raw_decode(stdout[match.start():].lstrip())
            if isinstance(candidate, dict) and "id" in candidate: row = candidate
        except json.JSONDecodeError: continue
    if row is None: raise Refusal("enqueue output has no JSON row; outcome must be reconciled, not retried blindly")
    if not isinstance(row, dict) or not row.get("id") or row.get("queue") != QUEUE:
        raise Refusal("enqueue row lacks task ID or names the wrong queue")
    return row


def queue_readback(args, task_id, expected_dedupe):
    # Parent verified that non-rich /list task filtering works; rich mode's
    # filter is faulty. Request one exact task in non-rich mode and still
    # independently check ID, queue, key and LIVE status in the returned row.
    payload = {"queue":QUEUE,"task":int(task_id),"limit":1,"offset":0}
    request = urllib.request.Request(args._broker_url.rstrip("/") + "/list",
        data=json.dumps(payload).encode(), headers={"X-Agent-Token":args._broker_token,"Content-Type":"application/json"}, method="POST")
    try:
        with urllib.request.urlopen(request, timeout=60) as response:
            result = json.loads(response.read())
    except (urllib.error.URLError, OSError, json.JSONDecodeError) as exc:
        raise Refusal(f"queue broker task readback failed for {task_id}: {exc}") from exc
    if not isinstance(result, dict) or result.get("ok") is not True or not isinstance(result.get("rows"), list):
        raise Refusal("queue broker list returned invalid response")
    rows = result["rows"]
    if (len(rows) != 1 or not isinstance(rows[0], dict) or rows[0].get("id") != int(task_id)
            or rows[0].get("queue") != QUEUE or rows[0].get("dedupe_key") != expected_dedupe
            or rows[0].get("status") not in LIVE):
        raise Refusal(f"queue readback lacks exactly one matching LIVE task ({LIVE}) for task {task_id}")
    return rows[0]


def raise_if_unresolved_enqueue(item, sid, dedupe):
    """Refuse to replay an enqueue whose outcome was never recorded.

    The journal saves phase "queue-intent" before enqueueing and
    "queue-unverified" when the readback fails; either phase with no
    queue_id means a previous enqueue may have committed while its
    response was lost. The queue creates a fresh task when the dedupe
    predecessor is terminal, so a blind replay can duplicate
    already-completed work. Stop: the existing row must be reconciled
    by dedupe key, including terminal outcomes, and the disposition
    recorded in the journal before any retry."""
    if item.get("phase") in ("queue-intent", "queue-unverified") and not item.get("queue_id"):
        raise Refusal(
            f"unresolved enqueue intent for {sid} (dedupe {dedupe}): a previous enqueue may have "
            "committed without its response being recorded; reconcile the existing queue row by "
            "dedupe key, including terminal outcomes, and record the disposition in the journal "
            "before retrying — refusing to replay the enqueue blindly")


def configure_broker(args):
    url = args.broker_url or os.environ.get("REPO_QUEUE_BROKER_URL")
    if not url:
        try: url = (Path.home() / ".config/repo-queue/broker-url").read_text().strip()
        except OSError as exc: raise Refusal(f"queue broker URL unavailable: {exc}") from exc
    token_path = Path(args.token_file or os.environ.get("REPO_QUEUE_TOKEN_FILE") or (Path.home() / ".config/queue-broker/token")).expanduser()
    if not url.startswith(("https://", "http://")):
        raise Refusal("queue broker URL must use http:// or https://")
    try: token = token_path.read_text().strip()
    except OSError as exc: raise Refusal(f"queue broker token file unavailable: {exc}") from exc
    if not token: raise Refusal("queue broker token file is empty")
    args._broker_url, args._broker_token = url.rstrip("/"), token


def record_queue_gap(args, issue_url, sid, state, item, reason):
    """Post a stable, read-before-write note; never imply queue coverage."""
    issue_no = item["issue_number"]
    marker = f"<!-- id-atlas-delivery-uncovered: #9106:{sid} -->"
    body = (f"Queue coverage was not verified for this delivery. The publisher stopped after: {reason[:500]}\n\n"
            f"Do not treat this issue as covered until a live task in {QUEUE} is read back.\n\n{marker}")
    item["queue_failure"] = reason[:1000]
    item["queue_gap_comment_intent"] = marker
    atomic_save(Path(args.journal).expanduser().resolve(), state)
    # Read every comment page so a lost POST response cannot create duplicates.
    raw = run([args.gh,"api","--paginate",f"repos/{REPO}/issues/{issue_no}/comments?per_page=100","--jq",'.[] | {body}'])
    existing = []
    for line in raw.splitlines():
        if not line.strip(): continue
        try: existing.append(json.loads(line).get("body", ""))
        except json.JSONDecodeError: raise Refusal("cannot parse issue-comment readback; no duplicate comment attempted")
    if any(marker in text for text in existing):
        item["queue_gap_comment"] = "readback-existing"
        atomic_save(Path(args.journal).expanduser().resolve(), state)
        return
    run([args.gh,"issue","comment",issue_url,"--repo",REPO,"--body",body],timeout=120)
    item["queue_gap_comment"] = "posted"
    atomic_save(Path(args.journal).expanduser().resolve(), state)


def publish(args, entries, digest, report_digest, orchestration):
    journal = Path(args.journal).expanduser().resolve()
    if journal.is_relative_to(ROOT): raise Refusal("journal must be outside the repository")
    if not args.execute:
        byid, hard, order = validate_dag(entries)
        rows = []
        for sid in order:
            e = byid[sid]
            rows.append({"id":sid,"parent":e.get("parent"),"title":e["title"],"hard_blockers":hard[sid],"marker":MARKER.format(id=sid),"queue":QUEUE,"body_sha256":hashlib.sha256(e["_body"].encode()).hexdigest()})
        rows.append({"id":"orchestration","parent":orchestration.get("parent"),"title":orchestration.get("title"),"marker":orchestration.get("marker")})
        return rows
    journal.parent.mkdir(parents=True, exist_ok=True)
    lockpath = journal.with_suffix(journal.suffix + ".lock")
    with lockpath.open("a+") as lock:
        fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
        state = strict_json(journal) if journal.exists() else {"repo":REPO,"digest":digest,"report_digest":report_digest,"design_sha":args.design_sha.lower(),"items":{}}
        if (state.get("repo") != REPO or state.get("digest") != digest or state.get("report_digest") != report_digest
                or state.get("design_sha") != args.design_sha.lower()):
            raise Refusal("journal approval binding does not match this run")
        byid, hard, order = validate_dag(entries)
        # Orchestration is last and is not part of the slice DAG.
        work = [(sid, byid[sid]) for sid in order] + [("orchestration", orchestration)]
        outputs = []
        for sid, e in work:
            marker = MARKER.format(id=sid) if sid != "orchestration" else orchestration["marker"]
            item = state["items"].setdefault(sid, {"marker":marker,"phase":"new"})
            if item.get("marker") != marker: raise Refusal(f"journal marker drift for {sid}")
            parent = issue_number(e.get("parent"))
            if e.get("_body"):
                body = e["_body"]
            else:
                section = str(e.get("design_section", ""))
                anchor = re.sub(r"\s+", "-", re.sub(r"[^\w -]", "", section.lower())).strip("-")
                design_url = f"https://github.com/{REPO}/blob/main/{e.get('design_source','')}#{anchor}"
                body = (f"## Goal\n{e.get('goal','')}\n\n## Settled design\n{design_url} (section: **{section}**).\n\n"
                        f"Manifest SHA-256 (approval binding): `{digest}`.\n\n{marker}\n")
            deps = hard.get(sid, [])
            if not args.execute:
                outputs.append({"id":sid,"parent":parent,"title":e["title"],"hard_blockers":hard.get(sid,[]),"marker":marker,"queue":QUEUE,"body_sha256":hashlib.sha256(body.encode()).hexdigest()})
                continue
            if item.get("issue_url"):
                issue = gh_issue(args, item["issue_url"])
                if issue.get("body") != body or issue.get("url") != item["issue_url"] or (issue.get("parent") or {}).get("number") != parent:
                    raise Refusal(f"journal issue readback mismatch for {sid}; no repair/recreate attempted")
            else:
                had_intent = "intent" in item
                intent = item.setdefault("intent", {"at":time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()),"title":e["title"],"body_sha256":hashlib.sha256(body.encode()).hexdigest()})
                item["phase"] = "create-intent"; atomic_save(journal, state)
                prior = graphql_marker_search(args, marker, intent["at"]) if had_intent else []
                if len(prior) > 1: raise Refusal(f"multiple issue candidates for {sid}; stop for manual reconciliation")
                if prior:
                    if prior[0].get("body") != body: raise Refusal(f"candidate issue body differs for {sid}; stop")
                    issue = prior[0]
                else:
                    if had_intent:
                        raise Refusal(f"unresolved prior create intent for {sid}; absence from a scan does not prove an in-flight POST cannot commit; no duplicate create attempted")
                    blockers = [str(x if isinstance(x, int) else x["id"]) for x in deps if isinstance(x, int) or isinstance(x, dict) and x.get("type") == "issue"]
                    for dep in deps:
                        dep_id = dep if isinstance(dep, str) else dep.get("id") if isinstance(dep, dict) and dep.get("type") == "slice" else None
                        if dep_id is not None:
                            dep_issue = state["items"].get(dep_id, {}).get("issue_number")
                            if not dep_issue: raise Refusal(f"hard prerequisite {dep} has not been published")
                            blockers.append(str(dep_issue))
                    create = [args.gh,"issue","create","--repo",REPO,"--title",e["title"],"--body",body,"--parent",str(parent)]
                    if blockers: create += ["--blocked-by", ",".join(blockers)]
                    out = run(create, timeout=120)
                    m = re.search(r"https://github\.com/developing-today/code/issues/(\d+)", out)
                    if not m: raise Refusal("issue create output lacks a canonical issue URL; intent preserved")
                    issue = {"url":m.group(0),"number":int(m.group(1))}
                issue.setdefault("number", int(urlparse(issue["url"]).path.rsplit("/",1)[-1]))
                item.update({"issue_url":issue["url"],"issue_number":issue["number"],"phase":"issue-created"})
                atomic_save(journal, state)
                issue = gh_issue(args, issue["url"])
            if issue.get("body") != body or (issue.get("parent") or {}).get("number") != parent:
                raise Refusal(f"native issue parent/body readback mismatch for {sid}; retained issue, stopped")
            blockers_read = {b.get("number") for b in issue.get("blockedBy", [])}
            expected = set()
            for x in deps:
                if isinstance(x, int): expected.add(x)
                elif isinstance(x, dict) and x.get("type") == "issue": expected.add(x["id"])
                else:
                    xid = x if isinstance(x,str) else x["id"]
                    expected.add(state["items"][xid]["issue_number"])
            if blockers_read != expected: raise Refusal(f"native hard blocker readback mismatch for {sid}")
            item["native_blocker_issue_numbers"] = sorted(expected)
            atomic_save(journal, state)
            dedupe = f"repo-health:v1:triage:{REPO}:issue:{item['issue_number']}"
            item["queue_dedupe_key"] = dedupe
            if item.get("queue_id"):
                # Never recreate a covered task or revive a terminal row on
                # resume; a missing/non-live row requires human disposition.
                row = queue_readback(args, item["queue_id"], dedupe)
                if row["dedupe_key"] != dedupe: raise Refusal("existing queue row dedupe key mismatch")
                item["queue_status"] = row["status"]
                item["phase"] = "covered"; atomic_save(journal,state)
                outputs.append({"id":sid,"issue":item["issue_url"],"queue_id":item["queue_id"],"status":item["queue_status"]})
                continue
            raise_if_unresolved_enqueue(item, sid, dedupe)
            item["phase"] = "queue-intent"; atomic_save(journal,state)
            packet = queue_packet(e, item["issue_number"], digest, time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime()), item["native_blocker_issue_numbers"])
            try:
                queued = enqueue_row(enqueue(args, packet))
                item["queue_id"] = queued["id"]
                item["queue_status"] = queued.get("status")
                atomic_save(journal,state)
                row = queue_readback(args, item["queue_id"], dedupe)
                if row["dedupe_key"] != dedupe: raise Refusal("queue readback dedupe key mismatch")
            except Refusal as failure:
                item["phase"] = "queue-unverified"
                atomic_save(journal,state)
                try: record_queue_gap(args, item["issue_url"], sid, state, item, str(failure))
                except Refusal: pass  # journal retains the gap even if GitHub comment could not be verified
                raise
            item.update({"phase":"covered","queue_id":row.get("id"),"queue_status":row.get("status"),"queue_readback_at":time.strftime("%Y-%m-%dT%H:%M:%SZ", time.gmtime())})
            atomic_save(journal,state)
            outputs.append({"id":sid,"issue":item["issue_url"],"queue_id":item["queue_id"],"status":item["queue_status"]})
            print(json.dumps({"event":"covered","sequence":len(outputs),"id":sid,"issue":item["issue_url"],"title":e["title"],"queue_id":item["queue_id"],"status":item["queue_status"]}, ensure_ascii=False), flush=True)
        return outputs


def main(argv=None):
    args = parse_args(argv)
    try:
        if Path.cwd().resolve() != ROOT.resolve(): raise Refusal(f"run from fixed repository root {ROOT}")
        top = run([args.git,"rev-parse","--show-toplevel"]).strip()
        if Path(top).resolve() != ROOT.resolve(): raise Refusal("wrong repository checkout")
        # Inputs are validated before any remote calls. Dry-run does no network,
        # subprocess except local git-root check, lock, or writes.
        entries, digest, report_digest, orchestration, manifests = read_inputs(args)
        byid, graph, order = validate_dag(entries)
        if args.execute:
            verify_tracked_inputs(args, args.design_sha.lower(), approved_tree_inputs(entries, orchestration, manifests))
            configure_broker(args)  # fail before any GitHub issue writes if readback auth is unavailable
            verify_main(args)
        result = publish(args, entries, digest, report_digest, orchestration)
        print(json.dumps({"repo":REPO,"digest":digest,"report_digest":report_digest,"design_sha":args.design_sha.lower(),"execute":args.execute,"issues":result}, ensure_ascii=False, indent=2))
        return 0
    except Refusal as e:
        print(f"REFUSED: {e}", file=sys.stderr); return 2


if __name__ == "__main__": raise SystemExit(main())
