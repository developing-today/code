#!/usr/bin/env python3
"""test_reconcile_clone_pass.py — regression test for #3300.

reconcile.py used to discard clone_all.py's non-zero exit and write its
dated report before the clone pass, so a failed mirror creation was
reported as success. Now:

- clone_all rc=1 -> reconcile exits 1 and the dated report records the
  clone failure, naming the failing repos from clone-status.jsonl.
- clone_all rc=0 -> reconcile exits 0 and the report records success.
- inventory-error control -> still exits non-zero, clone pass not run.

Drives the real reconcile.main() with inventory and the clone_all
subprocess stubbed, against a temp backup root.

Run: python3 infra/github-backup/tests/test_reconcile_clone_pass.py
"""

import glob, json, os, subprocess, sys, tempfile

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import reconcile


def bare_mirror(root, repo):
    """A REAL valid mirror: bare + one commit on refs/heads/main.

    Since #3238 a bare skeleton (zero refs) is NOT a valid mirror —
    validity requires a resolvable HEAD — so a fixture mirror that
    must count as present needs history.
    """
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


CFG = {
    "owners": ["good-owner"],
    "tier_rules": {
        "include_nonforks": True,
        "skip_zero_commit_repos": True,
        "fork_max_size_mb": 500,
    },
    "exclusions": [],
    "gaps": [],
}

PASS = []


def check(name, cond):
    PASS.append((name, bool(cond)))
    print(("PASS " if cond else "FAIL ") + name)
    assert cond, name


REAL_RUN = subprocess.run


def run_scenario(clone_rc, inventory_result, with_mirror, fail_repos=(), extra_lines=()):
    """Run reconcile.main() in a fresh temp root with stubs installed.

    clone_rc=None means the clone pass must not be invoked at all.
    Returns (rc, report_text).
    """
    root = tempfile.mkdtemp(prefix="ghb-clone-pass-test-")
    item = plan_item("good-owner/newrepo")
    with open(os.path.join(root, "plan.json"), "w") as f:
        json.dump([item], f)
    if with_mirror:
        bare_mirror(root, item["repo"])

    calls = []

    def fake_run(cmd, *args, **kwargs):
        if any(str(c).endswith("clone_all.py") for c in cmd):
            calls.append(cmd)
            assert clone_rc is not None, "clone pass ran unexpectedly"
            # Simulate clone_all appending per-repo status records.
            with open(os.path.join(root, "clone-status.jsonl"), "a") as f:
                for line in extra_lines:
                    f.write(line + "\n")
                for repo in fail_repos:
                    f.write(json.dumps({"repo": repo, "status": "fail"}) + "\n")
                if clone_rc == 0:
                    f.write(json.dumps({"repo": item["repo"], "status": "ok"}) + "\n")
            return subprocess.CompletedProcess(cmd, clone_rc)
        return REAL_RUN(cmd, *args, **kwargs)

    saved = (
        reconcile.load_config,
        reconcile.backup_root,
        reconcile.inventory,
        subprocess.run,
    )
    reconcile.load_config = lambda: CFG
    reconcile.backup_root = lambda cfg=None: root
    reconcile.inventory = lambda owners: inventory_result
    subprocess.run = fake_run
    try:
        rc = reconcile.main()
    finally:
        (
            reconcile.load_config,
            reconcile.backup_root,
            reconcile.inventory,
            subprocess.run,
        ) = saved

    reports = glob.glob(os.path.join(root, "reconcile-*.md"))
    assert len(reports) == 1, f"expected one report, got {reports}"
    report = open(reports[0]).read()
    return rc, report, calls


CLEAN = ({"good-owner/newrepo": inv_repo("good-owner/newrepo")}, {})

# --- Scenario 1: clone pass fails -------------------------------------
rc, report, calls = run_scenario(
    clone_rc=1,
    inventory_result=CLEAN,
    with_mirror=False,
    fail_repos=("good-owner/newrepo",),
)
check("clone rc=1 -> reconcile exits 1", rc == 1)
check("clone pass was invoked once", len(calls) == 1)
check(
    "report records the clone failure",
    "clone pass: FAILED" in report and "clone_all exit 1" in report,
)
check(
    "report names the failing repo",
    "## clone pass failures" in report and "good-owner/newrepo" in report,
)

# --- Scenario 2: clone pass succeeds ----------------------------------
rc, report, calls = run_scenario(clone_rc=0, inventory_result=CLEAN, with_mirror=False)
check("clone rc=0 + clean inventory -> reconcile exits 0", rc == 0)
check("report records clone success", "clone pass: succeeded" in report)

# --- Scenario 3: inventory-error control ------------------------------
rc, report, calls = run_scenario(
    clone_rc=None,
    inventory_result=({}, {"good-owner": "gh api graphql: owner gone"}),
    with_mirror=True,
)
check("inventory error -> reconcile still exits non-zero", rc == 1)
check("clone pass not run when nothing is missing", len(calls) == 0)
check(
    "report notes clone pass not run",
    "clone pass: not run" in report,
)

# --- Scenario 4: failed clone pass with a poisoned status file (#6606) --
# Valid-JSON non-dict lines and an unparseable line land in
# clone-status.jsonl after the offset, before the real fail record.
# The failed-clone consumer must ignore them (shared contract:
# ghb_common.clone_status_record), still name the real failure in the
# dated report, and still exit 1 — no AttributeError traceback.
rc, report, calls = run_scenario(
    clone_rc=1,
    inventory_result=CLEAN,
    with_mirror=False,
    fail_repos=("good-owner/newrepo",),
    extra_lines=("123", '"fail"', '["owner/bad"]', "null", "true", "{torn line"),
)
check("poisoned status file -> reconcile still exits 1", rc == 1)
check(
    "poisoned status file -> report still names the real failing repo",
    "## clone pass failures" in report and "good-owner/newrepo" in report,
)
check(
    "poison shapes never surface as repos",
    "owner/bad" not in report,
)

# Direct reader check: _clone_status_entries keeps only dict records;
# the consumer expression over them returns exactly the valid fails.
root = tempfile.mkdtemp(prefix="ghb-status-shape-test-")
with open(os.path.join(root, "clone-status.jsonl"), "w") as f:
    f.write(json.dumps({"repo": "owner/bad", "status": "fail"}) + "\n")
    f.write("{torn line\n")
    for shape in ("123", '"x"', "[]", "null", "true"):
        f.write(shape + "\n")
    f.write(json.dumps({"repo": "owner/good", "status": "ok"}) + "\n")
    f.write(json.dumps({"status": "fail"}) + "\n")
    f.write(json.dumps({"repo": ["owner/bad2"], "status": "fail"}) + "\n")
entries = reconcile._clone_status_entries(root, 0)
check("reader keeps only dict records", all(isinstance(e, dict) for e in entries) and len(entries) == 4)
consumer = sorted(
    {
        e["repo"]
        for e in entries
        if e.get("status") == "fail"
        and isinstance(e.get("repo"), str)
        and e.get("repo")
    }
)
check("consumer returns exactly the valid fail repos", consumer == ["owner/bad"])

print(f"\n{sum(1 for _, ok in PASS if ok)}/{len(PASS)} checks passed")
