#!/usr/bin/env python3
"""Tests for review_loop_checks.py — design #2025 §7.3 checks (#2312).

Run:  python3 -m unittest pm-stack/fleet-monitor/test_review_loop_checks.py

Covers the #2312 acceptance shape:
- a planted divergence (label pass != payload review.pass) is
  detected, and the reported repair is derived from the payload
  (the authoritative record);
- a verdict with no live packet is a protocol-gap finding;
- a review/* label surviving closure is a finding;
- stale passes (unclaimed past claim latency; claimed unverdicted
  past the effort deadline) are flagged for the tracker;
- a fully consistent loop state produces NO findings (the checks
  appear in the monitor's run summary only when they fire);
- no check mutates its input state (report-only at Stage B).
"""

import copy
import sys
import unittest
from datetime import datetime, timezone
from pathlib import Path

sys.path.insert(0, str(Path(__file__).parent))
import review_loop_checks as checks

NOW = datetime(2026, 11, 16, 12, 0, 0, tzinfo=timezone.utc)


def artifact(number, labels=(), verdicts=(), state="open", kind="pr"):
    return {
        "repo": "developing-today/code",
        "number": number,
        "kind": kind,
        "state": state,
        "labels": list(labels),
        "verdicts": list(verdicts),
    }


def packet(
    task_id,
    issue,
    review_pass,
    status="queued",
    created="2026-11-16T11:30:00Z",
    claimed=None,
    effort=None,
):
    return {
        "task_id": task_id,
        "status": status,
        "issue": issue,
        "review_pass": review_pass,
        "review_pass_cap": 2,
        "effort": effort,
        "created_at": created,
        "claimed_at": claimed,
    }


class VerdictWithoutPacketTests(unittest.TestCase):
    def test_verdict_with_no_packet_is_a_gap(self):
        state = {
            "artifacts": [
                artifact(
                    101, verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-16T10:00:00Z"}]
                )
            ],
            "packets": [],
        }
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual([f["check"] for f in findings], ["verdict_without_packet"])
        self.assertEqual(findings[0]["verdict_passes_without_packet"], [1])

    def test_verdict_with_live_packet_is_clean(self):
        state = {
            "artifacts": [
                artifact(
                    101,
                    labels=["review/second-pass"],
                    verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-16T10:00:00Z"}],
                )
            ],
            "packets": [packet(9001, 101, 1)],
        }
        self.assertEqual(checks.run_checks(state, now=NOW), [])

    def test_label_without_any_packet_is_a_gap(self):
        state = {
            "artifacts": [artifact(102, labels=["review/second-pass"])],
            "packets": [],
        }
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual([f["check"] for f in findings], ["verdict_without_packet"])
        self.assertTrue(findings[0]["has_second_pass_label"])


class CountDivergenceTests(unittest.TestCase):
    def test_planted_divergence_detected_and_repaired_from_payload(self):
        # The #2312 acceptance case: label says pass-2, the packet
        # payload (authoritative) says pass 1, the verdict footer
        # agrees with the payload. The repair must set the label
        # from the payload.
        state = {
            "artifacts": [
                artifact(
                    103,
                    labels=["review/pass-2"],
                    verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-16T10:00:00Z"}],
                )
            ],
            "packets": [packet(9002, 103, 1)],
        }
        findings = checks.run_checks(state, now=NOW)
        divergence = [f for f in findings if f["check"] == "count_divergence"]
        self.assertEqual(len(divergence), 1)
        self.assertEqual(
            divergence[0]["values"], {"label": 2, "payload": 1, "verdict_footer": 1}
        )
        self.assertEqual(divergence[0]["authoritative"], 1)
        self.assertEqual(divergence[0]["repair"]["add"], "review/pass-1")
        self.assertEqual(divergence[0]["repair"]["remove"], ["review/pass-2"])

    def test_agreeing_records_are_clean(self):
        state = {
            "artifacts": [
                artifact(
                    104,
                    labels=["review/pass-1"],
                    verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-16T10:00:00Z"}],
                )
            ],
            "packets": [packet(9003, 104, 1)],
        }
        self.assertEqual(checks.run_checks(state, now=NOW), [])

    def test_review_label_surviving_closure_is_a_finding(self):
        state = {
            "artifacts": [artifact(105, labels=["review/pass-1"], state="closed")],
            "packets": [],
        }
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual(
            [f["check"] for f in findings], ["review_label_survives_closure"]
        )
        self.assertEqual(findings[0]["labels"], ["review/pass-1"])


