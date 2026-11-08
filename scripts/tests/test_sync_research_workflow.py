# Copyright developing.today LLC & contributors to the project.
# Licensed under the MIT and Apache-2.0 licenses in the repository root.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""The mirrored workflow must preserve local adapters and reject lost local work."""

import importlib.util
import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SPEC = importlib.util.spec_from_file_location(
    "sync_workflow", Path(__file__).parents[1] / "sync_research_workflow.py"
)
MODULE = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(MODULE)


class SyncWorkflowTest(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        root = Path(self.temp.name)
        self.source, self.target = root / "cairn", root / "code"
        for directory, docs in (
            (self.source, "docs/workflow"),
            (self.target, "planning/workflow"),
        ):
            (directory / ".agents").mkdir(parents=True)
            (directory / ".agents/research-workflow.json").write_text(
                json.dumps({"workflow_docs": docs, "issue_repository": directory.name})
            )
        (self.source / "docs/workflow").mkdir(parents=True)
        (self.source / "docs/workflow/README.md").write_bytes(b"# Portable workflow\n")
        for name in MODULE.SKILLS:
            path = self.source / ".agents/skills" / name / "SKILL.md"
            path.parent.mkdir(parents=True)
            path.write_text(f"---\nname: {name}\n---\n")
        (self.source / "scripts/tests").mkdir(parents=True)
        (self.source / "scripts/research_workflow.py").write_text("print('workflow')\n")
        (self.source / "scripts/tests/test_research_workflow.py").write_text(
            "# Portable tests\n"
        )
        subprocess.run(["git", "init", "-q", str(self.source)], check=True)
        paths = [
            str(path.relative_to(self.source))
            for path in self.source.rglob("*")
            if path.is_file() and ".git" not in path.parts
        ]
        subprocess.run(["git", "-C", str(self.source), "add", "--", *paths], check=True)
        subprocess.run(
            [
                "git",
                "-C",
                str(self.source),
                "-c",
                "user.name=Workflow test",
                "-c",
                "user.email=test@example.invalid",
                "commit",
                "-qm",
                "fixture",
            ],
            check=True,
        )

    def assert_clean_failure(self, path, value, diagnostic):
        path.write_text(json.dumps(value))
        result = subprocess.run(
            [
                sys.executable,
                str(Path(MODULE.__file__).resolve()),
                "--source",
                str(self.source),
                "--repo-root",
                str(self.target),
                "--write",
                "--allow-dirty",
            ],
            text=True,
            capture_output=True,
            check=False,
        )
        self.assertEqual(result.returncode, 1)
        self.assertEqual(result.stdout, "")
        self.assertIn(diagnostic, result.stderr)
        self.assertNotIn("Traceback", result.stderr)
        self.assertEqual(len(result.stderr.splitlines()), 1)
        self.assertFalse((self.target / "planning/workflow/README.md").exists())

    def test_copy_is_repeatable_and_keeps_repository_adapter(self):
        profile = (self.target / ".agents/research-workflow.json").read_bytes()
        MODULE.sync(self.source, self.target, write=True)
        count, changed = MODULE.sync(self.source, self.target)
        self.assertEqual((count, changed), (12, 0))
        self.assertEqual(
            (self.target / "planning/workflow/README.md").read_bytes(),
            b"# Portable workflow\n",
        )
        self.assertEqual(
            (self.target / ".agents/research-workflow.json").read_bytes(), profile
        )
        self.assertEqual(MODULE.sync(self.source, self.target, write=True)[1], 0)

    def test_profile_roots_must_be_objects(self):
        for directory in (self.source, self.target):
            path = directory / ".agents/research-workflow.json"
            for value in ([], None, 123, "profile"):
                with self.subTest(directory=directory.name, value=value):
                    self.assert_clean_failure(
                        path, value, "research-workflow.json: expected a JSON object"
                    )

    def test_workflow_docs_must_be_a_string_in_both_profiles(self):
        for directory in (self.source, self.target):
            path = directory / ".agents/research-workflow.json"
            for value in ({}, {"workflow_docs": 123}, {"workflow_docs": None}):
                with self.subTest(directory=directory.name, value=value):
                    self.assert_clean_failure(
                        path,
                        value,
                        "research-workflow.json: workflow_docs must be a string",
                    )

    def test_lifecycle_skills_must_be_a_list_of_strings(self):
        path = self.source / ".agents/research-workflow.json"
        for value in ("research-workflow", [123], None):
            with self.subTest(value=value):
                self.assert_clean_failure(
                    path,
                    {"workflow_docs": "docs/workflow", "lifecycle_skills": value},
                    "research-workflow.json: lifecycle_skills must be a list of strings",
                )

    def test_manifest_and_files_must_be_objects(self):
        path = self.target / MODULE.MANIFEST
        for value in ([], None, 123, "manifest"):
            with self.subTest(value=value):
                self.assert_clean_failure(
                    path,
                    value,
                    "research-workflow.upstream.json: expected a JSON object",
                )
        with self.subTest(value={}):
            self.assert_clean_failure(
                path, {}, "research-workflow.upstream.json: files must be an object"
            )
        for files in ([], "not an object"):
            with self.subTest(files=files):
                self.assert_clean_failure(
                    path,
                    {"files": files},
                    "research-workflow.upstream.json: files must be an object",
                )

    def test_local_edits_block_all_writes(self):
        MODULE.sync(self.source, self.target, write=True)
        destination = self.target / "planning/workflow/README.md"
        destination.write_text("Local decisions that must survive\n")
        (self.source / "docs/workflow/README.md").write_text("Changed upstream\n")
        (self.source / "docs/workflow/new.md").write_text("new upstream file\n")
        with self.assertRaisesRegex(ValueError, "local edits"):
            MODULE.sync(self.source, self.target, write=True)
        self.assertEqual(destination.read_text(), "Local decisions that must survive\n")
        self.assertFalse((self.target / "planning/workflow/new.md").exists())

    def test_upstream_removal_is_reviewed_before_deletion(self):
        MODULE.sync(self.source, self.target, write=True)
        (self.source / "docs/workflow/README.md").unlink()
        with self.assertRaisesRegex(ValueError, "upstream removed"):
            MODULE.sync(self.source, self.target, write=True)
        self.assertTrue((self.target / "planning/workflow/README.md").exists())

    def test_destination_symlink_cannot_escape_checkout(self):
        outside = Path(self.temp.name) / "outside"
        outside.mkdir()
        (self.target / "planning").mkdir()
        (self.target / "planning/workflow").symlink_to(
            outside, target_is_directory=True
        )
        with self.assertRaisesRegex(ValueError, "escapes repository"):
            MODULE.sync(self.source, self.target, write=True)
        self.assertEqual(list(outside.iterdir()), [])


if __name__ == "__main__":
    unittest.main()
