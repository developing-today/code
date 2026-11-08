#!/usr/bin/env python3
"""Regression tests for malformed previous plans in reconcile (#6831).

Drives reconcile.main() with inventory stubbed and a temporary backup
root. Invalid old-plan entries must be named and skipped without
discarding valid entries or preventing the report and MANIFEST writes.

Run: python3 infra/github-backup/tests/test_reconcile_old_plan.py
"""

import contextlib
import glob
import io
import json
import os
import sys
import tempfile

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import manifest
import reconcile


CFG = {
    "owners": ["good-owner", "bad-owner"],
    "tier_rules": {
        "include_nonforks": True,
        "skip_zero_commit_repos": True,
        "fork_max_size_mb": 500,
    },
    "exclusions": [],
    "gaps": [],
}


def plan_item(repo):
    return {
        "repo": repo,
        "reference": None,
        "size_kb": 10,
        "fork": False,
        "parent": None,
        "branch": "main",
        "head": "abc123",
        "private": None,
    }


def inv_repo(repo):
    return {
        "repo": repo,
        "fork": False,
        "archived": False,
        "size_kb": 10,
        "parent": None,
        "branch": "main",
        "head": "abc123",
        "pushed_at": "2026-11-01T00:00:00Z",
        "empty": False,
    }


def run_case(old_plan, errors=None):
    root = tempfile.mkdtemp(prefix="ghb-reconcile-old-plan-test-")
    with open(os.path.join(root, "plan.json"), "w") as f:
        json.dump(old_plan, f)

    inventory = {"good-owner/newrepo": inv_repo("good-owner/newrepo")}
    errors = errors or {}
    saved = (
        reconcile.load_config,
        reconcile.backup_root,
        reconcile.inventory,
        reconcile.valid_mirror,
        manifest.load_config,
        manifest.build,
    )
    manifest_calls = []
    real_manifest_build = manifest.build

    def build_manifest(build_root=None):
        manifest_calls.append(build_root)
        return real_manifest_build(build_root)

    reconcile.load_config = lambda: CFG
    reconcile.backup_root = lambda cfg=None: root
    reconcile.inventory = lambda owners: (inventory, errors)
    # Avoid invoking clone_all; manifest generation itself remains real.
    reconcile.valid_mirror = lambda path: True
    manifest.load_config = lambda: CFG
    manifest.build = build_manifest
    stdout, stderr = io.StringIO(), io.StringIO()
    try:
        with contextlib.redirect_stdout(stdout), contextlib.redirect_stderr(stderr):
            rc = reconcile.main()
    finally:
        (
            reconcile.load_config,
            reconcile.backup_root,
            reconcile.inventory,
            reconcile.valid_mirror,
            manifest.load_config,
            manifest.build,
        ) = saved

    reports = glob.glob(os.path.join(root, "reconcile-*.md"))
    assert len(reports) == 1, f"expected one dated report, got {reports}"
    with open(reports[0]) as f:
        report = f.read()
    with open(os.path.join(root, "plan.json")) as f:
        plan = json.load(f)
    with open(os.path.join(root, "MANIFEST.json")) as f:
        generated_manifest = json.load(f)
    return rc, stderr.getvalue(), report, plan, generated_manifest, manifest_calls


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


PASS = []

# A malformed entry cannot hide the valid entry before it: the valid old
# repo still participates in added/removed diff calculation.
rc, errors, report, plan, generated, calls = run_case(
    [plan_item("good-owner/dropped"), 123]
)
check("integer old-plan entry does not abort reconcile", rc == 0)
check(
    "integer entry is named and skipped",
    "entry 1" in errors and "int" in errors,
)
check(
    "valid old entry still appears in removal diff",
    "good-owner/dropped" in report and "no longer planned" in report,
)
check(
    "current inventory is written to plan",
    [p["repo"] for p in plan] == ["good-owner/newrepo"],
)
check(
    "integer case regenerates MANIFEST",
    "good-owner/newrepo" in generated["mirrors"] and len(calls) == 1,
)

# A string and an object missing repo used to throw TypeError and KeyError.
rc, errors, report, plan, generated, calls = run_case(["old-owner/repo"])
check("string old-plan entry does not abort reconcile", rc == 0)
check(
    "string entry is reported by index and shape",
    "entry 0" in errors and "str" in errors,
)
check(
    "string case writes report, plan and manifest",
    bool(report)
    and plan[0]["repo"] == "good-owner/newrepo"
    and "good-owner/newrepo" in generated["mirrors"],
)

rc, errors, report, plan, generated, calls = run_case([{"name": "old-owner/repo"}])
check("entry missing repo does not abort reconcile", rc == 0)
check(
    "missing-repo entry is reported by index",
    "entry 0" in errors and "dict" in errors,
)
check(
    "missing-repo case reaches manifest generation",
    len(calls) == 1 and "good-owner/newrepo" in generated["mirrors"],
)

# Exercise the #3229 retention path: invalid repo=123 is dropped, while a
# valid entry belonging to the failed owner is still retained.
rc, errors, report, plan, generated, calls = run_case(
    [plan_item("bad-owner/keep"), {"repo": 123}],
    errors={"bad-owner": "repositoryOwner is null"},
)
check("numeric repo on inventory-error path does not abort", rc == 1)
check(
    "numeric repo entry is named and skipped",
    "entry 1" in errors and "string 'repo'" in errors,
)
check(
    "valid failed-owner entry is retained",
    "bad-owner/keep" in [p["repo"] for p in plan],
)
check(
    "retention and inventory error remain in report",
    "bad-owner/keep" in report and "repositoryOwner is null" in report,
)
check(
    "retention case regenerates MANIFEST",
    "bad-owner/keep" in generated["mirrors"] and len(calls) == 1,
)

# A non-list top level is one named problem and behaves like an empty old
# plan; it must not prevent the current plan/report/manifest from landing.
rc, errors, report, plan, generated, calls = run_case({"repo": "old-owner/repo"})
check("object top-level old plan does not abort reconcile", rc == 0)
check(
    "top-level problem is named and treated as empty",
    "plan.json: top level is dict" in errors
    and "whole file treated as empty" in errors,
)
check(
    "empty previous plan means current repo is newly planned",
    "newly planned" in report and "good-owner/newrepo" in report,
)
check(
    "top-level case regenerates MANIFEST",
    plan[0]["repo"] == "good-owner/newrepo"
    and "good-owner/newrepo" in generated["mirrors"]
    and len(calls) == 1,
)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
