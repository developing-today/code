#!/usr/bin/env python3
"""test_reconcile_old_plan_shape.py — regression test for #6831.

reconcile.py is the last plan reader that must go through the shared
validated load: a wrong-shape entry in the PREVIOUS plan.json used to
crash the weekly sweep with a raw TypeError/KeyError/AttributeError
(`old = [p["repo"] for p in old_items]` and the #3229 retention loop's
`item["repo"].split(...)` were unguarded). The fix routes the previous
plan through ghb_common.load_plan/validate_plan, so a poison entry is
reported by index and skipped, valid entries keep participating in
the added/removed diff and the failed-owner retention, and a fully
poisoned previous plan behaves as an empty one.

Drives the real reconcile.main() with inventory stubbed, against a
temp backup root, exactly as test_reconcile_clone_pass.py does.

Run: python3 infra/github-backup/tests/test_reconcile_old_plan_shape.py
"""

import contextlib, glob, io, json, os, subprocess, sys, tempfile

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import reconcile


def bare_mirror(root, repo):
    """A REAL valid mirror (bare + one commit), as in the sibling tests."""
    path = os.path.join(root, repo + ".git")
    os.makedirs(os.path.dirname(path), exist_ok=True)
    subprocess.run(["git", "init", "--bare", "-q", path], check=True)
    env = {
        **os.environ,
        "GIT_AUTHOR_NAME": "ghb-test",
        "GIT_AUTHOR_EMAIL": "ghb-test@example.invalid",
        "GIT_COMMITTER_NAME": "ghb-test",
        "GIT_COMMITTER_EMAIL": "ghb-test@example.invalid",
    }
    tree = subprocess.run(
        ["git", "-C", path, "mktree"], input="", capture_output=True, text=True, check=True
    ).stdout.strip()
    commit = subprocess.run(
        ["git", "-C", path, "commit-tree", tree, "-m", "seed"],
        capture_output=True, text=True, check=True, env=env,
    ).stdout.strip()
    subprocess.run(["git", "-C", path, "update-ref", "refs/heads/main", commit], check=True)
    subprocess.run(["git", "-C", path, "symbolic-ref", "HEAD", "refs/heads/main"], check=True)


def plan_item(repo, parent=None):
    return {
        "repo": repo,
        "reference": None,
        "size_kb": 10,
        "fork": False,
        "parent": parent,
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


PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


def run_main(root, cfg, inventory_result):
    saved = (reconcile.load_config, reconcile.backup_root, reconcile.inventory)
    reconcile.load_config = lambda: cfg
    reconcile.backup_root = lambda cfg=None: root
    reconcile.inventory = lambda owners: inventory_result
    stderr = io.StringIO()
    try:
        with contextlib.redirect_stderr(stderr):
            rc = reconcile.main()
    finally:
        (reconcile.load_config, reconcile.backup_root, reconcile.inventory) = saved
    reports = glob.glob(os.path.join(root, "reconcile-*.md"))
    report = open(reports[0]).read() if reports else ""
    with open(os.path.join(root, "plan.json")) as f:
        plan = json.load(f)
    return rc, report, plan, stderr.getvalue(), reports


CLEAN_CFG = {
    "owners": ["good-owner"],
    "tier_rules": {
        "include_nonforks": True,
        "skip_zero_commit_repos": True,
        "fork_max_size_mb": 500,
    },
    "exclusions": [],
    "gaps": [],
}
CLEAN_INV = ({"good-owner/newrepo": inv_repo("good-owner/newrepo")}, {})


def clean_scenario(name, old_plan, expect_problem_for=None):
    root = tempfile.mkdtemp(prefix="ghb-old-plan-test-")
    with open(os.path.join(root, "plan.json"), "w") as f:
        json.dump(old_plan, f)
    bare_mirror(root, "good-owner/newrepo")
    rc, report, plan, stderr, reports = run_main(root, CLEAN_CFG, CLEAN_INV)
    check(f"{name}: sweep completes (rc 0)", rc == 0)
    check(f"{name}: new plan.json written", [p["repo"] for p in plan] == ["good-owner/newrepo"])
    check(f"{name}: dated report written", len(reports) == 1)
    check(f"{name}: MANIFEST regenerated", os.path.exists(os.path.join(root, "MANIFEST.json")))
    if expect_problem_for is not None:
        check(
            f"{name}: poison named in problems output",
            expect_problem_for in stderr,
        )
    return report


# --- Poison entries in the previous plan (clean inventory) ------------
clean_scenario("entry 123", [plan_item("good-owner/oldrepo"), 123], "entry 1")
clean_scenario("entry as string", ["good-owner/oldrepo"], "entry 0")
clean_scenario("entry missing repo", [{"name": "x"}], "entry 0")
clean_scenario("entry null", [None], "entry 0")
clean_scenario("top-level object", {"repo": "good-owner/oldrepo"}, "top level")

# A valid old entry still participates: it shows up as removed in the diff.
report = clean_scenario("mixed valid + poison", [plan_item("good-owner/oldrepo"), 123], "entry 1")
check("mixed valid + poison: valid entry counted as removed", "1 no longer planned" in report)

# A fully poisoned previous plan behaves as an empty previous plan.
report = clean_scenario("fully poisoned", [123, "x", None, {"repo": 123}])
check("fully poisoned: nothing counted as removed", "0 no longer planned" in report)

# --- {repo: 123} on the inventory-error retention path (#3229) --------
root = tempfile.mkdtemp(prefix="ghb-old-plan-retention-test-")
with open(os.path.join(root, "plan.json"), "w") as f:
    json.dump([{"repo": 123}, plan_item("bad-owner/keep1")], f)
bare_mirror(root, "bad-owner/keep1")
bare_mirror(root, "good-owner/stays")
RET_CFG = {
    "owners": ["bad-owner", "good-owner"],
    "tier_rules": {
        "include_nonforks": True,
        "skip_zero_commit_repos": True,
        "fork_max_size_mb": 500,
    },
    "exclusions": [],
    "gaps": [],
}
RET_INV = (
    {"good-owner/stays": inv_repo("good-owner/stays")},
    {"bad-owner": "gh api graphql for bad-owner: repositoryOwner is null"},
)
rc, report, plan, stderr, reports = run_main(root, RET_CFG, RET_INV)
check("retention path: sweep survives {repo: 123} (rc 1 = inventory error)", rc == 1)
plan_repos = [p["repo"] for p in plan]
check("retention path: valid failed-owner entry retained", "bad-owner/keep1" in plan_repos)
check("retention path: poison entry named in problems output", "entry 0" in stderr)
check(
    "retention path: report names the retention",
    "retained for failed owners" in report and "bad-owner/keep1" in report,
)

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
