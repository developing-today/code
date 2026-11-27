#!/usr/bin/env python3
"""Tests for scripts/fleet/drift-census-report.py (#14134 U1 slice 2).

Oracle: `scripts/drift-census.py` `summary()` — silent means converged or
pending-only and prints nothing; held/drifted rows produce the delivered
report. Run:
  python3 -I -m unittest discover -s scripts/fleet -p 'test_drift_census_report.py' -v
"""

import contextlib
import importlib.util
import io
import json
import pathlib
import re
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
_spec = importlib.util.spec_from_file_location(
    "drift_census_report", HERE / "drift-census-report.py"
)
report_mod = importlib.util.module_from_spec(_spec)
_spec.loader.exec_module(report_mod)


def converged_report() -> dict:
    return {
        "total": 2,
        "counts": {"converged": 2, "pending": 0, "held": 0, "drifted": 0},
        "silent": True,
        "needs_you": [],
        "rows": [
            {
                "plane": "sandbox",
                "service": "fleet",
                "pinned": "1.2.3",
                "running": "1.2.3",
                "lag_s": 0,
                "state": "converged",
                "overrides": [],
                "held": [],
                "reason": "",
            },
            {
                "plane": "cloudflare",
                "service": "w",
                "pinned": "aaa111",
                "running": "aaa111",
                "lag_s": 0,
                "state": "converged",
                "overrides": [],
                "held": [],
                "reason": "",
            },
        ],
    }


def attention_report() -> dict:
    return {
        "total": 3,
        "counts": {"converged": 1, "pending": 1, "held": 1, "drifted": 1},
        "silent": False,
        "needs_you": ["restic", "pm-aggregator"],
        "rows": [
            {
                "plane": "sandbox",
                "service": "fleet",
                "pinned": "1.2.3",
                "running": "1.2.3",
                "lag_s": 0,
                "state": "converged",
                "overrides": [],
                "held": [],
                "reason": "",
            },
            {
                "plane": "sandbox",
                "service": "restic",
                "pinned": "0.20.0",
                "running": "0.19.0",
                "lag_s": None,
                "state": "held",
                "overrides": ["pin until fix"],
                "held": ["update 42"],
                "reason": "",
            },
            {
                "plane": "cloudflare",
                "service": "pm-aggregator",
                "pinned": "aaa111",
                "running": "old111",
                "lag_s": 90000,
                "state": "drifted",
                "overrides": [],
                "held": [],
                "reason": "running is beyond the drift window",
            },
        ],
    }


def run_main(args: list[str], stdin: str = "") -> tuple[int, str]:
    out = io.StringIO()
    with contextlib.redirect_stdout(out):
        code = report_mod.main(args)
    return code, out.getvalue()


class FormattingTests(unittest.TestCase):
    def test_held_and_drifted_rows_are_named_with_pins_and_lag(self):
        body = report_mod.render_markdown(attention_report(), None)
        self.assertIn("2 of 3 rows need you", body)
        self.assertIn("(held=1, drifted=1)", body)
        self.assertIn("`sandbox/restic`: held", body)
        self.assertIn("pinned=0.20.0 running=0.19.0", body)
        self.assertIn("`cloudflare/pm-aggregator`: drifted", body)
        self.assertIn("lag=90000s", body)
        self.assertIn("beyond the drift window", body)

    def test_run_url_is_footed_when_given(self):
        body = report_mod.render_markdown(
            attention_report(), "https://example.test/run/1"
        )
        self.assertIn("Census run: https://example.test/run/1", body)

    def test_full_counts_line_covers_quiet_rows(self):
        body = report_mod.render_markdown(attention_report(), None)
        self.assertIn("converged=1, pending=1, held=1, drifted=1", body)


class SilenceTests(unittest.TestCase):
    def test_fully_converged_run_renders_nothing(self):
        self.assertEqual(report_mod.render_markdown(converged_report(), None), "")

    def test_pending_only_run_stays_silent(self):
        pending = converged_report()
        pending["rows"][0] = dict(
            pending["rows"][0],
            state="pending",
            lag_s=60,
            reason="behind, inside window",
        )
        pending["counts"] = {"converged": 1, "pending": 1, "held": 0, "drifted": 0}
        pending["silent"] = True
        self.assertEqual(report_mod.render_markdown(pending, None), "")

    def test_cli_prints_nothing_on_silent_report(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(converged_report(), f)
            path = f.name
        code, out = run_main(["--report", path])
        self.assertEqual(code, 0)
        self.assertEqual(out, "")


class DeliveryTests(unittest.TestCase):
    def test_cli_renders_attention_report_to_stdout(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            json.dump(attention_report(), f)
            path = f.name
        code, out = run_main(
            ["--report", path, "--run-url", "https://example.test/run/1"]
        )
        self.assertEqual(code, 0)
        self.assertIn("rows need you", out)
        self.assertIn("https://example.test/run/1", out)

    def test_malformed_report_is_a_report_error_not_a_post(self):
        with tempfile.NamedTemporaryFile("w", suffix=".json", delete=False) as f:
            f.write('{"total": 1}')
            path = f.name
        err = io.StringIO()
        with contextlib.redirect_stderr(err):
            code = report_mod.main(["--report", path])
        self.assertEqual(code, 1)
        self.assertIn("missing", err.getvalue())


class ReadOnlyTests(unittest.TestCase):
    """The renderer formats text only: no writes, no subprocess, no network."""

    FORBIDDEN = (
        r"\bwrite_text\b",
        r"\bwrite_bytes\b",
        r"open\([^)]*['\"][wa]",
        r"\bsubprocess\b",
        r"\bos\.system\b",
        r"shell\s*=\s*True",
        r"\burllib\b",
        r"\brequests\b",
        r"\bgh\b.*issue",
    )

    def test_source_has_no_mutating_call(self):
        source = (HERE / "drift-census-report.py").read_text()
        for pattern in self.FORBIDDEN:
            self.assertIsNone(re.search(pattern, source), pattern)


class ArgumentContractTests(unittest.TestCase):
    def test_abbreviated_options_are_rejected(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                report_mod.main(["--rep", "x.json"])
        self.assertEqual(raised.exception.code, 2)

    def test_missing_report_is_usage(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                report_mod.main([])
        self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
