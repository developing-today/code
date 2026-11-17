#!/usr/bin/env python3
"""Untracked-vs-staged parity tests for pkgs/mcpx/scripts/affected-tests.py.

Queue task 5925 / issue #3320: untracked files are part of the selector's
change set (via `git ls-files --others`) but contribute no text to
`git diff <base>`, so content-derived selection silently missed them:

* T1b importer selection (exported idents come from the diff text),
* T2 registry-name selection (names come from the diff text),
* the internal/e2e `newEnv` harness rule (matched against the e2e diff).

Each parity test builds a scratch git repo with a pkgs/mcpx layout,
applies identical content once staged (`git add`) and once untracked,
invokes the real selector through its public CLI dry-run seam, and
requires identical selection. The deleted-file test locks in the
preserved behavior for tracked deletions.

No external network: fixture modules are stdlib-only, the selector runs
dry (never --run), and its subprocess env sets GOPROXY=off.
"""

import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

GIT = shutil.which("git")
GO = shutil.which("go")

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
SELECTOR = os.path.normpath(os.path.join(TESTS_DIR, "..", "affected-tests.py"))
MODULE = "github.com/developing-today/code/pkgs/mcpx"

HARNESS_SAMPLE = [
    "TestNoOrphanProcessesAfterStop",
    "TestEnvCleanupStopsTheDaemonItStarted",
    "TestEveryDeclaredRouteAnswersOverTheSocket",
]


def _git(*args, cwd):
    env = dict(
        os.environ,
        GIT_AUTHOR_NAME="fixture",
        GIT_AUTHOR_EMAIL="fixture@example.com",
        GIT_COMMITTER_NAME="fixture",
        GIT_COMMITTER_EMAIL="fixture@example.com",
    )
    return subprocess.run(
        [GIT, *args], cwd=cwd, capture_output=True, text=True, check=True, env=env
    )


@unittest.skipUnless(GIT, "git is required to build selector fixtures")
class SelectorFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="affected-untracked-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.mcpx = os.path.join(self.tmp, "pkgs", "mcpx")
        _git("init", "-q", "-b", "main", cwd=self.tmp)
        self.write("go.mod", "module %s\n\ngo 1.23\n" % MODULE)

    def write(self, rel, content):
        full = os.path.join(self.mcpx, rel)
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as f:
            f.write(content)

    def commit(self, msg="base"):
        _git("add", "-A", cwd=self.tmp)
        _git("-c", "commit.gpgsign=false", "commit", "-qm", msg, cwd=self.tmp)
        return _git("rev-parse", "HEAD", cwd=self.tmp).stdout.strip()

    def stage(self, rel):
        _git("add", "--", "pkgs/mcpx/" + rel, cwd=self.tmp)

    def unstage(self, rel):
        # Unstaging a never-committed path leaves identical content on
        # disk as an untracked file: the parity counterpart of stage().
        _git("reset", "-q", "HEAD", "--", "pkgs/mcpx/" + rel, cwd=self.tmp)

    def run_selector(self, base):
        env = dict(os.environ, GOPROXY="off", GOTOOLCHAIN="local")
        dirs = [os.path.dirname(GIT)]
        if GO:
            dirs.append(os.path.dirname(GO))
        dirs.append(os.environ.get("PATH", ""))
        env["PATH"] = os.pathsep.join(dirs)
        proc = subprocess.run(
            [sys.executable, SELECTOR, "--base", base],
            cwd=self.tmp,
            capture_output=True,
            text=True,
            env=env,
        )
        self.assertEqual(proc.returncode, 0, msg=proc.stderr)
        return proc.stdout

    def t2_run_set(self, output):
        m = re.search(r"\./internal/e2e -run '\^\((.+?)\)\$'", output)
        self.assertIsNotNone(m, msg="no T2 e2e selection in output:\n" + output)
        return set(m.group(1).split("|"))

    @unittest.skipUnless(GO, "go is required for the T1b go-list lookup")
    def test_t1b_untracked_parity_with_staged(self):
        self.write("internal/aaa/aaa.go", "package aaa\n\nfunc Existing() {}\n")
        self.write(
            "internal/bbb/bbb.go",
            'package bbb\n\nimport "%s/internal/aaa"\n\nvar _ = aaa.Existing\n'
            % MODULE,
        )
        base = self.commit()
        self.write(
            "internal/aaa/newapi.go", "package aaa\n\nfunc BrandNewExport() {}\n"
        )
        self.stage("internal/aaa/newapi.go")
        staged_out = self.run_selector(base)
        self.unstage("internal/aaa/newapi.go")
        untracked_out = self.run_selector(base)

        importer_cmd = "(cd pkgs/mcpx && go test -count=1 ./internal/bbb/)"
        self.assertIn("## T1b", staged_out)
        self.assertIn(importer_cmd, staged_out)
        self.assertEqual(staged_out, untracked_out)

    def test_t2_registry_name_untracked_parity_with_staged(self):
        self.write(
            "internal/settings/wire.go", 'package settings\n\nvar wire = "seed"\n'
        )
        self.write(
            "internal/e2e/alpha_test.go",
            'package e2e\n\nimport "testing"\n\n'
            'func TestAlphaMentionsSetting(t *testing.T) {\n\t_ = "some.setting.path"\n}\n',
        )
        base = self.commit()
        self.write(
            "internal/settings/extra.go",
            'package settings\n\nvar extra = "some.setting.path"\n',
        )
        self.stage("internal/settings/extra.go")
        staged_out = self.run_selector(base)
        staged_set = self.t2_run_set(staged_out)
        self.assertIn("TestAlphaMentionsSetting", staged_set)
        self.unstage("internal/settings/extra.go")
        untracked_out = self.run_selector(base)
        untracked_set = self.t2_run_set(untracked_out)
        self.assertIn("TestAlphaMentionsSetting", untracked_set)
        self.assertEqual(staged_set, untracked_set)

    def test_harness_newenv_untracked_parity_with_staged(self):
        self.write(
            "internal/e2e/base_test.go",
            'package e2e\n\nimport "testing"\n\nfunc TestBaseSanity(t *testing.T) {}\n',
        )
        base = self.commit()
        self.write(
            "internal/e2e/zz_helper_test.go",
            'package e2e\n\nimport "testing"\n\n'
            "func TestZedHelper(t *testing.T) {\n\tenv := newEnv(t)\n\t_ = env\n}\n",
        )
        self.stage("internal/e2e/zz_helper_test.go")
        staged_out = self.run_selector(base)
        staged_set = self.t2_run_set(staged_out)
        for sample in HARNESS_SAMPLE:
            self.assertIn(sample, staged_set)
        self.unstage("internal/e2e/zz_helper_test.go")
        untracked_out = self.run_selector(base)
        untracked_set = self.t2_run_set(untracked_out)
        for sample in HARNESS_SAMPLE:
            self.assertIn(sample, untracked_set)
        self.assertEqual(staged_set, untracked_set)

    def test_deleted_tracked_file_behavior_preserved(self):
        self.write("internal/ddd/old.go", "package ddd\n\nfunc Old() {}\n")
        base = self.commit()
        os.remove(os.path.join(self.mcpx, "internal/ddd/old.go"))
        out = self.run_selector(base)
        # The deleted file's package is still selected for T1 ...
        self.assertIn("(cd pkgs/mcpx && go test -count=1 ./internal/ddd/)", out)
        # ... but nothing tries to format a file that no longer exists.
        self.assertNotIn("gofmt", out)
        self.assertIn("no e2e selected; CI runs the full suite", out)


if __name__ == "__main__":
    unittest.main()
