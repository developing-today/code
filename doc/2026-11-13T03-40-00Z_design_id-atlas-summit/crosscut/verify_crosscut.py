#!/usr/bin/env python3
"""Bounded stdlib verification for the crosscut delivery manifests.

CLI contract: no arguments; reject extras before reading any manifest.
"""
import json
import re
import sys
from pathlib import Path

if len(sys.argv) != 1:
    print("usage: verify_crosscut.py", file=sys.stderr)
    raise SystemExit(2)

ROOT = Path(__file__).resolve().parent
SUMMIT = ROOT.parent
REPO = SUMMIT.parent.parent
EXPECTED_ISSUES = {9110, 9111, 9112, *range(9113, 9149)}
EXPECTED_STATEMENTS = {
    *(f"S0.{n}" for n in range(1, 18)),
    *(f"S1.{n}" for n in range(114, 121)),
    *(f"S7.{n}" for n in range(1, 17)),
}
ORACLE_KINDS = {"invariant", "model", "golden", "reference", "research-matrix"}
TIERS = {"crucial", "recommended", "contingent", "horizon"}


def fail(message):
    raise SystemExit(f"crosscut verification failed: {message}")


def statements(value):
    """Expand scalar/range statement forms to canonical S<section>.<n> IDs."""
    out = set()
    for item in value:
        text = str(item).strip().replace("–", "-").replace("—", "-")
        match = re.fullmatch(r"(S\d+)\.(\d+)\s*-\s*(?:S\d+\.)?(\d+)", text)
        if match:
            prefix, start, end = match.groups()
            out.update(f"{prefix}.{n}" for n in range(int(start), int(end) + 1))
        elif re.fullmatch(r"S\d+\.\d+", text):
            out.add(text)
        else:
            fail(f"invalid statement reference {item!r}")
    return out


def record_rows(path):
    data = json.loads(path.read_text())
    return data.get("slices", data.get("research_slices", [])), data


def path_row(file_row):
    if isinstance(file_row, dict):
        return file_row.get("path"), file_row.get("status"), file_row.get("owner")
    if isinstance(file_row, str):
        m = re.fullmatch(r"(NEW|EXISTING)\s+(.+)", file_row.strip(), re.I)
        if m:
            return m.group(2), m.group(1).lower(), None
        m = re.fullmatch(r"(.+?)\s+\((new|existing)\)", file_row.strip(), re.I)
        if m:
            return m.group(1), m.group(2).lower(), None
    return None, None, None


def dependency_refs(row, field):
    refs = []
    for dep in row.get(field, []):
        if not isinstance(dep, dict) or not isinstance(dep.get("description"), str) or not dep["description"].strip():
            fail(f"{row.get('id')}:{field} dependency must have a ref/issue and needed-output description")
        if "ref" in dep:
            ref = dep["ref"]
            if not isinstance(ref, str) or not re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", ref):
                fail(f"{row.get('id')}:{field} has non-exact ref {ref!r}")
            refs.append(("ref", ref))
        elif "issue" in dep:
            issue = dep["issue"]
            if isinstance(issue, int) and issue > 0:
                refs.append(("issue", issue))
            elif isinstance(issue, str) and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9-]*", issue):
                # Part B's registry contract uses this field for an exact crosscut slice ID.
                refs.append(("ref", issue))
            else:
                fail(f"{row.get('id')}:{field} has invalid issue {issue!r}")
        else:
            fail(f"{row.get('id')}:{field} requires exactID or issue reference")
    return refs


def load_external_slices():
    paths = [
        SUMMIT / "part-a-addressing" / "implementation-slices.json",
        SUMMIT / "part-a-graph" / "implementation-slices.json",
        SUMMIT / "part-b" / "implementation-slices.json",
    ]
    rows = []
    for path in paths:
        if path.exists():
            section, _ = record_rows(path)
            rows.extend(section)
    return rows


