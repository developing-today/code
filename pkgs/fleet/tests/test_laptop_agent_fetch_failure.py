#!/usr/bin/env python3
"""Tests for laptop-agent-legacy.py fetch failure parity (code#16088).

Oracle: the module's own loud-failure rule (silence audit, code#5343) —
a failed call never looks like an empty success, transport failures
exit 1 with a named one-line `laptop-agent: ...` on stderr, no
tracebacks — plus binary parity: a fetch that fails before the first
byte is in hand creates no OUT file.

Run with: python3 -m unittest discover -s pkgs/fleet/tests
"""

import contextlib
import importlib.util
import io
import os
import sys
import tempfile
import unittest
from pathlib import Path
from unittest.mock import patch

LEGACY = Path(__file__).resolve().parents[1] / "laptop-agent-legacy.py"


def _load_legacy():
    spec = importlib.util.spec_from_file_location("laptop_agent_legacy", LEGACY)
    mod = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(mod)
    return mod


la = _load_legacy()


def _run_fetch(argv):
    """Run legacy main() with argv; return (exit_code, stderr_text)."""
    err = io.StringIO()
    with patch.object(sys, "argv", ["laptop-agent-legacy.py", *argv]):
        with contextlib.redirect_stderr(err):
            try:
                la.main()
            except SystemExit as e:
                return e.code, err.getvalue()
    return 0, err.getvalue()


class FetchFailureTest(unittest.TestCase):
    """A fetch whose transport fails must fail loud and leave no file."""

    def test_transport_failure_is_one_line_no_traceback_no_out_file(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "OUT")
            failing = RuntimeError("transport error: Connection refused")
            with patch.object(la, "_fetch_chunk", side_effect=failing):
                code, stderr = _run_fetch(["fetch", "/remote/path", out])
            # sys.exit(str) exits the process with status 1 and prints
            # the line on stderr: the module's loud-failure contract.
            self.assertEqual(
                code, "laptop-agent: fetch failed: transport error: Connection refused"
            )
            self.assertNotIn("Traceback", stderr)
            self.assertEqual(stderr, "")
            self.assertFalse(os.path.exists(out))

    def test_transport_failure_leaves_pre_existing_out_untouched(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "OUT")
            Path(out).write_bytes(b"partial-bytes")
            failing = RuntimeError("transport error: Connection refused")
            with patch.object(la, "_fetch_chunk", side_effect=failing):
                code, stderr = _run_fetch(["fetch", "/remote/path", out])
            self.assertEqual(
                code, "laptop-agent: fetch failed: transport error: Connection refused"
            )
            self.assertNotIn("Traceback", stderr)
            self.assertEqual(Path(out).read_bytes(), b"partial-bytes")

    def test_successful_fetch_still_writes_out(self):
        with tempfile.TemporaryDirectory() as d:
            out = os.path.join(d, "OUT")
            with patch.object(
                la, "_fetch_chunk", return_value=(b"hello", 5)
            ):
                code, stderr = _run_fetch(["fetch", "/remote/path", out])
            self.assertEqual(code, 0)
            self.assertEqual(Path(out).read_bytes(), b"hello")
            self.assertIn("fetched 5 bytes", stderr)


if __name__ == "__main__":
    unittest.main()
