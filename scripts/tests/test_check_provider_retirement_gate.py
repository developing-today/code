#!/usr/bin/env python3
"""Contract tests for check-provider-retirement-gate.sh (#5610).

Verifies that the retirement gate blocks retiring a provider while
configuration files still reference it, and passes once all
references are gone.
"""

import os
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
SCRIPT = ROOT / "scripts" / "check-provider-retirement-gate.sh"


def run_gate(args, env=None):
    return subprocess.run(
        ["bash", str(SCRIPT), *args],
        text=True,
        capture_output=True,
        check=False,
        env=env,
    )


class TestCheckProviderRetirementGate(unittest.TestCase):
    def test_no_references_passes(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text('{\n  "model": "native-provider/model"\n}\n')
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Retirement gate OK", result.stdout)

    def test_references_block_retirement(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                '{\n'
                '  "model": "opencode-route/workhorse",\n'
                '  "small_model": "opencode-route/speed"\n'
                '}\n'
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("Retirement BLOCKED", result.stderr)
            self.assertIn("opencode-route/workhorse", result.stderr)
            self.assertIn("opencode-route/speed", result.stderr)

    def test_multiple_files_checked(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            clean = Path(tmp) / "clean.jsonc"
            clean.write_text('{\n  "model": "native/model"\n}\n')
            dirty = Path(tmp) / "dirty.jsonc"
            dirty.write_text('{\n  "agents": {"a": {"model": "opencode-route/scout"}}\n}\n')
            result = run_gate(
                ["--config", str(clean), "--config", str(dirty), "opencode-route"]
            )
            self.assertEqual(result.returncode, 1)
            self.assertIn("Retirement BLOCKED", result.stderr)
            self.assertIn(str(dirty), result.stderr)

    def test_multiple_providers(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text('{\n  "model": "old-provider/one"\n}\n')
            result = run_gate(["--config", str(cfg), "old-provider", "new-provider"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("old-provider", result.stderr)
            self.assertIn("Retirement gate OK: no references to provider 'new-provider'", result.stdout)

    def test_missing_config_is_error(self):
        result = run_gate(["--config", "/nonexistent/opencode.jsonc", "opencode-route"])
        self.assertEqual(result.returncode, 2)
        self.assertIn("does not exist", result.stderr)

    def test_no_provider_is_usage_error(self):
        result = run_gate([])
        self.assertEqual(result.returncode, 2)
        self.assertIn("At least one provider ID", result.stderr)

    def test_allow_empty_with_no_default_configs(self):
        # Copy the script into an isolated tree whose repo root has no
        # .opencode config, and point HOME/XDG dirs at an empty temp dir,
        # so no default config resolves.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            root = Path(tmp) / "repo"
            (root / "scripts").mkdir(parents=True)
            isolated = root / "scripts" / SCRIPT.name
            isolated.write_text(SCRIPT.read_text())
            home = Path(tmp) / "home"
            home.mkdir()
            env = os.environ.copy()
            env["HOME"] = str(home)
            env["XDG_CONFIG_HOME"] = str(home / "config")
            env["XDG_DATA_HOME"] = str(home / "data")
            env["XDG_STATE_HOME"] = str(home / "state")
            result = subprocess.run(
                ["bash", str(isolated), "--allow-empty", "opencode-route"],
                text=True,
                capture_output=True,
                check=False,
                env=env,
            )
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("--allow-empty", result.stdout)


    def test_comment_only_references_pass(self):
        # Codex finding 4506184819 (PR #7679): a provider mentioned only
        # in JSONC comments (retirement history) is not an active
        # reference and must not block retirement.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                '{\n'
                '  // openai-codex retired 2026-11-01; see history.\n'
                '  /* opencode-route was here,\n'
                '     multi-line note about opencode-route */\n'
                '  "model": "native-provider/model"\n'
                '}\n'
            )
            for provider in ("openai-codex", "opencode-route"):
                result = run_gate(["--config", str(cfg), provider])
                self.assertEqual(result.returncode, 0, (provider, result.stderr))
                self.assertIn("Retirement gate OK", result.stdout)

    def test_value_reference_still_blocks_when_comments_present(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                '{\n'
                '  // opencode-route mentioned in a comment\n'
                '  "small_model": "opencode-route/speed"\n'
                '}\n'
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("Retirement BLOCKED", result.stderr)
            self.assertIn("opencode-route/speed", result.stderr)

    def test_slashes_inside_strings_are_not_comments(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                '{\n'
                '  "endpoint": "http://127.0.0.1:18750",\n'
                '  "model": "opencode-route/workhorse"\n'
                '}\n'
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("opencode-route/workhorse", result.stderr)

    @unittest.skipIf(hasattr(os, "geteuid") and os.geteuid() == 0,
                     "root can read chmod-0 files; grep-error path not exercisable")
    def test_unreadable_config_fails_closed(self):
        # Codex finding 4506184829 (PR #7679): a config that exists but
        # cannot be searched is a configuration error (exit 2), never
        # a silent "no references" pass.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text('{\n  "model": "native/model"\n}\n')
            cfg.chmod(0)
            try:
                result = run_gate(["--config", str(cfg), "opencode-route"])
            finally:
                cfg.chmod(0o644)
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("failing closed", result.stderr)


if __name__ == "__main__":
    unittest.main()
