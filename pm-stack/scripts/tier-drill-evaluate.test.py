#!/usr/bin/env python3
"""Oracle: synthetic probe records, never a live outage or real drill evidence.

The evaluator must accept only a complete evidence package — same explicit
surface set in before/during/after, every record labelled provesDrill false,
and the during record inside an honestly ordered outage window — and reject
missing coverage, tampered completion claims and absent outage/restoration
proof with named gaps.
"""

import json
import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

SCRIPT = Path(__file__).with_name("tier-drill-evaluate.py")

BASE = "https://probe.invalid"
URLS = [BASE + "/console/health", BASE + "/board"]

BEFORE = {
    "schemaVersion": 1,
    "kind": "tier-probe-observations",
    "provesDrill": False,
    "startedAt": "2026-11-27T12:00:00Z",
    "finishedAt": "2026-11-27T12:00:05Z",
    "baseUrl": BASE,
    "observations": [
        {
            "url": URLS[0],
            "startedAt": "2026-11-27T12:00:00Z",
            "finishedAt": "2026-11-27T12:00:01Z",
            "status": 200,
            "httpSuccess": True,
            "error": None,
        },
        {
            "url": URLS[1],
            "startedAt": "2026-11-27T12:00:01Z",
            "finishedAt": "2026-11-27T12:00:02Z",
            "status": 200,
            "httpSuccess": True,
            "error": None,
        },
    ],
}

DURING = {
    "schemaVersion": 1,
    "kind": "tier-probe-observations",
    "provesDrill": False,
    "startedAt": "2026-11-27T12:10:00Z",
    "finishedAt": "2026-11-27T12:10:08Z",
    "baseUrl": BASE,
    "observations": [
        {
            "url": URLS[0],
            "startedAt": "2026-11-27T12:10:00Z",
            "finishedAt": "2026-11-27T12:10:01Z",
            "status": 503,
            "httpSuccess": False,
            "error": None,
        },
        {
            "url": URLS[1],
            "startedAt": "2026-11-27T12:10:01Z",
            "finishedAt": "2026-11-27T12:10:02Z",
            "status": 503,
            "httpSuccess": False,
            "error": None,
        },
    ],
}

AFTER = {
    "schemaVersion": 1,
    "kind": "tier-probe-observations",
    "provesDrill": False,
    "startedAt": "2026-11-27T12:40:00Z",
    "finishedAt": "2026-11-27T12:40:05Z",
    "baseUrl": BASE,
    "observations": [
        {
            "url": URLS[0],
            "startedAt": "2026-11-27T12:40:00Z",
            "finishedAt": "2026-11-27T12:40:01Z",
            "status": 200,
            "httpSuccess": True,
            "error": None,
        },
        {
            "url": URLS[1],
            "startedAt": "2026-11-27T12:40:01Z",
            "finishedAt": "2026-11-27T12:40:02Z",
            "status": 200,
            "httpSuccess": True,
            "error": None,
        },
    ],
}

WINDOW = {
    "--outage-start": "2026-11-27T12:05:00Z",
    "--outage-end": "2026-11-27T12:35:00Z",
    "--restored-at": "2026-11-27T12:36:00Z",
    "--seat": "off-laptop-seat",
    "--command": "tier-drill-probe.py --base-url ORIGIN --path /console/health --path /board",
}


