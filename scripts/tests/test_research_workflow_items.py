# Copyright developing.today LLC & contributors to the project.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Behavioral checks for item trails, ids and generated views (research_workflow_items).

Each test drives the public CLI in a disposable repository, as the core tests do.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "research_workflow.py"
CREATED = "2026-10-19T01:02:03Z"
LATER = "2026-10-21T10:00:00Z"
LATEST = "2026-10-22T09:30:00Z"
ITEM = re.compile(r"^( *)- \*\*(C\d{3}(?:\.\d+)+)\*\* ", re.MULTILINE)


class ItemsTests(unittest.TestCase):
    """Items get tool-allocated ids, keep their trails and views stay current."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="research-items-test-")
        self.addCleanup(self.temp.cleanup)
        self.scratch = Path(self.temp.name)
        self.root = self.scratch / "repo"
        self.make_repo(self.root)

    def make_repo(self, root):
        (root / ".agents").mkdir(parents=True)
        (root / "docs/requests").mkdir(parents=True)
        (root / "docs/plans").mkdir(parents=True)
        (root / "docs/workflow.md").write_text("# Workflow\n", encoding="utf-8")
        for catalog in ("requests", "plans"):
            (root / "docs" / catalog / "README.md").write_text(
                f"# {catalog.title()}\n", encoding="utf-8"
            )
        profile = {
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
        (root / ".agents/research-workflow.json").write_text(
            json.dumps(profile, indent=2) + "\n", encoding="utf-8"
        )
        (root / ".gitattributes").write_text(
            "**/transcript.txt -text\n**/changes.md merge=union\n", encoding="utf-8"
        )

    def cli(self, *args, ok=True, root=None):
        root = root or self.root
        result = subprocess.run(
            [sys.executable, "-I", str(SCRIPT), *map(str, args), "--repo-root", root],
            cwd=root,
            text=True,
            capture_output=True,
            check=False,
        )
        if ok:
            self.assertEqual(
                result.returncode, 0, f"{args!r}\n{result.stdout}\n{result.stderr}"
            )
            return json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0, f"accepted {args!r}\n{result.stdout}")
        self.assertRegex(result.stderr, r"^error: [^\n]+\n$")
        return result.stderr

    def init(self, root=None, single=False, outline=None, slug="items"):
        root = root or self.root
        source = self.scratch / "input.txt"
        source.write_bytes(
            "First § idea: keep the source.\n\nSecond idea — own your files.\n\n"
            "Third idea: plan the order.\n".encode()
        )
        args = ["init", "--input", source, "--slug", slug, "--timestamp", CREATED]
        if single:
            args += ["--concept", "One concept"]
        else:
            path = self.scratch / "outline.json"
            path.write_text(json.dumps(outline or self.outline()), encoding="utf-8")
            args += ["--outline", path, "--allow-duplicate"]
        return Path(self.cli(*args, root=root)["package"])

    def outline(self):
        concept = lambda title, slug, line: {  # noqa: E731
            "title": title,
            "slug": slug,
            "summary": f"Summary of {title.lower()}.",
            "source_lines": [[line, line]],
            "priority": "crucial",
            "horizon": "now",
        }
        return {
            "title": "Items fixture",
            "concepts": [
                concept("Source preservation", "source", 1),
                concept("Parallel ownership", "ownership", 3),
                concept("Ordering", "ordering", 5),
            ],
            "dependencies": [
                {
                    "from": "C002",
                    "to": "C001",
                    "kind": "requires",
                    "reason": "Needs source.",
                },
                {
                    "from": "C003",
                    "to": "C001",
                    "kind": "requires",
                    "reason": "Needs source.",
                },
                {
                    "from": "C003",
                    "to": "C002",
                    "kind": "shares-files-with",
                    "reason": "Both edit the same plan.",
                },
            ],
        }

    def rendered(self, root=None, **kwargs):
        package = self.init(root, **kwargs)
        self.cli("render", "--package", package, root=root)
        return package

    def add(self, package, parent, text, stage="S0", at=LATER, root=None, ok=True):
        return self.cli(
            "item", "add", "--package", package, "--parent", parent, "--stage", stage,
            "--status", "created", "--text", text, "--at", at, root=root, ok=ok,
        )  # fmt: skip

    def set_status(self, package, item, status, note, *extra, at=LATEST, ok=True):
        return self.cli(
            "item", "set-status", "--package", package, "--id", item, "--status", status,
            "--stage", "S2", "--note", note, "--at", at, *extra, ok=ok,
        )  # fmt: skip

    def check(self, ok=True, root=None):
        return self.cli("check", root=root, ok=ok)

    def main(self, package):
        return (package / "main.md").read_text(encoding="utf-8")

    def section(self, package, concept):
        text = self.main(package)
        start = text.index(f"## {concept}: ")
        following = re.search(r"\n<a id=|\n## ", text[start + 1 :])
        return (
            text[start : start + 1 + following.start()] if following else text[start:]
        )

    # ------------------------------------------------------------ item add

    def test_add_allocates_stable_ids_and_places_children_below_their_parent(self):
        package = self.rendered()
        self.assertEqual(self.add(package, "C001", "First statement.")["id"], "C001.1")
        self.assertEqual(self.add(package, "C001", "Second statement.")["id"], "C001.2")
        self.assertEqual(
            self.add(package, "C001.1", "Refines the first.")["id"], "C001.1.1"
        )
        self.assertEqual(self.add(package, "C001.1.1", "Deeper.")["id"], "C001.1.1.1")
        self.assertEqual(self.add(package, "C001.1", "Second child.")["id"], "C001.1.2")
        self.assertEqual(self.add(package, "C002", "Other concept.")["id"], "C002.1")
        order = [
            (len(m[1]), m[2]) for m in ITEM.finditer(self.section(package, "C001"))
        ]
        self.assertEqual(
            order,
            [
                (0, "C001.1"),
                (2, "C001.1.1"),
                (4, "C001.1.1.1"),
                (2, "C001.1.2"),
                (0, "C001.2"),
            ],
        )
        self.assertIn("C002.1", self.section(package, "C002"))
        self.assertNotIn("C002.1", self.section(package, "C001"))
        self.check()

    def test_add_writes_the_exact_grammar_and_one_change_line(self):
        package = self.rendered()
        before = (package / "changes.md").read_text(encoding="utf-8")
        result = self.add(package, "C001", "The original statement.", stage="S2")
        line = "- **C001.1** `created` · S2 · 2026-10-21T10:00:00Z — The original statement."
        self.assertEqual(result["line"], line)
        self.assertIn("\n" + line + "\n", self.main(package))
        after = (package / "changes.md").read_text(encoding="utf-8")
        self.assertTrue(after.startswith(before))
        self.assertEqual(
            after[len(before) :],
            "- 2026-10-21T10:00:00Z | S2 | created | C001.1 added under C001: The original statement.\n",
        )

    def test_add_refuses_unknown_parent_bad_text_and_unrendered_packages(self):
        package = self.rendered()
        before = self.main(package)
        self.add(package, "C009", "No such concept.", ok=False)
        self.add(package, "C001.4", "No such item.", ok=False)
        self.add(package, "C001", "Line one\nline two.", ok=False)
        self.add(package, "C001", "Has ← an arrow.", ok=False)
        self.add(package, "C001", "Bad time.", at="yesterday", ok=False)
        self.cli(
            "item", "add", "--package", package, "--parent", "C001", "--stage", "S9",
            "--text", "Bad stage.", ok=False,
        )  # fmt: skip
        self.assertEqual(self.main(package), before)
        fresh = self.init(slug="second")
        self.assertIn(
            "run render first", self.add(fresh, "C001", "Too early.", ok=False)
        )

    # ------------------------------------------------------- set-status

    def test_set_status_prepends_the_trail_and_keeps_the_original_text(self):
        package = self.rendered()
        self.add(package, "C001", "The original statement.")
        self.add(package, "C001.1", "The nuance.", stage="S2", at=LATEST)
        result = self.set_status(
            package,
            "C001.1",
            "clarified",
            "The situation is more nuanced",
            "--see",
            "C001.1.1",
        )
        line = (
            "- **C001.1** `clarified` · S2 · 2026-10-22T09:30:00Z — The situation is more nuanced; "
            "see C001.1.1. ← `created` · S0 · 2026-10-21T10:00:00Z — The original statement."
        )
        self.assertEqual(result["line"], line)
        self.assertIn(line + "\n  - **C001.1.1** ", self.main(package))
        self.set_status(package, "C001.1", "confirmed", "Checked against the code.")
        text = self.main(package)
        self.assertIn(
            "`confirmed` · S2 · 2026-10-22T09:30:00Z — Checked against the code. ← `clarified`",
            text,
        )
        self.assertIn("— The original statement.", text)
        changes = (package / "changes.md").read_text(encoding="utf-8")
        self.assertIn(
            "| C001.1 created → clarified: The situation is more nuanced; see C001.1.1.\n",
            changes,
        )
        self.check()

    def test_set_status_refuses_bad_transitions_without_writing(self):
        package = self.rendered()
        self.add(package, "C001", "Statement.")
        before = self.main(package), (package / "changes.md").read_bytes()
        self.set_status(package, "C001.1", "created", "Again.", ok=False)
        self.set_status(package, "C001.1", "invented", "Unknown status.", ok=False)
        self.set_status(package, "C001.1", "superseded", "Replaced.", ok=False)
        self.set_status(
            package, "C001.1", "clarified", "See nothing.", "--see", "C001.7", ok=False
        )
        self.set_status(
            package, "C001.1", "clarified", "Too early.", at=CREATED, ok=False
        )
        self.set_status(package, "C001.5", "clarified", "No such item.", ok=False)
        self.assertEqual(
            (self.main(package), (package / "changes.md").read_bytes()), before
        )

    # -------------------------------------------------------------- check

    def test_check_rejects_broken_item_grammar_parents_ids_and_timestamps(self):
        package = self.rendered()
        self.add(package, "C001", "First.")
        self.add(package, "C001.1", "Child.")
        good = self.main(package)
        line = "- **C001.1** `created` · S0 · 2026-10-21T10:00:00Z — First."
        child = "  - **C001.1.1** `created` · S0 · 2026-10-21T10:00:00Z — Child."
        broken = {
            "unknown status": good.replace(
                line, line.replace("`created`", "`drafted`")
            ),
            "unknown stage": good.replace(line, line.replace("S0", "S9")),
            "bad timestamp": good.replace(line, line.replace("10:00:00Z", "10:00Z")),
            "missing separator": good.replace(line, line.replace(" — ", " ")),
            "old before new": good.replace(
                line, line + " ← `created` · S0 · 2026-10-22T10:00:00Z — Older."
            ),
            "before package": good.replace(
                line, line.replace("2026-10-21", "2026-10-18")
            ),
            "duplicate id": good.replace(child, child + "\n" + child),
            "orphan child": good.replace(child, child.replace("C001.1.1", "C001.2.1")),
            "wrong indent": good.replace(child, child.strip()),
            "number gap": good.replace(child, child.replace("C001.1.1", "C001.1.2")),
            "wrong concept": good.replace(line, line.replace("C001.1", "C002.1")),
            "unknown see": good.replace(" — First.", " — First; see C001.9."),
            "malformed item": good.replace(line, "- **C001.1** created · S0 — First."),
        }
        for name, text in broken.items():
            with self.subTest(name):
                (package / "main.md").write_text(text, encoding="utf-8")
                self.check(ok=False)
        (package / "main.md").write_text(good, encoding="utf-8")
        self.check()

    def test_example_grammar_from_the_contract_is_accepted(self):
        package = self.rendered()
        self.add(package, "C001", "Seed.")
        self.add(package, "C001", "Seed two.")
        text = self.main(package).replace(
            "- **C001.2** `created` · S0 · 2026-10-21T10:00:00Z — Seed two.",
            "- **C001.2** `clarified` · S2 · 2026-10-21T10:00:00Z — The situation is more nuanced; "
            "see C001.2.1. ← `created` · S0 · 2026-10-19T06:09:02Z\n"
            "  - **C001.2.1** `created` · S2 · 2026-10-21T10:00:00Z — The nuance, written directly "
            "below the item it refines.",
        )
        (package / "main.md").write_text(text, encoding="utf-8")
        self.cli("render", "--package", package)
        self.check()

    # ---------------------------------------------------------------- ids

    def test_ids_lists_concepts_items_notes_and_decisions(self):
        package = self.rendered()
        self.add(package, "C001", "First.")
        self.add(package, "C001.1", "Child.")
        note = self.scratch / "note.txt"
        note.write_text("A note.\n", encoding="utf-8")
        self.cli(
            "note",
            "--package",
            package,
            "--from",
            "C001",
            "--to",
            "C002",
            "--input",
            note,
        )
        (package / "decisions").mkdir()
        (package / "decisions/D001-pick-format.md").write_text(
            "# D001\n", encoding="utf-8"
        )
        result = self.cli("ids", "--package", package)
        self.assertEqual(
            [c["id"] for c in result["concepts"]], ["C001", "C002", "C003"]
        )
        self.assertEqual(
            result["items"],
            [
                {"id": "C001.1", "status": "created", "stage": "S0", "at": LATER},
                {"id": "C001.1.1", "status": "created", "stage": "S0", "at": LATER},
            ],
        )
        self.assertEqual(len(result["notes"]), 1)
        self.assertTrue(result["notes"][0].startswith("C001__to__C002__"))
        self.assertEqual(result["decisions"], ["D001"])
        self.assertEqual(result["segments"], [])
        self.assertEqual(result["reserved"], ["G"])

    # ------------------------------------------------------------- render

    def test_render_is_idempotent_and_replaces_init_placeholders(self):
        package = self.rendered()
        main, design = (
            self.main(package),
            (package / "design.md").read_text(encoding="utf-8"),
        )
        self.cli("render", "--package", package)
        self.assertEqual(self.main(package), main)
        self.assertEqual((package / "design.md").read_text(encoding="utf-8"), design)
        for name in ("concept-C001", "concept-C002", "concept-C003", "issues-and-prs"):
            self.assertEqual(main.count(f"<!-- workflow:view {name} -->"), 1, name)
            self.assertEqual(main.count(f"<!-- /workflow:view {name} -->"), 1, name)
        for name in ("waves", "run-status"):
            self.assertEqual(design.count(f"<!-- workflow:view {name} -->"), 1, name)
        self.assertNotIn("Related issue/PR results: pending", main)
        self.assertNotIn("Current status: created", main)
        self.assertNotIn("None created.", main)
        self.assertNotIn("Ready frontier:", design)
        self.assertNotIn("|", main.split("<!-- workflow:view", 1)[1])
        self.assertIn("- **Required by:** [C002](#c002)", self.section(package, "C001"))
        self.assertIn("- **Wave 1:** [C001](main.md#c001)", design)
        self.assertIn("- **Wave 2:** [C002](main.md#c002)", design)
        self.assertIn("- **Wave 3:** [C003](main.md#c003)", design)
        self.assertIn(
            "- **C002** [Parallel ownership](main.md#c002): S0 `validated` · S1 `blocked` (requires C001) · S2–S8 `not-started`",
            design,
        )
        self.check()

    def test_template_generated_views_render_and_check(self):
        repo = Path(__file__).resolve().parents[2]
        main_template = (repo / "planning/workflow/templates/main.md").read_text(
            encoding="utf-8"
        )
        design_template = (repo / "planning/workflow/templates/design.md").read_text(
            encoding="utf-8"
        )

        def template_views(text):
            pattern = re.compile(
                r"<!-- workflow:view (?P<name>[A-Za-z0-9._-]+) -->.*?"
                r"<!-- /workflow:view (?P=name) -->",
                re.DOTALL,
            )
            return {match["name"]: match[0] for match in pattern.finditer(text)}

        main_views = template_views(main_template)
        design_views = template_views(design_template)
        self.assertEqual(set(main_views), {"concept-C001", "issues-and-prs"})
        self.assertEqual(set(design_views), {"waves", "run-status"})
        self.assertIn("## C001: <title>", main_template)
        self.assertIn("- **State:**", main_template)
        self.assertIn("Issue [#<n>]", main_template)
        self.assertIn("PR [#<n>]", main_template)
        self.assertNotIn("- None yet.", main_template)
        self.assertIn(
            "## Review status\n\n<!-- workflow:view run-status -->", design_template
        )
        self.assertNotIn("run-table", design_template)

        package = self.rendered()

        def install_template_views(path, replacements):
            text = path.read_text(encoding="utf-8")
            for name, block in replacements.items():
                pattern = re.compile(
                    rf"<!-- workflow:view {re.escape(name)} -->.*?"
                    rf"<!-- /workflow:view {re.escape(name)} -->",
                    re.DOTALL,
                )
                text, count = pattern.subn(
                    lambda _, replacement=block: replacement, text, count=1
                )
                self.assertEqual(count, 1, name)
            path.write_text(text, encoding="utf-8")

        install_template_views(package / "main.md", main_views)
        install_template_views(package / "design.md", design_views)
        self.cli("render", "--package", package)
        self.check()

    def test_check_fails_when_a_view_is_stale_missing_or_unknown(self):
        package = self.rendered()
        self.check()
        main = package / "main.md"
        good = main.read_text(encoding="utf-8")
        state = re.search(
            r"<!-- workflow:state -->\n(.*?)\n<!-- /workflow:state -->", good, re.S
        )
        data = json.loads(state[1])
        data["concepts"][0]["status"] = "clarified"
        main.write_text(
            good.replace(state[1], json.dumps(data, indent=2)), encoding="utf-8"
        )
        self.assertIn("concept-C001 is stale", self.check(ok=False))
        self.cli("render", "--package", package)
        self.check()
        fresh = main.read_text(encoding="utf-8")
        hand_item = fresh.replace(
            "<!-- /workflow:view concept-C002 -->",
            "<!-- /workflow:view concept-C002 -->\n\n"
            "- **C002.1** `created` · S0 · 2026-10-21T10:00:00Z — Written by hand.",
        )
        main.write_text(hand_item, encoding="utf-8")
        self.assertIn("concept-C002 is stale", self.check(ok=False))
        start = fresh.index("<!-- workflow:view issues-and-prs -->")
        end = fresh.index("<!-- /workflow:view issues-and-prs -->") + len(
            "<!-- /workflow:view issues-and-prs -->"
        )
        main.write_text(fresh[:start] + fresh[end:], encoding="utf-8")
        self.assertIn("issues-and-prs is missing", self.check(ok=False))
        main.write_text(
            fresh
            + "\n<!-- workflow:view extra -->\n\nx\n\n<!-- /workflow:view extra -->\n",
            encoding="utf-8",
        )
        self.assertIn("unknown generated view extra", self.check(ok=False))
        main.write_text(fresh, encoding="utf-8")
        design = package / "design.md"
        design.write_text(
            design.read_text(encoding="utf-8").replace("**Wave 2:**", "**Wave 9:**"),
            encoding="utf-8",
        )
        self.assertIn("waves is stale", self.check(ok=False))

    def test_issues_and_prs_render_inline_and_grouped_at_the_bottom(self):
        package = self.rendered()
        main = package / "main.md"
        text = main.read_text(encoding="utf-8")
        state = re.search(
            r"<!-- workflow:state -->\n(.*?)\n<!-- /workflow:state -->", text, re.S
        )
        data = json.loads(state[1])
        url = "https://github.com/developing-today/code/issues/12"
        data["issues"] = [
            {
                "concept": "C002",
                "number": 12,
                "url": url,
                "stage": "S1",
                "state": "open",
                "title": "Research: Parallel *ownership* [x]",
                "item": "C002.1",
            }
        ]
        data["tasks"] = [
            {
                "id": "R1",
                "concept": "C002",
                "stage": "S1",
                "status": "running",
                "owner": "alpha",
                "paths": ["concepts/C002-ownership.md"],
            }
        ]
        main.write_text(
            text.replace(state[1], json.dumps(data, indent=2)), encoding="utf-8"
        )
        self.cli("render", "--package", package)
        entry = f"Issue [#12]({url}) · S1 · `open` — Research: Parallel \\*ownership\\* \\[x\\] (for C002.1)"
        self.assertIn("  - " + entry, self.section(package, "C002"))
        bottom = self.main(package).split("## Issues and PRs", 1)[1]
        self.assertIn("- [C002](#c002) Parallel ownership\n  - " + entry, bottom)
        design = (package / "design.md").read_text(encoding="utf-8")
        self.assertIn(f"S1 `running` ([#12]({url}))", design)
        self.check()
        data["tasks"] = [
            {"id": "R1", "concept": "C002", "stage": "S1", "status": "merged"}
        ]
        text = self.main(package)
        state = re.search(
            r"<!-- workflow:state -->\n(.*?)\n<!-- /workflow:state -->", text, re.S
        )
        main.write_text(
            text.replace(state[1], json.dumps(data, indent=2)), encoding="utf-8"
        )
        self.assertIn("status --live", self.check(ok=False))

    def test_single_concept_package_gets_a_section_for_items(self):
        package = self.rendered(single=True)
        self.assertIn('<a id="c001"></a>\n\n## C001: One concept\n', self.main(package))
        self.assertEqual(self.add(package, "C001", "Single.")["id"], "C001.1")
        self.assertTrue((package / "changes.md").is_file())
        self.check()

    def test_render_keeps_crlf_excerpt_bytes(self):
        source = self.scratch / "crlf.txt"
        source.write_bytes("Keep this\r\nexact.\r\n".encode())
        package = Path(
            self.cli("init", "--input", source, "--slug", "crlf", "--concept", "Exact")[
                "package"
            ]
        )
        before = (package / "main.md").read_bytes()
        self.cli("render", "--package", package)
        self.assertIn(
            "Keep this\r\nexact.\r\n".encode(), (package / "main.md").read_bytes()
        )
        self.assertNotEqual((package / "main.md").read_bytes(), before)
        self.check()

    # ---------------------------------------------------- formatter output

    @unittest.skipUnless(shutil.which("prettier"), "prettier is not installed")
    def test_rendered_views_and_items_are_prettier_stable(self):
        # init's own state block is reflowed by prettier (core, not this module), so
        # format once as the pre-commit hook would, then everything after must hold.
        package = self.init()
        subprocess.run(
            [
                "prettier",
                "--write",
                str(package / "main.md"),
                str(package / "design.md"),
            ],
            cwd=self.root,
            capture_output=True,
            check=True,
        )
        self.cli("render", "--package", package)
        self.add(package, "C001", "Keep `code`, snake_case and a link [docs](main.md).")
        self.add(package, "C001.1", "A child.", stage="S2", at=LATEST)
        self.set_status(
            package, "C001.1", "clarified", "More nuance", "--see", "C001.1.1"
        )
        files = [package / name for name in ("main.md", "design.md", "changes.md")]
        result = subprocess.run(
            ["prettier", "--check", *map(str, files)],
            cwd=self.root,
            capture_output=True,
            text=True,
            check=False,
        )
        if result.returncode:
            for path in files:
                formatted = subprocess.run(
                    ["prettier", "--stdin-filepath", str(path)],
                    input=path.read_text(encoding="utf-8"),
                    capture_output=True,
                    text=True,
                    check=True,
                ).stdout
                self.assertEqual(path.read_text(encoding="utf-8"), formatted, path.name)

    # ------------------------------------------------------ parallel merge

    @unittest.skipUnless(shutil.which("git"), "git is not installed")
    def test_item_edits_on_two_concepts_merge_cleanly_from_two_branches(self):
        package = self.rendered()

        def git(*args):
            env = {
                **{k: v for k, v in os.environ.items() if not k.startswith("GIT_")},
                "GIT_AUTHOR_NAME": "test",
                "GIT_AUTHOR_EMAIL": "test@example.invalid",
                "GIT_COMMITTER_NAME": "test",
                "GIT_COMMITTER_EMAIL": "test@example.invalid",
                "GIT_CONFIG_GLOBAL": os.devnull,
                "GIT_CONFIG_NOSYSTEM": "1",
            }
            return subprocess.run(
                ["git", *args],
                cwd=self.root,
                env=env,
                text=True,
                capture_output=True,
                check=True,
            ).stdout

        git("init", "-q", "-b", "main")
        git("add", "-A")
        git("commit", "-q", "-m", "package")
        git("checkout", "-q", "-b", "c001")
        self.add(package, "C001", "From the C001 owner.")
        self.add(package, "C001.1", "A refinement.")
        git("commit", "-q", "-am", "c001 items")
        git("checkout", "-q", "main")
        git("checkout", "-q", "-b", "c002")
        self.add(package, "C002", "From the C002 owner.")
        self.set_status(package, "C002.1", "confirmed", "Checked.")
        git("commit", "-q", "-am", "c002 items")
        git("checkout", "-q", "c001")
        git("merge", "-q", "--no-edit", "c002")
        self.assertEqual(git("status", "--porcelain"), "")
        text = self.main(package)
        for item in ("C001.1", "C001.1.1", "C002.1"):
            self.assertIn(f"**{item}**", text)
        changes = (package / "changes.md").read_text(encoding="utf-8")
        self.assertEqual(changes.count(" | C00"), 4)
        self.check()


if __name__ == "__main__":
    unittest.main()
