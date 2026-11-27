#!/usr/bin/env python3
"""Hermetic acceptance tests. Oracle: documented Python CLI v1 selector contract.
No daemon, secrets, network, Rust build, or installed fleet is used.
"""

import contextlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile
import unittest
from unittest.mock import patch

ROOT = Path(__file__).resolve().parents[1]


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


selector = load("selector", ROOT / "laptop-agent.py")
installer = load("installer", ROOT / "install-laptop-agent-client.py")
DOCUMENT = {
    "compatibility_contracts": [selector.CONTRACT],
    "domains": [{"name": "laptop-agent", "available": True, "state": "available"}],
}


class SelectorTests(unittest.TestCase):
    """Oracle: marker AND availability, never a version/domain-only inference."""

    def test_marker_gate(self):
        self.assertTrue(selector.compatible(DOCUMENT))
        for doc in (
            None,
            [],
            {},
            {"build": {"version": "0.1.0"}},
            {"domains": DOCUMENT["domains"]},
            dict(DOCUMENT, compatibility_contracts=["laptop-agent-python-cli/v0"]),
            dict(DOCUMENT, compatibility_contracts=selector.CONTRACT),
            dict(DOCUMENT, domains=[{"name": "laptop-agent", "available": False}]),
            dict(
                DOCUMENT,
                domains=[
                    {"name": "laptop-agent", "available": 1, "state": "available"}
                ],
            ),
        ):
            with self.subTest(doc=doc):
                self.assertFalse(selector.compatible(doc))

    def test_strict_argv_before_probe_or_legacy(self):
        invalid = [
            [],
            ["wat"],
            ["health", "extra"],
            ["jobs", "--typo"],
            ["kill"],
            ["kill", "j", "extra"],
            ["exec-batch"],
            ["exec-batch", "f", "extra"],
            ["exec", "--cwd"],
            ["exec", "--time", "2", "--", "x"],
            ["exec", "--"],
            ["run", "--timeout"],
            ["run", "--timeout=0", "--", "x"],
            ["poll", "j", "--wait"],
            ["poll", "j", "--wait=-1"],
            ["fetch", "--chunk", "0", "a", "b"],
            ["fetch", "a", "b", "extra"],
        ]
        for args in invalid:
            with (
                self.subTest(args=args),
                patch.object(sys, "argv", ["client", *args]),
                patch.object(selector.shutil, "which") as which,
                patch.object(selector, "_load_legacy") as legacy,
                contextlib.redirect_stderr(io.StringIO()),
            ):
                with self.assertRaises(SystemExit) as exited:
                    selector.select_main()
                self.assertEqual(exited.exception.code, 2)
                which.assert_not_called()
                legacy.assert_not_called()
        self.assertEqual(
            selector.validate(["fetch", "--chunk=12", "a", "b"]),
            ["fetch", "--chunk", "12", "a", "b"],
        )
        self.assertEqual(
            selector.validate(["poll", "j", "--wait=2"]), ["poll", "j", "--wait", "2"]
        )

    def test_missing_binary_and_unready_use_legacy_once(self):
        for binary, ready in ((None, False), ("/fake/fleet", False)):
            with (
                patch.object(sys, "argv", ["client", "health"]),
                patch.object(selector.shutil, "which", return_value=binary),
                patch.object(selector, "probe", return_value=ready) as probe,
                patch.object(selector, "_load_legacy") as legacy,
            ):
                selector.select_main()
                legacy.return_value.main.assert_called_once_with()
                self.assertEqual(probe.call_count, int(binary is not None))

    def test_exec_failure_never_replays(self):
        with (
            patch.object(sys, "argv", ["client", "jobs"]),
            patch.object(selector.shutil, "which", return_value="/fake/fleet"),
            patch.object(selector, "probe", return_value=True),
            patch.object(selector.os, "execv", side_effect=OSError("gone")),
            patch.object(selector, "_load_legacy") as legacy,
            contextlib.redirect_stderr(io.StringIO()),
        ):
            with self.assertRaises(SystemExit) as exited:
                selector.select_main()
            self.assertEqual(exited.exception.code, 126)
            legacy.assert_not_called()

    def test_import_api_is_legacy_without_probe(self):
        with patch.object(
            selector, "probe", side_effect=AssertionError("probe on import")
        ):
            client = load("broker_client", ROOT / "laptop-agent.py")
            self.assertTrue(callable(client.make_cf_http))
            self.assertTrue(callable(client.make_tunnel_http))
            self.assertTrue(callable(client._cf_access_headers))
            self.assertEqual(client.TARGET_PORT, 4799)
            self.assertEqual(client._agent_slug("Muse Felix"), "felix")
            client._deadline = 12345
            with patch.object(client.time, "monotonic", return_value=12340):
                self.assertEqual(client._remaining(), 5)
            self.assertIs(client._remaining.__globals__, client.__dict__)


