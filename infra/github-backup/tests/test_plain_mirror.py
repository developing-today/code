#!/usr/bin/env python3
"""Plain mirror sync oracle: destination refs equal the source after a run."""

import os
import random
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

    def test_generated_ref_mutations_preserve_exact_ref_set_invariant(self):
        # A fixed seed makes failures reproducible while generating varied
        # source/destination sets and create/update/delete sequences.
        rng = random.Random(0x8584)
        with tempfile.TemporaryDirectory() as root:
            source = os.path.join(root, "source.git")
            destination = os.path.join(root, "destination.git")
            work = os.path.join(root, "work")
            git("init", "--bare", source)
            git("init", "--bare", destination)
            git("init", work)
            git("-C", work, "config", "user.name", "Test")
            git("-C", work, "config", "user.email", "test@example.invalid")
            commits = []
            for index in range(5):
                with open(os.path.join(work, "README.md"), "w", encoding="utf-8") as f:
                    f.write(f"generated mirror commit {index}\n")
                git("-C", work, "add", "README.md")
                git("-C", work, "commit", "-m", f"generated {index}")
                commits.append(git("-C", work, "rev-parse", "HEAD"))

            git("-C", work, "remote", "add", "source", source)
            git("-C", work, "remote", "add", "destination", destination)
            for index, oid in enumerate(commits):
                git("-C", work, "update-ref", f"refs/heads/seed-{index}", oid)
            git("-C", work, "push", "--all", "source")
            git("-C", work, "push", "--all", "destination")
            for index in range(len(commits)):
                git("--git-dir", source, "update-ref", "-d", f"refs/heads/seed-{index}")
                git("--git-dir", destination, "update-ref", "-d", f"refs/heads/seed-{index}")

            source_refs = {"refs/heads/main": commits[0], "refs/tags/v0": commits[0]}
            destination_refs = {
                "refs/heads/obsolete": commits[1],
                "refs/tags/old": commits[2],
            }
            for ref, oid in source_refs.items():
                git("--git-dir", source, "update-ref", ref, oid)
            for ref, oid in destination_refs.items():
                git("--git-dir", destination, "update-ref", ref, oid)
            git("--git-dir", source, "symbolic-ref", "HEAD", "refs/heads/main")

            ref_names = [
                f"refs/{kind}/generated-{index}"
                for kind in ("heads", "tags")
                for index in range(6)
            ]
            operations = []
            for _ in range(60):
                ref = rng.choice(ref_names)
                operation = rng.choice(("create", "update", "delete"))
                oid = rng.choice(commits)
                operations.append((operation, ref, oid))

            # The first pass must erase arbitrary destination-only refs.
            plain_mirror.sync_mirror(source, destination)
            self.assertEqual(
                ref_map(
                    git(
                        "--git-dir",
                        destination,
                        "for-each-ref",
                        "--format=%(objectname) %(refname)",
                        "refs/heads",
                        "refs/tags",
                    )
                ),
                source_refs,
            )
            for operation, ref, oid in operations:
                exists = ref in source_refs
                if operation == "delete":
                    if exists:
                        git("--git-dir", source, "update-ref", "-d", ref)
                        del source_refs[ref]
                elif operation == "create" and not exists:
                    git("--git-dir", source, "update-ref", ref, oid)
                    source_refs[ref] = oid
                elif operation == "update" and exists:
                    git("--git-dir", source, "update-ref", ref, oid)
                    source_refs[ref] = oid

                plain_mirror.sync_mirror(source, destination)
                actual_refs = ref_map(
                    git(
                        "--git-dir",
                        destination,
                        "for-each-ref",
                        "--format=%(objectname) %(refname)",
                        "refs/heads",
                        "refs/tags",
                    )
                )
                self.assertEqual(actual_refs, source_refs, (operation, ref, oid))

    def test_default_branch_is_replaced_before_pruning_old_default(self):
        calls = mock.Mock()
        with (
            mock.patch.object(plain_mirror, "valid_mirror", return_value=True),
            mock.patch.object(
                plain_mirror,
                "_refs",
                side_effect=[{"refs/heads/new": "oid"}, {"refs/heads/new": "oid"}],
            ) as refs,
            mock.patch.object(
                plain_mirror,
                "_gh",
                side_effect=["old", ""],
            ) as gh,
            mock.patch.object(
                plain_mirror,
                "_git",
                side_effect=["refs/heads/new", "", ""],
            ) as git_call,
        ):
            calls.attach_mock(refs, "refs")
            calls.attach_mock(gh, "gh")
            calls.attach_mock(git_call, "git")
            result = plain_mirror.sync_mirror(
                "/source.git", "https://github.com/developing-today/code-mirror.git"
            )

        self.assertEqual(result, {"status": "verified", "refs": 1})
        self.assertEqual(
            calls.mock_calls,
            [
                mock.call.refs("/source.git"),
                mock.call.gh(["repos/developing-today/code-mirror", "--jq", ".default_branch"]),
                mock.call.git(["--git-dir", "/source.git", "symbolic-ref", "HEAD"]),
                mock.call.git(
                    [
                        "--git-dir",
                        "/source.git",
                        "push",
                        "https://github.com/developing-today/code-mirror.git",
                        "+refs/heads/*:refs/heads/*",
                        "+refs/tags/*:refs/tags/*",
                    ]
                ),
                mock.call.gh(
                    [
                        "repos/developing-today/code-mirror",
                        "--method",
                        "PATCH",
                        "-f",
                        "default_branch=new",
                    ]
                ),
                mock.call.git(
                    [
                        "--git-dir",
                        "/source.git",
                        "push",
                        "--prune",
                        "https://github.com/developing-today/code-mirror.git",
                        "+refs/heads/*:refs/heads/*",
                        "+refs/tags/*:refs/tags/*",
                    ]
                ),
                mock.call.refs("https://github.com/developing-today/code-mirror.git", remote=True),
            ],
        )

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
