#!/usr/bin/env python3
"""Contract and behavior tests for per-seat fleet bootstrap scripts (#12236 item 3)."""

from __future__ import annotations

import hashlib
import http.server
import os
import shutil
import stat
import subprocess
import tempfile
import threading
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
FLEET_SCRIPTS = ROOT / "scripts" / "fleet"
BOOTSTRAP = FLEET_SCRIPTS / "bootstrap.sh"
BOOTSTRAP_LINUX = FLEET_SCRIPTS / "bootstrap-linux.sh"
BOOTSTRAP_CODESPACE = FLEET_SCRIPTS / "bootstrap-codespace.sh"
BOOTSTRAP_LAPTOP = FLEET_SCRIPTS / "bootstrap-laptop-service.sh"
MUSE_BOOTSTRAP = ROOT / "scripts" / "muse" / "bootstrap.sh"


FAKE_FLEET_SCRIPT = """#!/usr/bin/env bash
set -euo pipefail
# Enforce exact binary filename check just like the real fleet multicall binary.
if [ "$(basename "$0")" != "fleet" ]; then
  echo "fleet: refused argv[0]=$0 (must be named fleet)" >&2
  exit 1
fi
LOG_FILE="${FAKE_FLEET_LOG:?}"
printf '%s\\n' "$*" >> "$LOG_FILE"
case "${1:-}" in
  version)
    echo "fleet 0.1.0 (test)"
    ;;
  agent)
    shift
    case "${1:-}" in
      enroll)
        exit "${FAKE_ENROLL_EXIT:-0}"
        ;;
      doctor)
        exit "${FAKE_DOCTOR_EXIT:-0}"
        ;;
      next)
        COUNT_FILE="${FAKE_NEXT_COUNT_FILE:-}"
        if [ -n "$COUNT_FILE" ]; then
          n=0
          [ -f "$COUNT_FILE" ] && n=$(cat "$COUNT_FILE")
          n=$((n + 1))
          printf '%s' "$n" > "$COUNT_FILE"
          if [ "$n" -ge "${FAKE_NEXT_EXIT5_ON:-999}" ]; then
            exit 5
          fi
        fi
        if [ "${FAKE_NEXT_EXIT:-0}" = "5" ]; then
          exit 5
        fi
        echo '{"agent":"test","rung":"shortlist","task":{"issue":12236}}'
        exit "${FAKE_NEXT_EXIT:-0}"
        ;;
      *)
        exit 2
        ;;
    esac
    ;;
  *)
    exit 2
    ;;
esac
"""


class _BytesHandler(http.server.BaseHTTPRequestHandler):
    payload: bytes = b""
    requests_count: int = 0

    def do_GET(self):  # noqa: N802
        type(self).requests_count += 1
        self.send_response(200)
        self.send_header("Content-Length", str(len(self.payload)))
        self.end_headers()
        self.wfile.write(self.payload)

    def log_message(self, format, *args):  # noqa: A002
        return


def serve_bytes(payload: bytes):
    class Handler(_BytesHandler):
        pass

    Handler.payload = payload
    Handler.requests_count = 0
    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    thread = threading.Thread(target=server.serve_forever, daemon=True)
    thread.start()
    port = server.server_address[1]
    return server, Handler, f"http://127.0.0.1:{port}/fleet"