class EvaluateTests(unittest.TestCase):
    def run_evaluate(self, *args):
        return subprocess.run(
            [sys.executable, str(SCRIPT), *args], capture_output=True, text=True
        )

    def write(self, directory, name, record):
        path = Path(directory) / name
        path.write_text(json.dumps(record), encoding="utf-8")
        return str(path)

    def package(self, directory, before=None, during=None, after=None, **window):
        args = [
            "--before",
            self.write(directory, "before.json", before if before is not None else BEFORE),
            "--during",
            self.write(directory, "during.json", during if during is not None else DURING),
            "--after",
            self.write(directory, "after.json", after if after is not None else AFTER),
        ]
        merged = dict(WINDOW)
        merged.update(window)
        for key, value in merged.items():
            args.extend([key, value])
        return args

    def test_complete_package_verdict_complete(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_evaluate(*self.package(directory))
        self.assertEqual(result.returncode, 0, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual(verdict["kind"], "tier-drill-evaluation")
        self.assertEqual(verdict["verdict"], "complete")
        self.assertEqual(verdict["gaps"], [])

    def test_missing_surface_in_during_is_rejected(self):
        partial = dict(DURING)
        partial["observations"] = DURING["observations"][:1]
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_evaluate(*self.package(directory, during=partial))
        self.assertEqual(result.returncode, 1, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual(verdict["verdict"], "incomplete")
        self.assertTrue(
            any(URLS[1] in gap for gap in verdict["gaps"]),
            verdict["gaps"],
        )

    def test_tampered_completion_claim_is_rejected(self):
        tampered = dict(AFTER)
        tampered["provesDrill"] = True
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_evaluate(*self.package(directory, after=tampered))
        self.assertEqual(result.returncode, 1, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual(verdict["verdict"], "incomplete")
        self.assertTrue(
            any("provesDrill" in gap for gap in verdict["gaps"]),
            verdict["gaps"],
        )

    def test_during_outside_outage_window_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_evaluate(
                *self.package(directory, **{"--outage-start": "2026-11-27T12:20:00Z"})
            )
        self.assertEqual(result.returncode, 1, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual(verdict["verdict"], "incomplete")
        self.assertTrue(verdict["gaps"], verdict)

    def test_after_before_restoration_is_rejected(self):
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_evaluate(
                *self.package(directory, **{"--restored-at": "2026-11-27T12:41:00Z"})
            )
        self.assertEqual(result.returncode, 1, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual(verdict["verdict"], "incomplete")
        self.assertTrue(verdict["gaps"], verdict)

    def test_wrong_record_kind_is_rejected(self):
        foreign = {"kind": "something-else", "observations": []}
        with tempfile.TemporaryDirectory() as directory:
            result = self.run_evaluate(*self.package(directory, during=foreign))
        self.assertEqual(result.returncode, 1, result.stderr)
        verdict = json.loads(result.stdout)
        self.assertEqual(verdict["verdict"], "incomplete")
        self.assertTrue(
            any("during" in gap for gap in verdict["gaps"]), verdict["gaps"]
        )

    def test_strict_arguments_before_file_reads(self):
        cases = [
            ["--typo"],
            ["extra"],
            ["--seat", ""],
            ["--command", ""],
            ["--outage-start", "not-a-time"],
            ["--outage-end", "2026-11-27T12:00:00Z"],
        ]
        with tempfile.TemporaryDirectory() as directory:
            base = self.package(directory)
            for extra in cases:
                with self.subTest(args=extra):
                    result = self.run_evaluate(*base, *extra)
                    self.assertEqual(result.returncode, 2, result.stderr)
                    self.assertEqual(result.stdout, "")

    def test_unreadable_evidence_is_usage_error(self):
        with tempfile.TemporaryDirectory() as directory:
            missing = str(Path(directory) / "absent.json")
            result = self.run_evaluate(
                "--before",
                missing,
                "--during",
                missing,
                "--after",
                missing,
                *[item for pair in WINDOW.items() for item in pair],
            )
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(result.stdout, "")
            corrupt = Path(directory) / "corrupt.json"
            corrupt.write_text("{not json", encoding="utf-8")
            result = self.run_evaluate(
                "--before",
                str(corrupt),
                "--during",
                str(corrupt),
                "--after",
                str(corrupt),
                *[item for pair in WINDOW.items() for item in pair],
            )
            self.assertEqual(result.returncode, 2, result.stderr)
            self.assertEqual(result.stdout, "")


if __name__ == "__main__":
    unittest.main()
