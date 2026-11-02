#!/usr/bin/env python3
"""test_clone_all_partial.py — regression tests for #3233.

clone_all.clone_one only checked valid_mirror(dest) before cloning; a
destination left partial by a killed clone (driver timeout, interrupt,
laptop sleep) fell through to `git clone --mirror`, which git refuses
('already exists and is not an empty directory'), and nothing ever
cleared it — the repo stayed unmirrored on every rerun.

Drives the real clone_all.main() against a temp backup root with a
fake subprocess standing in for `git clone` (the fake mimics real git:
cloning into a non-empty destination fails; otherwise it creates a
real bare mirror with git init --bare). valid_mirror stays real.

  partial dest        -> cleared, clone retried, ok + cleared_partial
  partial dest, clone
    fails anyway      -> fail record still carries cleared_partial
  partial dest, clone
    times out         -> fail/timeout record carries cleared_partial
  valid mirror dest   -> already-present, untouched, never re-cloned
  absent dest         -> normal clone, no cleared_partial marker
  empty dir dest      -> normal clone (git accepts), no marker

Run: python3 infra/github-backup/tests/test_clone_all_partial.py
"""

import json, os, subprocess, sys, tempfile

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import clone_all
from ghb_common import mirror_path, valid_mirror

REAL_RUN = subprocess.run
REPO = "octo/widgets"


