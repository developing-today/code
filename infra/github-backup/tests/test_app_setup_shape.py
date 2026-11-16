#!/usr/bin/env python3
"""Regression tests for app_setup shape validation (#6849).

Run: python3 infra/github-backup/tests/test_app_setup_shape.py
"""

import contextlib
import io
import json
import os
import subprocess
import sys
import unittest
from types import SimpleNamespace
from unittest.mock import patch

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import ghb_common


def good_config():
    return {
        "app": {
            "name": "developing-today-backup",
            "events": ["push", "repository"],
            "bitwarden_secret_names": {
                "app_id": "a",
                "client_id": "b",
                "client_secret": "c",
                "private_key": "d",
                "webhook_secret": "e",
            },
        },
        "worker": {"webhook_path": "/github"},
    }


class ValidateConfigTests(unittest.TestCase):
    def test_real_config_passes(self):
        cfg = ghb_common.load_config()
        clean, problems = ghb_common.validate_config(cfg)
        self.assertEqual(problems, [])
        self.assertEqual(clean, cfg)

    def test_good_config_passes(self):
        clean, problems = ghb_common.validate_config(good_config())
        self.assertEqual(problems, [])
        self.assertEqual(clean["app"]["name"], "developing-today-backup")

    def test_wrong_shapes_name_the_field(self):
        cases = []
        cfg = good_config()
        del cfg["app"]
        cases.append((cfg, "'app'"))
        cfg = good_config()
        cfg["app"] = [1]
        cases.append((cfg, "'app'"))
        cfg = good_config()
        cfg["app"]["bitwarden_secret_names"] = "names"
        cases.append((cfg, "bitwarden_secret_names"))
        cfg = good_config()
        del cfg["app"]["bitwarden_secret_names"]["pem" if False else "private_key"]
        cases.append((cfg, "private_key"))
        cfg = good_config()
        cfg["app"]["events"] = "push"
        cases.append((cfg, "app.events"))
        cfg = good_config()
        cfg["worker"] = {}
        cases.append((cfg, "webhook_path"))
        cases.append(([1, 2], "top level"))
        for cfg, needle in cases:
            clean, problems = ghb_common.validate_config(cfg)
            self.assertEqual(clean, {}, needle)
            self.assertTrue(any(needle in p for p in problems), (needle, problems))


class AppSetupImportTests(unittest.TestCase):
    def test_import_with_real_config(self):
        import app_setup

        self.assertEqual(app_setup.NAMES["app_id"], "github-app.backup-app-id")
        manifest = app_setup.app_manifest("https://worker.example")
        self.assertEqual(manifest["name"], "developing-today-backup")
        self.assertTrue(manifest["hook_attributes"]["url"].endswith("/github"))

    def test_import_with_wrong_shape_config_exits_cleanly(self):
        driver = (
            "import sys; sys.path.insert(0, %r)\n"
            "import ghb_common\n"
            "def bad(): raise ValueError(\"config.json: 'app' is a list, expected an object\")\n"
            "ghb_common.load_config = bad\n"
            "import app_setup\n" % BIN
        )
        r = subprocess.run(
            [sys.executable, "-c", driver], capture_output=True, text=True
        )
        self.assertEqual(r.returncode, 2)
        self.assertIn("'app'", r.stderr)
        self.assertNotIn("Traceback", r.stderr)


def fake_run(result):
    return patch.object(__import__("app_setup").subprocess, "run", return_value=result)


class BwsReaderTests(unittest.TestCase):
    def setUp(self):
        import app_setup

        self.app_setup = app_setup

    def test_bws_list_error_object_is_named_shape_error(self):
        with fake_run(
            SimpleNamespace(returncode=0, stdout='{"error": "x"}', stderr="")
        ):
            with self.assertRaises(RuntimeError) as cm:
                self.app_setup._bws_list()
        self.assertIn("unexpected shape", str(cm.exception))

    def test_bws_list_non_object_items_are_named_shape_error(self):
        with fake_run(SimpleNamespace(returncode=0, stdout="[1, 2]", stderr="")):
            with self.assertRaises(RuntimeError) as cm:
                self.app_setup._bws_list()
        self.assertIn("unexpected shape", str(cm.exception))

    def test_bws_list_failure_reports_bws_stderr(self):
        with fake_run(SimpleNamespace(returncode=1, stdout="", stderr="token expired")):
            with self.assertRaises(RuntimeError) as cm:
                self.app_setup._bws_list()
        self.assertIn("token expired", str(cm.exception))

    def test_project_id_failure_reports_bws_not_no_projects(self):
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("GHB_BWS_PROJECT_ID", None)
            with fake_run(
                SimpleNamespace(returncode=1, stdout="", stderr="bws exploded")
            ):
                with self.assertRaises(RuntimeError) as cm:
                    self.app_setup._project_id()
        self.assertIn("bws exploded", str(cm.exception))
        self.assertNotIn("no Bitwarden projects", str(cm.exception))

    def test_project_id_empty_list_keeps_no_projects_message(self):
        with patch.dict(os.environ, {}, clear=False):
            os.environ.pop("GHB_BWS_PROJECT_ID", None)
            with fake_run(SimpleNamespace(returncode=0, stdout="[]", stderr="")):
                with self.assertRaises(RuntimeError) as cm:
                    self.app_setup._project_id()
        self.assertIn("no Bitwarden projects", str(cm.exception))

    def test_bws_get_failure_is_runtime_not_json_error(self):
        seq = [
            SimpleNamespace(
                returncode=0, stdout='[{"id": "1", "key": "k"}]', stderr=""
            ),
            SimpleNamespace(returncode=1, stdout="", stderr="get failed hard"),
        ]
        with patch.object(self.app_setup.subprocess, "run", side_effect=seq):
            with self.assertRaises(RuntimeError) as cm:
                self.app_setup.bws_get("k")
        self.assertIn("get failed hard", str(cm.exception))


class ConversionTests(unittest.TestCase):
    def setUp(self):
        import app_setup

        self.app_setup = app_setup

    def run_convert(self, conv):
        stored = []
        payload = json.dumps(conv).encode()

        class Resp:
            def __enter__(self):
                return SimpleNamespace(read=lambda: payload)

            def __exit__(self, *a):
                return False

        import urllib.request

        with (
            patch.object(urllib.request, "urlopen", return_value=Resp()),
            patch.object(
                self.app_setup,
                "bws_set",
                side_effect=lambda n, v: stored.append((n, v)),
            ),
            contextlib.redirect_stdout(io.StringIO()),
        ):
            self.app_setup.cmd_convert("CODE")
        return stored

    def test_partial_conversion_stores_nothing(self):
        conv = {"id": 1, "client_id": "c", "client_secret": "s", "slug": "app"}
        with self.assertRaises(RuntimeError) as cm:
            self.run_convert(conv)
        self.assertIn("pem", str(cm.exception))

    def test_full_conversion_stores_all_four_in_order(self):
        conv = {
            "id": 42,
            "client_id": "cid",
            "client_secret": "sec",
            "pem": "PEM",
            "slug": "my-app",
        }
        stored = self.run_convert(conv)
        self.assertEqual([v for _, v in stored], ["42", "cid", "sec", "PEM"])

    def test_installations_shape_validated_before_indexing(self):
        bad = [{"id": 1, "account": "not-an-object"}]
        with self.assertRaises(RuntimeError):
            self.app_setup.validate_installations(bad)
        good = [{"id": 1, "account": {"login": "octo"}}]
        self.assertEqual(self.app_setup.validate_installations(good), good)


if __name__ == "__main__":
    unittest.main()