class TestFleetBootstrap(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp(prefix="fleet-bootstrap-test-"))
        self.home = self.tmp / "home"
        self.home.mkdir(parents=True)
        self.bin_dir = self.tmp / "bin"
        self.log_file = self.tmp / "fleet.log"
        self.payload = FAKE_FLEET_SCRIPT.encode("utf-8")
        self.sha256 = hashlib.sha256(self.payload).hexdigest()

    def tearDown(self):
        shutil.rmtree(self.tmp, ignore_errors=True)

    def base_env(self, extra=None):
        env = os.environ.copy()
        env.pop("FLEET_AGENT", None)
        env.pop("FLEET_AGENT_GROUPS", None)
        env["HOME"] = str(self.home)
        env["FAKE_FLEET_LOG"] = str(self.log_file)
        if extra:
            env.update(extra)
        return env

    def run_script(self, script: Path, args: list[str], extra_env=None):
        return subprocess.run(
            ["bash", str(script), *args],
            cwd=ROOT,
            env=self.base_env(extra_env),
            text=True,
            capture_output=True,
            check=False,
        )

    def test_scripts_exist_and_help_exits_zero(self):
        for script in (
            BOOTSTRAP,
            BOOTSTRAP_LINUX,
            BOOTSTRAP_CODESPACE,
            BOOTSTRAP_LAPTOP,
        ):
            self.assertTrue(script.is_file(), f"missing script: {script}")
            res = self.run_script(script, ["--help"])
            self.assertEqual(res.returncode, 0, res.stderr)
            self.assertIn("fleet agent next", res.stdout)
            res_short = self.run_script(script, ["-h"])
            self.assertEqual(res_short.returncode, 0, res_short.stderr)

    def test_strict_argument_contract_rejects_bad_args_before_side_effects(self):
        cases = [
            (BOOTSTRAP, ["--bogus"]),
            (BOOTSTRAP, ["extra-positional"]),
            (BOOTSTRAP, ["--seat"]),
            (BOOTSTRAP, ["--seat="]),
            (BOOTSTRAP, ["--seat=unknown"]),
            (BOOTSTRAP, ["--kind"]),
            (BOOTSTRAP, ["--name", "Bad Name!"]),
            (BOOTSTRAP, ["--bin-dir"]),
            (BOOTSTRAP, ["--fleet-sha256", "not-hex"]),
            (BOOTSTRAP, ["--max-iterations", "abc"]),
            (BOOTSTRAP_LINUX, ["--seat", "codespace"]),
            (BOOTSTRAP_CODESPACE, ["--seat=linux"]),
            (BOOTSTRAP_LAPTOP, ["stray"]),
        ]
        for script, args in cases:
            with self.subTest(script=script.name, args=args):
                res = self.run_script(script, args)
                self.assertEqual(res.returncode, 2, (res.stdout, res.stderr))
                self.assertFalse(
                    self.bin_dir.exists(),
                    f"{script.name} {args} created bin_dir despite invalid args",
                )

    def test_sha256_mismatch_fails_and_leaves_no_fleet_binary(self):
        server, _, url = serve_bytes(self.payload)
        try:
            bad_sha = "0" * 64
            res = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    bad_sha,
                ],
            )
            self.assertEqual(res.returncode, 1, (res.stdout, res.stderr))
            self.assertIn("sha256", res.stderr.lower())
            self.assertFalse((self.bin_dir / "fleet").exists())
            self.assertFalse(self.log_file.exists())
        finally:
            server.shutdown()
            server.server_close()

    def test_linux_installs_verifies_enrolls_doctors_and_runs_loop_idempotently(self):
        server, handler, url = serve_bytes(self.payload)
        try:
            res = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
            )
            self.assertEqual(res.returncode, 0, (res.stdout, res.stderr))
            fleet_bin = self.bin_dir / "fleet"
            self.assertTrue(fleet_bin.is_file())
            self.assertTrue(bool(fleet_bin.stat().st_mode & stat.S_IXUSR))
            self.assertEqual(handler.requests_count, 1)

            calls = self.log_file.read_text().splitlines()
            self.assertEqual(
                calls,
                [
                    "agent enroll --kind generic --name generic",
                    "agent doctor --agent generic",
                    "agent next --agent generic",
                ],
            )

            # Second run with matching sha256 must skip re-downloading.
            res2 = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                    "--no-next",
                ],
            )
            self.assertEqual(res2.returncode, 0, (res2.stdout, res2.stderr))
            self.assertEqual(handler.requests_count, 1)
            calls2 = self.log_file.read_text().splitlines()
            self.assertEqual(
                calls2[3:],
                [
                    "agent enroll --kind generic --name generic",
                    "agent doctor --agent generic",
                ],
            )
        finally:
            server.shutdown()
            server.server_close()

    def test_codespace_and_laptop_service_defaults_and_overrides(self):
        server, _, url = serve_bytes(self.payload)
        try:
            res_cs = self.run_script(
                BOOTSTRAP_CODESPACE,
                [
                    "--kind",
                    "claude-code",
                    "--name",
                    "cs-1",
                    "--groups",
                    "claude-code,codespace",
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
            )
            self.assertEqual(res_cs.returncode, 0, (res_cs.stdout, res_cs.stderr))

            res_lap = self.run_script(
                BOOTSTRAP_LAPTOP,
                [
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
            )
            self.assertEqual(res_lap.returncode, 0, (res_lap.stdout, res_lap.stderr))

            calls = self.log_file.read_text().splitlines()
            self.assertEqual(
                calls,
                [
                    "agent enroll --kind claude-code --name cs-1 --groups claude-code,codespace",
                    "agent doctor --agent cs-1",
                    "agent next --agent cs-1",
                    "agent enroll --kind laptop --name laptop",
                    "agent doctor --agent laptop",
                    "agent next --agent laptop",
                ],
            )
        finally:
            server.shutdown()
            server.server_close()

    def test_dry_run_and_exit5_and_loop_iterations(self):
        server, _, url = serve_bytes(self.payload)
        try:
            res_dry = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--dry-run",
                    "--name",
                    "grok",
                    "--kind",
                    "grok",
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
            )
            self.assertEqual(res_dry.returncode, 0, (res_dry.stdout, res_dry.stderr))
            self.assertEqual(
                self.log_file.read_text().splitlines(),
                ["agent enroll --kind grok --name grok --dry-run"],
            )

            self.log_file.unlink()
            count_file = self.tmp / "next-count"
            res_loop = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--name",
                    "grok",
                    "--kind",
                    "grok",
                    "--max-iterations",
                    "3",
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
                extra_env={
                    "FAKE_NEXT_COUNT_FILE": str(count_file),
                    "FAKE_NEXT_EXIT5_ON": "2",
                },
            )
            self.assertEqual(res_loop.returncode, 0, (res_loop.stdout, res_loop.stderr))
            self.assertEqual(
                self.log_file.read_text().splitlines(),
                [
                    "agent enroll --kind grok --name grok",
                    "agent doctor --agent grok",
                    "agent next --agent grok",
                    "agent next --agent grok",
                ],
            )
            self.assertIn("no work on any rung", res_loop.stdout)
        finally:
            server.shutdown()
            server.server_close()

    def test_muse_seat_delegates_to_muse_bootstrap_help(self):
        self.assertTrue(MUSE_BOOTSTRAP.is_file())
        res = self.run_script(BOOTSTRAP, ["--seat", "muse", "--help"])
        self.assertEqual(res.returncode, 0, res.stderr)
        self.assertIn("Phases", res.stdout)



    def test_fleet_agent_env_default_and_no_rungs_fails(self):
        server, _, url = serve_bytes(self.payload)
        try:
            res_env = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--kind",
                    "grok",
                    "--no-next",
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
                extra_env={"FLEET_AGENT": "grok-2"},
            )
            self.assertEqual(res_env.returncode, 0, (res_env.stdout, res_env.stderr))
            self.assertEqual(
                self.log_file.read_text().splitlines(),
                [
                    "agent enroll --kind grok --name grok-2",
                    "agent doctor --agent grok-2",
                ],
            )

            res_down = self.run_script(
                BOOTSTRAP_LINUX,
                [
                    "--name",
                    "grok-down",
                    "--bin-dir",
                    str(self.bin_dir),
                    "--fleet-url",
                    url,
                    "--fleet-sha256",
                    self.sha256,
                ],
                extra_env={"FAKE_ENROLL_EXIT": "3", "FAKE_DOCTOR_EXIT": "3"},
            )
            self.assertEqual(res_down.returncode, 1, (res_down.stdout, res_down.stderr))
            self.assertIn("no usable intake rungs", res_down.stderr)
        finally:
            server.shutdown()
            server.server_close()


if __name__ == "__main__":
    unittest.main()
