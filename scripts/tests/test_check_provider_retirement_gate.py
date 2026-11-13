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
                "{\n"
                '  "model": "opencode-route/workhorse",\n'
                '  "small_model": "opencode-route/speed"\n'
                "}\n"
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
            dirty.write_text(
                '{\n  "agents": {"a": {"model": "opencode-route/scout"}}\n}\n'
            )
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
            self.assertIn(
                "Retirement gate OK: no references to provider 'new-provider'",
                result.stdout,
            )

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

    def test_provider_id_substring_of_others_passes(self):
        # Codex finding 4506371633 (PR #7679): matching is by provider
        # identity, not substring. Retiring "opencode" must not be
        # blocked by unrelated longer IDs or package names that merely
        # contain it.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                '  "model": "opencode-route/workhorse",\n'
                '  "provider": {"opencode-go.drewrypope": {}},\n'
                '  "plugin": ["@opencode/ai/providers/claude-code"]\n'
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "opencode"])
            self.assertEqual(result.returncode, 0, result.stderr)
            self.assertIn("Retirement gate OK", result.stdout)

    def test_provider_identity_key_and_model_block(self):
        # The identity forms that DO block: a provider key and a
        # "<id>/<model>" reference.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                '  "provider": {"opencode": {"options": {}}},\n'
                '  "model": "opencode/some-model"\n'
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "opencode"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("Retirement BLOCKED", result.stderr)

    def test_deployed_runtime_overlay_checked_despite_xdg_override(self):
        # Codex finding 4506371638 (PR #7679): the deployed runtime
        # overlay lives at the fixed $HOME/.local/state location
        # (home/common/default.nix OPENCODE_CONFIG) even when
        # XDG_STATE_HOME points elsewhere; the gate must check it.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            root = Path(tmp) / "repo"
            (root / "scripts").mkdir(parents=True)
            isolated = root / "scripts" / SCRIPT.name
            isolated.write_text(SCRIPT.read_text())
            home = Path(tmp) / "home"
            overlay = home / ".local" / "state" / "opencode" / "opencode-runtime.jsonc"
            overlay.parent.mkdir(parents=True)
            overlay.write_text('{\n  "model": "opencode-route/workhorse"\n}\n')
            env = os.environ.copy()
            env["HOME"] = str(home)
            env["XDG_CONFIG_HOME"] = str(home / "config")
            env["XDG_DATA_HOME"] = str(home / "data")
            env["XDG_STATE_HOME"] = str(home / "other-state")
            result = subprocess.run(
                ["bash", str(isolated), "opencode-route"],
                text=True,
                capture_output=True,
                check=False,
                env=env,
            )
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn(str(overlay), result.stderr)

    def test_comment_only_references_pass(self):
        # Codex finding 4506184819 (PR #7679): a provider mentioned only
        # in JSONC comments (retirement history) is not an active
        # reference and must not block retirement.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                "  // openai-codex retired 2026-11-01; see history.\n"
                "  /* opencode-route was here,\n"
                "     multi-line note about opencode-route */\n"
                '  "model": "native-provider/model"\n'
                "}\n"
            )
            for provider in ("openai-codex", "opencode-route"):
                result = run_gate(["--config", str(cfg), provider])
                self.assertEqual(result.returncode, 0, (provider, result.stderr))
                self.assertIn("Retirement gate OK", result.stdout)

    def test_value_reference_still_blocks_when_comments_present(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                "  // opencode-route mentioned in a comment\n"
                '  "small_model": "opencode-route/speed"\n'
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("Retirement BLOCKED", result.stderr)
            self.assertIn("opencode-route/speed", result.stderr)

    def test_slashes_inside_strings_are_not_comments(self):
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                '  "endpoint": "http://127.0.0.1:18750",\n'
                '  "model": "opencode-route/workhorse"\n'
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("opencode-route/workhorse", result.stderr)

    def test_disabled_providers_tombstone_passes(self):
        # Codex finding 4506484591 (PR #7679): an entry in
        # "disabled_providers" is a tombstone that SUPPRESSES a
        # retired provider (removing it would reactivate the provider
        # through the catalog); it is not an active reference and
        # must not block that provider's retirement.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                '  "disabled_providers": [\n'
                '    "google-vertex",\n'
                '    "openai-codex"\n'
                "  ],\n"
                '  "model": "native-provider/model"\n'
                "}\n"
            )
            for provider in ("google-vertex", "openai-codex"):
                result = run_gate(["--config", str(cfg), provider])
                self.assertEqual(result.returncode, 0, (provider, result.stderr))
                self.assertIn("Retirement gate OK", result.stdout)

    def test_disabled_tombstone_plus_active_reference_blocks(self):
        # The tombstone exclusion is scoped to the array: the same
        # provider referenced as a real key elsewhere still blocks.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                '  "disabled_providers": ["google-vertex"],\n'
                '  "provider": {"google-vertex": {"options": {}}}\n'
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "google-vertex"])
            self.assertEqual(result.returncode, 1)
            self.assertIn("Retirement BLOCKED", result.stderr)

    def test_nested_disabled_providers_array_is_not_a_tombstone(self):
        # CodeRabbit finding 4506743525 (PR #7679): tombstone
        # filtering applies only to the TOP-LEVEL disabled_providers
        # list. A nested array with the same key inside provider or
        # plugin options is ordinary data; a provider/model reference
        # inside it must still block retirement.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                '  "provider": {"some-provider": {"options": {\n'
                '    "disabled_providers": ["opencode-route/workhorse"]\n'
                "  }}}\n"
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 1, result.stderr)
            self.assertIn("Retirement BLOCKED", result.stderr)

    def test_unterminated_block_comment_fails_closed(self):
        # CodeRabbit finding 4506377624 (PR #7679): a block comment
        # that is never closed blanks the rest of the file; without a
        # fail-closed check, references after it would be hidden and
        # the gate would pass. The strip must error (exit 2 overall).
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text(
                "{\n"
                "  /* retirement note that is never closed\n"
                '  "model": "opencode-route/workhorse"\n'
                "}\n"
            )
            result = run_gate(["--config", str(cfg), "opencode-route"])
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertIn("failing closed", result.stderr)

    def test_empty_provider_id_is_usage_error(self):
        # CodeRabbit finding 4506377635 (PR #7679), second half: an
        # empty provider ID would search for the patterns '""' and
        # '"/' and block on unrelated config; reject it up front.
        with tempfile.TemporaryDirectory(prefix="retirement-gate-") as tmp:
            cfg = Path(tmp) / "opencode.jsonc"
            cfg.write_text('{\n  "model": "native/model"\n}\n')
            result = run_gate(["--config", str(cfg), ""])
            self.assertEqual(result.returncode, 2)
            self.assertIn("must not be empty", result.stderr)

    @unittest.skipIf(
        hasattr(os, "geteuid") and os.geteuid() == 0,
        "root can read chmod-0 files; grep-error path not exercisable",
    )
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