# Program issue schedule remains a machine-checkable, separate issue-level DAG.
sequence = json.loads((ROOT / "sequencing.json").read_text())
components = sequence["components"]
issue_map = {row["id"]: row for row in components}
if set(issue_map) != EXPECTED_ISSUES or len(issue_map) != len(components):
    fail("sequencing must cover each of the 39 IDs exactly once")
for row in components:
    for stage in ("slot", "research_slot", "design_slot"):
        if row.get(stage) not in {"now", "next", "later"}:
            fail(f"issue #{row['id']} has invalid {stage}")
    if row.get("implementation_slot") not in {"now", "next", "later", "ongoing"}:
        fail(f"issue #{row['id']} has invalid implementation_slot")
    if row["slot"] == "later" and (not row.get("reason") or row.get("review_date") != sequence["review_date"]):
        fail(f"later issue #{row['id']} needs its own deferral reason and review date")
    for dep in row["hard_dependencies"]:
        if dep not in issue_map or issue_map[dep]["wave"] >= row["wave"]:
            fail(f"issue hard edge #{dep} -> #{row['id']} is not topologically ordered")
indegree = {ident: 0 for ident in issue_map}
children = {ident: [] for ident in issue_map}
for row in components:
    for dep in row["hard_dependencies"]:
        indegree[row["id"]] += 1
        children[dep].append(row["id"])
ready = [ident for ident, n in indegree.items() if n == 0]
seen = 0
while ready:
    ident = ready.pop()
    seen += 1
    for child in children[ident]:
        indegree[child] -= 1
        if indegree[child] == 0:
            ready.append(child)
if seen != len(issue_map):
    fail("issue-level hard prerequisite graph contains a cycle")

impl_rows, impl_doc = record_rows(ROOT / "implementation-slices.json")
research_rows, research_doc = record_rows(ROOT / "research-slices.json")
if len(impl_rows) != 17 or len(research_rows) != 2:
    fail(f"expected 17 implementation + 2 research records; found {len(impl_rows)} + {len(research_rows)}")
if sum(row.get("research_scope") == "producer-contract-gap" for row in research_rows) != 1:
    fail("exactly one producer-contract-gap research scope is required")
if next(row for row in research_rows if row.get("research_scope") == "producer-contract-gap").get("parent") != 9113:
    fail("producer-contract-gap research must be parented under #9113")

all_rows = impl_rows + research_rows
row_by_id = {row["id"]: row for row in all_rows}
if len(row_by_id) != len(all_rows):
    fail("duplicate crosscut slice IDs")
external_rows = load_external_slices()
external_by_id = {row.get("id"): row for row in external_rows if row.get("id")}
all_known_ids = set(row_by_id) | set(external_by_id)
statement_coverage = set()
owned_by_path = {}
hard_edges = []

