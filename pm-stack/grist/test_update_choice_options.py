"""Unit tests for the pure merge logic in update-choice-options.py.

Run: python3 -m unittest test_update_choice_options -v
(from this directory; stdlib only, no Grist needed.)
"""

import importlib.util
import io
import json
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


class MergeChoicesShape(unittest.TestCase):
    """#6588: a stored shape the merge does not understand is refused
    by name -- never shredded, coerced, or crashed on."""

    def test_string_choices_refused_not_shredded(self):
        wo = {"choices": "code"}
        with self.assertRaises(_mod.WidgetOptionsError) as raised:
            merge_choices(wo, ["id"])
        self.assertIn("not a list", str(raised.exception))
        self.assertEqual(wo, {"choices": "code"})

    def test_dict_choices_refused(self):
        with self.assertRaises(_mod.WidgetOptionsError):
            merge_choices({"choices": {"code": True}}, ["id"])

    def test_non_object_widget_options_refused(self):
        for value in ("justastring", ["code"], 42):
            with self.subTest(value=value):
                with self.assertRaises(_mod.WidgetOptionsError) as raised:
                    merge_choices(value, ["x"])
                self.assertIn("not a JSON object", str(raised.exception))

    def test_non_string_choice_entries_refused(self):
        with self.assertRaises(_mod.WidgetOptionsError) as raised:
            merge_choices({"choices": [1, None, True]}, ["id"])
        self.assertIn("entry 0 is not a string", str(raised.exception))

    def test_null_choices_still_means_no_choices(self):
        merged, added = merge_choices({"choices": None}, ["a"])
        self.assertEqual(merged["choices"], ["a"])
        self.assertEqual(added, ["a"])


class ShapeFailureCli(unittest.TestCase):
    """#6588 at the CLI boundary: a shape failure is a clean exit 1
    on stderr, and no PATCH is ever sent."""

    def run_with_stored(self, widget_options_json):
        patches = []

        def fake_api(base, key, method, path, body=None):
            if method == "PATCH":
                patches.append(body)
                return None
            return {"columns": [{"id": "Area", "fields": {"widgetOptions": widget_options_json}}]}

        stderr = io.StringIO()
        with patch.object(_mod, "api", fake_api), patch.dict(
                os.environ, {"GRIST_API_KEY": "test"}), redirect_stderr(stderr):
            exit_code = _mod.run([
                "--doc", "doc", "--table", "Roadmap", "--column", "Area",
                "--add", "id",
            ])
        return exit_code, stderr.getvalue(), patches

    def test_string_choices_exit_1_and_no_patch(self):
        exit_code, stderr, patches = self.run_with_stored(json.dumps({"choices": "code"}))
        self.assertEqual(exit_code, 1)
        self.assertIn("not a list", stderr)
        self.assertNotIn("Traceback", stderr)
        self.assertEqual(patches, [])

    def test_non_object_widget_options_exit_1_and_no_patch(self):
        exit_code, stderr, patches = self.run_with_stored(json.dumps("justastring"))
        self.assertEqual(exit_code, 1)
        self.assertIn("not a JSON object", stderr)
        self.assertEqual(patches, [])

    def test_invalid_json_widget_options_exit_1_and_no_patch(self):
        exit_code, stderr, patches = self.run_with_stored("{not json")
        self.assertEqual(exit_code, 1)
        self.assertIn("not valid JSON", stderr)
        self.assertEqual(patches, [])


