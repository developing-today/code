#!/usr/bin/env python3
"""Offline issue-draft gate. No GitHub, queue, or other publication mode.

The full CLI is parsed before filesystem activity. Writes occur only when an
explicit --report path or digest-approved offline --out is supplied.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shlex
import sys
import unicodedata
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
REPO_ROOT = ROOT.parents[1]
MAP = ROOT / "inventory" / "statement-map.json"
FIXED_COUNTS = {"S0": 17, "S1": 120, "S2": 19, "S3": 38, "S4": 93, "S5": 64, "S6": 17, "S7": 16}
REQUIRED_COMPONENTS = {9110, 9111, *range(9113, 9149)}
TIERS = {"crucial", "recommended", "contingent", "horizon"}
ORACLE_KINDS = {"invariant", "model", "golden", "reference", "research-matrix"}
RESEARCH_SCOPES = {
    "crdt-automerge", "crdt-diamond-types", "crdt-willow", "crdt-ocapn",
    "crdt-common-matrix", "real-encoding-bakeoff", "cloud-benchmark",
    "producer-contract-gap", "original-30-provenance-recovery",
}
RESEARCH_SCOPE_PATTERN = re.compile(r"[a-z0-9][a-z0-9-]*\Z")
PLACEHOLDER = re.compile(r"\b(TODO|TBD|to be determined|resolve later|placeholder|to confirm|selected by implementer|section to be added|verify before approval)\b", re.I)

DEFAULT_MANIFESTS = (
    "crosscut/implementation-slices.json",
    "part-a-addressing/implementation-slices.json",
    "part-a-graph/implementation-slices.json",
    "part-b/implementation-slices.json",
    "part-c/implementation-slices.json",
    "crosscut/research-slices.json",
    "delivery/research-slices.json",
)


def canonical(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def text(value):
    if isinstance(value, list):
        return "; ".join(text(v) for v in value)
    if isinstance(value, dict):
        return "; ".join(f"{k}={text(v)}" for k, v in sorted(value.items()))
    return str(value or "").strip()


def issue_number(value):
    if type(value) is int and value > 0:
        return value
    if isinstance(value, str):
        match = re.fullmatch(r"(?:(?:https://github\.com/developing-today/code/issues/)|developing-today/code)?#?(\d+)", value.strip())
        if match and int(match.group(1)) > 0:
            return int(match.group(1))
    return None


def iso_timestamp(value):
    try:
        import datetime as dt
        parsed = dt.datetime.fromisoformat(value.replace("Z", "+00:00"))
        return parsed.tzinfo is not None
    except (AttributeError, TypeError, ValueError):
        return False


def expand_statements(value, sid, errors):
    values = value if isinstance(value, list) else [value] if isinstance(value, str) else []
    result = []
    for raw in values:
        if not isinstance(raw, str):
            errors.append(f"{sid}: statement must be a string ID/range: {raw!r}")
            continue
        match = re.fullmatch(r"S([0-7])\.(\d+)(?:[–-]S?\1?\.(\d+))?", raw.strip())
        if not match:
            errors.append(f"{sid}: malformed statement ID/range {raw!r}")
            continue
        section, first, last = int(match.group(1)), int(match.group(2)), int(match.group(3) or match.group(2))
        if first > last:
            errors.append(f"{sid}: reversed statement range {raw!r}")
            continue
        result.extend(f"S{section}.{n}" for n in range(first, last + 1))
    if len(result) != len(set(result)):
        errors.append(f"{sid}: duplicate statement IDs after range expansion")
    return sorted(set(result), key=lambda x: (int(x[1]), int(x.split(".")[1])))


def dependency_edges(item, sid, errors):
    """Exact reference identities only; requires=hard, after=soft."""
    hard, soft = [], []
    deps = item.get("dependencies")
    if deps is not None:
        if not isinstance(deps, dict) or set(deps) - {"hard", "soft"}:
            errors.append(f"{sid}: dependencies must contain only hard/soft arrays")
            deps = {}
        else:
            for label, target in (("hard", hard), ("soft", soft)):
                rows = deps.get(label, [])
                if not isinstance(rows, list):
                    errors.append(f"{sid}: dependencies.{label} must be an array")
                else:
                    target.extend(rows)
    for key, target in (("requires", hard), ("hard_dependencies", hard), ("after", soft), ("soft_dependencies", soft)):
        if key not in item:
            continue
        rows = item[key]
        if not isinstance(rows, list):
            errors.append(f"{sid}: {key} must be an array")
        else:
            target.extend(rows)

    def normalize(rows, strength):
        edges = []
        for raw in rows:
            desc = ""
            ref = raw
            if isinstance(raw, dict):
                desc = text(raw.get("description", raw.get("condition", "")))
                if raw.get("issue") is not None:
                    number = issue_number(raw["issue"])
                    if number is None:
                        errors.append(f"{sid}: issue dependency needs a positive issue number: {raw!r}")
                    else:
                        edges.append({"type": "issue", "id": number, "description": desc, "strength": strength})
                    continue
                ref = raw.get("ref", raw.get("id"))
            if not isinstance(ref, str):
                errors.append(f"{sid}: dependency requires exact ref string or issue number: {raw!r}")
                continue
            ref = ref.strip()
            issue_match = re.fullmatch(r"#(\d+)(?:\s+(.+))?", ref)
            if issue_match:
                number = int(issue_match.group(1))
                inline_desc = issue_match.group(2) or ""
                if desc and inline_desc and desc != inline_desc:
                    errors.append(f"{sid}: conflicting issue dependency descriptions: {raw!r}")
                edges.append({"type": "issue", "id": number, "description": desc or inline_desc, "strength": strength})
            elif re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", ref):
                edges.append({"type": "slice", "id": ref, "description": desc, "strength": strength})
            else:
                errors.append(f"{sid}: dependency reference must be an exact slice ID or '#N' with description: {raw!r}")
        return edges

    return normalize(hard, "hard"), normalize(soft, "soft")


def design_file(raw_path):
    if not isinstance(raw_path, str) or not raw_path or Path(raw_path).is_absolute() or ".." in Path(raw_path).parts:
        return None, None
    for candidate in (ROOT / raw_path, REPO_ROOT / raw_path):
        if candidate.is_file():
            resolved = candidate.resolve()
            return resolved, resolved.relative_to(REPO_ROOT.resolve()).as_posix()
    return None, None


ATX_HEADING = re.compile(r"^#{1,6}[ \t]+(.*?)(?:[ \t]+#+[ \t]*)?$")


def atx_headings(source_text):
    """Return exact raw ATX headings and rendered text after one prefix removal."""
    headings = []
    for line in source_text.splitlines():
        raw = line.strip()
        match = ATX_HEADING.fullmatch(raw)
        if match:
            headings.append({"raw": raw, "rendered": match.group(1).strip()})
    return headings


def owned_files(raw, sid, errors):
    if not isinstance(raw, list) or not raw:
        errors.append(f"{sid}: owned_files must be a nonempty exact-path list")
        return []
    result = []
    for row in raw:
        if isinstance(row, dict):
            path, status, owner = row.get("path"), row.get("status"), row.get("owner")
        elif isinstance(row, str):
            prefix = re.fullmatch(r"(NEW|EXISTING)\s+(.+)", row.strip(), re.I)
            suffix = re.fullmatch(r"(.+?)\s+\((new|existing)\)", row.strip(), re.I)
            if prefix:
                status, path, owner = prefix.group(1).lower(), prefix.group(2), None
            elif suffix:
                path, status, owner = suffix.group(1), suffix.group(2).lower(), None
            elif (REPO_ROOT / row.strip()).is_file():
                path, status, owner = row.strip(), "existing", None
            else:
                path, status, owner = None, None, None
        else:
            path = status = owner = None
        if not isinstance(path, str) or not path or Path(path).is_absolute() or ".." in Path(path).parts:
            errors.append(f"{sid}: owned file needs an exact safe repo-relative path: {row!r}")
            continue
        path = Path(path).as_posix()
        if not isinstance(status, str) or status not in {"new", "existing"}:
            errors.append(f"{sid}: owned file {path!r} must be labeled new or existing")
            continue
        exists = (REPO_ROOT / path).is_file()
        if status == "existing" and not exists:
            errors.append(f"{sid}: existing owned file is absent: {path}")
        if status == "new" and exists:
            errors.append(f"{sid}: file labeled new already exists: {path}")
        result.append({"path": path, "status": status, "owner": text(owner) or sid})
    return result


def check_file_collisions(path_owners, hard_reachability, errors):
    """Require pairwise transitive hard ordering for every shared path."""
    for path, owners in path_owners.items():
        if len(owners) < 2:
            continue
        if len({file["owner"] for _, file in owners}) != len(owners):
            errors.append(f"file collision {path}: each slice needs a distinct effective owner")
        for index, (left, _) in enumerate(owners):
            for right, _ in owners[index + 1:]:
                if (right["id"] not in hard_reachability.get(left["id"], set())
                    and left["id"] not in hard_reachability.get(right["id"], set())):
                    errors.append(f"file collision {path}: slices {left['id']} and {right['id']} lack pairwise transitive hard ordering")


def check_crucial_tier_reachability(entries, hard_reachability, errors):
    by_id = {row["id"]: row for row in entries}
    for row in entries:
        if row.get("tier") != "crucial":
            continue
        for dep_id in hard_reachability.get(row["id"], set()):
            if dep_id in by_id and by_id[dep_id].get("tier") != "crucial":
                errors.append(f"{row['id']}: crucial slice transitively depends on lower tier {dep_id}")


def draft_stem(sid):
    return re.sub(r"[^A-Za-z0-9._-]+", "-", sid).strip("-")


def research_command_has_artifact_validator(command, test_file, research_scope, owned=()):
    try:
        tokens = shlex.split(command)
    except ValueError:
        return False
    new_validators = {file["path"] for file in owned if file.get("status") == "new"}
    scripts = [token for token in tokens if token.endswith(".py") and (
        (REPO_ROOT / token).is_file() or token in new_validators)]
    if not scripts or test_file not in tokens:
        return False
    if research_scope == "original-30-provenance-recovery":
        return any(Path(token).name == "validate_research_report.py" for token in scripts) and any(
            token == "--input" and index + 1 < len(tokens) and tokens[index + 1] == test_file
            for index, token in enumerate(tokens)
        )
    return True


def duplicate_draft_stems(entries):
    stems, duplicates = {}, []
    for row in entries:
        stem = draft_stem(row["id"])
        if stem in stems and stems[stem] != row["id"]:
            duplicates.append((stems[stem], row["id"], stem))
        stems[stem] = row["id"]
    return duplicates


def normalize_row(raw, source, errors):
    sid = raw.get("id")
    where = f"{source} ({sid or 'missing-id'})"
    if not isinstance(sid, str) or not sid.strip():
        errors.append(f"{where}: missing stable id")
        return None
    sid = sid.strip()
    # Do not alias `enables` or `problem` into goal. The distinction is semantic.
    required = ("title", "goal", "enables", "version", "tier", "kind", "rollback", "design_section")
    row = dict(raw)
    for key in required:
        value = row.get(key)
        if not text(value):
            errors.append(f"{sid}: missing exact {key}")
        elif PLACEHOLDER.search(text(value)):
            errors.append(f"{sid}: placeholder in {key}")
    if not isinstance(row.get("tier"), str) or row.get("tier") not in TIERS:
        errors.append(f"{sid}: tier must be one of {sorted(TIERS)}")
    if not isinstance(row.get("kind"), str) or not row["kind"].strip():
        errors.append(f"{sid}: exact kind is required")
    parent = issue_number(row.get("parent", row.get("issue", row.get("parent_issue"))))
    scope = row.get("research_scope")
    if scope is not None and (not isinstance(scope, str) or not RESEARCH_SCOPE_PATTERN.fullmatch(scope)):
        errors.append(f"{sid}: research_scope must be a lowercase kebab-case string")
        scope = None
    parent_ok = parent in REQUIRED_COMPONENTS or (parent == 9112 and scope in {"original-30-provenance-recovery", "producer-contract-gap"})
    if not parent_ok:
        errors.append(f"{sid}: parent must be an existing component issue, or approved #9112 research scope")
    row["parent"] = parent
    row["statements"] = expand_statements(row.get("statements", row.get("statement_ids")), sid, errors)
    if not row["statements"] and scope != "original-30-provenance-recovery":
        errors.append(f"{sid}: non-provenance slice requires at least one statement ID")

    row["owned"] = owned_files(row.get("owned_files", row.get("owned_surfaces")), sid, errors)
    test = row.get("test")
    if not isinstance(test, dict) and isinstance(row.get("tests"), dict):
        test = row["tests"]
    test = dict(test) if isinstance(test, dict) else {}
    module = row.get("test_module")
    raw_owned = row.get("owned_files", row.get("owned_surfaces", []))
    test_path = row.get("test_first_file")
    if not test.get("file") and module and isinstance(raw_owned, list):
        mod = Path(re.sub(r"^NEW\s+", "", str(module).strip(), flags=re.I).split(" and ")[0]).name
        matches = []
        for file_record in raw_owned:
            if isinstance(file_record, dict):
                candidate = file_record.get("path", "")
            elif isinstance(file_record, str):
                candidate = re.sub(r"^NEW\s+", "", file_record.strip(), flags=re.I).split(" (", 1)[0]
            else:
                candidate = ""
            if candidate and Path(candidate).name == mod:
                matches.append(candidate)
        if len(matches) == 1:
            test_path = matches[0]
    test_file = test.get("file", test.get("path", test_path))
    command = test.get("command", row.get("test_command", row.get("test_first_command", row.get("acceptance_command", row.get("done_command", row.get("acceptance_commands"))))))
    if isinstance(command, str):
        command = re.sub(r"^FUTURE:\s*", "", command.strip(), flags=re.I)
    test_oracle = test.get("oracle", row.get("test_oracle", row.get("oracle", row.get("failing_test_or_oracle"))))
    if isinstance(test_file, str):
        test_file = Path(test_file).as_posix()
    if not isinstance(test_file, str) or not test_file:
        errors.append(f"{sid}: test.file must be an exact repo-relative file path")
    elif Path(test_file).is_absolute() or ".." in Path(test_file).parts:
        errors.append(f"{sid}: test.file must be safe repo-relative path")
    else:
        test_file = Path(test_file).as_posix()
        if not any(f["path"] == test_file and f["status"] == "new" for f in row["owned"]):
            errors.append(f"{sid}: test.file must be owned and marked new")
    if "research" in str(row.get("kind", "")).lower() and isinstance(command, str) and isinstance(test_file, str) and test_file not in command:
        errors.append(f"{sid}: research test.command must reference its future report/fixture test.file")
    if not isinstance(command, str) or not command.strip():
        errors.append(f"{sid}: test.command is required")
    elif PLACEHOLDER.search(command):
        errors.append(f"{sid}: placeholder in test.command")
    elif re.search(r"cargo\s+test\s+(?:--workspace|--all|--all-targets)(?:\s|$)", command):
        errors.append(f"{sid}: generic workspace test command is not slice-scoped")
    if isinstance(test_oracle, dict):
        if not isinstance(test_oracle.get("kind"), str) or test_oracle.get("kind") not in ORACLE_KINDS or not text(test_oracle.get("expected")) or not text(test_oracle.get("verifier")):
            errors.append(f"{sid}: test.oracle requires supported kind, expected result, and verifier")
        else:
            test_oracle = {"kind": test_oracle["kind"], "expected": text(test_oracle["expected"]), "verifier": text(test_oracle["verifier"])}
    elif isinstance(test_oracle, str) and test_oracle.strip():
        # Test-level free text follows the same explicit fresh-judge rule.
        review = row.get("fresh_judge_review")
        if (not isinstance(review, dict) or not text(review.get("reviewer")) or not iso_timestamp(review.get("reviewed_at"))
            or review.get("decision") != "accepted" or not text(review.get("reference"))):
            errors.append(f"{sid}: free-text test.oracle requires a fresh_judge_review record")
        else:
            test_oracle = {"kind": "reviewed-text", "expected": test_oracle.strip(), "verifier": text(review["reference"])}
    else:
        errors.append(f"{sid}: test.oracle must be structured or explicitly reviewed")
    row["test"] = {"file": test_file, "command": command, "oracle": test_oracle}

    design_source = row.get("design_source")
    if not design_source:
        # Legacy source is allowed only as an explicit manifest-level design path.
        design_source = row.get("_manifest_design")
    design, canonical_source = design_file(design_source)
    if not design:
        errors.append(f"{sid}: design_source must resolve to an exact existing repo file")
    else:
        row["design_source"] = canonical_source
        section = text(row.get("design_section"))
        headings = atx_headings(design.read_text())
        matching_heading = next((heading for heading in headings if section in {heading["raw"], heading["rendered"]}), None)
        if not matching_heading:
            errors.append(f"{sid}: design_section is not an exact heading in design_source")
        else:
            row["design_section"] = matching_heading["rendered"]
    hard, soft = dependency_edges(row, sid, errors)
    row["hard"], row["soft"] = hard, soft
    row["id"] = sid
    row["kind"] = text(row.get("kind"))
    row["version"] = text(row.get("version"))
    row["title"] = text(row.get("title"))
    row["goal"] = text(row.get("goal"))
    row["enables"] = text(row.get("enables"))
    row["rollback"] = text(row.get("rollback"))
    row["owner"] = text(row.get("owner")) or sid
    row["trigger"] = text(row.get("trigger"))
    row["research_scope"] = scope
    if "research" in row["kind"].lower():
        evidence = row.get("research_evidence", row.get("evidence"))
        if not isinstance(evidence, list) or not evidence or not all(text(x) for x in evidence):
            errors.append(f"{sid}: research row requires nonempty research_evidence references")
        row["research_evidence"] = evidence
        if not isinstance(test_oracle, dict) or test_oracle.get("kind") not in ORACLE_KINDS:
            errors.append(f"{sid}: research test must name a structured artifact/result oracle")
        # A research slice may create its own scoped validator, just as an
        # implementation slice creates its test. It must own that exact new
        # script and specify its oracle; this is not a claim it has run already.
        if not isinstance(command, str) or not research_command_has_artifact_validator(command, test_file, scope, row["owned"]):
            errors.append(f"{sid}: research test.command must invoke an existing or explicitly owned new artifact validator")
    if scope in {"original-30-provenance-recovery", "producer-contract-gap"} and parent not in {9112, 9113}:
        errors.append(f"{sid}: {scope} must be parented under #9112 or #9113")
    return row


def github_anchor(heading):
    """Mirror github-slugger's source algorithm for authored Markdown headings."""
    allowed = []
    for char in heading.strip().lower():
        category = unicodedata.category(char)
        if char in {" ", "-", "_"} or category[0] in {"L", "N", "M"}:
            allowed.append(char)
        # github-slugger removes punctuation/symbol/control characters, then
        # replaces each literal ASCII space independently with a hyphen.
    return "".join(allowed).replace(" ", "-")


