#!/usr/bin/env python3
"""Fleet-sync oracle: each planned repo is attempted and failures are reported."""

import io
import os
import sys
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import plain_mirror_all


class PlainMirrorAllTests(unittest.TestCase):
    def test_rejects_unaccounted_arguments_before_syncing(self):
        with (
            mock.patch.object(plain_mirror_all, "mirror_repo") as sync,
            mock.patch("sys.stderr", new_callable=io.StringIO) as stderr,
        ):
            self.assertEqual(plain_mirror_all.main(["--unexpected"]), 2)
            sync.assert_not_called()
        self.assertIn("unrecognized arguments: --unexpected", stderr.getvalue())

    def test_root_option_is_passed_to_every_sync(self):
        # Oracle: the #15000 hosted adapter — a runner-supplied --root
        # reaches mirror_repo verbatim (expanded/absolutized) for every
        # fleet repo, and the configured backup_root is never consulted.
        repos = plain_mirror_all.FLEET_REPOS
        with (
            mock.patch.object(plain_mirror_all, "mirror_repo") as sync,
            mock.patch.object(plain_mirror_all, "backup_root") as configured,
        ):
            sync.return_value = {"refs": 1}
            self.assertEqual(plain_mirror_all.main(["--root", "/tmp/hosted-root"]), 0)
            configured.assert_not_called()
        self.assertEqual(
            [call.args for call in sync.call_args_list],
            [(repo, "/tmp/hosted-root") for repo in repos],
        )

    def test_default_root_comes_from_configuration(self):
        # Oracle: with no --root, the root is backup_root() exactly once
        # and is shared by every repo's sync.
        with (
            mock.patch.object(plain_mirror_all, "mirror_repo") as sync,
            mock.patch.object(
                plain_mirror_all, "backup_root", return_value="/configured/root"
            ) as configured,
        ):
            sync.return_value = {"refs": 1}
            self.assertEqual(plain_mirror_all.main([]), 0)
            configured.assert_called_once_with()
        self.assertTrue(
            all(call.args[1] == "/configured/root" for call in sync.call_args_list)
        )

    def test_attempts_every_repo_and_returns_failure_if_one_sync_fails(self):
        repos = plain_mirror_all.FLEET_REPOS
        with mock.patch.object(plain_mirror_all, "mirror_repo") as sync:
            sync.side_effect = [
                {"refs": 1},
                RuntimeError("push failed"),
                {"refs": 3},
                {"refs": 4},
            ]
            self.assertEqual(plain_mirror_all.main([]), 1)
        self.assertEqual([call.args[0] for call in sync.call_args_list], list(repos))


if __name__ == "__main__":
    unittest.main()
