#!/usr/bin/env python3
"""Oracle: the actual workflow invokes exactly the landed manifest's Bun files.

No arguments; unittest owns its standard CLI. Bun is a recording stub: this
proves offline CI selection and failure propagation, not adapter behavior.
"""

import json
import os
from pathlib import Path
import re
import subprocess
import tempfile
import unittest

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = "pkgs/fleet/crates/fleet-queue/tests/fixtures/pmo-conformance/manifest.json"


class ProjectorSelection(unittest.TestCase):
    def run_workflow_step(
        self,
        install_rc=0,
        test_rc=0,
        extra_file="pm-stack/projector/test/config.test.ts",
    ):
        workflow = (ROOT / ".github/workflows/pmo-contract-conformance.yml").read_text()
        block = workflow.split(
            "- name: Run the projector's forwarding tests by exact file", 1
        )[1]
        command = re.search(r"        run: \|\n(.*?)(?=\n      - name:)", block, re.S)[
            1
        ]
        command = "\n".join(line[10:] for line in command.splitlines())
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            projector = root / "pm-stack/projector"
            projector.mkdir(parents=True)
            tests = projector / "test"
            tests.mkdir()
            for name in [
                "webhook-delivery-id.test.ts",
                "webhook-operation-forwarding.test.ts",
                "config.test.ts",
            ]:
                (tests / name).touch()
            (tests / "directory.test.ts").mkdir()
            script = root / ".github/scripts/pmo-contract-conformance.py"
            script.parent.mkdir(parents=True)
            script.write_text((ROOT / script.relative_to(root)).read_text())
            manifest = root / MANIFEST
            manifest.parent.mkdir(parents=True)
            manifest.write_text(
                json.dumps(
                    {
                        "families": [
                            {
                                "status": "landed",
                                "projector_tests": [
                                    "pm-stack/projector/test/webhook-delivery-id.test.ts",
                                    "pm-stack/projector/test/webhook-operation-forwarding.test.ts",
                                    extra_file,
                                ],
                            },
                            {
                                "status": "gated",
                                "projector_tests": [
                                    "pm-stack/projector/test/gated.test.ts"
                                ],
                            },
                            {
                                "status": "landed",
                                "projector_tests": [
                                    "pm-stack/projector/test/config.test.ts"
                                ],
                            },
                        ]
                    }
                )
            )
            binary = root / "bun"
            binary.write_text(
                "#!/usr/bin/env python3\nimport json, os, sys\n"
                "with open(os.environ['CALLS'], 'a') as f: f.write(json.dumps(sys.argv[1:]) + '\\n')\n"
                "sys.exit(int(os.environ['INSTALL_RC']) if sys.argv[1] == 'install' else int(os.environ['TEST_RC']))\n"
            )
            binary.chmod(0o755)
            calls = root / "calls"
            output = root / "output"
            env = dict(
                os.environ,
                PATH=f"{root}:{os.environ['PATH']}",
                GITHUB_WORKSPACE=str(root),
                MANIFEST=MANIFEST,
                GITHUB_OUTPUT=str(output),
                CALLS=str(calls),
                INSTALL_RC=str(install_rc),
                TEST_RC=str(test_rc),
            )
            subprocess.run(
                ["bash", "-e", "-o", "pipefail", "-c", command],
                cwd=projector,
                env=env,
                check=True,
            )
            return (
                [json.loads(line) for line in calls.read_text().splitlines()]
                if calls.exists()
                else []
            ), output.read_text()

    def test_workflow_executes_all_and_only_landed_manifest_files(self):
        calls, output = self.run_workflow_step()
        self.assertEqual(
            calls,
            [
                ["install", "--frozen-lockfile"],
                [
                    "test",
                    "./test/webhook-delivery-id.test.ts",
                    "./test/webhook-operation-forwarding.test.ts",
                    "./test/config.test.ts",
                ],
            ],
        )
        self.assertEqual(output, "rc=0\n")

    def test_install_failure_prevents_tests_and_preserves_failure(self):
        calls, output = self.run_workflow_step(17)
        self.assertEqual(calls, [["install", "--frozen-lockfile"]])
        self.assertEqual(output, "rc=17\n")

    def test_test_failure_is_not_reported_successful(self):
        _, output = self.run_workflow_step(test_rc=23)
        self.assertEqual(output, "rc=23\n")

    def test_invalid_selection_is_rejected_before_install(self):
        # Invariant oracle: none of these selectors may launch any subprocess.
        for path in [
            "--help",
            "pm-stack/projector/test/../src/config.test.ts",
            "pm-stack/projector/test",
            "/tmp/example.test.ts",
            "pm-stack/projector/test/directory.test.ts",
        ]:
            with self.subTest(path=path):
                calls, output = self.run_workflow_step(extra_file=path)
                self.assertEqual(calls, [])
                self.assertEqual(output, "rc=2\n")


if __name__ == "__main__":
    unittest.main()
