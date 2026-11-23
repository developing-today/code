#!/usr/bin/env python3
"""Offline claimant/poll tests (#8742). No arguments, no live smoke mode.

Oracle: existing mocked unit suites, plus explicit fail-closed I/O guards.
The guard is process-local and cannot affect an operator's running claimant.
"""

import sys

if len(sys.argv) != 1:
    print("usage: run_hermetic_tests.py (no arguments)", file=sys.stderr)
    raise SystemExit(2)

import importlib.util
import os
import tempfile
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parent
sys.dont_write_bytecode = True
violations = []


def guard(event, args):
    if event in {
        "subprocess.Popen",
        "os.system",
        "os.exec",
        "os.posix_spawn",
        "os.fork",
    } or event.startswith("socket."):
        violations.append(event)
        raise RuntimeError("offline test attempted external I/O: " + event)


def load(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


with tempfile.TemporaryDirectory(prefix="claimant-offline-") as tmp:
    # Reset all inherited service/credential overrides before module imports.
    os.environ.clear()
    os.environ.update(
        HOME=tmp,
        XDG_STATE_HOME=tmp + "/state",
        XDG_CONFIG_HOME=tmp + "/config",
        OPENCODE_AUTH_FILE=tmp + "/absent-auth.json",
    )
    # Tests using tempfile also write inside the disposable tree.
    tempfile.tempdir = tmp
    sys.addaudithook(guard)
    suite = unittest.TestSuite()
    for name, path in (
        ("offline_claimant", ROOT / "test_claimant.py"),
        ("offline_babysitter", ROOT / "test_babysitter.py"),
        ("offline_nudges", ROOT / "test_nudges.py"),
        ("offline_poll", ROOT / "test_poll.py"),
        ("offline_mode", ROOT / "test" / "test_claimant.py"),
        ("offline_prompt_constants", ROOT / "test_prompt_constants.py"),
    ):
        suite.addTests(unittest.defaultTestLoader.loadTestsFromModule(load(name, path)))
    result = unittest.TextTestRunner(verbosity=2).run(suite)
    # Even code that catches the guard exception cannot turn an attempted
    # real queue/server call into a successful offline check.
    if violations:
        print(
            "Forbidden external I/O attempted: " + ", ".join(violations),
            file=sys.stderr,
        )
    status = 0 if result.wasSuccessful() and not violations else 1
raise SystemExit(status)