def validate(manifest_paths):
    errors, entries, digest_items = [], [], []
    for path in manifest_paths:
        try:
            raw = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            errors.append(f"cannot read manifest {path}: {exc}")
            continue
        rel = path.resolve().relative_to(REPO_ROOT.resolve()).as_posix()
        digest_items.append((rel, raw))
        records = raw.get("slices", raw.get("research_slices", []))
        if not isinstance(records, list):
            errors.append(f"{rel}: expected slices/research_slices array")
            continue
        for record in records:
            if not isinstance(record, dict):
                errors.append(f"{rel}: each row must be an object")
                continue
            row = dict(record)
            row["_manifest_design"] = raw.get("design") or raw.get("source")
            norm = normalize_row(row, rel, errors)
            if norm:
                norm["source"] = rel
                entries.append(norm)

    idmap = {row["id"]: row for row in entries}
    if len(idmap) != len(entries):
        errors.append("duplicate slice IDs across included manifests")
    hard_graph = {sid: set() for sid in idmap}
    try:
        live = json.loads((ROOT / "inventory-live" / "live-issues.json").read_text())
        known_issues = set(live.get("requested_numbers", []))
    except (OSError, json.JSONDecodeError):
        known_issues = set()
        errors.append("cannot read known issue snapshot; external dependency validation unavailable")
    for row in entries:
        for edge in row["hard"] + row["soft"]:
            if edge["type"] == "slice":
                if edge["id"] not in idmap:
                    errors.append(f"{row['id']}: unknown exact slice dependency {edge['id']!r}")
                elif edge["strength"] == "hard":
                    hard_graph[row["id"]].add(edge["id"])
            elif edge["id"] not in known_issues:
                errors.append(f"{row['id']}: external dependency #{edge['id']} absent from checked issue snapshot")

    cycle_nodes = set()
    visiting, visited = set(), set()
    def visit(sid):
        if sid in visiting:
            cycle_nodes.add(sid)
            return
        if sid in visited:
            return
        visiting.add(sid)
        for dep in hard_graph[sid]:
            if dep in hard_graph: visit(dep)
        visiting.remove(sid)
        visited.add(sid)
    for sid in hard_graph: visit(sid)
    if cycle_nodes:
        errors.append(f"hard dependency cycle includes {sorted(cycle_nodes)}")

    # A shared file is safe only with distinct owners and every pair ordered
    # transitively in the hard DAG. One incidental edge elsewhere is insufficient.
    reach = {sid: set() for sid in hard_graph}
    def closure(sid, seen=None):
        seen = seen or set()
        for dep in hard_graph.get(sid, set()):
            if dep not in seen:
                seen.add(dep)
                closure(dep, seen)
        return seen
    for sid in hard_graph: reach[sid] = closure(sid)
    check_crucial_tier_reachability(entries, reach, errors)
    path_owners = {}
    for row in entries:
        for file in row["owned"]:
            path_owners.setdefault(file["path"], []).append((row, file))
    check_file_collisions(path_owners, reach, errors)

    for row in entries:
        if not draft_stem(row["id"]):
            errors.append(f"slice ID {row['id']!r} has no safe draft filename")
    for left, right, stem in duplicate_draft_stems(entries):
        errors.append(f"draft filename collision: {left!r} and {right!r} both sanitize to {stem!r}")

    expected = {f"{section}.{n}" for section, count in FIXED_COUNTS.items() for n in range(1, count + 1)}
    try:
        statement_rows = json.loads(MAP.read_text())
    except (OSError, json.JSONDecodeError) as exc:
        statement_rows = []
        errors.append(f"cannot load fixed 384-statement map: {exc}")
    actual = {row.get("id") for row in statement_rows if isinstance(row, dict)}
    if actual != expected:
        errors.append(f"statement map differs from fixed 384-ID oracle (missing {len(expected-actual)}, extra {len(actual-expected)})")
    covered = {statement for row in entries for statement in row["statements"]}
    exemptions_path = ROOT / "delivery" / "statement-exemptions.json"
    try:
        exemptions = json.loads(exemptions_path.read_text()).get("exemptions", [])
    except (OSError, json.JSONDecodeError, AttributeError):
        exemptions = []
    exempted = set()
    for record in exemptions:
        if (not isinstance(record, dict) or not isinstance(record.get("kind"), str) or record.get("kind") not in {"research", "horizon"}
            or not record.get("issue") or not record.get("owner") or not record.get("reason")
            or not isinstance(record.get("statements"), list)):
            errors.append(f"invalid statement exemption record: {record!r}")
            continue
        if any(not isinstance(statement, str) for statement in record["statements"]):
            errors.append("statement exemption IDs must be strings")
            continue
        exempted.update(record["statements"])
        if issue_number(record.get("issue")) not in known_issues:
            errors.append(f"statement exemption issue is absent from checked issue snapshot: {record.get('issue')!r}")
        if not set(record["statements"]).issubset(expected):
            errors.append("statement exemption contains IDs outside fixed inventory")
    if covered & exempted: errors.append(f"statements both sliced and exempted: {sorted(covered & exempted)}")
    if (covered | exempted) != expected:
        errors.append(f"statement coverage mismatch: {len(expected-(covered|exempted))} uncovered, {len((covered|exempted)-expected)} unknown")

    for scope in RESEARCH_SCOPES:
        matches = [row for row in entries if row.get("research_scope") == scope and "research" in row["kind"].lower()]
        if len(matches) != 1:
            errors.append(f"research scope {scope}: expected exactly one research row, found {len(matches)}")
        elif not text(matches[0].get("owner")) or not text(matches[0].get("trigger")):
            errors.append(f"research scope {scope}: owner and trigger are required")

    meta = {}
    meta_source = ""
    try:
        meta = json.loads((ROOT / "delivery" / "orchestration.json").read_text())
        if not isinstance(meta, dict):
            raise TypeError("orchestration record must be an object")
        design, meta_source = design_file(meta.get("design_source"))
        if meta.get("parent") != 9112 or not all(text(meta.get(k)) for k in ("title", "goal", "owner", "design_section")) or not design:
            errors.append("#9112 requires a separate complete orchestration/meta issue record")
        else:
            if any(PLACEHOLDER.search(text(meta.get(k))) for k in ("title", "goal", "owner", "design_section")):
                errors.append("#9112 orchestration record contains placeholder metadata")
            meta_headings = atx_headings(design.read_text())
            matching_meta_heading = next((heading for heading in meta_headings if meta["design_section"] in {heading["raw"], heading["rendered"]}), None)
            if not matching_meta_heading:
                errors.append("#9112 orchestration design section is not an exact heading")
            else:
                meta["design_section"] = matching_meta_heading["rendered"]
    except (OSError, json.JSONDecodeError, TypeError, AttributeError):
        errors.append("missing delivery/orchestration.json for #9112 meta issue")

    parent_homes = {row["parent"] for row in entries if row["parent"] in REQUIRED_COMPONENTS}
    missing_homes = REQUIRED_COMPONENTS - parent_homes
    if missing_homes: errors.append(f"missing slice records for component homes {sorted(missing_homes)}")
    digest = hashlib.sha256(canonical(digest_items)).hexdigest()
    orchestration = {
        "id": text(meta.get("id")) if isinstance(meta, dict) else "",
        "parent": meta.get("parent") if isinstance(meta, dict) else None,
        "title": text(meta.get("title")) if isinstance(meta, dict) else "",
        "goal": text(meta.get("goal")) if isinstance(meta, dict) else "",
        "owner": text(meta.get("owner")) if isinstance(meta, dict) else "",
        "design_source": meta_source if isinstance(meta, dict) else "",
        "design_section": text(meta.get("design_section")) if isinstance(meta, dict) else "",
        "hard": [], "soft": [],
        "marker": f"<!-- id-atlas-delivery: #9106:{text(meta.get('id'))} -->" if isinstance(meta, dict) else "",
    }
    summary = {
        "manifest_sha256": digest,
        "records": len(entries),
        "component_homes": len(parent_homes),
        "component_homes_expected": len(REQUIRED_COMPONENTS),
        "statements_covered": len(covered | exempted),
        "statements_expected": len(expected),
        "exemptions": len(exempted),
        "errors": errors,
        "valid": not errors,
        "entries": [normalized_entry(row) for row in entries],
        "orchestration": orchestration,
    }
    return entries, summary


