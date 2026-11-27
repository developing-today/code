#!/usr/bin/env python3
"""Live rehost clone verification contract.

Oracle: each live clone must have the same HEAD symref, HEAD oid, and heads/tags
with peeled tag targets as its reconstructed source mirror; any mismatch makes
the four-repository check fail while later repositories are still attempted.

Run: python3 -I infra/github-backup/tests/test_verify_rehost_clones.py
"""

import os
import pathlib
import sys
import tempfile
import unittest
from unittest.mock import patch

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import verify_rehost_clones


def refs(head):
    return (
        "refs/heads/main",
        head,
        [{"name": "refs/heads/main", "oid": head, "peeled": None}],
    )


class VerifyRehostClonesTests(unittest.TestCase):
    def test_all_live_clones_match_source_heads_and_full_ref_sets(self):
        repos = verify_rehost_clones.FLEET_REPOS
        expected = [refs(f"{index:040x}") for index in range(1, len(repos) + 1)]
        collect = [value for pair in zip(expected, expected) for value in pair]
        clones = []

        def clone(args):
            clones.append(args)

        with tempfile.TemporaryDirectory() as root:
            with (
                patch.object(verify_rehost_clones, "valid_mirror", return_value=True),
                patch.object(
                    verify_rehost_clones.rehost_publish,
                    "collect_refs",
                    side_effect=collect,
                ),
                patch.object(verify_rehost_clones, "_run_git", side_effect=clone),
            ):
                self.assertEqual(verify_rehost_clones.run_all(root), 0)

        self.assertEqual(len(clones), len(repos))
        self.assertEqual(
            [args[2] for args in clones],
            [f"{verify_rehost_clones.BASE_URL}/{repo}" for repo in repos],
        )
        self.assertTrue(all(args[:2] == ["clone", "--bare"] for args in clones))

    def test_ref_mismatch_fails_but_later_endpoints_are_still_cloned(self):
        repos = verify_rehost_clones.FLEET_REPOS
        expected = [refs(f"{index:040x}") for index in range(1, len(repos) + 1)]
        mismatch = refs("f" * 40)
        collect = [expected[0], mismatch]
        for item in expected[1:]:
            collect.extend((item, item))
        clones = []

        with tempfile.TemporaryDirectory() as root:
            with (
                patch.object(verify_rehost_clones, "valid_mirror", return_value=True),
                patch.object(
                    verify_rehost_clones.rehost_publish,
                    "collect_refs",
                    side_effect=collect,
                ),
                patch.object(
                    verify_rehost_clones,
                    "_run_git",
                    side_effect=lambda args: clones.append(args),
                ),
            ):
                self.assertEqual(verify_rehost_clones.run_all(root), 1)

        self.assertEqual(len(clones), len(repos))

    def test_unknown_argument_fails_before_tempdir_or_git(self):
        with tempfile.TemporaryDirectory() as parent:
            root = os.path.join(parent, "not-created")
            with patch.object(verify_rehost_clones, "_run_git") as git:
                self.assertEqual(
                    verify_rehost_clones.main(["--root", root, "--unexpected"]), 2
                )
            self.assertFalse(os.path.exists(root))
            git.assert_not_called()


if __name__ == "__main__":
    unittest.main()
