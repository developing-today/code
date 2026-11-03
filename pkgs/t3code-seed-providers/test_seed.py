#!/usr/bin/env python3
"""Run with python3 test_seed.py; optionally pass the original seed.sh to compare."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest

SEED = Path(__file__).with_name("seed.sh")
ORIGINAL = Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else None


class SeedTest(unittest.TestCase):
    def run_seed(self, content, script=SEED):
        with tempfile.TemporaryDirectory() as home:
            path = Path(home) / "userdata" / "settings.json"
            path.parent.mkdir()
            path.write_text(content)
            env = dict(
                os.environ,
                HOME=home,
                T3CODE_HOME=home,
                OPENCODE_V2_BIN="/new/opencode-2.0/bin/opencode",
                ANTIGRAVITY_ACP_BIN="",
                MUSE_BIN="",
                PI_BIN="",
                GROK_AUTH="",
                GROK_API_KEY="",
            )
            result = subprocess.run(
                ["bash", str(script)], env=env, capture_output=True, text=True
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "")
            return path.read_text(), result.stderr

    def test_invalid_document_unchanged(self):
        for content in (
            "[]",
            "null",
            '{"providerInstances": []}',
            '{"providerInstances": null}',
            "{broken",
        ):
            with self.subTest(content=content):
                result, stderr = self.run_seed(content)
                self.assertEqual(result, content)
                self.assertTrue(stderr)

    def test_malformed_fleet_entries_are_preserved(self):
        for bad in (
            "broken",
            {"driver": "opencode", "config": None},
            {"driver": "opencode", "config": "broken"},
        ):
            with self.subTest(bad=bad):
                data = {
                    "providerInstances": {
                        "opencode-v2": bad,
                        "legacy": {
                            "driver": "opencode",
                            "enabled": True,
                            "config": {"binaryPath": "/bin/opencode-1.2/bin/opencode"},
                        },
                    }
                }
                result, stderr = self.run_seed(json.dumps(data))
                instances = json.loads(result)["providerInstances"]
                self.assertEqual(instances["opencode-v2"], bad)
                self.assertFalse(instances["legacy"]["enabled"])
                self.assertIn("codex", instances)
                self.assertIn("opencode-v2", stderr)
                self.assertIn("malformed", stderr)

    def test_well_formed_matches_original(self):
        cases = [
            "{}",
            json.dumps(
                {
                    "other": 42,
                    "providerInstances": {
                        "opencode-v2": {
                            "driver": "opencode",
                            "enabled": False,
                            "config": {"binaryPath": "/old", "model": "custom"},
                        },
                        "legacy": {
                            "driver": "opencode",
                            "enabled": True,
                            "config": {
                                "binaryPath": "/nix/store/hash-opencode-1.5/bin/opencode"
                            },
                        },
                    },
                }
            ),
            json.dumps({"providerInstances": {"opencode-v2": {"driver": "opencode"}}}),
        ]
        for content in cases:
            with self.subTest(content=content):
                actual = self.run_seed(content)
                if ORIGINAL:
                    self.assertEqual(actual, self.run_seed(content, ORIGINAL))
                self.assertIn("codex", json.loads(actual[0])["providerInstances"])


if __name__ == "__main__":
    unittest.main()
