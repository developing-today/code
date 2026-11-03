#!/usr/bin/env python3
"""Run with python3 test_seed.py; optionally pass the original seed.sh to compare."""

import json
import os
from pathlib import Path
import stat
import subprocess
import sys
import tempfile
import unittest

SEED = Path(__file__).with_name("seed.sh")
ORIGINAL = Path(sys.argv.pop(1)).resolve() if len(sys.argv) > 1 else None


class SeedTest(unittest.TestCase):
    def seed_env(self, home):
        return dict(
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

    def run_script(self, home, script=SEED):
        return subprocess.run(
            ["bash", str(script)],
            env=self.seed_env(home),
            capture_output=True,
            text=True,
        )

    def run_seed(self, content, script=SEED):
        with tempfile.TemporaryDirectory() as home:
            path = Path(home) / "userdata" / "settings.json"
            path.parent.mkdir()
            path.write_text(content)
            result = self.run_script(home, script)
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(result.stdout, "")
            return path.read_text(), result.stderr

    def test_existing_file_mode_is_preserved(self):
        with tempfile.TemporaryDirectory() as home:
            path = Path(home) / "userdata" / "settings.json"
            path.parent.mkdir()
            path.write_text("{}")
            path.chmod(0o600)

            result = self.run_script(home)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o600)

    def test_new_file_keeps_default_umask_mode(self):
        with tempfile.TemporaryDirectory() as home:
            path = Path(home) / "userdata" / "settings.json"
            current_umask = os.umask(0)
            os.umask(current_umask)

            result = self.run_script(home)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertEqual(stat.S_IMODE(path.stat().st_mode), 0o666 & ~current_umask)

    def test_symlink_settings_is_refused_unchanged(self):
        with tempfile.TemporaryDirectory() as home:
            path = Path(home) / "userdata" / "settings.json"
            target = path.with_name("settings-target.json")
            path.parent.mkdir()
            original = "{}"
            target.write_text(original)
            path.symlink_to(target.name)

            result = self.run_script(home)

            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertTrue(path.is_symlink())
            self.assertEqual(target.read_text(), original)
            self.assertIn("symlink", result.stderr.lower())
            self.assertIn("refus", result.stderr.lower())

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
