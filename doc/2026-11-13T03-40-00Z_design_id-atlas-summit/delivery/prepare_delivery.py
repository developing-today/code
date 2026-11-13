#!/usr/bin/env python3
"""Offline ID/Atlas issue draft validator/generator. No publication mode.

The full CLI is parsed before any filesystem access. Manifests are proposals;
this gate requires exact ownership, tests, design anchors and coverage evidence.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
MAP = ROOT / "inventory" / "statement-map.json"
FIXED_COUNTS = {"S0": 17, "S1": 120, "S2": 19, "S3": 38, "S4": 93, "S5": 64, "S6": 17, "S7": 16}
REQUIRED_COMPONENTS = {9110, 9111, *range(9113, 9149)}
TIERS = {"crucial", "recommended", "contingent", "horizon"}
KINDS = {"implementation", "research", "research+implementation", "research/benchmark", "research-and-test", "research-and-decision", "test", "test-and-compatibility", "schema-and-test", "design-and-implementation", "design-and-contract", "design-and-schema", "design", "prototype", "codec", "contract", "storage", "protocol", "query", "compatibility", "projection", "security", "comparator", "derived-family", "migration", "state", "conformance", "core", "verification"}
PLACEHOLDERS = re.compile(r"\b(TODO|TBD|to be determined|resolve later|placeholder|to confirm|selected by implementer|section to be added|verify before approval)\b|<[^>]+>", re.I)
ALIASES = {
    "id": ("id",), "parent": ("parent", "issue", "parent_issue"),
    "title": ("title",), "goal": ("goal", "problem"), "enables": ("enables",),
    "statements": ("statements", "statement_ids"), "version": ("version", "versions"),
    "tier": ("tier",), "kind": ("kind",), "dependencies": ("dependencies",),
    "requires": ("requires",), "after": ("after",),
    "hard_dependencies": ("hard_dependencies",), "soft_dependencies": ("soft_dependencies",),
    "owned": ("owned_files", "owned_surfaces"), "test_file": ("test_file", "failing_test_path", "test_first_file"),
    "test": ("test", "tests", "test_schema", "failing_test", "failing_test_oracle", "failing_test_or_oracle", "oracle", "test_oracle"),
    "oracle": ("oracle", "failing_test_oracle", "failing_test_or_oracle", "test_oracle"),
    "command": ("test_command", "done_command", "done_commands", "acceptance_commands", "acceptance_command", "test_first_command", "done"),
    "rollback": ("rollback",), "research_evidence": ("research_evidence", "evidence"),
    "design_source": ("design_source", "source"), "design_section": ("design_section", "section"),
    "trigger": ("trigger",), "owner": ("owner",), "research_scope": ("research_scope",),
}
RESEARCH_SCOPES = {
    "crdt-automerge", "crdt-diamond-types", "crdt-willow", "crdt-ocapn",
    "crdt-common-matrix", "real-encoding-bakeoff", "cloud-benchmark",
    "producer-contract-gap", "original-30-provenance-recovery",
}


def canonical(obj):
    return json.dumps(obj, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def get(item, key):
    for alias in ALIASES[key]:
        if alias in item:
            return item[alias]
    return None


def as_text(value):
    if isinstance(value, list):
        return "; ".join(as_text(v) for v in value)
    if isinstance(value, dict):
        return "; ".join(f"{k}={as_text(v)}" for k, v in sorted(value.items()))
    return str(value or "").strip()


def issue_number(value):
    if isinstance(value, int): return value
    if isinstance(value, str):
        m = re.fullmatch(r"(?:(?:https://github\.com/developing-today/code/issues/)|developing-today/code)?#?(\d+)", value.strip())
        if m: return int(m.group(1))
    return None


def expand_statements(value, sid, errors):
    values = value if isinstance(value, list) else [value] if isinstance(value, str) else []
    result = []
    for raw in values:
        if not isinstance(raw, str):
            errors.append(f"{sid}: statement mapping must be string IDs/ranges, got {raw!r}"); continue
        m = re.fullmatch(r"S([0-7])\.(\d+)(?:[–-]S?\1?\.(\d+))?", raw.strip())
        if not m:
            errors.append(f"{sid}: malformed exact statement ID/range {raw!r}"); continue
        section, first, last = int(m.group(1)), int(m.group(2)), int(m.group(3) or m.group(2))
        if first > last:
            errors.append(f"{sid}: reversed statement range {raw!r}"); continue
        result.extend(f"S{section}.{n}" for n in range(first, last + 1))
    if not result: errors.append(f"{sid}: statements must resolve to a nonempty explicit set")
    if len(result) != len(set(result)): errors.append(f"{sid}: duplicate statement IDs after range expansion")
    return sorted(set(result), key=lambda x: (int(x[1]), int(x.split(".")[1])))


def normalize_dependencies(item, sid, errors):
    """Return separate edges; never turn soft order into hard blockers."""
    hard, soft = [], []
    dep = get(item, "dependencies")
    if isinstance(dep, dict):
        if set(dep) - {"hard", "soft"}: errors.append(f"{sid}: unknown dependency classes {sorted(set(dep)-{'hard','soft'})}")
        if any(not isinstance(dep.get(k, []), list) for k in ("hard", "soft")): errors.append(f"{sid}: dependencies.hard and dependencies.soft must be lists")
        hard.extend(dep.get("hard", [])); soft.extend(dep.get("soft", []))
    elif isinstance(dep, list):
        for edge in dep:
            if isinstance(edge, dict) and set(edge) >= {"ref", "strength"} and edge["strength"] in {"hard", "soft"}:
                (hard if edge["strength"] == "hard" else soft).append(edge)
            else:
                errors.append(f"{sid}: dependency list edge must explicitly declare ref and hard/soft strength: {edge!r}")
    elif dep is not None:
        errors.append(f"{sid}: dependencies must be object {hard,soft} or explicit typed list")
    def add_many(target, value):
        if value is None: return
        target.extend(value if isinstance(value, list) else [value])
    add_many(hard, get(item, "hard_dependencies"))
    add_many(soft, get(item, "soft_dependencies"))
    add_many(hard, get(item, "requires"))
    add_many(soft, get(item, "after"))

    def clean(raw, strength):
        out = []
        for edge in raw:
            if isinstance(edge, dict) and edge.get("issue") is not None:
                number = issue_number(edge.get("issue"))
                if number is None:
                    errors.append(f"{sid}: issue dependency must have a numeric issue field: {edge!r}"); continue
                out.append({"type": "issue", "id": number, "description": as_text(edge.get("condition") or edge.get("description")), "strength": strength})
                continue
            ref = edge.get("ref", edge.get("id")) if isinstance(edge, dict) else edge
            desc = edge.get("description", edge.get("condition", "")) if isinstance(edge, dict) else ""
            if not isinstance(ref, str):
                errors.append(f"{sid}: {strength} dependency ref must be a string: {edge!r}"); continue
            ref = ref.strip()
            if ref.startswith("#") and (m := re.fullmatch(r"#(\d+)(?:\s+(.+))?", ref)):
                if desc and m.group(2) and desc != m.group(2):
                    errors.append(f"{sid}: issue dependency has conflicting descriptions: {edge!r}")
                out.append({"type": "issue", "id": int(m.group(1)), "description": desc or (m.group(2) or ""), "strength": strength})
            elif re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", ref):
                out.append({"type": "slice", "id": ref, "description": desc, "strength": strength})
            else:
                errors.append(f"{sid}: dependency ref must be exact slice ID or '#<issue> description': {edge!r}")
        return out
    return clean(hard, "hard"), clean(soft, "soft")


def normalize_owned(raw, sid, errors):
    if not isinstance(raw, list) or not raw:
        errors.append(f"{sid}: owned_files must be a nonempty list of exact path/status/owner records")
        return []
    out = []
    for row in raw:
        if isinstance(row, dict):
            path, status, owner = row.get("path"), row.get("status"), row.get("owner")
        elif isinstance(row, str):
            # Accept explicit existing/new labels in authored text, not ambiguous surfaces.
            m = re.fullmatch(r"(?:(NEW|EXISTING)\s+)?(.+?)\s+\((new|existing)(?:,\s*owner:\s*([^)]+))?\)", row.strip(), re.I)
            prefixed = re.fullmatch(r"(NEW|EXISTING)\s+(.+)", row.strip(), re.I)
            if m: path, status, owner = m.group(2), m.group(3).lower(), m.group(4)
            elif prefixed: path, status, owner = prefixed.group(2), prefixed.group(1).lower(), None
            elif (Path.cwd() / row.strip()).is_file(): path, status, owner = row.strip(), "existing", None
            else: path, status, owner = None, None, None
        else:
            path = status = owner = None
        if not isinstance(path, str) or not path or Path(path).is_absolute() or ".." in Path(path).parts:
            errors.append(f"{sid}: owned file requires safe repo-relative exact path: {row!r}"); continue
        if status not in {"new", "existing", "proposed new test"}:
            errors.append(f"{sid}: ownership status must be new or existing for {path!r}"); continue
        if status == "proposed new test": status = "new"
        actual = (Path.cwd() / path).is_file()
        if status == "existing" and not actual: errors.append(f"{sid}: existing path does not exist: {path}")
        if status == "new" and actual: errors.append(f"{sid}: path labeled new already exists: {path}")
        out.append({"path": path, "status": status, "owner": owner})
    return out


def validate(manifests: list[Path]):
    entries, errors, digest_inputs = [], [], []
    for path in manifests:
        try: raw = json.loads(path.read_text())
        except (OSError, json.JSONDecodeError) as exc:
            errors.append(f"{path}: cannot read JSON: {exc}"); continue
        try: rel = path.resolve().relative_to(ROOT.resolve()).as_posix()
        except ValueError:
            errors.append(f"manifest outside package root is unsupported: {path}"); continue
        digest_inputs.append((rel, raw))
        slices = raw.get("slices", raw.get("research_slices", [])) if isinstance(raw, dict) else []
        for item in slices:
            if not isinstance(item, dict): errors.append(f"{rel}: slice must be object"); continue
            e = {key: get(item, key) for key in ALIASES}
            e.update(source=rel, raw=item, id=get(item, "id"))
            schema_tests = item.get("tests")
            if isinstance(schema_tests, dict):
                e["test_file"] = e["test_file"] or schema_tests.get("file") or schema_tests.get("path")
                e["command"] = e["command"] or schema_tests.get("command")
                e["oracle"] = e["oracle"] or schema_tests.get("oracle")
            if isinstance(e["command"], str):
                e["command"] = re.sub(r"^FUTURE:\s*", "", e["command"].strip(), flags=re.I)
            e["design_source"] = e["design_source"] or raw.get("design") or raw.get("source")
            if not e["test_file"] and item.get("test_module"):
                module = re.sub(r"^NEW\s+", "", str(item["test_module"]).strip(), flags=re.I).split(" and ")[0]
                raw_owned = e["owned"] or []
                file_matches = []
                for x in raw_owned:
                    candidate = x.get("path") if isinstance(x, dict) else re.sub(r"^NEW\s+", "", x, flags=re.I) if isinstance(x, str) else ""
                    if candidate and Path(candidate).name == Path(module).name: file_matches.append(candidate)
                if len(file_matches) == 1:
                    e["test_file"] = re.sub(r"^NEW\s+", "", file_matches[0], flags=re.I)
            sid = e["id"]
            if not isinstance(sid, str) or not sid.strip(): errors.append(f"{rel}: slice missing stable id"); continue
            e["id"] = sid.strip()
            for field in ("title", "goal", "enables", "version", "tier", "kind", "rollback", "design_source", "design_section"):
                value = e[field]
                if not as_text(value): errors.append(f"{sid}: missing exact {field}")
                elif PLACEHOLDERS.search(as_text(value)): errors.append(f"{sid}: placeholder/ambiguous {field}: {value}")
                else: e[field] = as_text(value)
            if e["tier"] not in TIERS: errors.append(f"{sid}: tier must be one of {sorted(TIERS)}")
            if e["kind"] not in KINDS: errors.append(f"{sid}: unsupported/missing kind {e['kind']!r}")
            p = issue_number(e["parent"])
            research_parent_ok = p == 9112 and e["research_scope"] in {"original-30-provenance-recovery", "producer-contract-gap"}
            if p not in REQUIRED_COMPONENTS and not research_parent_ok: errors.append(f"{sid}: parent must be original supported component issue (or approved #9112 research scope), got {e['parent']!r}")
            e["parent"] = p
            e["statements"] = expand_statements(e["statements"], sid, errors)
            e["owned"] = normalize_owned(e["owned"], sid, errors)
            test_file = e["test_file"]
            if not isinstance(test_file, str) or not test_file.strip(): errors.append(f"{sid}: exact test_file required")
            elif Path(test_file).is_absolute() or ".." in Path(test_file).parts: errors.append(f"{sid}: test_file must be repo-relative: {test_file}")
            elif not any(f["path"] == test_file and f["status"] == "new" for f in e["owned"]): errors.append(f"{sid}: test_file must be explicitly listed as a new owned path")
            elif test_file not in as_text(e["test"]): errors.append(f"{sid}: exact proposed test_file must appear in test schema/oracle")
            if not isinstance(e["command"], str) or not e["command"].strip():
                errors.append(f"{sid}: exact future test command required")
            elif PLACEHOLDERS.search(e["command"]): errors.append(f"{sid}: ambiguous future test command")
            elif re.search(r"cargo\s+test\s+(?:--workspace|--all|--all-targets)(?:\s|$)", e["command"]): errors.append(f"{sid}: generic workspace test command is not a slice oracle")
            oracle = e["oracle"]
            if not isinstance(oracle, str) or len(oracle.split()) < 8 or PLACEHOLDERS.search(str(oracle)) or not re.search(r"\b(assert|equals|must|never|exactly|expected|matches|rejects|returns|property|fails|failure|compares|detects|reports|invariant|diverges|preserves|reaches|remains|without)\b", str(oracle), re.I):
                errors.append(f"{sid}: oracle must state a decidable expected result")
            if "research" in str(e["kind"]).lower() and (not isinstance(e["research_evidence"], list) or not e["research_evidence"]):
                errors.append(f"{sid}: research-class slice requires nonempty source/evidence references")
            design, repo_source = resolve_design_source(e["design_source"])
            if not design: errors.append(f"{sid}: design_source does not exist: {e['design_source']}")
            else: e["design_source"] = repo_source
            if design and isinstance(e["design_section"], str) and not any(line.strip().lstrip("# ").strip() == e["design_section"].strip().lstrip("# ") for line in design.read_text().splitlines() if line.lstrip().startswith("#")):
                errors.append(f"{sid}: exact design section heading not found: {e['design_section']}")
            e["hard"], e["soft"] = normalize_dependencies(item, sid, errors)
            entries.append(e)
    ids = [e["id"] for e in entries]
    for sid in sorted({x for x in ids if ids.count(x) > 1}): errors.append(f"duplicate slice id {sid}")
    idmap = {e["id"]: e for e in entries}
    parents_with_slices = {e["parent"] for e in entries}
    missing = sorted(REQUIRED_COMPONENTS - parents_with_slices)
    if missing: errors.append(f"missing implementation slices for component issues {missing}; #9112 is separately orchestration/meta")
    hard_graph = {sid: [] for sid in idmap}
    file_owners = {}
    for e in entries:
        for edge in e["hard"] + e["soft"]:
            if edge["type"] == "slice":
                if edge["id"] not in idmap: errors.append(f"{e['id']}: unknown exact dependency ID {edge['id']!r}"); continue
                if edge["strength"] == "hard":
                    hard_graph[e["id"]].append(edge["id"])
                    if e["tier"] == "crucial" and idmap[edge["id"]]["tier"] != "crucial": errors.append(f"{e['id']}: crucial depends on lower tier {edge['id']}")
            else:
                try: known_issues = set(json.loads((ROOT / "inventory-live" / "live-issues.json").read_text()).get("requested_numbers", []))
                except (OSError, json.JSONDecodeError): known_issues = set()
                if edge["id"] not in known_issues:
                    errors.append(f"{e['id']}: dependency references unsupported/untracked issue #{edge['id']}")
        for file in e["owned"]: file_owners.setdefault(file["path"], []).append((e, file))
    for path, owners in file_owners.items():
        if len(owners) > 1:
            owner_ids = {f.get("owner") for _, f in owners}
            has_hard_edge = any(b["id"] in hard_graph.get(a["id"], []) or a["id"] in hard_graph.get(b["id"], []) for a, _ in owners for b, _ in owners if a != b)
            if None in owner_ids or len(owner_ids) != len(owners) or not has_hard_edge:
                errors.append(f"cross-slice file collision {path}: require distinct named ownership plus an explicit hard edge; no auto-sequencing")
    state, visited = set(), set()
    def visit(node):
        if node in state: errors.append(f"hard dependency cycle includes {node}"); return
        if node in visited: return
        state.add(node)
        for nxt in hard_graph[node]: visit(nxt)
        state.remove(node); visited.add(node)
    for sid in hard_graph: visit(sid)

    # Verify the fixed 384-statement inventory and require direct coverage.
    try: rows = json.loads(MAP.read_text())
    except Exception as exc: rows = []; errors.append(f"cannot read statement map: {exc}")
    expected = {f"{section}.{i}" for section, count in FIXED_COUNTS.items() for i in range(1, count + 1)}
    actual = {r.get("id") for r in rows}
    if actual != expected: errors.append(f"statement inventory mismatch: expected fixed 384 IDs, missing={sorted(expected-actual)[:8]}, extra={sorted(actual-expected)[:8]}")
    covered = {s for e in entries for s in (e["statements"] if isinstance(e["statements"], list) else [])}
    exemptions_path = ROOT / "delivery" / "statement-exemptions.json"
    try: exemptions = json.loads(exemptions_path.read_text()).get("exemptions", [])
    except (OSError, json.JSONDecodeError, AttributeError): exemptions = []
    exempted = set()
    for x in exemptions:
        if not isinstance(x, dict) or not isinstance(x.get("statements"), list) or not x.get("issue") or not x.get("owner") or not x.get("reason") or x.get("kind") not in {"research", "horizon"}:
            errors.append(f"invalid statement exemption; require statements, tracked issue, owner, research/horizon kind and reason: {x!r}"); continue
        exempted.update(x["statements"])
    if (covered | exempted) != expected: errors.append(f"statement-to-slice/exemption mismatch: uncovered={sorted(expected-(covered|exempted))}, unknown={sorted((covered|exempted)-expected)}")
    if covered & exempted: errors.append(f"statements cannot be both sliced and exempted: {sorted(covered & exempted)}")
    for scope in RESEARCH_SCOPES:
        research_entries = [e for e in entries if e["research_scope"] == scope and "research" in str(e["kind"]).lower()]
        if len(research_entries) != 1: errors.append(f"require exactly one research-kind slice with research_scope={scope}")
        elif not research_entries[0]["owner"] or not research_entries[0]["trigger"]:
            errors.append(f"research scope {scope} requires explicit owner and start/decision trigger")
        elif scope == "original-30-provenance-recovery" and research_entries[0]["parent"] not in {9112, 9113}:
            errors.append("original-30-provenance-recovery must be parented to #9112 or #9113")
        elif scope == "producer-contract-gap" and research_entries[0]["parent"] not in {9112, 9113}:
            errors.append("producer-contract-gap must be parented to #9112 or #9113")
    try:
        orchestration = json.loads((ROOT / "delivery" / "orchestration.json").read_text())
        if orchestration.get("parent") != 9112 or not orchestration.get("title") or not orchestration.get("goal") or not orchestration.get("design_source") or not orchestration.get("design_section") or not orchestration.get("owner") or PLACEHOLDERS.search(as_text(orchestration)):
            errors.append("9112 requires a separate complete sequencing/orchestration issue record; do not create a slice under 9112")
        elif not (ROOT / orchestration["design_source"]).is_file(): errors.append("9112 orchestration design_source does not exist")
        elif not any(line.lstrip().startswith("#") and line.strip().lstrip("# ").strip() == orchestration["design_section"] for line in (ROOT / orchestration["design_source"]).read_text().splitlines()): errors.append("9112 orchestration design section is not an exact heading")
    except (OSError, json.JSONDecodeError, AttributeError, TypeError):
        errors.append("9112 requires a separate delivery/orchestration.json meta-issue record")
    digest = hashlib.sha256(canonical(digest_inputs)).hexdigest()
    return entries, errors, digest


def draft(e, digest):
    marker = f"<!-- id-atlas-delivery: #9106:{e['id']} -->"
    hard = "; ".join(f"{x['id']} — {x['description']}" for x in e["hard"]) or "none"
    soft = "; ".join(f"{x['id']} — {x['description']}" for x in e["soft"]) or "none"
    owned = "\n".join(f"- `{x['path']}` ({x['status']}; owner: {x['owner'] or e['owner']})" for x in e["owned"])
    design_url = f"https://github.com/developing-today/code/blob/main/{e['design_source']}#{github_anchor(e['design_section'])}"
    return f'''## Problem and goal
{e['goal']}

## Version and design basis
Version: {e['version']}. Settled design: {design_url} (section: **{e['design_section']}**).
Statement IDs: {', '.join(e['statements'])}
Enables: {e['enables']}

## Scope and ownership
Parent: #{e['parent']} (native component parent; existing component home).
{owned}
Hard dependencies (native blockers): {hard}
Soft sequencing only (not blockers): {soft}

## Test first — proposed new test
Create `{e['test_file']}` first and demonstrate it fails before implementation. Exact proposed command: `{e['command']}`
Decidable oracle: {e['oracle']}

## Done, evidence, rollback
Done when the named oracle passes, the stated version contract is maintained, and test output plus changed-file/evidence summary is attached. Research evidence required: {e['research_evidence'] or 'not applicable; implementation test evidence is required'}.
Rollback: {e['rollback']}

Manifest SHA-256 (approval binding): `{digest}`.
{marker}
'''


def check_output_path(out: Path, inputs: list[Path]):
    destination = out.expanduser().resolve()
    for source in [*inputs, MAP, ROOT / "delivery" / "statement-exemptions.json", ROOT / "delivery" / "orchestration.json"]:
        resolved = source.expanduser().resolve()
        if destination == resolved or resolved.is_relative_to(destination):
            raise ValueError(f"output path aliases or contains source input: {source}")
    return destination


def github_anchor(heading):
    slug = heading.strip().lower()
    slug = re.sub(r"[^\w -]", "", slug, flags=re.UNICODE)
    return re.sub(r"\s+", "-", slug).strip("-")


def resolve_design_source(raw_path):
    """Accept precise package-relative or repo-relative authored paths."""
    value = Path(str(raw_path))
    for absolute in (ROOT / value, Path.cwd() / value):
        if absolute.is_file():
            try: return absolute.resolve(), absolute.resolve().relative_to(Path.cwd().resolve()).as_posix()
            except ValueError: continue
    return None, None


def main(argv=None):
    parser = argparse.ArgumentParser(description=__doc__, allow_abbrev=False)
    parser.add_argument("--manifest", action="append", type=Path)
    parser.add_argument("--out", type=Path, default=ROOT / "delivery" / "drafts")
    parser.add_argument("--approve-digest")
    args = parser.parse_args(argv)  # Parse complete CLI before filesystem activity.
    paths = args.manifest or sorted(ROOT.glob("*/*-slices.json"))
    entries, errors, digest = validate(paths)
    print(f"manifest_sha256={digest}")
    parents = {e["parent"] for e in entries if e["parent"] in REQUIRED_COMPONENTS}
    statements = {s for e in entries for s in e["statements"]}
    print(f"coverage: {len(entries)} records; component slice homes {len(parents)}/{len(REQUIRED_COMPONENTS)}; statements {len(statements)}/384; #9112 meta separately")
    if args.approve_digest is not None and args.approve_digest != digest: errors.append("approval digest differs from current manifests")
    if errors:
        print("VALIDATION BLOCKED:", file=sys.stderr)
        for err in errors: print(f"- {err}", file=sys.stderr)
        return 1
    if args.approve_digest is None:
        print(f"DRY RUN: {len(entries)} slices pass. Offline draft writing requires --approve-digest {digest}.")
        return 0
    try: out = check_output_path(args.out, paths)
    except ValueError as exc:
        print(f"REFUSED: {exc}", file=sys.stderr); return 2
    out.mkdir(parents=True, exist_ok=True)
    for e in entries:
        name = re.sub(r"[^A-Za-z0-9._-]+", "-", e["id"]).strip("-")
        (out / f"{name}.md").write_text(draft(e, digest))
    print(f"Wrote {len(entries)} offline drafts to {out}; no external writes.")
    return 0


if __name__ == "__main__": raise SystemExit(main())
