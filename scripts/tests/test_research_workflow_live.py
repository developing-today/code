# Copyright developing.today LLC & contributors to the project.
# SPDX-License-Identifier: MIT OR Apache-2.0
"""Behavioral checks for live coordination (research_workflow_live).

Each test drives the public CLI in a disposable git repository. Git runs with
the caller's GIT_* variables removed and no global or system config, so the
repository under test is the temporary one and nothing else.
"""

from __future__ import annotations

import json
import os
from pathlib import Path
import re
import subprocess
import sys
import tempfile
import unittest


SCRIPT = Path(__file__).resolve().parents[1] / "research_workflow.py"
CREATED = "2026-10-19T01:02:03Z"
STATE = re.compile(
    r"(<!-- workflow:state -->\n)(.*?)(\n<!-- /workflow:state -->)", re.S
)
ALL = ["C001", "C002", "C003", "C004"]


def clean_env(**extra):
    env = {
        key: value
        for key, value in os.environ.items()
        if not key.startswith(("GIT_", "RESEARCH_WORKFLOW_"))
    }
    env.update(GIT_CONFIG_GLOBAL=os.devnull, GIT_CONFIG_NOSYSTEM="1", **extra)
    return env


def edge(source, kind, target):
    return {"from": source, "to": target, "kind": kind, "reason": f"{kind} fixture."}


