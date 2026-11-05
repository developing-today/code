"""Unit tests for the pure merge logic in update-choice-options.py.

Run: python3 -m unittest test_update_choice_options -v
(from this directory; stdlib only, no Grist needed.)
"""

import importlib.util
import io
import os
import pathlib
import threading
import time
import unittest
from contextlib import redirect_stderr
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from unittest.mock import patch

_spec = importlib.util.spec_from_file_location(
    "update_choice_options",
    pathlib.Path(__file__).with_name("update-choice-options.py"),
)
_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(_mod)
merge_choices = _mod.merge_choices


class MergeChoices(unittest.TestCase):
    def test_appends_new_preserving_order_and_other_keys(self):
        wo = {
            "choices": ["code", "infra"],
            "choiceOptions": {"code": {"fillColor": "#1d76db"}},
        }
        merged, added = merge_choices(wo, ["id"])
        self.assertEqual(merged["choices"], ["code", "infra", "id"])
        self.assertEqual(added, ["id"])
        self.assertEqual(merged["choiceOptions"], {"code": {"fillColor": "#1d76db"}})

    def test_idempotent_no_duplicates(self):
        wo = {"choices": ["code", "infra", "id"]}
        merged, added = merge_choices(wo, ["id", "code"])
        self.assertEqual(merged["choices"], ["code", "infra", "id"])
        self.assertEqual(added, [])

    def test_empty_widget_options(self):
        merged, added = merge_choices({}, ["a", "b"])
        self.assertEqual(merged["choices"], ["a", "b"])
        self.assertEqual(added, ["a", "b"])

    def test_duplicate_additions_in_one_call(self):
        merged, added = merge_choices({"choices": []}, ["x", "x"])
        self.assertEqual(merged["choices"], ["x"])
        self.assertEqual(added, ["x"])

    def test_input_not_mutated(self):
        wo = {"choices": ["a"]}
        merge_choices(wo, ["b"])
        self.assertEqual(wo["choices"], ["a"])


class ApiTimeout(unittest.TestCase):
    def assert_black_hole_times_out(self, invoke):
        request_received = threading.Event()
        release_handler = threading.Event()

        class BlackHoleHandler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.hold_response()

            def do_PATCH(self):
                self.hold_response()

            def hold_response(self):
                request_received.set()
                release_handler.wait()

            def log_message(self, format, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), BlackHoleHandler)
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        base = f"http://127.0.0.1:{server.server_port}"
        try:
            with patch.object(_mod, "API_TIMEOUT_SECONDS", 0.05):
                started = time.monotonic()
                with self.assertRaises(TimeoutError) as raised:
                    invoke(base)
                elapsed = time.monotonic() - started

            self.assertTrue(
                request_received.wait(1), "stub server did not receive the request"
            )
            self.assertLess(elapsed, 1, "urlopen exceeded the short test timeout")
            self.assertIn("timed out after 0.05s", str(raised.exception))
        finally:
            release_handler.set()
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=1)

    def test_get_patch_and_readback_calls_time_out(self):
        calls = {
            "GET columns": lambda base: _mod.api(base, "test", "GET", "/columns"),
            "PATCH columns": lambda base: _mod.api(
                base, "test", "PATCH", "/columns", {"columns": []}),
            "read-back GET": lambda base: _mod.get_widget_options(
                base, "test", "doc", "table", "column"),
        }
        for label, invoke in calls.items():
            with self.subTest(call=label):
                self.assert_black_hole_times_out(invoke)

    def test_cli_reports_timeout_and_exits_nonzero(self):
        request_received = threading.Event()
        release_handler = threading.Event()

        class BlackHoleHandler(BaseHTTPRequestHandler):
            def do_GET(self):
                request_received.set()
                release_handler.wait()

            def log_message(self, format, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), BlackHoleHandler)
        server_thread = threading.Thread(target=server.serve_forever, daemon=True)
        server_thread.start()
        stderr = io.StringIO()
        try:
            with patch.object(_mod, "API_TIMEOUT_SECONDS", 0.05), patch.dict(
                    os.environ, {"GRIST_API_KEY": "test"}), redirect_stderr(stderr):
                exit_code = _mod.run([
                    "--base", f"http://127.0.0.1:{server.server_port}",
                    "--doc", "doc", "--table", "table", "--column", "column",
                    "--add", "choice",
                ])

            self.assertTrue(
                request_received.wait(1), "stub server did not receive the request"
            )
            self.assertEqual(exit_code, 1)
            self.assertIn("timed out after 0.05s", stderr.getvalue())
        finally:
            release_handler.set()
            server.shutdown()
            server.server_close()
            server_thread.join(timeout=1)


if __name__ == "__main__":
    unittest.main()
