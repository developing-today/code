#!/usr/bin/env python3
"""Behavior tests for the muse bootstrap check phase's 1Password leg (#13048).

The check phase verifies the `op` CLI + service account, but `op` only
arrives with the restore phase (it is restored from ~/tools). A seat that
intentionally skips restore -- a non-muse seat run with
`--skip restore,persist,init`, grok being the first -- has the service
account token yet no `op` on PATH, and the check failed it for a phase it
was told to skip. The fix: when restore is out of scope for the run
(`--skip restore`, or `--only` without restore), the op leg warns and
does not fail the check; when restore was in scope, the hard failure
stays.

The tests run the real scripts/muse/bootstrap.sh in a throwaway HOME
with fake `fleet`/`restic`/`bws` binaries and a fake CONNECT proxy for
the laptop-agent leg, so every other check leg passes and the op leg is
the only variable. No network, no real secrets.
"""

from __future__ import annotations

import json
import shutil
import socket
import subprocess
import tempfile
import time
import threading
import unittest
from pathlib import Path


ROOT = Path(__file__).resolve().parents[2]
BOOTSTRAP = ROOT / "scripts" / "muse" / "bootstrap.sh"

FAKE_FLEET = """#!/usr/bin/env bash
case "${1:-} ${2:-}" in
  "capabilities ")
    printf '%s\\n' '{"domains":[{"name":"bridge","available":true}]}'
    ;;
  "secrets get"|"secrets list")
    ;;
  "bridge hello")
    exit "${FAKE_BRIDGE_HELLO_RC:-0}"
    ;;
  "bridge inbox")
    printf '%s\\n' '[]'
    ;;
  *)
    echo "fake fleet: unhandled: $*" >&2
    exit 1
    ;;
esac
exit 0
"""

FAKE_RESTIC = """#!/usr/bin/env bash
case "${1:-}" in
  restore)
    ;;
  snapshots)
    printf '%s\\n' '[]'
    ;;
  *)
    echo "fake restic: unhandled: $*" >&2
    exit 1
    ;;
esac
exit 0
"""

FAKE_BWS = """#!/usr/bin/env bash
if [ "${1:-}" = "secret" ] && [ "${2:-}" = "list" ]; then
  cat "${FAKE_BWS_FIXTURE:?}"
  exit 0
fi
echo "fake bws: unhandled: $*" >&2
exit 1
"""

# The names restic_env asks `secret` for during the restore phase.
RESTORE_SECRETS = {
    "cloudflare.r2-backups-access-key-id": "fake-access-key-id",
    "cloudflare.r2-backups-secret-access-key": "fake-secret-access-key",
    "restic.sandbox-config-password": "fake-config-password",
    "restic.sandbox-home-password": "fake-home-password",
}

SKIP_ALL_BUT_CHECK = "tools,token,restore,tailnet,fleet,persist,init"
SKIP_ALL_BUT_RESTORE_CHECK = "tools,token,tailnet,fleet,persist,init"

HARD_FAILURE = "did not verify (op is restored from ~/tools"
WARNING = "restore phase was skipped"


class ConnectProxy(threading.Thread):
    """One-shot-loop CONNECT proxy: answers 200 to any CONNECT."""

    def __init__(self) -> None:
        super().__init__(daemon=True)
        self.sock = socket.socket()
        self.sock.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
        # The previous test's proxy may still be releasing the port.
        for attempt in range(25):
            try:
                self.sock.bind(("127.0.0.1", 3130))
                break
            except OSError:
                if attempt == 24:
                    raise
                time.sleep(0.2)
        self.sock.listen(5)
        self.sock.settimeout(0.5)
        self.stopped = threading.Event()

    def run(self) -> None:
        while not self.stopped.is_set():
            try:
                conn, _ = self.sock.accept()
            except socket.timeout:
                continue
            except OSError:
                return
            with conn:
                conn.settimeout(5)
                try:
                    data = b""
                    while b"\r\n\r\n" not in data:
                        chunk = conn.recv(4096)
                        if not chunk:
                            break
                        data += chunk
                    conn.sendall(b"HTTP/1.1 200 Connection established\r\n\r\n")
                except OSError:
                    pass

    def stop(self) -> None:
        self.stopped.set()
        self.sock.close()


