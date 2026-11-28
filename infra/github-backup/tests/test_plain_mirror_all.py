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
        self.assertIn("unexpected argument: '--unexpected'", stderr.getvalue())

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