for row in all_rows:
    sid = row.get("id")
    required_fields = (
        "parent", "title", "kind", "version", "tier", "statements", "goal", "enables",
        "owned_files", "requires", "after", "test", "rollback", "design_source",
        "design_section", "owner",
    )
    for key in required_fields:
        if key not in row or row[key] is None or row[key] == "" or (key not in {"requires", "after"} and row[key] == []):
            fail(f"{sid}: missing required delivery field {key}")
    if row["tier"] not in TIERS:
        fail(f"{sid}: invalid tier {row['tier']}")
    if row["owner"] != sid:
        fail(f"{sid}: owner must identify the single owning slice")
    if row["kind"] == "research" and (not row.get("research_evidence") or not isinstance(row["research_evidence"], list)):
        fail(f"{sid}: research slice needs evidence references")
    if row["kind"] == "research":
        if row.get("research_scope") not in {None, "producer-contract-gap"}:
            fail(f"{sid}: unsupported research_scope")
        for evidence in row["research_evidence"]:
            if not evidence.startswith("http") and not (REPO / evidence).is_file():
                fail(f"{sid}: research evidence path does not exist: {evidence}")
    if row["tier"] == "contingent" and not row.get("trigger"):
        fail(f"{sid}: contingent slice needs a separate trigger")
    if row.get("parent") not in EXPECTED_ISSUES:
        fail(f"{sid}: parent is not an existing owning component")
    if row.get("parent") == 9112:
        fail(f"{sid}: #9112 must not become a parent for slice delivery")
    statement_coverage |= statements(row["statements"])
    design_path = REPO / row["design_source"]
    if not design_path.is_file():
        fail(f"{sid}: design_source is not a repo-relative existing file")
    headings = {line.strip().lstrip("# ").strip() for line in design_path.read_text().splitlines() if line.lstrip().startswith("#")}
    if row["design_section"] not in headings:
        fail(f"{sid}: design_section is not an exact heading in design_source")

    owned = row["owned_files"]
    if not isinstance(owned, list) or not owned:
        fail(f"{sid}: owned_files must be a nonempty list")
    seen_paths = set()
    for file in owned:
        path, status, owner = path_row(file)
        if not path or Path(path).is_absolute() or ".." in Path(path).parts:
            fail(f"{sid}: owned file needs exact safe repo-relative path")
        if path in seen_paths:
            fail(f"{sid}: duplicate owned file {path}")
        seen_paths.add(path)
        if status not in {"new", "existing"}:
            fail(f"{sid}: {path} must be marked new/existing")
        if owner != sid:
            fail(f"{sid}: {path} owner must be explicit and equal to its owning slice ID")
        exists = (REPO / path).is_file()
        if status == "new" and exists:
            fail(f"{sid}: future file unexpectedly exists: {path}")
        if status == "existing" and not exists:
            fail(f"{sid}: existing file is absent: {path}")
        owned_by_path.setdefault(path, []).append(sid)

    test = row["test"]
    if not isinstance(test, dict) or not all(key in test for key in ("file", "command", "oracle")):
        fail(f"{sid}: test must name file, command, and structured oracle")
    test_file = test["file"]
    file_record = next((x for x in owned if path_row(x)[0] == test_file), None)
    if not file_record or path_row(file_record)[1] != "new" or path_row(file_record)[2] != sid:
        fail(f"{sid}: test.file must be an exact new owned file")
    if not isinstance(test["command"], str) or not test["command"].strip() or "--workspace" in test["command"] or "--all-targets" in test["command"]:
        fail(f"{sid}: test command must be specific and executable")
    oracle = test["oracle"]
    if not isinstance(oracle, dict) or oracle.get("kind") not in ORACLE_KINDS or not oracle.get("expected") or not oracle.get("verifier"):
        fail(f"{sid}: oracle needs a supported kind, decidable expected result, and independent verifier")

    for refkind, ref in dependency_refs(row, "requires"):
        if refkind == "ref":
            if ref not in all_known_ids:
                fail(f"{sid}: unknown hard slice prerequisite {ref}")
            if ref in row_by_id:
                hard_edges.append((sid, ref))
        # issue references are deliberately terminal interface contracts and do not create slice DAG nodes
    for refkind, ref in dependency_refs(row, "after"):
        if refkind == "ref" and ref not in all_known_ids:
            fail(f"{sid}: unknown soft after reference {ref}")

if statement_coverage != EXPECTED_STATEMENTS:
    fail(f"crosscut statement coverage missing={sorted(EXPECTED_STATEMENTS-statement_coverage)}, extra={sorted(statement_coverage-EXPECTED_STATEMENTS)}")

# Crosscut and Part A/B dependencies share one DAG. Soft `after` edges are never blockers.
combined_ids = set(row_by_id) | set(external_by_id)
edge_rows = list(all_rows) + external_rows
graph = {ident: [] for ident in combined_ids}
degree = {ident: 0 for ident in combined_ids}
combined_edges = []
for row in edge_rows:
    sid = row.get("id")
    if sid not in combined_ids:
        continue
    for refkind, ref in dependency_refs(row, "requires"):
        if refkind == "ref" and ref in combined_ids:
            graph[ref].append(sid)
            degree[sid] += 1
            combined_edges.append((sid, ref))
