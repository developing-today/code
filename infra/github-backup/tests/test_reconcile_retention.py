#!/usr/bin/env python3
"""test_reconcile_retention.py — regression test for #3229 (reconcile half).

When an owner's inventory fails, reconcile must keep that owner's
previously planned repos in plan.json + MANIFEST.json (last-known-good),
name the failure in the report, and exit non-zero. An owner that
inventories cleanly is unaffected: repos it no longer has drop out of
the plan as before.

Drives the real reconcile.main() with inventory stubbed, against a
temp backup root holding real (empty) bare mirrors.

Run: python3 infra/github-backup/tests/test_reconcile_retention.py
"""
import glob, json, os, subprocess, sys, tempfile

BIN = os.path.abspath(os.path.join(os.path.dirname(os.path.abspath(__file__)),
                                   "..", "bin"))
sys.path.insert(0, BIN)

import reconcile


def bare_mirror(root, repo):
    path = os.path.join(root, repo + ".git")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    subprocess.run(["git", "init", "--bare", "-q", path], check=True)


def plan_item(repo, parent=None):
    return {"repo": repo, "reference": None, "size_kb": 10, "fork": False,
            "parent": parent, "branch": "main", "head": "abc123",
            "private": None}


def inv_repo(repo):
    return {"repo": repo, "fork": False, "archived": False, "size_kb": 10,
            "parent": None, "branch": "main", "head": "abc123",
            "pushed_at": "2026-10-31T00:00:00Z", "empty": False}


PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


root = tempfile.mkdtemp(prefix="ghb-reconcile-test-")
old_plan = [plan_item("bad-owner/keep1"), plan_item("bad-owner/keep2"),
            plan_item("good-owner/stays"), plan_item("good-owner/dropped")]
with open(os.path.join(root, "plan.json"), "w") as f:
    json.dump(old_plan, f)
for item in old_plan:
    bare_mirror(root, item["repo"])

CFG = {"owners": ["bad-owner", "good-owner"],
       "tier_rules": {"include_nonforks": True, "skip_zero_commit_repos": True,
                      "fork_max_size_mb": 500},
       "exclusions": [], "gaps": []}


def stub_inventory(owners):
    # bad-owner failed (the #3229 null-owner shape, post-fix); good-owner
    # inventories cleanly and now has only one of its two old repos.
    return ({"good-owner/stays": inv_repo("good-owner/stays")},
            {"bad-owner": "gh api graphql for bad-owner: repositoryOwner "
                          "is null — owner not visible to the token"})


saved = (reconcile.load_config, reconcile.backup_root, reconcile.inventory)
reconcile.load_config = lambda: CFG
reconcile.backup_root = lambda cfg=None: root
reconcile.inventory = stub_inventory
try:
    rc = reconcile.main()
finally:
    (reconcile.load_config, reconcile.backup_root,
     reconcile.inventory) = saved

check("exit code non-zero on owner inventory failure", rc == 1)

with open(os.path.join(root, "plan.json")) as f:
    plan_repos = [p["repo"] for p in json.load(f)]
check("failed owner's repos RETAINED in plan.json",
      "bad-owner/keep1" in plan_repos and "bad-owner/keep2" in plan_repos)
check("healthy owner's current repo planned", "good-owner/stays" in plan_repos)
check("healthy owner's vanished repo dropped (real drop still works)",
      "good-owner/dropped" not in plan_repos)

with open(os.path.join(root, "MANIFEST.json")) as f:
    manifest = json.load(f)
check("failed owner's repos RETAINED in MANIFEST.json",
      "bad-owner/keep1" in manifest["mirrors"]
      and "bad-owner/keep2" in manifest["mirrors"])
check("retained mirrors recorded as mirrored (they exist on disk)",
      manifest["mirrors"]["bad-owner/keep1"]["status"] == "mirrored")

reports = glob.glob(os.path.join(root, "reconcile-*.md"))
check("report written", len(reports) == 1)
report = open(reports[0]).read()
check("report names the inventory error", "bad-owner" in report
      and "repositoryOwner is null" in report)
check("report names the retention", "retained for failed owners" in report
      and "bad-owner/keep1" in report)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
