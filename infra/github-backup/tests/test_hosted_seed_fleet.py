#!/usr/bin/env python3
"""Hosted seed reconstruction contract for the four fleet repositories.

Oracle: the design's fixed FLEET_REPOS inventory must be mirrored directly
from GitHub into the supplied ephemeral root, all mirrors must validate, and
the published plan must name exactly those verified repositories.

Run: python3 -I infra/github-backup/tests/test_hosted_seed_fleet.py
"""

import json
import os
import sys
import tempfile
import unittest
from unittest.mock import patch

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import hosted_seed_fleet


class HostedSeedFleetTests(unittest.TestCase):
    def test_clones_all_fleet_repos_and_writes_plan_after_validation(self):
        # Oracle: exact GitHub source URLs and mirror destinations for the
        # design's four-repository inventory, followed by plan readback.
        with tempfile.TemporaryDirectory() as root:
            with (
                patch.object(hosted_seed_fleet, "_git") as git,
                patch.object(
                    hosted_seed_fleet, "valid_mirror", return_value=True
                ) as valid,
            ):
                self.assertEqual(hosted_seed_fleet.run_all(root), 0)

            expected = [
                [
                    "clone",
                    "--mirror",
                    "--",
                    f"https://github.com/{repo}.git",
                    os.path.join(root, repo + ".git"),
                ]
                for repo in hosted_seed_fleet.FLEET_REPOS
            ]
            self.assertEqual([call.args[0] for call in git.call_args_list], expected)
            self.assertEqual(
                [call.args[0] for call in valid.call_args_list],
                [
                    os.path.join(root, repo + ".git")
                    for repo in hosted_seed_fleet.FLEET_REPOS
                ],
            )
            with open(os.path.join(root, "plan.json")) as f:
                plan = json.load(f)
            self.assertEqual(
                plan, [{"repo": repo} for repo in hosted_seed_fleet.FLEET_REPOS]
            )

    def test_failed_clone_never_writes_a_partial_publish_plan(self):
        # Oracle: a plan consumed by publishers exists only when every
        # required source mirror was fetched and validated.
        with tempfile.TemporaryDirectory() as root:

            def clone(args):
                if hosted_seed_fleet.FLEET_REPOS[1] in args[-2]:
                    raise RuntimeError("source unavailable")

            with (
                patch.object(hosted_seed_fleet, "_git", side_effect=clone) as git,
                patch.object(hosted_seed_fleet, "valid_mirror", return_value=True),
            ):
                self.assertEqual(hosted_seed_fleet.run_all(root), 1)
            self.assertEqual(git.call_count, len(hosted_seed_fleet.FLEET_REPOS))
            self.assertFalse(os.path.exists(os.path.join(root, "plan.json")))

    def test_unknown_argument_fails_before_creating_root_or_running_git(self):
        # Oracle: strict operator CLI parsing precedes every filesystem and
        # subprocess side effect (#3352).
        with tempfile.TemporaryDirectory() as parent:
            root = os.path.join(parent, "ephemeral")
            with patch.object(hosted_seed_fleet, "_git") as git:
                self.assertEqual(hosted_seed_fleet.main(["--root", root, "--extra"]), 2)
            self.assertFalse(os.path.exists(root))
            git.assert_not_called()


if __name__ == "__main__":
    unittest.main()