class StalePassTests(unittest.TestCase):
    def test_unclaimed_past_claim_latency_is_stale(self):
        state = {
            "artifacts": [artifact(106)],
            "packets": [packet(9004, 106, 1, created="2026-11-16T08:00:00Z")],
        }
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual([f["check"] for f in findings], ["stale_pass"])
        self.assertIn("unclaimed", findings[0]["detail"])

    def test_freshly_commissioned_pass_is_not_stale(self):
        state = {
            "artifacts": [artifact(107)],
            "packets": [packet(9005, 107, 1, created="2026-11-16T11:45:00Z")],
        }
        self.assertEqual(checks.run_checks(state, now=NOW), [])

    def test_claimed_unverdicted_past_effort_deadline_is_stale(self):
        state = {
            "artifacts": [artifact(108)],
            "packets": [
                packet(
                    9006,
                    108,
                    1,
                    status="claimed",
                    created="2026-11-15T08:00:00Z",
                    claimed="2026-11-15T09:00:00Z",
                    effort="s",
                )
            ],
        }
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual([f["check"] for f in findings], ["stale_pass"])
        self.assertIn("unverdicted", findings[0]["detail"])

    def test_claimed_and_verdicted_is_not_stale(self):
        state = {
            "artifacts": [
                artifact(
                    109, verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-15T10:00:00Z"}]
                )
            ],
            "packets": [
                packet(
                    9007,
                    109,
                    1,
                    status="claimed",
                    created="2026-11-15T08:00:00Z",
                    claimed="2026-11-15T09:00:00Z",
                    effort="s",
                )
            ],
        }
        self.assertEqual(checks.run_checks(state, now=NOW), [])


class ReportOnlyTests(unittest.TestCase):
    def test_checks_do_not_mutate_state(self):
        state = {
            "artifacts": [
                artifact(
                    110,
                    labels=["review/pass-2"],
                    verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-16T10:00:00Z"}],
                )
            ],
            "packets": [packet(9008, 110, 1, created="2026-11-16T08:00:00Z")],
        }
        snapshot = copy.deepcopy(state)
        checks.run_checks(state, now=NOW)
        # run_checks joins verdicts onto copies: the caller's state,
        # packets included, is byte-for-byte untouched.
        self.assertEqual(state, snapshot)


class ShepherdRegressionTests(unittest.TestCase):
    """Regressions from the PR #10676 shepherd pass (CodeRabbit majors)."""

    def test_preset_verdicted_flag_is_recomputed_not_trusted(self):
        # A stale preset `verdicted: True` in the input must not
        # suppress a stale finding for an unverdicted claimed packet.
        stale_packet = packet(
            9010,
            111,
            1,
            status="claimed",
            created="2026-11-15T08:00:00Z",
            claimed="2026-11-15T09:00:00Z",
            effort="s",
        )
        stale_packet["verdicted"] = True
        state = {"artifacts": [artifact(111)], "packets": [stale_packet]}
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual([f["check"] for f in findings], ["stale_pass"])

    def test_earlier_pass_verdict_does_not_cover_later_pass_packet(self):
        # A pass-1 verdict must not mark a claimed pass-2 packet
        # verdicted; a pass-2 verdict must.
        base_packet = packet(
            9011,
            112,
            2,
            status="claimed",
            created="2026-11-15T08:00:00Z",
            claimed="2026-11-15T09:00:00Z",
            effort="s",
        )
        state = {
            "artifacts": [
                artifact(
                    112, verdicts=[{"pass": 1, "cap": 2, "at": "2026-11-15T10:00:00Z"}]
                )
            ],
            "packets": [dict(base_packet)],
        }
        # (The pass-1 footer also fires verdict_without_packet and
        # count_divergence against the pass-2 packet — correctly;
        # what matters here is that it does NOT suppress staleness.)
        self.assertIn(
            "stale_pass", [f["check"] for f in checks.run_checks(state, now=NOW)]
        )
        state["artifacts"] = [
            artifact(
                112, verdicts=[{"pass": 2, "cap": 2, "at": "2026-11-15T10:00:00Z"}]
            )
        ]
        self.assertEqual(checks.run_checks(state, now=NOW), [])

    def test_naive_timestamps_do_not_crash_stale_check(self):
        state = {
            "artifacts": [artifact(113)],
            "packets": [packet(9012, 113, 1, created="2026-11-16T08:00:00")],
        }
        findings = checks.run_checks(state, now=NOW)
        self.assertEqual([f["check"] for f in findings], ["stale_pass"])

    def test_live_collection_failure_is_an_error_record_not_a_crash(self):
        import io
        import subprocess as sp
        from contextlib import redirect_stdout
        from unittest import mock

        with mock.patch.object(
            checks,
            "collect_live_state",
            side_effect=sp.CalledProcessError(1, ["gh"]),
        ):
            out = io.StringIO()
            with redirect_stdout(out):
                rc = checks.main(["--repo", "developing-today/code"])
        self.assertEqual(rc, 0)
        record = __import__("json").loads(out.getvalue().strip())
        self.assertEqual(record["check"], "collection_error")


class VerdictFooterParsingTests(unittest.TestCase):
    def test_footer_parsed(self):
        body = (
            "Some review text.\n"
            "<!-- agent-note:v1 agent=pr-diff-review state=verdict "
            "verdict=second-pass pass=1 cap=2 at=2026-11-16T10:00:00Z -->"
        )
        verdicts = checks.parse_verdict_footers([body])
        self.assertEqual(
            verdicts, [{"pass": 1, "cap": 2, "at": "2026-11-16T10:00:00Z"}]
        )

    def test_non_verdict_notes_ignored(self):
        body = "<!-- agent-note:v1 agent=repo-worker-code state=done task=1 -->"
        self.assertEqual(checks.parse_verdict_footers([body]), [])


if __name__ == "__main__":
    unittest.main()
