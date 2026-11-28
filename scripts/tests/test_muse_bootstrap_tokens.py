#!/usr/bin/env python3
"""Behavior tests for the muse bootstrap token phase (#12898).

The token phase must self-provision the three shared broker token files a
fleet seat's client verbs read — ~/tools/secrets-broker-token,
~/tools/queue-broker-token, ~/tools/laptop-agent-token — from the BWS
secrets fleet.secrets-broker-token, fleet.queue-broker-token and
fleet.laptop-agent-token. Before the fix, bootstrap never fetched them:
only the muse seat had the files, by accident of the home restore, and a
fresh non-muse seat had no provisioning path at all.

The tests run the real scripts/muse/bootstrap.sh with --only token in a
throwaway HOME, with a fake `bws` on the script's own bin dir serving a
fixture secret list. No network, no real secrets.
"""

from __future__ import annotations

import json
import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
BOOTSTRAP = ROOT / "scripts" / "muse" / "bootstrap.sh"

FAKE_BWS = """#!/usr/bin/env bash
# Fake bws: only `secret list -o json` is implemented, from a fixture file.
if [ "${1:-}" = "secret" ] && [ "${2:-}" = "list" ]; then
  cat "${FAKE_BWS_FIXTURE:?}"
  exit 0
fi
echo "fake bws: unhandled: $*" >&2
exit 1
"""

# Fixture values are 64-char stand-ins for the real 64-char service tokens.
SECRETS_VALUE = "s" * 64
QUEUE_VALUE = "q" * 64
LAPTOP_VALUE = "l" * 64

FILES = {
    "fleet.secrets-broker-token": ("secrets-broker-token", SECRETS_VALUE),
    "fleet.queue-broker-token": ("queue-broker-token", QUEUE_VALUE),
    "fleet.laptop-agent-token": ("laptop-agent-token", LAPTOP_VALUE),
}


class TestMuseBootstrapTokens(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="muse-bootstrap-tokens-")
        self.home = Path(self.tmp.name) / "home"
        self.bin_dir = self.home / ".local" / "share" / "muse-bootstrap" / "bin"
        self.bin_dir.mkdir(parents=True)
        fake = self.bin_dir / "bws"
        fake.write_text(FAKE_BWS)
        fake.chmod(0o755)
        self.fixture = Path(self.tmp.name) / "secrets.json"

    def tearDown(self) -> None:
        self.tmp.cleanup()

    def write_fixture(self, entries: dict[str, str]) -> None:
        self.fixture.write_text(
            json.dumps([{"key": k, "value": v} for k, v in entries.items()])
        )

    def run_bootstrap(self) -> subprocess.CompletedProcess[str]:
        env = dict(os.environ)
        env["HOME"] = str(self.home)
        env["BWS_ACCESS_TOKEN"] = "test-access-token"
        env["FAKE_BWS_FIXTURE"] = str(self.fixture)
        return subprocess.run(
            ["bash", str(BOOTSTRAP), "--only", "token"],
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
        )

    def test_all_three_broker_tokens_provisioned(self) -> None:
        # One fixture value carries a trailing newline, as a value copied
        # from the laptop file may; the written file must still be exactly
        # one token line (the laptop files are 65 bytes: 64 + newline).
        self.write_fixture(
            {
                "fleet.secrets-broker-token": SECRETS_VALUE + "\n",
                "fleet.queue-broker-token": QUEUE_VALUE,
                "fleet.laptop-agent-token": LAPTOP_VALUE,
            }
        )
        result = self.run_bootstrap()
        self.assertEqual(result.returncode, 0, result.stderr)
        for bws_name, (filename, value) in FILES.items():
            dest = self.home / "tools" / filename
            self.assertTrue(dest.is_file(), f"{filename} not provisioned ({bws_name})")
            self.assertEqual(dest.read_bytes(), (value + "\n").encode())
            self.assertEqual(stat.S_IMODE(dest.stat().st_mode), 0o600)
            # Secret values are never printed, logged or put in argv.
            self.assertNotIn(value, result.stdout)
            self.assertNotIn(value, result.stderr)
        # The BWS access token itself is stored, mode 600, as before.
        access = self.home / ".config" / "bitwarden" / "access-token"
        self.assertTrue(access.is_file())
        self.assertEqual(stat.S_IMODE(access.stat().st_mode), 0o600)

    def test_missing_bws_name_warns_and_continues(self) -> None:
        self.write_fixture(
            {
                "fleet.secrets-broker-token": SECRETS_VALUE,
                "fleet.laptop-agent-token": LAPTOP_VALUE,
            }
        )
        result = self.run_bootstrap()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("fleet.queue-broker-token", result.stderr)
        self.assertIn("not provisioned", result.stderr)
        self.assertFalse((self.home / "tools" / "queue-broker-token").exists())
        self.assertTrue((self.home / "tools" / "secrets-broker-token").is_file())
        self.assertTrue((self.home / "tools" / "laptop-agent-token").is_file())

    def test_rerun_is_idempotent(self) -> None:
        self.write_fixture({name: value for name, (_, value) in FILES.items()})
        first = self.run_bootstrap()
        self.assertEqual(first.returncode, 0, first.stderr)
        before = {
            filename: (self.home / "tools" / filename).read_bytes()
            for _, (filename, _) in FILES.items()
        }
        second = self.run_bootstrap()
        self.assertEqual(second.returncode, 0, second.stderr)
        for filename, content in before.items():
            self.assertEqual((self.home / "tools" / filename).read_bytes(), content)


if __name__ == "__main__":
    unittest.main()
