#!/usr/bin/env python3
"""Tests for openai-cred-watchdog.py (issue #5250).

The main OpenAI account was renamed `openai` -> `openai.main` in
PR #5085, but the watchdog kept checking the retired `openai`
provider: it permanently reported `openai: no credential rows found`
and never applied the banned chatgpt-token-sharing check to the
renamed account. These tests drive the REAL watchdog module against
fixture credential stores covering the renamed-provider case:

  - a healthy store keyed by `openai.main` passes;
  - a banned token-sharing row on `openai.main` is flagged (even when
    inactive — the invariant is that no such row may exist);
  - a store carrying only the retired `openai` identity does NOT
    pass: the watchdog reports `openai.main` (and `openai.alternate`)
    as missing instead of being satisfied by the retired rows.

Run: python3 openai-cred-watchdog.test.py
"""

import importlib.util
import json
import os
import sqlite3
import sys
import tempfile
import unittest

SELF_DIR = os.path.dirname(os.path.abspath(__file__))
SPEC = importlib.util.spec_from_file_location(
    "openai_cred_watchdog", os.path.join(SELF_DIR, "openai-cred-watchdog.py")
)
watchdog = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(watchdog)


def make_db(rows):
    """rows: list of (integration_id, active, methodID). Returns db path."""
    fd, path = tempfile.mkstemp(suffix=".db")
    os.close(fd)
    con = sqlite3.connect(path)
    try:
        con.execute(
            "CREATE TABLE credential ("
            "id INTEGER PRIMARY KEY, integration_id TEXT, label TEXT, "
            "active INTEGER, value TEXT)"
        )
        for i, (provider, active, method) in enumerate(rows, start=1):
            con.execute(
                "INSERT INTO credential VALUES (?, ?, ?, ?, ?)",
                (
                    i,
                    provider,
                    f"{provider}-cred-{i}",
                    1 if active else 0,
                    json.dumps({"methodID": method}),
                ),
            )
        con.commit()
    finally:
        con.close()
    return path


class WatchdogRenameTests(unittest.TestCase):
    def check_rows(self, rows):
        path = make_db(rows)
        try:
            return watchdog.check(path)
        finally:
            os.unlink(path)

    def test_providers_are_the_renamed_identities(self):
        self.assertEqual(watchdog.PROVIDERS, ("_openai.main", "_openai.alt"))

    def test_healthy_renamed_store_passes(self):
        violations = self.check_rows(
            [
                ("_openai.main", True, "chatgpt-browser"),
                ("_openai.alt", True, "chatgpt-headless"),
            ]
        )
        self.assertEqual(violations, [])

    def test_banned_method_on_renamed_main_is_flagged(self):
        violations = self.check_rows(
            [
                ("_openai.main", True, "chatgpt-browser"),
                ("_openai.main", False, "chatgpt-token-sharing"),
                ("_openai.alt", True, "chatgpt-headless"),
            ]
        )
        self.assertTrue(
            any(v.startswith("_openai.main:") and "banned" in v for v in violations),
            f"expected a banned-method violation for _openai.main, got {violations}",
        )

    def test_retired_ids_only_store_does_not_pass(self):
        violations = self.check_rows(
            [
                ("openai", True, "chatgpt-browser"),
                ("openai.alternate", True, "chatgpt-headless"),
            ]
        )
        self.assertIn("_openai.main: no credential rows found", violations)
        self.assertIn("_openai.alt: no credential rows found", violations)


if __name__ == "__main__":
    unittest.main(verbosity=2)
