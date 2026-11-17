#!/usr/bin/env python3
"""Regression tests for clone_all reference-anchor scheduling (#4163).

Run: python3 infra/github-backup/tests/test_clone_all_references.py
"""

import json, os, subprocess, sys, tempfile, threading, time

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import clone_all
from ghb_common import mirror_path


class FakeSubprocess:
    """Fake clone commands while preserving clone_all's real scheduler."""

    TimeoutExpired = subprocess.TimeoutExpired

    def __init__(self, fail_repos=()):
        self.fail_repos = set(fail_repos)
        self.clone_calls = []
        self.lock = threading.Lock()

    def run(self, cmd, **_kwargs):
        assert cmd[:2] == ["git", "clone"], f"unexpected command: {cmd}"
        repo = cmd[-2].removeprefix("https://github.com/").removesuffix(".git")
        with self.lock:
            self.clone_calls.append((repo, list(cmd)))

        # Keep the anchor in flight while clone_all reaches its member.
        if repo == "o/anchor":
            time.sleep(3)
        if repo in self.fail_repos:
            return subprocess.CompletedProcess(cmd, 1, "", "simulated clone failure")

        dest = cmd[-1]
        os.makedirs(dest, exist_ok=True)
        with open(os.path.join(dest, ".fake-mirror"), "w") as f:
            f.write("ok")
        return subprocess.CompletedProcess(cmd, 0, "", "")


class Harness:
    def __init__(self, plan, fail_repos=()):
        self.temp = tempfile.TemporaryDirectory(prefix="ghb-reference-test-")
        self.root = self.temp.name
        with open(os.path.join(self.root, "plan.json"), "w") as f:
            json.dump(plan, f)
        self.fake = FakeSubprocess(fail_repos)
        self.saved = (
            clone_all.load_config,
            clone_all.backup_root,
            clone_all.subprocess,
            clone_all.valid_mirror,
            clone_all.disk_kb,
            clone_all.PAR,
        )
        clone_all.load_config = lambda: {}
        clone_all.backup_root = lambda cfg=None: self.root
        clone_all.subprocess = self.fake
        clone_all.valid_mirror = lambda path: os.path.isfile(
            os.path.join(path, ".fake-mirror")
        )
        clone_all.disk_kb = lambda _path: 1
        clone_all.PAR = 4

    def run(self):
        try:
            return clone_all.main()
        finally:
            (
                clone_all.load_config,
                clone_all.backup_root,
                clone_all.subprocess,
                clone_all.valid_mirror,
                clone_all.disk_kb,
                clone_all.PAR,
            ) = self.saved

    def records(self):
        path = os.path.join(self.root, "clone-status.jsonl")
        with open(path) as f:
            return [json.loads(line) for line in f if line.strip()]

    def command(self, repo):
        return next(cmd for name, cmd in self.fake.clone_calls if name == repo)

    def cleanup(self):
        self.temp.cleanup()


def test_slow_anchor_is_used_as_reference():
    h = Harness(
        [
            {"repo": "o/anchor"},
            {"repo": "o/member", "reference": "o/anchor"},
            {"repo": "o/filler1"},
            {"repo": "o/filler2"},
        ]
    )
    try:
        assert h.run() == 0
        member = h.command("o/member")
        assert "--reference" in member
        assert mirror_path(h.root, "o/anchor") in member
        assert "--reference" not in h.command("o/filler1")
        records = {record["repo"]: record for record in h.records()}
        assert records["o/member"]["reference_used"] == "o/anchor"
    finally:
        h.cleanup()


def test_failed_anchor_uses_full_clone_fallback():
    h = Harness(
        [{"repo": "o/anchor"}, {"repo": "o/member", "reference": "o/anchor"}],
        fail_repos=("o/anchor",),
    )
    try:
        assert h.run() == 1
        member = h.command("o/member")
        assert "--reference" not in member
        records = {record["repo"]: record for record in h.records()}
        assert records["o/anchor"]["status"] == "fail"
        assert records["o/member"]["status"] == "ok"
    finally:
        h.cleanup()


def test_reference_cycle_uses_no_inflight_fallback():
    h = Harness(
        [
            {"repo": "o/a", "reference": "o/b"},
            {"repo": "o/b", "reference": "o/a"},
        ]
    )
    try:
        assert h.run() == 0
        calls = {repo: cmd for repo, cmd in h.fake.clone_calls}
        # With neither member initially runnable, the no-inflight fallback
        # starts one without a reference; the other then uses that result.
        assert "--reference" not in calls["o/a"]
        assert "--reference" in calls["o/b"]
        assert len(h.records()) == 2
    finally:
        h.cleanup()


def test_reference_outside_plan_does_not_deadlock_or_get_used():
    h = Harness([{"repo": "o/member", "reference": "o/not-planned"}])
    try:
        assert h.run() == 0
        assert "--reference" not in h.command("o/member")
        assert h.records()[0]["reference_used"] is None
    finally:
        h.cleanup()


def test_wrong_shape_reference_does_not_crash_scheduler():
    # Issue #6834: a list/dict reference is unhashable in the
    # scheduler's state.get(ref) and an int reference can never match
    # a repo name — pre-fix the whole clone run died before DONE.
    # validate_plan now neutralizes them to None: full clones, run
    # completes, no --reference flag is passed for the poisoned rows.
    h = Harness(
        [
            {"repo": "o/anchor"},
            {"repo": "o/member", "reference": ["o/anchor"]},
            {"repo": "o/other", "reference": 123},
        ]
    )
    try:
        assert h.run() == 0
        assert "--reference" not in h.command("o/member")
        assert "--reference" not in h.command("o/other")
        records = {record["repo"]: record for record in h.records()}
        assert len(records) == 3
        assert all(r["status"] == "ok" for r in records.values())
        assert records["o/member"]["reference_used"] is None
    finally:
        h.cleanup()


if __name__ == "__main__":
    tests = [
        test_slow_anchor_is_used_as_reference,
        test_failed_anchor_uses_full_clone_fallback,
        test_reference_cycle_uses_no_inflight_fallback,
        test_reference_outside_plan_does_not_deadlock_or_get_used,
        test_wrong_shape_reference_does_not_crash_scheduler,
    ]
    for test in tests:
        test()
        print(f"PASS {test.__name__}")
    print(f"\n{len(tests)}/{len(tests)} checks passed")