def github_anchor(heading):
    """Mirror github-slugger: remove punctuation/control, then replace literal spaces individually."""
    kept = []
    for char in heading.strip().lower():
        category = unicodedata.category(char)
        if char in {" ", "-", "_"} or category[0] in {"L", "N", "M"}:
            kept.append(char)
    return "".join(kept).replace(" ", "-")


def draft(row, digest):
    hard = "; ".join(f"{e['id']} — {e['description']}" for e in row["hard"]) or "none"
    soft = "; ".join(f"{e['id']} — {e['description']}" for e in row["soft"]) or "none"
    owned = "\n".join(f"- `{f['path']}` ({f['status']}; owner: {f['owner']})" for f in row["owned"])
    design_url = f"https://github.com/developing-today/code/blob/main/{row['design_source']}#{github_anchor(row['design_section'])}"
    oracle = row["test"]["oracle"]
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
Create or validate `{row['test']['file']}` first. Proposed command: `{row['test']['command']}`
Oracle: **{oracle['kind']}** — expected `{oracle['expected']}`; verifier: `{oracle['verifier']}`.

## Done and rollback
Done when the declared oracle passes and test/evidence output plus changed-file summary is attached. Research evidence: {text(row.get('research_evidence')) or 'not applicable'}.
Rollback: {row['rollback']}

