#!/usr/bin/env python3
"""CEN-06 — experiment report writer tests (#9436).

Oracles are independent of the writer's internals: the required
measurement names come from the CEN-06 slice text, the digest oracle
is hashlib applied to the raw artifact bytes in this file, and the
no-production-benefit oracle is a text property of the rendered
report. The committed CEN-05 artifact is read as data, never imported.

Run:  python3 -m unittest -v test_report
      python3 test_report.py
"""

from __future__ import annotations

import hashlib
import importlib.util
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
spec = importlib.util.spec_from_file_location("cen06_report", HERE / "report.py")
report = importlib.util.module_from_spec(spec)
sys.modules[spec.name] = report
spec.loader.exec_module(report)

REQUIRED = ["run_id", "seed", "sample_count", "oracle",
            "uncertainty", "fixture_provenance", "threshold"]


class TestMissingMeasurements(unittest.TestCase):
    def test_report_marks_missing_measurement_and_does_not_claim_production_benefit(self):
        # An artifact carrying nothing: every required measurement is
        # marked missing by name, and the rendered report still refuses
        # to call a synthetic simulation a production result.
        artifact = {}
        self.assertEqual(report.missing_measurements(artifact), REQUIRED)
        text = report.render_report(artifact, "0" * 64)
        for name in REQUIRED:
            self.assertIn(f"MISSING: {name}", text)
        self.assertIn("not a production result", text)
        self.assertIn("no production benefit is claimed", text)
        self.assertNotIn("production benefit of", text)

    def test_partial_artifact_marks_only_what_is_absent(self):
        artifact = {"inputs": {"seed": 9196, "fixture_sha256": "abc"}}
        missing = report.missing_measurements(artifact)
        self.assertNotIn("seed", missing)
        self.assertNotIn("fixture_provenance", missing)
        self.assertIn("run_id", missing)
        self.assertIn("oracle", missing)


class TestCommittedArtifact(unittest.TestCase):
    def setUp(self):
        self.raw = (HERE / "report.json").read_bytes()
        self.artifact = report.load_artifact(self.raw)

    def test_report_reproduces_input_result_digest(self):
        digest = report.artifact_digest(self.raw)
        self.assertEqual(digest, hashlib.sha256(self.raw).hexdigest())
        text = report.render_report(self.artifact, digest)
        self.assertIn(digest, text)
        self.assertIn("9196", text)  # the recorded seed, verbatim

    def test_committed_artifact_missing_only_run_id(self):
        # The CEN-05 artifact predates the run_id requirement: the
        # writer marks exactly that one measurement missing and
        # invents nothing else.
        self.assertEqual(report.missing_measurements(self.artifact), ["run_id"])

    def test_report_separates_facts_outputs_inference_recommendation(self):
        text = report.render_report(self.artifact, report.artifact_digest(self.raw))
        for header in ("## External source facts", "## Measured model outputs",
                       "## Inference", "## Recommendation"):
            self.assertIn(header, text)

    def test_report_is_deterministic(self):
        digest = report.artifact_digest(self.raw)
        self.assertEqual(report.render_report(self.artifact, digest),
                         report.render_report(self.artifact, digest))


if __name__ == "__main__":
    unittest.main()
