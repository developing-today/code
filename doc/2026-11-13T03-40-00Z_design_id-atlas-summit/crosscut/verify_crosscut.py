#!/usr/bin/env python3
"""Stdlib verifier for crosscut sequencing, slice contracts, and config examples."""
import json
from pathlib import Path

ROOT = Path(__file__).resolve().parent
EXPECTED = {9110, 9111, 9112, *range(9113, 9149)}


def fail(message):
    raise SystemExit(f"crosscut verification failed: {message}")


sequence = json.loads((ROOT / "sequencing.json").read_text())
components = sequence["components"]
by_id = {row["id"]: row for row in components}
if set(by_id) != EXPECTED:
    fail(f"issue coverage missing={sorted(EXPECTED - set(by_id))}, extra={sorted(set(by_id) - EXPECTED)}")
if len(components) != len(by_id):
    fail("duplicate issue IDs")

for row in components:
    if row["slot"] not in {"now", "next", "later"}:
        fail(f"bad slot for #{row['id']}")
    for stage in ("research_slot", "design_slot"):
        if row.get(stage) not in {"now", "next", "later"}:
            fail(f"bad {stage} for #{row['id']}")
    if row.get("implementation_slot") not in {"now", "next", "later", "ongoing"}:
        fail(f"bad implementation_slot for #{row['id']}")
    if row["slot"] == "later" and (not row.get("reason") or row.get("review_date") != sequence["review_date"]):
        fail(f"later item #{row['id']} needs its own reason and review date")
    for dep in row["hard_dependencies"]:
        if dep not in by_id:
            fail(f"#{row['id']} has unknown hard dependency #{dep}")
        if by_id[dep]["wave"] >= row["wave"]:
            fail(f"hard edge #{dep} -> #{row['id']} is not topologically ordered by wave")

# Kahn's algorithm verifies the issue graph is acyclic independent of wave labels.
indegree = {issue: 0 for issue in by_id}
children = {issue: [] for issue in by_id}
for row in components:
    for dep in row["hard_dependencies"]:
        indegree[row["id"]] += 1
        children[dep].append(row["id"])
ready = [issue for issue, degree in indegree.items() if degree == 0]
visited = 0
while ready:
    issue = ready.pop()
    visited += 1
    for child in children[issue]:
        indegree[child] -= 1
        if indegree[child] == 0:
            ready.append(child)
if visited != len(by_id):
    fail("hard dependency graph contains a cycle")

slices_path = ROOT / "implementation-slices.json"
slice_doc = json.loads(slices_path.read_text())
slices = slice_doc["slices"]
research_slices = json.loads((ROOT / "research-slices.json").read_text())["research_slices"]
slice_ids = {row["id"] for row in slices}
research_ids = {row["id"] for row in research_slices}
all_slice_ids = slice_ids | research_ids
if len(slice_ids) != len(slices) or len(research_ids) != len(research_slices) or slice_ids & research_ids:
    fail("duplicate implementation slice IDs")
slice_indegree = {ident: 0 for ident in all_slice_ids}
slice_children = {ident: [] for ident in all_slice_ids}
files_to_slices = {}
for row in research_slices:
    for key in ("title", "kind", "tier", "statements", "problem", "scope", "evidence", "test_oracle", "done_criteria", "test_first_file", "test_first_command"):
        if key not in row:
            fail(f"research slice {row.get('id')} missing {key}")
    if row["kind"] != "research" or (row["tier"] == "contingent" and not row.get("trigger")):
        fail(f"research slice {row['id']} has invalid kind or missing contingent trigger")
    if not row.get("test_first_required") or not row["test_first_file"] in row["test_first_command"] and Path(row["test_first_file"]).stem not in row["test_first_command"]:
        fail(f"research slice {row['id']} must create its exact failing test first")
    files_to_slices.setdefault(row["test_first_file"], []).append(row["id"])
