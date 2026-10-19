# Copyright developing.today LLC & contributors to the project.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Behavioral checks for the research intake CLI, using disposable repositories.

These exercise preservation, atomic refusal, graph and ownership safety, and
cross-agent handoffs. They do not depend on incidental template prose.
"""

from __future__ import annotations

import hashlib
import json
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "research_workflow.py"
TIMESTAMP = "2026-10-19T01:02:03Z"
STATE_START = "<!-- workflow:state -->"
STATE_END = "<!-- /workflow:state -->"


class ResearchWorkflowTests(unittest.TestCase):
    """The public CLI must preserve requests and refuse unsafe workflow state."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="research-workflow-test-")
        self.addCleanup(self.temp.cleanup)
        self.scratch = Path(self.temp.name)
        self.root = self.scratch / "repo"
        (self.root / ".agents").mkdir(parents=True)
        (self.root / "docs/requests").mkdir(parents=True)
        (self.root / "docs/plans").mkdir(parents=True)
        (self.root / "docs/README.md").write_text(
            "# Documentation\n\n[Workflow](workflow.md)\n"
            "[Requests](requests/README.md)\n[Plans](plans/README.md)\n",
            encoding="utf-8",
        )
        (self.root / "docs/workflow.md").write_text(
            "# Research workflow\n\n## Intake\n\nPreserve source material.\n",
            encoding="utf-8",
        )
        for catalog in ("requests", "plans"):
            (self.root / "docs" / catalog / "README.md").write_text(
                f"# {catalog.title()}\n", encoding="utf-8"
            )
        self.profile = {
            "version": 1,
            "workflow_docs": "docs/workflow.md",
            "requests_root": "docs/requests",
            "plans_root": "docs/plans",
            "reference_roots": ["docs"],
            "request_catalog": "docs/requests/README.md",
            "plan_catalog": "docs/plans/README.md",
            "issue_repository": "developing-today/code",
            "trunk": "main",
            "validation_commands": [],
        }
        self.write_profile()
        self.raw = (
            "First § idea: preserve café, typos, and bad TTS??\r\n"
            "\r\n"
            "Second α idea — parallel agents must keep their own files.\r\n"
        ).encode("utf-8")
        self.input = self.scratch / "input.txt"
        self.input.write_bytes(self.raw)

    def write_profile(self):
        (self.root / ".agents/research-workflow.json").write_text(
            json.dumps(self.profile, indent=2) + "\n", encoding="utf-8"
        )

    def cli(self, *args, ok=True):
        result = subprocess.run(
            [sys.executable, str(SCRIPT), *map(str, args)],
            cwd=self.root,
            text=True,
            capture_output=True,
            check=False,
        )
        if ok:
            self.assertEqual(
                result.returncode,
                0,
                f"CLI failed: {args!r}\n{result.stdout}\n{result.stderr}",
            )
        else:
            self.assertNotEqual(
                result.returncode,
                0,
                f"CLI accepted invalid input: {args!r}\n{result.stdout}",
            )
        return result

    def outline(self):
        return {
            "title": "Preserve ideas and coordinate future work",
            "introduction": "A first pass over a noisy request, before full research.",
            "concepts": [
                {
                    "title": "Exact source preservation",
                    "slug": "source-preservation",
                    "summary": "Keep the original request recoverable.",
                    "source_lines": [[1, 1]],
                    "thoughts": ["Record normalization separately from source."],
                    "suggestions": ["Consider checksums as a provenance aid."],
                },
                {
                    "title": "Parallel ownership",
                    "slug": "parallel-ownership",
                    "summary": "Give each writer a dedicated artifact.",
                    "source_lines": [[3, 3]],
                    "thoughts": ["Make ownership visible before dispatch."],
                    "suggestions": [],
                },
            ],
            "dependencies": [
                {
                    "from": "C002",
                    "to": "C001",
                    "kind": "requires",
                    "reason": "Stable source material is needed by each handoff.",
                }
            ],
        }

    def init(self, outline=None, slug="assorted-ideas", single=False, ok=True):
        args = [
            "init", "--repo-root", self.root, "--input", self.input,
            "--slug", slug, "--timestamp", TIMESTAMP,
        ]
        if single:
            args += ["--concept", "One coherent concept"]
        else:
            self.outline_file = self.scratch / "outline.json"
            self.outline_file.write_text(
                json.dumps(outline if outline is not None else self.outline()),
                encoding="utf-8",
            )
            args += ["--outline", self.outline_file]
        result = self.cli(*args, ok=ok)
        if not ok:
            return result
        payload = json.loads(result.stdout)
        package = Path(payload["package"])
        if not package.is_absolute():
            package = self.root / package
        self.assertTrue(package.is_dir())
        self.assertTrue(package.is_relative_to(self.root))
        return package

    def state(self, package):
        text = (package / "main.md").read_text(encoding="utf-8")
        # The authoritative block precedes excerpts; raw source may itself
        # contain literal examples of workflow markers which must be ignored.
        raw = text.split(STATE_START, 1)[1].split(STATE_END, 1)[0].strip()
        return json.loads(raw)

    def write_state(self, package, state):
        main = package / "main.md"
        text = main.read_text(encoding="utf-8")
        before, remainder = text.split(STATE_START, 1)
        _, after = remainder.split(STATE_END, 1)
        main.write_text(
            before + STATE_START + "\n" + json.dumps(state, indent=2)
            + "\n" + STATE_END + after,
            encoding="utf-8",
        )

    def check(self, package, ok=True, *extra):
        return self.cli(
            "check", "--repo-root", self.root, "--package", package,
            *extra, ok=ok,
        )

    def snapshot(self):
        return {
            str(path.relative_to(self.root)): path.read_bytes()
            for path in self.root.rglob("*") if path.is_file()
        }

    def create_note(self, package):
        body = self.scratch / "cross-agent-note.txt"
        body.write_bytes(b"Keep the original sender's evidence intact.\n")
        result = self.cli(
            "note", "--repo-root", self.root, "--package", package,
            "--from", "C001", "--to", "C002", "--input", body,
        )
        return json.loads(result.stdout)["note"]

    def test_multi_concept_preserves_exact_source_and_relevant_unicode_excerpts(self):
        package = self.init()
        self.assertEqual((package / "transcript.txt").read_bytes(), self.raw)
        state = self.state(package)
        self.assertEqual(state["version"], 1)
        self.assertEqual(state["created_at"], TIMESTAMP)
        self.assertEqual(state["stage"], "intake")
        self.assertEqual(state["status"], "prepared")
        self.assertEqual(state["transcript"]["sha256"], hashlib.sha256(self.raw).hexdigest())
        self.assertEqual(len(state["concepts"]), 2)
        for concept, line in zip(state["concepts"], self.raw.splitlines(keepends=True)[::2]):
            document = package / concept["path"]
            self.assertIn(line, document.read_bytes())
            self.assertEqual(concept["status"], "created")
        for name in ("main.md", "transcript.txt", "design.md", "changes.md"):
            self.assertTrue((package / name).is_file(), name)
        self.check(package)

    def test_single_concept_uses_main_without_unnecessary_meta_hierarchy(self):
        package = self.init(single=True)
        state = self.state(package)
        self.assertEqual(len(state["concepts"]), 1)
        self.assertEqual(state["concepts"][0]["path"], "main.md")
        self.assertFalse((package / "design.md").exists())
        self.assertFalse((package / "changes.md").exists())
        self.assertFalse((package / "concepts").exists())
        self.assertEqual((package / "transcript.txt").read_bytes(), self.raw)
        self.check(package)

    def test_uncovered_nonblank_input_is_rejected_without_partial_writes(self):
        outline = self.outline()
        outline["concepts"][1]["source_lines"] = [[2, 2]]
        before = self.snapshot()
        self.init(outline=outline, ok=False)
        self.assertEqual(self.snapshot(), before)

    def test_source_line_range_outside_transcript_is_rejected_atomically(self):
        outline = self.outline()
        outline["concepts"][0]["source_lines"] = [[0, 99]]
        before = self.snapshot()
        self.init(outline=outline, ok=False)
        self.assertEqual(self.snapshot(), before)

    def test_duplicate_concept_slugs_are_rejected_before_any_creation(self):
        outline = self.outline()
        outline["concepts"][1]["slug"] = outline["concepts"][0]["slug"]
        before = self.snapshot()
        self.init(outline=outline, ok=False)
        self.assertEqual(self.snapshot(), before)

    def test_existing_package_collision_never_overwrites_completed_intake(self):
        self.init()
        before = self.snapshot()
        self.init(ok=False)
        self.assertEqual(self.snapshot(), before)

    def test_parent_path_slug_is_rejected_without_writing_outside_root(self):
        before = self.snapshot()
        self.init(slug="../../escape", ok=False)
        self.assertEqual(self.snapshot(), before)
        self.assertFalse((self.scratch / "escape").exists())

    def test_profile_path_escape_is_rejected_before_writes(self):
        self.profile["requests_root"] = "../outside"
        self.write_profile()
        before = self.snapshot()
        self.init(ok=False)
        self.assertEqual(self.snapshot(), before)
        self.assertFalse((self.scratch / "outside").exists())

    def test_transcript_tampering_invalidates_provenance(self):
        package = self.init()
        (package / "transcript.txt").write_bytes(self.raw + b"Not the original request.\n")
        self.check(package, ok=False)

    def test_relevant_excerpt_cannot_be_silently_cleaned_up(self):
        package = self.init()
        concept = self.state(package)["concepts"][0]
        document = package / concept["path"]
        content = document.read_bytes()
        changed = content.replace(b"bad TTS??", b"edited transcription")
        self.assertNotEqual(content, changed)
        document.write_bytes(changed)
        self.check(package, ok=False)

    def test_missing_concept_document_is_reported(self):
        package = self.init()
        document = package / self.state(package)["concepts"][1]["path"]
        document.unlink()
        self.check(package, ok=False)

    def test_missing_local_link_is_rejected(self):
        package = self.init()
        document = package / self.state(package)["concepts"][0]["path"]
        with document.open("a", encoding="utf-8") as handle:
            handle.write("\n[Missing source](missing-evidence.md)\n")
        self.check(package, ok=False)

    def test_missing_local_anchor_is_rejected(self):
        package = self.init()
        document = package / self.state(package)["concepts"][0]["path"]
        with document.open("a", encoding="utf-8") as handle:
            handle.write("\n[Missing section](../main.md#this-heading-does-not-exist)\n")
        self.check(package, ok=False)

    def test_hard_dependency_cycle_is_rejected_before_writes(self):
        outline = self.outline()
        outline["dependencies"].append(
            {"from": "C001", "to": "C002", "kind": "requires", "reason": "Cycle."}
        )
        before = self.snapshot()
        self.init(outline=outline, ok=False)
        self.assertEqual(self.snapshot(), before)

    def test_hard_dependency_cycle_added_later_is_detected(self):
        package = self.init()
        state = self.state(package)
        state["dependencies"].append(
            {"from": "C001", "to": "C002", "kind": "requires", "reason": "Cycle."}
        )
        self.write_state(package, state)
        self.check(package, ok=False)

    def test_soft_ordering_cycle_does_not_become_a_hard_blocker(self):
        outline = self.outline()
        outline["dependencies"] = [
            {"from": "C001", "to": "C002", "kind": "recommended-after", "reason": "Helpful order."},
            {"from": "C002", "to": "C001", "kind": "recommended-after", "reason": "Alternative order."},
        ]
        package = self.init(outline=outline)
        self.check(package)

    def test_active_owners_cannot_claim_same_file(self):
        package = self.init()
        state = self.state(package)
        path = state["concepts"][0]["path"]
        state["tasks"] = [
            {"id": "T1", "owner": "alpha", "status": "running", "paths": [path]},
            {"id": "T2", "owner": "beta", "status": "running", "paths": [path]},
        ]
        self.write_state(package, state)
        self.check(package, ok=False)

    def test_active_directory_owner_conflicts_with_nested_file_owner(self):
        package = self.init()
        state = self.state(package)
        state["tasks"] = [
            {"id": "T1", "owner": "alpha", "status": "running", "paths": ["concepts/"]},
            {"id": "T2", "owner": "beta", "status": "running", "paths": [state["concepts"][0]["path"]]},
        ]
        self.write_state(package, state)
        self.check(package, ok=False)

    def test_distinct_active_files_and_finished_previous_owner_are_allowed(self):
        package = self.init()
        state = self.state(package)
        first, second = [concept["path"] for concept in state["concepts"]]
        state["tasks"] = [
            {"id": "T0", "owner": "previous", "status": "completed", "paths": [first]},
            {"id": "T1", "owner": "alpha", "status": "running", "paths": [first]},
            {"id": "T2", "owner": "beta", "status": "running", "paths": [second]},
        ]
        self.write_state(package, state)
        self.check(package)

    def test_unresolved_note_blocks_reconciliation_until_receiver_records_disposition(self):
        package = self.init()
        content = self.scratch / "note.txt"
        content.write_text("Please account for the ownership constraint.\n", encoding="utf-8")
        before = set(package.rglob("*.md"))
        self.cli(
            "note", "--repo-root", self.root, "--package", package,
            "--from", "C001", "--to", "C002", "--input", content,
        )
        created = set(package.rglob("*.md")) - before
        self.assertEqual(len(created), 1, "Each cross-agent note needs one dedicated artifact.")
        note = created.pop()
        raw_note = note.read_bytes()
        receiver = package / self.state(package)["concepts"][1]["path"]
        self.check(package, False, "--require-resolved")
        self.cli(
            "resolve-note", "--package", package,
            "--note", note.relative_to(package),
            "--disposition", "acknowledged",
            "--target", self.state(package)["concepts"][1]["path"],
            "--reason", "Kept as an explicit research constraint for the next stage.",
        )
        self.assertEqual(note.read_bytes(), raw_note, "Resolving cannot rewrite the sender's note.")
        self.assertIn("<!-- workflow:resolution -->", receiver.read_text(encoding="utf-8"))
        self.check(package, True, "--require-resolved")

    def test_completed_concept_cannot_still_be_an_unfilled_generated_stub(self):
        package = self.init()
        state = self.state(package)
        state["concepts"][0]["status"] = "completed"
        self.write_state(package, state)
        self.check(package, ok=False)

    def test_status_and_readiness_commands_can_resume_from_saved_artifacts(self):
        package = self.init()
        self.cli("status", "--repo-root", self.root, "--package", package)
        self.cli("ready", "--repo-root", self.root, "--package", package)

    def test_literal_workflow_markers_in_fenced_raw_input_are_preserved_as_evidence(self):
        self.raw += (
            "```example\r\n"
            "<!-- workflow:state -->\r\n"
            '{"version": 99, "status": "fictional example"}\r\n'
            "<!-- /workflow:state -->\r\n"
            "<!-- workflow:resolution -->\r\n"
            '{"note": "not-a-real-note", "disposition": "imaginary"}\r\n'
            "<!-- /workflow:resolution -->\r\n"
            "```\r\n"
        ).encode("utf-8")
        self.input.write_bytes(self.raw)
        package = self.init(single=True)
        self.assertEqual((package / "transcript.txt").read_bytes(), self.raw)
        self.assertIn(self.raw, (package / "main.md").read_bytes())
        self.assertEqual(self.state(package)["version"], 1)
        self.check(package, True, "--require-resolved")

    def test_note_raw_body_tampering_is_detected_without_changing_its_metadata(self):
        package = self.init()
        body = self.scratch / "note-body.txt"
        body.write_bytes("Keep § this exact suggestion.\r\n".encode("utf-8"))
        result = self.cli(
            "note", "--repo-root", self.root, "--package", package,
            "--from", "C001", "--to", "C002", "--input", body,
        )
        note = package / json.loads(result.stdout)["note"]
        self.check(package)
        original = note.read_bytes()
        changed = original.replace(b"exact suggestion", b"altered suggestion")
        self.assertNotEqual(original, changed)
        note.write_bytes(changed)
        self.check(package, ok=False)

    def test_requests_root_symlink_cannot_escape_repository(self):
        outside = self.scratch / "outside"
        outside.mkdir()
        sentinel = outside / "existing.txt"
        sentinel.write_bytes(b"Existing external work must survive.\n")
        (self.root / "docs/escape").symlink_to(outside, target_is_directory=True)
        self.profile["requests_root"] = "docs/escape"
        self.write_profile()
        before = self.snapshot()
        self.init(ok=False)
        self.assertEqual(self.snapshot(), before)
        self.assertEqual(list(outside.iterdir()), [sentinel])
        self.assertEqual(sentinel.read_bytes(), b"Existing external work must survive.\n")

    def test_workflow_document_directory_checks_links_in_each_nested_document(self):
        package = self.init()
        directory = self.root / "docs/workflow-reference/nested"
        directory.mkdir(parents=True)
        (directory / "guide.md").write_text(
            "# Guide\n\n[Missing reference](unwritten.md)\n", encoding="utf-8"
        )
        self.profile["workflow_docs"] = "docs/workflow-reference"
        self.write_profile()
        self.check(package, ok=False)

    def test_resolving_tampered_note_refuses_before_mutating_receiver(self):
        package = self.init()
        relative = self.create_note(package)
        note = package / relative
        original = note.read_bytes()
        changed = original.replace(b"sender's evidence", b"rewritten evidence")
        self.assertNotEqual(original, changed)
        note.write_bytes(changed)
        before = self.snapshot()
        self.cli(
            "resolve-note", "--package", package, "--note", relative,
            "--disposition", "acknowledged",
            "--target", self.state(package)["concepts"][1]["path"],
            "--reason", "This must not be recorded against corrupted source.",
            ok=False,
        )
        self.assertEqual(self.snapshot(), before)

    def test_resolving_note_to_missing_anchor_refuses_before_mutating_receiver(self):
        package = self.init()
        relative = self.create_note(package)
        before = self.snapshot()
        target = self.state(package)["concepts"][1]["path"]
        self.cli(
            "resolve-note", "--package", package, "--note", relative,
            "--disposition", "incorporated",
            "--target", target + "#heading-that-was-never-written",
            "--reason", "A missing section cannot substantiate incorporation.",
            ok=False,
        )
        self.assertEqual(self.snapshot(), before)

    def test_check_all_packages_detects_deleted_main_reference(self):
        package = self.init()
        (package / "main.md").unlink()
        self.cli("check", "--repo-root", self.root, ok=False)

    def test_resolution_of_nonexistent_note_cannot_silently_pass_reconciliation(self):
        package = self.init()
        target = self.state(package)["concepts"][1]["path"]
        entry = {
            "note": "notes/C001__to__C002__never-created.md",
            "at": TIMESTAMP,
            "disposition": "acknowledged",
            "target": target,
            "reason": "This note does not exist and must not count as acknowledged.",
        }
        with (package / target).open("a", encoding="utf-8") as handle:
            handle.write(
                "\n<!-- workflow:resolution -->\n" + json.dumps(entry)
                + "\n<!-- /workflow:resolution -->\n"
            )
        self.check(package, False, "--require-resolved")

    def test_profile_rejects_lifecycle_skill_description_over_standard_limit(self):
        package = self.init()
        skill = self.root / ".agents/skills/intake"
        skill.mkdir(parents=True)
        self.profile["lifecycle_skills"] = ["intake"]
        self.write_profile()
        document = skill / "SKILL.md"
        document.write_text(
            "---\nname: intake\ndescription: Capture a research request.\n---\n# Intake\n",
            encoding="utf-8",
        )
        self.check(package)
        document.write_text(
            "---\nname: intake\ndescription: " + "x" * 1025 + "\n---\n# Intake\n",
            encoding="utf-8",
        )
        self.check(package, ok=False)

    def test_profile_rejects_malformed_trigger_fixture(self):
        package = self.init()
        triggers = self.root / "docs/workflow-triggers.json"
        self.profile["triggers"] = "docs/workflow-triggers.json"
        self.write_profile()
        triggers.write_text(
            json.dumps([{"skill": "intake", "should_load": [], "should_not_load": []}]),
            encoding="utf-8",
        )
        self.check(package, ok=False)


if __name__ == "__main__":
    unittest.main()
