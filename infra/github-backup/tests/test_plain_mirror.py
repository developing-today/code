#!/usr/bin/env python3
"""Plain mirror sync oracle: destination refs equal the source after a run."""

import os
import subprocess
import sys
import tempfile
import unittest
from unittest import mock

sys.path.insert(0, os.path.join(os.path.dirname(__file__), "..", "bin"))
import plain_mirror


def git(*args):
    return subprocess.run(
        ["git", *args], check=True, capture_output=True, text=True
    ).stdout.strip()


def ref_map(output):
    return {
        fields[1]: fields[0]
        for line in output.splitlines()
        if len(fields := line.split()) == 2
    }


class PlainMirrorTests(unittest.TestCase):
    def test_sync_mirror_makes_destination_refs_equal_source(self):
        with tempfile.TemporaryDirectory() as root:
            source = os.path.join(root, "source.git")
            destination = os.path.join(root, "destination.git")
            work = os.path.join(root, "work")
            git("init", "--bare", source)
            git("init", "--bare", destination)
            git("init", work)
            git("-C", work, "config", "user.name", "Test")
            git("-C", work, "config", "user.email", "test@example.invalid")
            with open(os.path.join(work, "README.md"), "w", encoding="utf-8") as f:
                f.write("mirror test\n")
            git("-C", work, "add", "README.md")
            git("-C", work, "commit", "-m", "initial")
            git("-C", work, "branch", "-M", "main")
            git("-C", work, "tag", "v1")
            git("-C", work, "remote", "add", "origin", source)
            git("-C", work, "push", "--all", "origin")
            git("-C", work, "push", "--tags", "origin")
            git("clone", "--mirror", source, os.path.join(root, "local.git"))
            source_path = os.path.join(root, "local.git")
            head = git("--git-dir", source_path, "rev-parse", "refs/heads/main")
            git("--git-dir", source_path, "update-ref", "refs/heads/stale", head)
            git("--git-dir", source_path, "update-ref", "refs/tags/stale-tag", head)

            result = plain_mirror.sync_mirror(
                source_path, destination
            )

            source_refs = ref_map(
                git(
                    "--git-dir",
                    source_path,
                    "for-each-ref",
                    "--format=%(objectname) %(refname)",
                    "refs/heads",
                    "refs/tags",
                )
            )
            destination_refs = ref_map(git("ls-remote", "--refs", destination))
            self.assertEqual(destination_refs, source_refs)
            self.assertEqual(result["status"], "verified")

            git("--git-dir", source_path, "update-ref", "-d", "refs/heads/stale")
            git("--git-dir", source_path, "update-ref", "-d", "refs/tags/stale-tag")
            plain_mirror.sync_mirror(source_path, destination)

            source_refs = ref_map(
                git(
                    "--git-dir",
                    source_path,
                    "for-each-ref",
                    "--format=%(objectname) %(refname)",
                    "refs/heads",
                    "refs/tags",
                )
            )
            destination_refs = ref_map(git("ls-remote", "--refs", destination))
            self.assertEqual(destination_refs, source_refs)
            self.assertNotIn("refs/heads/stale", destination_refs)
            self.assertNotIn("refs/tags/stale-tag", destination_refs)

    def test_rejects_non_bare_source_before_push(self):
        with tempfile.TemporaryDirectory() as root:
            destination = os.path.join(root, "destination.git")
            git("init", "--bare", destination)
            with self.assertRaisesRegex(ValueError, "not a valid mirror"):
                plain_mirror.sync_mirror(root, destination)

    def test_cli_rejects_abbreviated_options_before_loading_plan(self):
        with mock.patch.object(plain_mirror, "backup_root") as backup_root:
            with self.assertRaises(SystemExit):
                plain_mirror.main(["--rep", "developing-today/code"])
            backup_root.assert_not_called()


if __name__ == "__main__":
    unittest.main()