Manifest SHA-256 approval binding: `{digest}`.
<!-- id-atlas-delivery: #9106:{row['id']} -->
'''


def safe_output_path(out, inputs):
    target = out.expanduser().resolve()
    for source in inputs:
        source = source.expanduser().resolve()
        if target == source or source.is_relative_to(target):
            raise ValueError(f"output path aliases/contains source input {source}")
        if target.exists() and source.exists() and os.path.samefile(target, source):
            raise ValueError(f"output path is the same file as source input {source}")
    return target


def output_paths_overlap(first: Path, second: Path):
    first, second = first.expanduser().resolve(), second.expanduser().resolve()
    if first == second or first.is_relative_to(second) or second.is_relative_to(first):
        return True
    return first.exists() and second.exists() and os.path.samefile(first, second)


def normalized_entry(row):
    """Stable, publisher-facing row object in manifest order."""
    marker = f"<!-- id-atlas-delivery: #9106:{row['id']} -->"
    return {
        "id": row["id"], "source": row.get("source"), "parent": row["parent"], "title": row["title"],
        "goal": row["goal"], "enables": row["enables"], "version": row["version"],
        "kind": row["kind"], "tier": row["tier"], "statements": row["statements"],
        "owned": row["owned"], "test_file": row["test"]["file"],
        "command": row["test"]["command"], "oracle": row["test"]["oracle"],
        "rollback": row["rollback"], "research_evidence": row.get("research_evidence", []),
        "design_source": row.get("design_source"), "design_section": row.get("design_section"),
        "owner": row["owner"], "trigger": row["trigger"],
        "research_scope": row["research_scope"], "hard": row["hard"],
        "soft": row["soft"], "marker": marker,
    }


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--manifest", action="append", type=Path, help="override default set; repeatable")
    parser.add_argument("--out", type=Path, default=ROOT / "delivery" / "drafts", help="offline draft directory; used only with approved digest")
    parser.add_argument("--approve-digest", help="exact current digest; allows offline drafts only")
    parser.add_argument("--report", type=Path, help="explicit JSON validation report destination")
    args = parser.parse_args(argv)
    paths = args.manifest or [ROOT / p for p in DEFAULT_MANIFESTS]
    entries, result = validate(paths)
    if args.approve_digest is not None and args.approve_digest != result["manifest_sha256"]:
        result["errors"].append("approval digest differs from current manifest digest")
        result["valid"] = False
    summary = {k: v for k, v in result.items() if k not in {"errors", "entries", "orchestration"}}
    print(json.dumps(summary, sort_keys=True))

    inputs = [*paths, MAP, ROOT / "delivery" / "statement-exemptions.json", ROOT / "delivery" / "orchestration.json"]
    inputs.extend(REPO_ROOT / row["design_source"] for row in entries if row.get("design_source"))
    try:
        meta_source = json.loads((ROOT / "delivery" / "orchestration.json").read_text()).get("design_source")
        meta_file, _ = design_file(meta_source)
        if meta_file: inputs.append(meta_file)
    except (OSError, json.JSONDecodeError, AttributeError):
        pass
    report_path = None
    if args.report:
        try:
            report_path = safe_output_path(args.report, inputs)
        except ValueError as exc:
            print(f"REFUSED: {exc}", file=sys.stderr)
            return 2
    out = None
    if args.approve_digest is not None and not result["errors"]:
        try:
            out = safe_output_path(args.out, inputs)
        except ValueError as exc:
            print(f"REFUSED: {exc}", file=sys.stderr)
            return 2
    if report_path is not None and out is not None:
        if output_paths_overlap(report_path, out):
            print("REFUSED: --report and --out paths overlap", file=sys.stderr)
            return 2
    if report_path is not None:
        report_path.parent.mkdir(parents=True, exist_ok=True)
        report_path.write_text(json.dumps(result, indent=2, sort_keys=True) + "\n")
    if result["errors"]:
        for error in result["errors"][:12]: print(f"- {error}", file=sys.stderr)
        if len(result["errors"]) > 12: print(f"- … {len(result['errors'])-12} more; use --report PATH for the full machine-readable list", file=sys.stderr)
        return 1
    if args.approve_digest is None:
        print(f"DRY RUN: validation passed for {len(entries)} issue rows; no drafts written.")
        return 0
    assert out is not None  # computed above before any writes
    out.mkdir(parents=True, exist_ok=True)
    for row in entries:
        name = draft_stem(row["id"])
        (out / f"{name}.md").write_text(draft(row, result["manifest_sha256"]))
    print(f"Wrote {len(entries)} offline drafts to {out}; no external writes.")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
