#!/usr/bin/env python3
"""Synthetic go test -json regressions for gotest-report.py."""

import json
import pathlib
import subprocess
import tempfile
import unittest


REPORTER = pathlib.Path(__file__).with_name("gotest-report.py")
STORED = json.dumps(
    {
        "timestamp": "2026-01-01T00:00:00Z",
        "run": "previous",
        "passed_count": 10,
        "failed_count": 0,
        "skipped_count": 2,
        "skipped": ["widgets.TestA", "widgets.TestB"],
    }
)


class GoTestReportTests(unittest.TestCase):
    def run_report(self, events, stored=STORED):
        with tempfile.TemporaryDirectory() as directory:
            root = pathlib.Path(directory)
            event_file = root / "events.json"
            summary = root / "summary.md"
            record = root / "record.json"
            event_file.write_text("".join(json.dumps(event) + "\n" for event in events))
            result = subprocess.run(
                [
                    "python3",
                    str(REPORTER),
                    str(event_file),
                    stored,
                    "run/1",
                    "2026-11-02T00:00:00Z",
                    str(summary),
                    str(record),
                ],
                capture_output=True,
                text=True,
                check=False,
            )
            return (
                result,
                summary.read_text(),
                json.loads(record.read_text()) if record.exists() else None,
            )

    def test_build_failure_without_test_is_reported_and_does_not_lower_record(self):
        result, summary, record = self.run_report(
            [
                {
                    "Action": "output",
                    "Package": "example.test/mcpx/widgets",
                    "Output": "undefined: missing\n",
                },
                {"Action": "fail", "Package": "example.test/mcpx/widgets"},
            ]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("example.test/mcpx/widgets", result.stdout)
        self.assertIn("example.test/mcpx/widgets", summary)
        self.assertIsNone(record)

    def test_testmain_failure_without_test_is_reported_and_does_not_lower_record(self):
        result, summary, record = self.run_report(
            [
                {
                    "Action": "output",
                    "Package": "example.test/mcpx/daemon",
                    "Output": "TestMain: startup failed\n",
                },
                {"Action": "fail", "Package": "example.test/mcpx/daemon"},
            ]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("example.test/mcpx/daemon", result.stdout)
        self.assertIn("Failed packages", summary)
        self.assertIsNone(record)

    def test_test_failure_prevents_record_even_when_skip_count_falls(self):
        result, _, record = self.run_report(
            [
                {
                    "Action": "fail",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestBroken",
                },
                {
                    "Action": "pass",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestGood",
                },
            ]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIsNone(record)

    def test_test_failure_plus_package_event_counts_once(self):
        result, summary, record = self.run_report(
            [
                {
                    "Action": "fail",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestBroken",
                },
                {"Action": "fail", "Package": "example.test/mcpx/widgets"},
                {
                    "Action": "pass",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestGood",
                },
            ]
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIn("1 failed", result.stdout)
        self.assertNotIn("Failed packages", summary)
        self.assertIsNone(record)

    def test_clean_pass_stream_can_write_lower_record(self):
        result, _, record = self.run_report(
            [
                {
                    "Action": "pass",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestGood",
                },
            ]
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(record["skipped_count"], 0)

    def test_skip_increase_fails_without_record(self):
        result, _, record = self.run_report(
            [
                {
                    "Action": "skip",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestNew",
                },
                {
                    "Action": "output",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestNew",
                    "Output": "    skip reason\n",
                },
            ],
            stored=json.dumps({"skipped_count": 0, "skipped": [], "run": "previous"}),
        )
        self.assertNotEqual(result.returncode, 0)
        self.assertIsNone(record)

    def test_quarantine_skip_remains_exempt_from_skip_record(self):
        result, _, record = self.run_report(
            [
                {
                    "Action": "skip",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestQuarantined",
                },
                {
                    "Action": "output",
                    "Package": "example.test/mcpx/widgets",
                    "Test": "TestQuarantined",
                    "Output": "quarantined until 2027-01-01 (https://github.com/example/1)\n",
                },
            ],
            stored="",
        )
        self.assertEqual(result.returncode, 0)
        self.assertEqual(record["skipped_count"], 0)


if __name__ == "__main__":
    unittest.main()