class LiveTests(unittest.TestCase):
    """`ready` keeps taken and hint concepts out and orders the rest into waves."""

    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="research-live-test-")
        self.addCleanup(self.temp.cleanup)
        self.scratch = Path(self.temp.name)
        self.root = self.scratch / "repo"
        self.make_repo(self.root)
        self.package = self.init()

    def make_repo(self, root):
        (root / ".agents").mkdir(parents=True)
        (root / "docs/requests").mkdir(parents=True)
        (root / "docs/plans").mkdir(parents=True)
        (root / "docs/workflow.md").write_text("# Workflow\n", encoding="utf-8")
        for catalog in ("requests", "plans"):
            (root / "docs" / catalog / "README.md").write_text(
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
        self.write_profile(root)
        self.git("init", "-q", cwd=root)

    def write_profile(self, root=None):
        (root or self.root).joinpath(".agents/research-workflow.json").write_text(
            json.dumps(self.profile, indent=2) + "\n", encoding="utf-8"
        )

    def git(self, *args, cwd=None):
        return subprocess.run(
            ["git", "-c", "user.name=Test", "-c", "user.email=test@example.com"]
            + ["-c", "commit.gpgsign=false", *args],
            cwd=cwd or self.root,
            env=clean_env(),
            text=True,
            capture_output=True,
            check=True,
        ).stdout.strip()

    def run_cli(self, *args, now=None, cwd=None):
        env = clean_env(**({"RESEARCH_WORKFLOW_NOW": now} if now else {}))
        cwd = cwd or self.root
        return subprocess.run(
            [sys.executable, "-I", str(SCRIPT), *map(str, args), "--repo-root", cwd],
            cwd=cwd,
            env=env,
            text=True,
            capture_output=True,
            check=False,
        )

    def cli(self, *args, ok=True, now=None, cwd=None):
        result = self.run_cli(*args, now=now, cwd=cwd)
        if ok:
            self.assertEqual(
                result.returncode, 0, f"{args!r}\n{result.stdout}\n{result.stderr}"
            )
            return json.loads(result.stdout)
        self.assertNotEqual(result.returncode, 0, f"accepted {args!r}\n{result.stdout}")
        self.assertRegex(result.stderr, r"^error: [^\n]+\n$")
        return result.stderr

    def init(self):
        source = self.scratch / "input.txt"
        source.write_text("One.\n\nTwo.\n\nThree.\n\nFour.\n", encoding="utf-8")
        outline = {
            "title": "Live fixture",
            "concepts": [
                {
                    "title": f"Concept {n}",
                    "slug": f"concept-{n}",
                    "summary": f"Summary {n}.",
                    "source_lines": [[2 * n - 1, 2 * n - 1]],
                    "thoughts": [f"Thought {n}."],
                }
                for n in range(1, 5)
            ],
            "dependencies": [],
        }
        path = self.scratch / "outline.json"
        path.write_text(json.dumps(outline), encoding="utf-8")
        args = ["init", "--input", source, "--slug", "live", "--timestamp", CREATED]
        return Path(self.cli(*args, "--outline", path)["package"])

    def state(self):
        text = (self.package / "main.md").read_text(encoding="utf-8")
        return json.loads(STATE.search(text)[2])

    def update(self, change):
        """Apply change(state) to the package state block."""
        main = self.package / "main.md"
        state = self.state()
        change(state)
        payload = json.dumps(state, ensure_ascii=False, indent=2)
        text = STATE.sub(
            lambda m: m[1] + payload + m[3], main.read_text(encoding="utf-8")
        )
        main.write_text(text, encoding="utf-8")

    def concept(self, state, identifier):
        return next(c for c in state["concepts"] if c["id"] == identifier)

    def set_concept(self, identifier, **fields):
        self.update(lambda state: self.concept(state, identifier).update(fields))

    def complete(self, identifier):
        """Mark a concept done; check requires its stage placeholder to be gone."""
        path = self.package / self.concept(self.state(), identifier)["path"]
        text = path.read_bytes().decode("utf-8")
        text = text.replace("Pending: research", "Finished: research")
        path.write_bytes(text.encode("utf-8"))
        self.set_concept(identifier, status="done")

    def depend(self, *edges):
        self.update(lambda state: state.update(dependencies=list(edges)))

    def ready(self, **kwargs):
        return self.cli("ready", "--package", self.package, **kwargs)

    def locks(self, root=None):
        common = self.git(
            "rev-parse", "--path-format=absolute", "--git-common-dir", cwd=root
        )
        return Path(common) / "agent-work" / "locks"

    def lock(self, name, owner=None):
        """Write a lock the way `lease acquire` does: a directory with an owner file."""
        path = self.locks() / f"{name}.lock"
        path.mkdir(parents=True)
        if owner is not None:
            (path / "owner").write_text(owner + "\n", encoding="utf-8")
        return path

    # ---------------------------------------------------------------- ready

    def test_requires_edges_order_the_waves(self):
        self.depend(edge("C002", "requires", "C001"), edge("C003", "requires", "C002"))
        output = self.ready()
        self.assertEqual(output["waves"], [["C001", "C004"], ["C002"], ["C003"]])
        self.assertEqual(output["ready"], ["C001", "C004"])
        self.assertEqual(output["excluded"], {})

    def test_concepts_that_share_files_never_share_a_wave(self):
        pairs = [("C001", "C002"), ("C004", "C003"), ("C001", "C004")]
        self.depend(*(edge(a, "shares-files-with", b) for a, b in pairs))
        output = self.ready()
        self.assertEqual(output["waves"], [["C001", "C003"], ["C002", "C004"]])
        for wave in output["waves"]:
            for pair in pairs:
                self.assertFalse(set(pair) <= set(wave), f"{pair} share {wave}")

    def test_running_and_claimed_concepts_are_excluded(self):
        self.set_concept("C001", status="running")
        self.set_concept("C002", status="claimed")
        self.depend(
            edge("C003", "requires", "C001"),
            edge("C004", "shares-files-with", "C002"),
        )
        output = self.ready()
        self.assertEqual(
            output["excluded"], {"C001": ["status running"], "C002": ["status claimed"]}
        )
        self.assertEqual(output["running"], ["C001", "C002"])
        # C003 waits for running C001; C004 may not run beside C002, which shares its files.
        self.assertEqual(output["ready"], [])
        self.assertEqual(output["waves"], [[], ["C003", "C004"]])

    def test_claimed_and_running_tasks_exclude_their_concept(self):
        state = self.state()
        owned = (self.package / self.concept(state, "C002")["path"]).relative_to(
            self.root
        )
        tasks = [
            {"id": "T1", "concept": "C001", "stage": "S2", "status": "claimed"},
            # A facet's task takes its whole concept.
            {
                "id": "T2",
                "concept": "C002.1",
                "stage": "S2",
                "status": "running",
                "owner": "worker-b",
                "paths": [owned.as_posix()],
            },
            {"id": "T3", "concept": "C003", "stage": "S1", "status": "validated"},
        ]
        self.update(lambda state: state.update(tasks=tasks))
        output = self.ready()
        self.assertEqual(
            output["excluded"],
            {"C001": ["task T1 claimed at S2"], "C002": ["task T2 running at S2"]},
        )
        self.assertEqual(output["ready"], ["C003", "C004"])

    def test_hints_are_excluded_and_hold_what_requires_them(self):
        self.set_concept("C002", type="hint")
        self.depend(edge("C003", "requires", "C002"))
        output = self.ready()
        self.assertEqual(list(output["excluded"]), ["C002"])
        self.assertIn("hint", output["excluded"]["C002"][0])
        self.assertEqual(output["held"], ["C002", "C003"])
        self.assertEqual(output["waves"], [["C001", "C004"]])
        # Once the hint is answered it is done like any concept, and frees C003.
        self.complete("C002")
        output = self.ready()
        self.assertEqual(output["excluded"], {})
        self.assertEqual(output["held"], [])
        self.assertEqual(output["ready"], ["C001", "C003", "C004"])

    def test_open_leases_exclude_their_concept_until_they_expire(self):
        owner = "worker-{} 2026-10-22T10:00:00Z 2026-10-22T10:20:00Z S2 research"
        self.lock(f"C001@{self.package.name}", owner.format("a"))
        self.lock("concept-C002", owner.format("b"))  # unscoped: every package
        self.lock(f"C003.2@{self.package.name}", owner.format("c"))  # a facet
        self.lock("C004@2026-10-19T00-00-00Z_research_other", owner.format("d"))
        self.lock("main.md", owner.format("e"))  # not a concept
        output = self.ready(now="2026-10-22T10:19:59Z")
        self.assertEqual(Path(output["leases"]), self.locks().parent)
        self.assertEqual(
            output["excluded"],
            {
                "C001": [
                    f"lease C001@{self.package.name}.lock held by worker-a until 2026-10-22T10:20:00Z"
                ],
                "C002": [
                    "lease concept-C002.lock held by worker-b until 2026-10-22T10:20:00Z"
                ],
                "C003": [
                    f"lease C003.2@{self.package.name}.lock held by worker-c until 2026-10-22T10:20:00Z"
                ],
            },
        )
        self.assertEqual(output["ready"], ["C004"])
        # A lease is open until its expiry time, then no longer counts.
        output = self.ready(now="2026-10-22T10:20:00Z")
        self.assertEqual(output["excluded"], {})
        self.assertEqual(output["ready"], ALL)

    def test_a_lock_still_being_written_counts_as_open(self):
        self.lock(f"C004@{self.package.name}")
        output = self.ready()
        self.assertEqual(list(output["excluded"]), ["C004"])
        self.assertIn("held by a new holder", output["excluded"]["C004"][0])


if __name__ == "__main__":
    unittest.main()