class ProcessTests(unittest.TestCase):
    """Oracle: fake fleet's bytes/argv/status and the 2s/64KiB probe bounds."""

    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.home = Path(self.tmp.name)
        self.binary = self.home / "fleet"
        self.env = dict(os.environ, PATH=str(self.home), HOME=str(self.home))

    def fleet(self, body):
        self.binary.write_text(f"#!{sys.executable}\n" + body)
        self.binary.chmod(0o755)

    def test_probe_missing_malformed_stale_timeout_oversize_and_nonzero(self):
        self.assertFalse(selector.probe(str(self.binary)))
        for body in (
            "print('not json')",
            "print('{}')",
            "print('[]')",
            "print("
            + repr(json.dumps(dict(DOCUMENT, compatibility_contracts=["old"])))
            + ")",
            "import time; time.sleep(5)",
            "print('x' * 70000)",
            "print('[' * 2000 + '0' + ']' * 2000)",
            "import sys; print(" + repr(json.dumps(DOCUMENT)) + "); sys.exit(1)",
        ):
            with self.subTest(body=body):
                self.fleet(body)
                with patch.object(selector, "PROBE_TIMEOUT_S", 0.08):
                    self.assertFalse(selector.probe(str(self.binary)))

    def test_probe_is_fresh_not_cached(self):
        self.fleet("print(" + repr(json.dumps(DOCUMENT)) + ")")
        self.assertTrue(selector.probe(str(self.binary)))
        self.fleet("print('{}')")
        self.assertFalse(selector.probe(str(self.binary)))

    def test_all_verbs_forward_exact_arguments_streams_status_and_stdin(self):
        self.fleet(
            "import json, sys\n"
            "if sys.argv[1:] == ['capabilities']:\n"
            " print(" + repr(json.dumps(DOCUMENT)) + ")\n"
            "else:\n"
            " print(json.dumps(sys.argv[1:]))\n"
            " sys.stdout.flush()\n"
            " sys.stdout.buffer.write(sys.stdin.buffer.read())\n"
            " sys.stderr.buffer.write(b'err\\x00bytes\\n')\n"
            " sys.exit(37)\n"
        )
        cases = [
            ["health"],
            ["jobs"],
            [
                "exec",
                "--cwd",
                "space dir",
                "--timeout=3",
                "--",
                "printf",
                "a b",
                "--weird",
            ],
            ["run", "--timeout", "9", "--shell", "--", "echo x"],
            ["poll", "sshfb-old", "--wait=2"],
            ["kill", "job"],
            ["fetch", "--chunk=123", "remote", "local"],
            ["exec-batch", "batch.json"],
        ]
        for args in cases:
            with self.subTest(args=args):
                result = subprocess.run(
                    [sys.executable, str(ROOT / "laptop-agent.py"), *args],
                    input=b"in\x00bytes\n",
                    capture_output=True,
                    env=self.env,
                    timeout=5,
                )
                self.assertEqual(result.returncode, 37)
                line, rest = result.stdout.split(b"\n", 1)
                self.assertEqual(json.loads(line), ["laptop-agent", *args])
                self.assertEqual(rest, b"in\x00bytes\n")
                self.assertEqual(result.stderr, b"err\x00bytes\n")
                # A replay would reach real legacy transport and alter bytes/status.


class InstallTests(unittest.TestCase):
    """Oracle: dependency published before selector, old caller usable mid-install."""

    def test_dependency_first_and_import_after_install(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp)
            old = ROOT / "laptop-agent-legacy.py"
            (dest / "laptop-agent.py").write_bytes(old.read_bytes())
            replace = os.replace
            seen = []

            def observe(source, target):
                seen.append(Path(target).name)
                replace(source, target)
                client = load("mid_install", dest / "laptop-agent.py")
                self.assertEqual(client._agent_slug("Muse Felix"), "felix")

            with patch.object(installer.os, "replace", side_effect=observe):
                installer.install(dest)
            self.assertEqual(seen, ["laptop-agent-legacy.py", "laptop-agent.py"])
            self.assertEqual(
                (dest / "laptop-agent-legacy.py").read_bytes(), old.read_bytes()
            )

    def test_missing_source_does_not_publish_partial_pair(self):
        with tempfile.TemporaryDirectory() as tmp:
            dest = Path(tmp) / "dest"
            source = Path(tmp) / "source"
            source.mkdir()
            (source / "laptop-agent-legacy.py").write_text("legacy")
            with self.assertRaises(FileNotFoundError):
                installer.install(dest, source)
            self.assertFalse(dest.exists())


if __name__ == "__main__":
    unittest.main()