class EnvelopeShape(unittest.TestCase):
    """#6867: the API envelope around widgetOptions is shape-checked
    by name -- a wrong-shape 200 is a named error, never a raw
    KeyError / TypeError / AttributeError traceback."""

    def get_with_data(self, data):
        with patch.object(_mod, "api", lambda *a, **k: data):
            return _mod.get_widget_options("base", "key", "doc", "Roadmap", "Area")

    def test_missing_columns_refused(self):
        for data in ({}, {"error": "nope"}):
            with self.subTest(data=data):
                with self.assertRaises(_mod.ApiResponseError) as raised:
                    self.get_with_data(data)
                self.assertIn("no 'columns' list", str(raised.exception))

    def test_non_list_columns_refused(self):
        for columns in ("Area", None, {"id": "Area"}, 42):
            with self.subTest(columns=columns):
                with self.assertRaises(_mod.ApiResponseError):
                    self.get_with_data({"columns": columns})

    def test_non_object_top_level_refused(self):
        for data in ([{"id": "Area"}], "columns", None, 42):
            with self.subTest(data=data):
                with self.assertRaises(_mod.ApiResponseError) as raised:
                    self.get_with_data(data)
                self.assertIn("not a JSON object", str(raised.exception))

    def test_entry_without_fields_refused(self):
        for col in ({"id": "Area"}, {"id": "Area", "fields": None},
                    {"id": "Area", "fields": []}):
            with self.subTest(col=col):
                with self.assertRaises(_mod.ApiResponseError) as raised:
                    self.get_with_data({"columns": [col]})
                self.assertIn("no 'fields' object", str(raised.exception))

    def test_non_object_entry_refused(self):
        for col in ("Area", ["Area"], None):
            with self.subTest(col=col):
                with self.assertRaises(_mod.ApiResponseError):
                    self.get_with_data({"columns": [col]})

    def test_non_string_widget_options_refused(self):
        for raw in ({"choices": ["id"]}, 42, ["id"]):
            with self.subTest(raw=raw):
                data = {"columns": [{"id": "Area", "fields": {"widgetOptions": raw}}]}
                with self.assertRaises(_mod.WidgetOptionsError) as raised:
                    self.get_with_data(data)
                self.assertIn("is not a string", str(raised.exception))

    def test_controls_unchanged(self):
        empty = {"columns": [{"id": "Area", "fields": {"widgetOptions": ""}}]}
        self.assertEqual(self.get_with_data(empty), {})
        no_key = {"columns": [{"id": "Area", "fields": {}}]}
        self.assertEqual(self.get_with_data(no_key), {})
        valid = {"columns": [{"id": "Area", "fields": {
            "widgetOptions": json.dumps({"choices": ["id"]})}}]}
        self.assertEqual(self.get_with_data(valid), {"choices": ["id"]})

    def test_absent_column_still_not_found(self):
        data = {"columns": [{"id": "Other", "fields": {"widgetOptions": ""}}]}
        with self.assertRaises(SystemExit) as raised:
            self.get_with_data(data)
        self.assertIn("not found", str(raised.exception))


class EnvelopeShapeCli(unittest.TestCase):
    """#6867 at the CLI boundary: a wrong-shape envelope is a clean
    exit 1 on stderr, and no PATCH is ever sent."""

    def run_with_data(self, data):
        patches = []

        def fake_api(base, key, method, path, body=None):
            if method == "PATCH":
                patches.append(body)
                return None
            return data

        stderr = io.StringIO()
        with patch.object(_mod, "api", fake_api), patch.dict(
                os.environ, {"GRIST_API_KEY": "test"}), redirect_stderr(stderr):
            exit_code = _mod.run([
                "--doc", "doc", "--table", "Roadmap", "--column", "Area",
                "--add", "id",
            ])
        return exit_code, stderr.getvalue(), patches

    def test_missing_columns_exit_1_and_no_patch(self):
        exit_code, stderr, patches = self.run_with_data({})
        self.assertEqual(exit_code, 1)
        self.assertIn("no 'columns' list", stderr)
        self.assertNotIn("Traceback", stderr)
        self.assertEqual(patches, [])

    def test_non_string_widget_options_exit_1_and_no_patch(self):
        data = {"columns": [{"id": "Area", "fields": {"widgetOptions": {"choices": []}}}]}
        exit_code, stderr, patches = self.run_with_data(data)
        self.assertEqual(exit_code, 1)
        self.assertIn("is not a string", stderr)
        self.assertNotIn("Traceback", stderr)
        self.assertEqual(patches, [])


class NonJsonResponseBody(unittest.TestCase):
    """#6867: a non-JSON 200 body from api() is a named API failure
    naming method + path, not a raw JSONDecodeError."""

    def serve_body(self, body):
        class Handler(BaseHTTPRequestHandler):
            def do_GET(self):
                self.send_response(200)
                self.send_header("Content-Type", "text/html")
                self.end_headers()
                self.wfile.write(body.encode())

            def log_message(self, format, *args):
                pass

        server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        return server, thread

    def test_api_non_json_body_is_named_failure(self):
        server, thread = self.serve_body("<html>not json</html>")
        try:
            base = f"http://127.0.0.1:{server.server_port}"
            with self.assertRaises(_mod.ApiResponseError) as raised:
                _mod.api(base, "test", "GET", "/api/docs/doc/tables/Roadmap/columns")
            self.assertIn("GET", str(raised.exception))
            self.assertIn("/api/docs/doc/tables/Roadmap/columns", str(raised.exception))
            self.assertIn("not valid JSON", str(raised.exception))
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=1)

    def test_run_non_json_body_exits_1(self):
        server, thread = self.serve_body("not json at all")
        stderr = io.StringIO()
        try:
            with patch.dict(os.environ, {"GRIST_API_KEY": "test"}), redirect_stderr(stderr):
                exit_code = _mod.run([
                    "--base", f"http://127.0.0.1:{server.server_port}",
                    "--doc", "doc", "--table", "Roadmap", "--column", "Area",
                    "--add", "id",
                ])
            self.assertEqual(exit_code, 1)
            self.assertIn("not valid JSON", stderr.getvalue())
            self.assertNotIn("Traceback", stderr.getvalue())
        finally:
            server.shutdown()
            server.server_close()
            thread.join(timeout=1)


if __name__ == "__main__":
    unittest.main()
