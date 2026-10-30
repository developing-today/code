#!/usr/bin/env python3
"""Tests for scripts/nightly-backup.sh — no network, stub binaries.

Run:  python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
  or: python3 pkgs/cold-store-reaper/tests/test_nightly_backup.py

Step isolation: one failing step must not starve the steps after it
(the 2026-10-27 run lost prune-backups, sync-manifest, and status to a
failed Turso export). The script still exits non-zero at the END so
the cron reports the failure loudly, and guarded skips (no R2 creds,
no Turso token) are not failures. Stubs arrive via the COLD_STORE_BIN
and EXPORT_TURSO_BIN seams.
"""

import os
import stat
import subprocess
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).resolve().parent.parent / "scripts" / "nightly-backup.sh"

STUB_COLD_STORE = r"""#!/usr/bin/env bash
printf '%s\n' "$*" >> "${STUB_CAPTURE_DIR:?}/cold-store.log"
case "$1" in
  status) echo "stub status summary: objects 0 cold" ;;
esac
if [[ "${STUB_FAIL_STEP:-}" == "$1" ]]; then
  echo "stub cold-store: forced failure of $1" >&2
  exit 3
fi
exit 0
"""

STUB_EXPORT_TURSO = r"""#!/usr/bin/env bash
printf '%s\n' "export-turso" >> "${STUB_CAPTURE_DIR:?}/cold-store.log"
if [[ "${STUB_FAIL_STEP:-}" == "export-turso" ]]; then
  echo "stub export-turso: forced failure" >&2
  exit 1
fi
echo "turso export: all 0 database(s) filed"
exit 0
"""


class TestNightlyBackup(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        root = Path(self.tmp.name)
        self.capture = root / "capture"
        self.capture.mkdir()
        stubbin = root / "bin"
        stubbin.mkdir()
        cold = stubbin / "cold-store"
        cold.write_text(STUB_COLD_STORE)
        cold.chmod(cold.stat().st_mode | stat.S_IEXEC)
        turso = stubbin / "export-turso"
        turso.write_text(STUB_EXPORT_TURSO)
        turso.chmod(turso.stat().st_mode | stat.S_IEXEC)
        self.env = {
            **os.environ,
            "STUB_CAPTURE_DIR": str(self.capture),
            "CLOUDFLARE_API_TOKEN": "fake-cf-token",
            "R2_ACCESS_KEY_ID": "fake-r2-id",
            "R2_SECRET_ACCESS_KEY": "fake-r2-secret",
            "COLD_STORE_BIN": str(cold),
            "EXPORT_TURSO_BIN": str(turso),
        }

    def tearDown(self):
        self.tmp.cleanup()

    def run_script(self, env=None):
        return subprocess.run(
            ["bash", str(SCRIPT)],
            env=env or self.env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def calls(self):
        log = self.capture / "cold-store.log"
        return log.read_text().splitlines() if log.exists() else []

    def test_failing_middle_step_does_not_starve_later_steps(self):
        env = {**self.env, "STUB_FAIL_STEP": "backup-supabase"}
        result = self.run_script(env=env)
        # loud failure: non-zero exit, the failed step is named
        self.assertNotEqual(result.returncode, 0, result.stdout)
        self.assertIn("step backup-supabase: FAILED (exit 3)", result.stderr)
        self.assertIn("FAILED steps: backup-supabase", result.stderr)
        # but every later step still ran, in order
        self.assertEqual(
            self.calls(),
            [
                "backup-d1",
                "backup-supabase",
                "export-turso",
                "prune-backups --apply",
                "sync-manifest",
                "status",
            ],
        )
        # and the status summary still printed
        self.assertIn("stub status summary", result.stdout)
        self.assertIn("step status: ok", result.stdout)

    def test_all_steps_ok_exits_zero(self):
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("nightly-backup: all steps ok", result.stdout)
        self.assertIn("step prune-backups: ok", result.stdout)
        self.assertIn("step sync-manifest: ok", result.stdout)

    def test_missing_r2_credentials_skip_supabase_without_failing(self):
        env = {
            k: v
            for k, v in self.env.items()
            if k not in ("R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY")
        }
        result = self.run_script(env=env)
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn(
            "step backup-supabase: skipped (R2 credentials not set)", result.stdout
        )
        self.assertNotIn("backup-supabase", self.calls())
        self.assertIn("nightly-backup: all steps ok", result.stdout)

    def test_long_status_output_still_ok_and_truncated(self):
        # Regression: status printing >20 lines must not trip pipefail/SIGPIPE.
        # The verdict depends only on cold-store's own exit code; display is
        # still truncated to ~20 lines.
        cold = Path(self.env["COLD_STORE_BIN"])
        cold.write_text(
            r"""#!/usr/bin/env bash
printf '%s\n' "$*" >> "${STUB_CAPTURE_DIR:?}/cold-store.log"
case "$1" in
  status) for i in $(seq 1 300); do echo "stub status line $i"; done ;;
esac
if [[ "${STUB_FAIL_STEP:-}" == "$1" ]]; then
  echo "stub cold-store: forced failure of $1" >&2
  exit 3
fi
exit 0
"""
        )
        result = self.run_script()
        self.assertEqual(result.returncode, 0, result.stderr)
        self.assertIn("step status: ok", result.stdout)
        self.assertIn("stub status line 1", result.stdout)
        self.assertIn("stub status line 20", result.stdout)
        self.assertNotIn("stub status line 21", result.stdout)
        self.assertNotIn("stub status line 300", result.stdout)


if __name__ == "__main__":
    unittest.main()