for row in slices:
    for key in ("title", "kind", "tier", "versions", "owned_files", "requires", "after", "tests", "enables"):
        if key not in row:
            fail(f"slice {row.get('id')} missing {key}")
    if not row["owned_files"]:
        fail(f"slice {row['id']} owns no files")
    for file in row["owned_files"]:
        if file.get("status") not in {"existing", "new"}:
            fail(f"slice {row['id']} has no existing/new status for {file.get('path')}")
        files_to_slices.setdefault(file["path"], []).append(row["id"])
    paths = {file["path"] for file in row["owned_files"]}
    test_status = next(file["status"] for file in row["owned_files"] if file["path"] == row["tests"]["file"])
    if test_status == "new" and row["tests"].get("new_before_code") is not True:
        fail(f"slice {row['id']} must create its new failing test first")
    test = row["tests"]
    if test["file"] not in paths or (Path(test["file"]).stem not in test["command"] and test["file"] not in test["command"]):
        fail(f"slice {row['id']} test command/file is not an owned surface")
    for prior in row["after"]:
        if prior not in all_slice_ids:
            fail(f"slice {row['id']} has unknown after-slice {prior}")
        slice_indegree[row["id"]] += 1
        slice_children[prior].append(row["id"])
    if row["tier"] == "contingent" and not row.get("trigger"):
        fail(f"contingent slice {row['id']} has no trigger")
    if row["tier"] == "contingent" and row["requires"]:
        fail(f"contingent slice {row['id']} must not encode its trigger as a hard dependency")
ready = [ident for ident, degree in slice_indegree.items() if degree == 0]
seen = 0
while ready:
    ident = ready.pop()
    seen += 1
    for child in slice_children[ident]:
        slice_indegree[child] -= 1
        if slice_indegree[child] == 0:
            ready.append(child)
if seen != len(all_slice_ids):
    fail("implementation slice after-graph contains a cycle")
declared_collisions = {tuple(sorted((item["slices"][0], item["slices"][1]))): item for item in slice_doc.get("file_collisions", [])}
for path, owners in files_to_slices.items():
    if len(owners) > 1:
        for i, left in enumerate(owners):
            for right in owners[i + 1:]:
                key = tuple(sorted((left, right)))
                collision = declared_collisions.get(key)
                if not collision or not collision.get("reason"):
                    fail(f"unresolved owned-file collision {path}: {left}, {right}")
                after_by_id = {s["id"]: s.get("after", []) for s in slices}
                if right not in after_by_id.get(left, []) and left not in after_by_id.get(right, []):
                    fail(f"file collision {path} has no hard slice-order edge: {left}, {right}")

cfg = sequence["config_cases"]
invariants = set(cfg["immutable_invariants"])
required = {"signature_required", "semantic_validation_required", "replay_check_required", "tenant_scope_required"}
if invariants != required:
    fail("fixed config gate differs from required immutable invariant set")
ceil = cfg["authority_ceiling"]["tenant_allowed_sync_scope"]
defaults = cfg["behavioral_defaults"]
expected_scope = defaults["tenant"].get("sync_scope", defaults["deployment"]["sync_scope"])
if expected_scope not in ceil or cfg["expected_resolution"]["sync_scope"] != expected_scope:
    fail("sync_scope behavioral override does not resolve within authority ceiling")
if defaults["deployment"]["sync_scope"] == defaults["mode"]["sync_scope"]:
    fail("sync_scope example does not demonstrate a replaceable default")
if cfg["expected_resolution"]["tier"] != defaults["tenant"]["tier"]:
    fail("tier example does not demonstrate later behavioral replacement")
if defaults["peer"]["max_hops"] > cfg["resource_ceiling"]["max_hops"]:
    fail("resolved hop limit exceeds hard resource ceiling")
invalid = cfg["invalid_overrides"]
if not any(x.get("max_hops", 0) > cfg["resource_ceiling"]["max_hops"] for x in invalid):
    fail("missing over-ceiling rejection example")
if not any(any(x.get(flag) is False for flag in required) for x in invalid):
    fail("missing fixed-gate disable rejection example")

print(f"crosscut verified: {len(components)} issue slots, {len(research_slices)} research slices, {len(slices)} implementation slices, DAG/file/config oracles pass")
