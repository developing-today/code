#!/usr/bin/env python3
"""Wrapper tests for pkgs/mcpx/scripts/test.sh (queue task 13250, issue #1949).

The wrapper is the local half of the run-seed transport: it must
mint or accept exactly one seed, export it to the go test process,
derive -shuffle from that same seed by the #1949 section 2 formula,
and fail closed in CI. Each test runs the real script with a fake
`go` on PATH that records its argv and environment, so the oracle
is what the test process would actually receive.

Run: python3 -m unittest discover -s pkgs/mcpx/scripts/tests -p 'test_mcpx_test_seed.py' -v
"""

import hashlib
import os
import re
import shutil
import subprocess
import sys
import tempfile
import unittest

REPO_MCPX = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", ".."))
SCRIPT = os.path.join(REPO_MCPX, "scripts", "test.sh")

FAKE_GO = """#!/usr/bin/env bash
printf 'ARGS:%s\\n' "$*"
printf 'SEED:%s\\n' "${MCPX_TEST_SEED:-<unset>}"
"""


def spec_shuffle(seed: str) -> int:
    """The #1949 section 2 formula, restated from the issue text."""
    root = hashlib.sha256(b"mcpx-test-seed/v1\x00" + seed.encode()).digest()
    return int.from_bytes(hashlib.sha256(root + b"shuffle").digest()[:8], "big") & (
        2**63 - 1
    )


class WrapperTest(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.mkdtemp(prefix="mcpx-test-seed-")
        self.addCleanup(shutil.rmtree, self.tmp, True)
        # The script cd's to pkgs/mcpx and writes .cache there when it
        # mints; run it from a scratch copy so the tree stays clean.
        self.work = os.path.join(self.tmp, "mcpx")
        os.makedirs(os.path.join(self.work, "scripts"))
        shutil.copy2(SCRIPT, os.path.join(self.work, "scripts", "test.sh"))
        bindir = os.path.join(self.tmp, "bin")
        os.makedirs(bindir)
        fake = os.path.join(bindir, "go")
        with open(fake, "w") as fh:
            fh.write(FAKE_GO)
        os.chmod(fake, 0o755)
        self.env = {
            k: v
            for k, v in os.environ.items()
            if k not in ("MCPX_TEST_SEED", "CI", "GITHUB_ACTIONS")
        }
        self.env["PATH"] = bindir + os.pathsep + self.env["PATH"]

    def run_wrapper(self, args=(), **env):
        e = dict(self.env, **env)
        return subprocess.run(
            ["bash", os.path.join(self.work, "scripts", "test.sh"), *args],
            capture_output=True,
            text=True,
            env=e,
            timeout=60,
        )

    def test_set_seed_is_exported_and_derives_shuffle(self):
        seed = "01a0f101-2378-7c1c-9dc1-5a1ae1571eca"
        r = self.run_wrapper(["./internal/codegen"], MCPX_TEST_SEED=seed)
        self.assertEqual(r.returncode, 0, r.stderr)
        self.assertIn(f"SEED:{seed}", r.stdout)
        self.assertIn(
            f"ARGS:test -shuffle={spec_shuffle(seed)} ./internal/codegen", r.stdout
        )

    def test_unset_locally_mints_v7_and_records_it(self):
        r = self.run_wrapper()
        self.assertEqual(r.returncode, 0, r.stderr)
        m = re.search(r"SEED:([0-9a-f-]{36})", r.stdout)
        self.assertIsNotNone(m, r.stdout + r.stderr)
        seed = m.group(1)
        self.assertEqual(seed[14], "7", seed)  # version nibble
        self.assertIn(seed[19], "89ab", seed)  # variant bits
        self.assertIn(f"ARGS:test -shuffle={spec_shuffle(seed)} ./...", r.stdout)
        with open(os.path.join(self.work, ".cache", "test-seed", "last")) as fh:
            self.assertEqual(fh.read().strip(), seed)

    def test_unset_in_ci_fails_closed(self):
        for extra in ({"CI": "true"}, {"GITHUB_ACTIONS": "true"}):
            r = self.run_wrapper(**extra)
            self.assertEqual(r.returncode, 1, extra)
            self.assertIn("never mint", r.stderr)
            self.assertNotIn("ARGS:", r.stdout)

    def test_invalid_seed_is_an_error_not_a_remint(self):
        r = self.run_wrapper(MCPX_TEST_SEED="bad seed!")
        self.assertEqual(r.returncode, 1)
        self.assertIn("not a valid seed id", r.stderr)
        self.assertNotIn("ARGS:", r.stdout)


if __name__ == "__main__":
    unittest.main()