class TestMuseBootstrapCheckOp(unittest.TestCase):
    def setUp(self) -> None:
        self.tmp = tempfile.TemporaryDirectory(prefix="muse-bootstrap-check-op-")
        base = Path(self.tmp.name)
        self.home = base / "home"
        self.tools_bin = self.home / "tools" / "bin"
        self.tools_bin.mkdir(parents=True)
        bin_dir = self.home / ".local" / "share" / "muse-bootstrap" / "bin"
        bin_dir.mkdir(parents=True)
        self._install(self.tools_bin / "fleet", FAKE_FLEET)
        self._install(self.tools_bin / "restic", FAKE_RESTIC)
        self._install(bin_dir / "bws", FAKE_BWS)
        self.fixture = base / "secrets.json"
        self.fixture.write_text(
            json.dumps([{"key": k, "value": v} for k, v in RESTORE_SECRETS.items()])
        )
        self.ts_state = base / "tsstate"
        self.ts_state.mkdir()
        (self.ts_state / "mode").write_text("userspace\n")
        # A minimal system bin dir: symlinks to the tools the script
        # needs, resolved from this box's PATH -- deliberately without
        # `op`, whose absence is the condition under test. (A bare
        # /usr/bin does not work everywhere: on NixOS bash lives in the
        # profile bin, and on the laptop `op` sits in /run/wrappers/bin.)
        self.sys_bin = base / "sysbin"
        self.sys_bin.mkdir()
        for name in (
            "bash",
            "sh",
            "env",
            "uname",
            "id",
            "curl",
            "tar",
            "python3",
            "sha256sum",
            "sed",
            "grep",
            "cut",
            "tr",
            "ls",
            "cat",
            "chmod",
            "mv",
            "rm",
            "mkdir",
            "dirname",
            "mktemp",
            "wc",
            "head",
            "tail",
            "sort",
            "readlink",
            "realpath",
            "basename",
            "sleep",
            "awk",
            "find",
            "xargs",
            "date",
            "hostname",
            # `timeout` bounds the check-phase bridge hello/inbox calls
            # (#13045); without it on PATH the bridge leg fails with
            # "command not found" and the check hard-fails (#13674).
            "timeout",
            "touch",
            "ln",
            "cp",
            "stat",
        ):
            resolved = shutil.which(name)
            if resolved and not (self.sys_bin / name).exists():
                (self.sys_bin / name).symlink_to(resolved)
        self.assertIsNotNone(shutil.which("bash", path=str(self.sys_bin)))
        self.assertIsNotNone(shutil.which("python3", path=str(self.sys_bin)))
        # The check leg dials the laptop agent via 127.0.0.1:3130 in
        # userspace mode. If that port is already taken on this box the
        # laptop leg cannot be made to pass here; the op-leg assertions
        # on stderr still hold, only the exit-code assertion relaxes.
        self.proxy = None
        try:
            self.proxy = ConnectProxy()
            self.proxy.start()
        except OSError:
            self.proxy = None

    def tearDown(self) -> None:
        if self.proxy is not None:
            self.proxy.stop()
            self.proxy.join(timeout=2)
        self.tmp.cleanup()

    @staticmethod
    def _install(path: Path, content: str) -> None:
        path.write_text(content)
        path.chmod(0o755)

    def run_bootstrap(
        self, skip: str, extra_env: dict[str, str] | None = None
    ) -> subprocess.CompletedProcess[str]:
        env = {
            "HOME": str(self.home),
            "PATH": f"{self.tools_bin}:{self.sys_bin}",
            "BWS_ACCESS_TOKEN": "test-access-token",
            # The grok-seat shape from #13048: the service-account token
            # is present, only the `op` CLI is missing.
            "OP_SERVICE_ACCOUNT_TOKEN": "test-op-token",
            "FAKE_BWS_FIXTURE": str(self.fixture),
            "TS_STATE": str(self.ts_state),
        }
        if extra_env:
            env.update(extra_env)
        if shutil.which("op", path=env["PATH"]):
            self.skipTest(
                "an `op` binary is on the test PATH; cannot stage its absence"
            )
        return subprocess.run(
            ["bash", str(BOOTSTRAP), "--skip", skip],
            env=env,
            capture_output=True,
            text=True,
            timeout=180,
        )

    def test_op_missing_warns_when_restore_skipped(self) -> None:
        result = self.run_bootstrap(SKIP_ALL_BUT_CHECK)
        self.assertIn(WARNING, result.stderr)
        self.assertNotIn(HARD_FAILURE, result.stderr)
        if self.proxy is not None:
            self.assertEqual(result.returncode, 0, result.stderr)

    def test_op_missing_fails_when_restore_in_scope(self) -> None:
        result = self.run_bootstrap(SKIP_ALL_BUT_RESTORE_CHECK)
        self.assertIn(HARD_FAILURE, result.stderr)
        self.assertNotIn(WARNING, result.stderr)
        self.assertEqual(result.returncode, 1, result.stderr)

    def test_bridge_hello_failure_fails_check_when_restore_skipped(self) -> None:
        # The bridge leg still gates the check even when restore is
        # skipped: a genuine bridge-hello failure must hard-fail (#13674).
        result = self.run_bootstrap(
            SKIP_ALL_BUT_CHECK, {"FAKE_BRIDGE_HELLO_RC": "1"}
        )
        self.assertIn("bridge hello failed", result.stderr)
        self.assertIn(WARNING, result.stderr)
        self.assertNotIn(HARD_FAILURE, result.stderr)
        self.assertEqual(result.returncode, 1, result.stderr)


if __name__ == "__main__":
    unittest.main()