ready = [ident for ident, n in degree.items() if n == 0]
visited = 0
while ready:
    ident = ready.pop()
    visited += 1
    for child in graph[ident]:
        degree[child] -= 1
        if degree[child] == 0:
            ready.append(child)
if visited != len(combined_ids):
    fail("crosscut + Part A + Part B hard dependency graph contains a cycle")

# Include sibling owned paths so crosscut root ownership is checked against AA/AG/B manifests.
for row in external_rows:
    sid = row.get("id")
    for raw in row.get("owned_files", row.get("owned_surfaces", [])):
        path, status, owner = path_row(raw)
        if not path:
            continue
        owner = owner or sid
        owned_by_path.setdefault(path, []).append(owner)

# Collisions are authorized by (ordered slice pair, exact path), never by pair alone.
declared = {}
for collision in impl_doc.get("file_collisions", []):
    pair = tuple(sorted(collision.get("slices", [])))
    path = collision.get("path")
    if len(pair) != 2 or not path or not collision.get("reason"):
        fail("file collision declaration requires exact path, pair, and reason")
    declared[(pair, path)] = collision
for path, owners in owned_by_path.items():
    if len(owners) < 2:
        continue
    for i, left in enumerate(owners):
        for right in owners[i + 1:]:
            key = (tuple(sorted((left, right))), path)
            if key not in declared:
                fail(f"undeclared exact-path collision {path}: {left}, {right}")
            if not any({dependent, prerequisite} == {left, right} for dependent, prerequisite in combined_edges):
                fail(f"collision {path} has no hard ownership-order edge between {left} and {right}")

# Config examples exercise full deployment -> mode -> tenant -> peer precedence under ceilings.
cfg = sequence["config_cases"]
required_invariants = {"signature_required", "semantic_validation_required", "replay_check_required", "tenant_scope_required"}
if set(cfg["immutable_invariants"]) != required_invariants:
    fail("fixed config invariants are missing or weakened")
layers = cfg["behavioral_defaults"]
ordered = [layers[name] for name in ("deployment", "mode", "tenant", "peer")]
merged = {}
for layer in ordered:
    merged.update(layer)
scope_ceiling = set(cfg["authority_ceiling"]["tenant_allowed_sync_scope"])
scope_candidates = [layer["sync_scope"] for layer in reversed(ordered) if "sync_scope" in layer]
resolved_scope = next((value for value in scope_candidates if value in scope_ceiling), None)
if resolved_scope is None:
    fail("sync_scope cannot be resolved inside tenant authority ceiling")
resolved = dict(merged)
resolved["sync_scope"] = resolved_scope
resolved["max_hops"] = min(merged["max_hops"], cfg["resource_ceiling"]["max_hops"])
for key, expected in cfg["expected_resolution"].items():
    if resolved.get(key) != expected:
        fail(f"config precedence resolved {key}={resolved.get(key)!r}, expected {expected!r}")
if layers["deployment"]["sync_scope"] == layers["mode"]["sync_scope"] or layers["deployment"]["tier"] == layers["mode"]["tier"]:
    fail("behavioral-default examples must demonstrate a mode override")
if resolved["tier"] != layers["tenant"]["tier"]:
    fail("tenant behavior must override mode tier within ceilings")
invalid = cfg["invalid_overrides"]
if not any(item.get("max_hops", 0) > cfg["resource_ceiling"]["max_hops"] for item in invalid):
    fail("no over-ceiling hop negative case")
if not {key for item in invalid for key, value in item.items() if key in required_invariants and value is False} >= required_invariants:
    fail("negative cases must attempt to disable every fixed gate")
if not any(item.get("sync_scope") not in scope_ceiling for item in invalid):
    fail("no out-of-authority sync-scope negative case")

print(f"crosscut verified: 39 issue slots; {len(impl_rows)} implementation + {len(research_rows)} research records; cross-part DAG, exact-path collision, statement-coverage, tier, and config oracles pass")
