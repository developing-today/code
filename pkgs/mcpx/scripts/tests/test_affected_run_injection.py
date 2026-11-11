#!/usr/bin/env python3
"""--run shell-injection regression tests for pkgs/mcpx/scripts/affected-tests.py.

Issue #5649: the selector collected Git-reported paths as text, split
them on whitespace, interpolated them into command strings, and ran
the strings with shell=True — so a crafted filename in an untrusted
checkout executed arbitrary shell syntax during `affected-tests.py
--run`. The fix keeps paths structured end to end (NUL-delimited Git
output, argv vectors, cwd= instead of `cd` strings).

Each test builds a scratch git repo with a pkgs/mcpx layout (mirroring
test_affected_untracked.py's fixture), adds an untracked .go file whose
name carries shell syntax, runs the real selector with --run, and
requires that no marker file appears. The metacharacter test also
requires a clean exit: the file is valid, formatted Go, so every tier
command succeeds once the name is passed as data.
"""

import os
import shutil
import subprocess
import sys
import tempfile
import unittest

GIT = shutil.which("git")
GO = shutil.which("go")
GOFMT = shutil.which("gofmt")

TESTS_DIR = os.path.dirname(os.path.abspath(__file__))
SELECTOR = os.path.normpath(os.path.join(TESTS_DIR, "..", "affected-tests.py"))
MODULE = "github.com/developing-today/code/pkgs/mcpx"


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


@unittest.skipUnless(GIT and GO and GOFMT, "git, go and gofmt are required")
class RunInjectionFixture(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="affected-run-injection-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.mcpx = os.path.join(self.tmp, "pkgs", "mcpx")
        _git("init", "-q", "-b", "main", cwd=self.tmp)
        full = os.path.join(self.mcpx, "go.mod")
        os.makedirs(os.path.dirname(full), exist_ok=True)
        with open(full, "w", encoding="utf-8") as f:
            f.write("module %s\n\ngo 1.23\n" % MODULE)
        _git("add", "-A", cwd=self.tmp)
        _git("-c", "commit.gpgsign=false", "commit", "-qm", "base", cwd=self.tmp)
        self.base = _git("rev-parse", "HEAD", cwd=self.tmp).stdout.strip()

    def write_untracked(self, name):
        with open(os.path.join(self.mcpx, name), "w", encoding="utf-8") as f:
            f.write("package mcpx\n")

    def run_selector(self):
        env = dict(os.environ, GOPROXY="off", GOTOOLCHAIN="local")
        proc = subprocess.run(
            [sys.executable, SELECTOR, "--base", self.base, "--run"],
            cwd=self.tmp,
            capture_output=True,
            text=True,
            env=env,
        )
        return proc


class TestRunDoesNotExecuteFilenames(RunInjectionFixture):
    def test_command_substitution_in_filename_is_data(self):
        # Under the old shell=True execution this name ran
        # `touch pwned` from the gofmt command line.
        self.write_untracked("evil$(touch${IFS}pwned).go")
        proc = self.run_selector()
        self.assertEqual(proc.returncode, 0, msg=proc.stdout + proc.stderr)
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp, "pwned")),
            "filename shell syntax executed during --run",
        )

    def test_semicolon_in_filename_is_data(self):
        self.write_untracked("x;touch${IFS}pwned2;.go")
        proc = self.run_selector()
        self.assertFalse(
            os.path.exists(os.path.join(self.tmp, "pwned2")),
            "filename shell syntax executed during --run",
        )

    def test_space_in_filename_stays_one_path(self):
        # Whitespace splitting used to break this into two bogus
        # paths; NUL-delimited collection keeps it a single file.
        self.write_untracked("with space.go")
        proc = self.run_selector()
        self.assertEqual(proc.returncode, 0, msg=proc.stdout + proc.stderr)


if __name__ == "__main__":
    unittest.main()
