#!/usr/bin/env python3
"""Tests for drift-census.py (#14134 slice U1).

Oracle: design of record
doc/2026-11-24T06-25-30Z_design_auto-update-system/ section 4 — the state
vocabulary (converged / pending / held / drifted), lag reporting, silence
when converged, and the read-only rule. Run:
  python3 -m unittest scripts.test_drift_census
(or execute this file directly).
"""

import importlib.util
import pathlib
import re
import unittest
from datetime import datetime, timedelta, timezone

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location("drift_census", HERE / "drift-census.py")
census = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(census)

NOW = datetime(2026, 11, 24, 12, 0, tzinfo=timezone.utc)
WINDOW = 24 * 60 * 60


def cf_fixture(workers, main_rev="aaa111"):
    return {"cloudflare": {"main_rev": main_rev, "workers": workers}}


class ClassifyTests(unittest.TestCase):
    def test_equal_pin_and_running_is_converged(self):
        row = census.classify(pinned="a1", running="a1", lag_s=0, window_s=WINDOW,
                              overrides=[], held=[])
        self.assertEqual(row.state, "converged")

    def test_behind_inside_window_is_pending(self):
        row = census.classify(pinned="a2", running="a1", lag_s=60, window_s=WINDOW,
                              overrides=[], held=[])
        self.assertEqual(row.state, "pending")

    def test_behind_beyond_window_is_drifted_with_reason(self):
        row = census.classify(pinned="a2", running="a1", lag_s=WINDOW + 1,
                              window_s=WINDOW, overrides=[], held=[])
        self.assertEqual(row.state, "drifted")
        self.assertIn("beyond", row.reason)

    def test_override_or_held_update_is_held_not_drift(self):
        row = census.classify(pinned="a2", running="a1", lag_s=WINDOW * 9,
                              window_s=WINDOW, overrides=["pin until fix"], held=[])
        self.assertEqual(row.state, "held")
        row = census.classify(pinned="a2", running="a1", lag_s=None, window_s=WINDOW,
                              overrides=[], held=["update 42"])
        self.assertEqual(row.state, "held")

    def test_unreadable_running_is_never_assumed_converged(self):
        row = census.classify(pinned="a2", running="unreadable", lag_s=None,
                              window_s=WINDOW, overrides=[], held=[])
        self.assertEqual(row.state, "drifted")
        self.assertIn("unreadable", row.reason)


class SummaryTests(unittest.TestCase):
    def test_fully_converged_run_is_silent(self):
        rows = [census.evaluate("cloudflare", "w1", "a1", "a1", 0, WINDOW, [], [])]
        report = census.summary(rows)
        self.assertTrue(report["silent"])
        self.assertEqual(report["needs_you"], [])

    def test_held_or_drifted_row_produces_a_needs_you_entry(self):
        rows = [
            census.evaluate("cloudflare", "w1", "a1", "a1", 0, WINDOW, [], []),
            census.evaluate("sandbox", "restic", "0.20.0", "0.19.0", None, WINDOW, [], []),
        ]
        report = census.summary(rows)
        self.assertFalse(report["silent"])
        self.assertEqual(report["needs_you"], ["restic"])


class BuildTests(unittest.TestCase):
    def test_cloudflare_drift_is_reported_then_returns_to_converged(self):
        root = HERE.parent
        behind = cf_fixture({"pm-aggregator": "old111"})
        report = census.build(["cloudflare"], root, behind, NOW, WINDOW)
        row = report["rows"][0]
        self.assertEqual(row["state"], "drifted", "pin not resolvable in this checkout: lag unknown")
        report = census.build(["cloudflare"], root, cf_fixture({"pm-aggregator": "aaa111"}),
                              NOW, WINDOW)
        self.assertEqual(report["rows"][0]["state"], "converged")
        self.assertTrue(report["silent"])

    def test_no_cloudflare_snapshot_reports_no_rows(self):
        report = census.build(["cloudflare"], HERE.parent, None, NOW, WINDOW)
        self.assertEqual(report["total"], 0)
        self.assertTrue(report["silent"])

    def test_text_render_names_each_row_and_the_verdict(self):
        report = census.build(["cloudflare"], HERE.parent,
                              cf_fixture({"w": "aaa111"}), NOW, WINDOW)
        text = census.render_text(report)
        self.assertIn("cloudflare/w: converged", text)
        self.assertIn("converged — nothing needs you", text)


class ReadOnlyTests(unittest.TestCase):
    """§4: the census never fixes. The module has no write path."""

    FORBIDDEN = (
        r"\bwrite_text\b", r"\bwrite_bytes\b", r"open\([^)]*['\"][wa]",
        r"\bos\.system\b", r"shell\s*=\s*True", r"\bunlink\b", r"\brename\(",
        r"\bshutil\.rmtree\b", r"systemctl\b(?!.*\bshow\b)",
        r"\bnixos-rebuild\b", r"\bwrangler\s+(deploy|publish)\b",
        r"\bgit\s+(push|reset|checkout|commit|fetch)\b",
    )

    def test_source_has_no_mutating_call(self):
        source = (HERE / "drift-census.py").read_text()
        for pattern in self.FORBIDDEN:
            self.assertIsNone(re.search(pattern, source), pattern)

    def test_subprocess_is_only_used_through_the_read_helper(self):
        source = (HERE / "drift-census.py").read_text()
        self.assertEqual(source.count("subprocess.run("), 1)


if __name__ == "__main__":
    unittest.main()
