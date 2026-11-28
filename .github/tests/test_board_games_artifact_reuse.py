#!/usr/bin/env python3
"""Workflow contract tests; no arguments except unittest's strict CLI.

Requires PyYAML 6.0.3. Oracle: the producer/consumer job graph must make
artifact reuse mutually exclusive with compilation, and source selection
must accept only a complete pair of positive decimal GitHub IDs.
"""

import itertools
import os
from pathlib import Path
import subprocess
import tempfile
import unittest

import yaml


WORKFLOW = Path(__file__).resolve().parents[1] / "workflows/id-roc-game.yml"


class ArtifactReuseTests(unittest.TestCase):
    @classmethod
    def setUpClass(cls):
        cls.workflow = yaml.safe_load(WORKFLOW.read_text())
        cls.jobs = cls.workflow["jobs"]

    def test_source_selection_model(self):
        # Oracle: a Cartesian input model, not GitHub/network availability.
        script = self.jobs["artifact-source"]["steps"][0]["run"]
        values = ["", "1", "45858218992", "0", "-1", "1.5", "abc", "1\n2"]
        for run_id, artifact_id in itertools.product(values, repeat=2):
            with self.subTest(run_id=run_id, artifact_id=artifact_id):
                fresh = run_id == artifact_id == ""
                valid_pair = all(
                    v.isascii() and v.isdecimal() and int(v) > 0
                    for v in (run_id, artifact_id)
                )
                with tempfile.TemporaryDirectory() as directory:
                    output = Path(directory) / "outputs"
                    env = dict(
                        os.environ,
                        SOURCE_RUN_ID=run_id,
                        SOURCE_ARTIFACT_ID=artifact_id,
                        GITHUB_OUTPUT=str(output),
                    )
                    result = subprocess.run(
                        ["bash", "-c", script], env=env, capture_output=True, text=True
                    )
                    self.assertEqual(result.returncode == 0, fresh or valid_pair)
                    if result.returncode == 0:
                        self.assertIn(
                            f"reuse={'false' if fresh else 'true'}\n",
                            output.read_text(),
                        )

    def test_reuse_skips_every_compiler_job(self):
        # Oracle: both existing producer jobs depend on the validated mode.
        for name in ("game", "board-games"):
            job = self.jobs[name]
            self.assertIn("artifact-source", job["needs"])
            self.assertEqual(
                job["if"], "${{ needs.artifact-source.outputs.reuse != 'true' }}"
            )

    def test_consumer_can_run_after_skipped_producer(self):
        # Oracle: the consumer DAG explicitly permits a skipped build only
        # after successful source validation, and never ignores cancellation.
        consumer = self.jobs["board-games-artifact-consumer"]
        self.assertEqual(set(consumer["needs"]), {"artifact-source", "board-games"})
        self.assertEqual(
            consumer["if"],
            "${{ !cancelled() && needs.artifact-source.result == 'success' && (needs.board-games.result == 'success' || needs.artifact-source.outputs.reuse == 'true') }}",
        )
        download = next(
            step
            for step in consumer["steps"]
            if step.get("uses", "").startswith("actions/download-artifact@")
        )
        self.assertEqual(
            download["with"]["artifact-ids"], "${{ inputs.source_artifact_id }}"
        )
        self.assertEqual(
            download["with"]["run-id"], "${{ inputs.source_run_id || github.run_id }}"
        )
        self.assertEqual(download["with"]["github-token"], "${{ github.token }}")
        self.assertIn("needs.artifact-source.outputs.reuse", download["with"]["name"])
        self.assertEqual(self.workflow["permissions"]["actions"], "read")
        # A consumer must not install a compiler or execute producer commands.
        consumer_text = yaml.safe_dump(consumer)
        for forbidden in ("nix develop", "cargo build", "build.sh", "roc build"):
            self.assertNotIn(forbidden, consumer_text)

    def test_all_run_scripts_parse_as_bash(self):
        # Oracle: the shell, not GitHub availability. Every inline run:
        # script must survive `bash -n`; an apostrophe inside a
        # double-quoted ${VAR:-default} word is a parse-time failure
        # (run 45992760834: "unexpected EOF while looking for
        # matching `''`" in the verify step).
        scripts = []

        def collect(node):
            if isinstance(node, dict):
                run = node.get("run")
                if isinstance(run, str):
                    scripts.append(run)
                for value in node.values():
                    collect(value)
            elif isinstance(node, list):
                for item in node:
                    collect(item)

        collect(self.workflow["jobs"])
        self.assertGreater(len(scripts), 0)
        for index, script in enumerate(scripts):
            with self.subTest(script_index=index):
                result = subprocess.run(
                    ["bash", "-n", "-c", script],
                    capture_output=True,
                    text=True,
                )
                self.assertEqual(result.returncode, 0, msg=result.stderr)


if __name__ == "__main__":
    unittest.main()