class FakeSubprocess:
    """Stand-in for the subprocess module as clone_all sees it.

    Only `git clone` is faked; TimeoutExpired is the real class so
    clone_all's except clause matches.
    """

    TimeoutExpired = subprocess.TimeoutExpired

    def __init__(self, mode):
        self.mode = mode  # "ok" | "fail" | "timeout"
        self.clone_calls = []

    def run(self, cmd, **kw):
        assert cmd[:2] == ["git", "clone"], f"unexpected command: {cmd}"
        self.clone_calls.append(cmd)
        dest = cmd[-1]
        if self.mode == "timeout":
            raise subprocess.TimeoutExpired(cmd, kw.get("timeout", 0))
        if os.path.isdir(dest) and os.listdir(dest):
            return subprocess.CompletedProcess(
                cmd,
                128,
                "",
                f"fatal: destination path '{dest}' already exists and is "
                "not an empty directory.",
            )
        if self.mode == "fail":
            return subprocess.CompletedProcess(
                cmd, 1, "", "fatal: simulated clone failure"
            )
        r = REAL_RUN(["git", "init", "--bare", dest], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr
        return subprocess.CompletedProcess(cmd, 0, "", "")


class Harness:
    """One clone_all.main() run over a temp root with one planned repo."""

    def __init__(self, mode="ok"):
        self.root = tempfile.mkdtemp(prefix="ghb-clone-test-")
        with open(os.path.join(self.root, "plan.json"), "w") as f:
            json.dump([{"repo": REPO}], f)
        self.dest = mirror_path(self.root, REPO)
        self.fake = FakeSubprocess(mode)
        self._saved = (
            clone_all.load_config,
            clone_all.backup_root,
            clone_all.subprocess,
        )
        clone_all.load_config = lambda: {}
        clone_all.backup_root = lambda cfg=None: self.root
        clone_all.subprocess = self.fake

    def run_main(self):
        try:
            return clone_all.main()
        finally:
            (
                clone_all.load_config,
                clone_all.backup_root,
                clone_all.subprocess,
            ) = self._saved

    def records(self):
        path = os.path.join(self.root, "clone-status.jsonl")
        if not os.path.exists(path):
            return []
        return [json.loads(line) for line in open(path) if line.strip()]

    def seed_partial(self):
        os.makedirs(self.dest)
        with open(os.path.join(self.dest, "partial-object.tmp"), "w") as f:
            f.write("interrupted transfer leftover")

    def seed_valid_mirror(self):
        os.makedirs(os.path.dirname(self.dest), exist_ok=True)
        r = REAL_RUN(["git", "init", "--bare", self.dest], capture_output=True, text=True)
        assert r.returncode == 0, r.stderr
        with open(os.path.join(self.dest, "marker.txt"), "w") as f:
            f.write("do not touch")


FAILURES = []


def check(name, cond, detail=""):
    print(("PASS " if cond else "FAIL ") + name + (f" — {detail}" if detail and not cond else ""))
    if not cond:
        FAILURES.append(name)


def test_partial_dest_cleared_and_cloned():
    h = Harness("ok")
    h.seed_partial()
    rc = h.run_main()
    recs = h.records()
    check("partial: main exits 0", rc == 0, f"rc={rc} recs={recs}")
    check("partial: one record", len(recs) == 1, f"recs={recs}")
    if recs:
        check("partial: status ok", recs[0].get("status") == "ok", f"rec={recs[0]}")
        check(
            "partial: cleared_partial marker set",
            recs[0].get("cleared_partial") is True,
            f"rec={recs[0]}",
        )
    check("partial: dest is now a valid mirror", valid_mirror(h.dest))
    check(
        "partial: leftover file gone",
        not os.path.exists(os.path.join(h.dest, "partial-object.tmp")),
    )
    check("partial: exactly one clone attempted", len(h.fake.clone_calls) == 1)


def test_partial_dest_clone_failure_marked():
    h = Harness("fail")
    h.seed_partial()
    rc = h.run_main()
    recs = h.records()
    check("partial+fail: main exits 1", rc == 1, f"rc={rc}")
    check("partial+fail: one record", len(recs) == 1, f"recs={recs}")
    if recs:
        check("partial+fail: status fail", recs[0].get("status") == "fail")
        check(
            "partial+fail: cleared_partial distinguishes the retry",
            recs[0].get("cleared_partial") is True,
            f"rec={recs[0]}",
        )
    check(
        "partial+fail: partial was cleared before the attempt",
        not os.path.exists(os.path.join(h.dest, "partial-object.tmp")),
    )


def test_partial_dest_timeout_marked():
    h = Harness("timeout")
    h.seed_partial()
    rc = h.run_main()
    recs = h.records()
    check("partial+timeout: main exits 1", rc == 1, f"rc={rc}")
    if recs:
        check(
            "partial+timeout: timeout error",
            recs[0].get("error") == "timeout",
            f"rec={recs[0]}",
        )
        check(
            "partial+timeout: cleared_partial marker set",
            recs[0].get("cleared_partial") is True,
            f"rec={recs[0]}",
        )
    else:
        check("partial+timeout: one record", False, "no records written")


def test_valid_mirror_untouched():
    h = Harness("ok")
    h.seed_valid_mirror()
    rc = h.run_main()
    recs = h.records()
    check("valid: main exits 0", rc == 0, f"rc={rc}")
    if recs:
        check(
            "valid: already-present note",
            recs[0].get("note") == "already-present",
            f"rec={recs[0]}",
        )
        check(
            "valid: no cleared_partial marker",
            "cleared_partial" not in recs[0],
            f"rec={recs[0]}",
        )
    check("valid: no clone attempted", len(h.fake.clone_calls) == 0)
    check(
        "valid: marker file intact",
        open(os.path.join(h.dest, "marker.txt")).read() == "do not touch",
    )


def test_absent_dest_normal_clone():
    h = Harness("ok")
    rc = h.run_main()
    recs = h.records()
    check("absent: main exits 0", rc == 0, f"rc={rc} recs={recs}")
    if recs:
        check("absent: status ok", recs[0].get("status") == "ok")
        check(
            "absent: no cleared_partial marker",
            "cleared_partial" not in recs[0],
            f"rec={recs[0]}",
        )
    check("absent: dest is a valid mirror", valid_mirror(h.dest))
    check("absent: exactly one clone attempted", len(h.fake.clone_calls) == 1)


def test_empty_dir_dest_normal_clone():
    h = Harness("ok")
    os.makedirs(h.dest)
    rc = h.run_main()
    recs = h.records()
    check("empty-dir: main exits 0", rc == 0, f"rc={rc} recs={recs}")
    if recs:
        check(
            "empty-dir: no cleared_partial marker",
            "cleared_partial" not in recs[0],
            f"rec={recs[0]}",
        )
    check("empty-dir: dest is a valid mirror", valid_mirror(h.dest))


def main():
    test_partial_dest_cleared_and_cloned()
    test_partial_dest_clone_failure_marked()
    test_partial_dest_timeout_marked()
    test_valid_mirror_untouched()
    test_absent_dest_normal_clone()
    test_empty_dir_dest_normal_clone()
    if FAILURES:
        print(f"\n{len(FAILURES)} FAILURE(S): {FAILURES}")
        return 1
    print("\nall clone_all partial-destination tests pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
