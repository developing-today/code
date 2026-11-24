"""Tests for scripts/change_ledger.py.

Oracle: issue #1339's change-event ledger spec (epic #1334, criterion 3)
— "a config change with no ledger event is a defect" — plus the
operator-script argument contract (#3352): the whole command line is
declared, anything unaccounted for is a usage error (exit 2).

Run: python3 scripts/test_change_ledger.py
"""

import json
import os
import subprocess
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import change_ledger as cl


def write(path, text):
    with open(path, "w", encoding="utf-8") as fh:
        fh.write(text)


def changes_doc(changes, since="2026-10-27", until="2026-11-23"):
    return {
        "config": ".opencode/opencode.jsonc",
        "window": {"since": since, "until": until},
        "changes": changes,
    }


def change(pr=None, sha=None, date="2026-11-20T00:00:00Z", title="config change"):
    entry = {"date": date, "title": title}
    if pr is not None:
        entry["pr"] = pr
    if sha is not None:
        entry["sha"] = sha
    return entry


def event(change, **overrides):
    base = {
        "timestamp": "2026-11-20T01:00:00Z",
        "pool": "workhorse",
        "change": change,
        "rationale": "burn down the refilled window before reset",
        "expected_effect": "workhorse absorbs the next hours of work",
        "window": "7d, reset 2026-11-25T13:00-06:00",
        "before_snapshot": {"workhorse": {"remaining_pct": 62}},
    }
    base.update(overrides)
    return base


class CheckTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.ledger = os.path.join(self.tmp.name, "ledger.jsonl")
        self.changes = os.path.join(self.tmp.name, "changes.json")

    def tearDown(self):
        self.tmp.cleanup()

    def run_check(self):
        return cl.check(self.ledger, self.changes)

    def test_pr_link_match_passes(self):
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(
            self.ledger,
            json.dumps(event("https://github.com/developing-today/code/pull/13838"))
            + "\n",
        )
        report = self.run_check()
        self.assertEqual(report["matched"], 1)
        self.assertEqual(report["unmatched"], [])
        self.assertEqual(report["defects"], [])
        self.assertEqual(report["exit_code"], 0)

    def test_hash_ref_and_pull_path_match(self):
        write(
            self.changes, json.dumps(changes_doc([change(pr=13838), change(pr=13784)]))
        )
        write(
            self.ledger,
            "\n".join(
                [
                    json.dumps(event("PR #13838")),
                    json.dumps(
                        event("https://github.com/developing-today/code/pull/13784")
                    ),
                ]
            )
            + "\n",
        )
        report = self.run_check()
        self.assertEqual(report["matched"], 2)
        self.assertEqual(report["exit_code"], 0)

    def test_unmatched_change_is_a_defect(self):
        write(
            self.changes, json.dumps(changes_doc([change(pr=13838), change(pr=13784)]))
        )
        write(self.ledger, json.dumps(event("PR #13838")) + "\n")
        report = self.run_check()
        self.assertEqual(report["matched"], 1)
        self.assertEqual(len(report["unmatched"]), 1)
        self.assertEqual(report["unmatched"][0]["pr"], 13784)
        self.assertEqual(report["exit_code"], 1)

    def test_duplicate_events_match_once(self):
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(
            self.ledger,
            "\n".join(
                [
                    json.dumps(event("PR #13838")),
                    json.dumps(
                        event(
                            "PR #13838",
                            timestamp="2026-11-21T01:00:00Z",
                            outcome="burn shifted as expected",
                        )
                    ),
                ]
            )
            + "\n",
        )
        report = self.run_check()
        self.assertEqual(report["matched"], 1)
        self.assertEqual(report["exit_code"], 0)

    def test_event_missing_before_snapshot_is_a_defect(self):
        bad = event("PR #13838")
        del bad["before_snapshot"]
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(self.ledger, json.dumps(bad) + "\n")
        report = self.run_check()
        self.assertEqual(report["exit_code"], 1)
        self.assertTrue(
            any(d["kind"] == "missing_before_snapshot" for d in report["defects"])
        )

    def test_event_with_empty_before_snapshot_is_a_defect(self):
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(self.ledger, json.dumps(event("PR #13838", before_snapshot={})) + "\n")
        report = self.run_check()
        self.assertEqual(report["exit_code"], 1)
        self.assertTrue(
            any(d["kind"] == "missing_before_snapshot" for d in report["defects"])
        )

    def test_event_missing_expected_effect_is_a_defect(self):
        bad = event("PR #13838")
        del bad["expected_effect"]
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(self.ledger, json.dumps(bad) + "\n")
        report = self.run_check()
        self.assertEqual(report["exit_code"], 1)
        self.assertTrue(
            any(d["kind"] == "missing_expected_effect" for d in report["defects"])
        )

    def test_malformed_ledger_line_is_a_defect(self):
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(self.ledger, json.dumps(event("PR #13838")) + "\n{not json\n")
        report = self.run_check()
        self.assertEqual(report["exit_code"], 1)
        self.assertTrue(any(d["kind"] == "malformed_event" for d in report["defects"]))

    def test_missing_ledger_file_means_zero_matched(self):
        write(
            self.changes, json.dumps(changes_doc([change(pr=13838), change(pr=13784)]))
        )
        # No ledger file written at all: the ledger does not exist yet.
        report = self.run_check()
        self.assertTrue(report["ledger"]["missing"])
        self.assertEqual(report["matched"], 0)
        self.assertEqual(len(report["unmatched"]), 2)
        self.assertEqual(report["exit_code"], 1)

    def test_event_for_pr_outside_window_is_noted_not_a_defect(self):
        write(self.changes, json.dumps(changes_doc([change(pr=13838)])))
        write(
            self.ledger,
            "\n".join(
                [
                    json.dumps(event("PR #13838")),
                    json.dumps(event("PR #13700")),
                ]
            )
            + "\n",
        )
        report = self.run_check()
        self.assertEqual(report["exit_code"], 0)
        self.assertEqual(len(report["outside_window_events"]), 1)

    def test_sha_change_matches_full_sha_reference(self):
        sha = "cc2708031280a69437749b640f36c8bf14383c16"
        write(self.changes, json.dumps(changes_doc([change(sha=sha)])))
        write(self.ledger, json.dumps(event("commit " + sha)) + "\n")
        report = self.run_check()
        self.assertEqual(report["matched"], 1)
        self.assertEqual(report["exit_code"], 0)

    def test_empty_window_passes_vacuously(self):
        write(self.changes, json.dumps(changes_doc([])))
        write(self.ledger, "")
        report = self.run_check()
        self.assertEqual(report["exit_code"], 0)


class CliTests(unittest.TestCase):
    def test_unknown_flag_is_usage_error(self):
        proc = subprocess.run(
            [
                sys.executable,
                "-I",
                "scripts/change_ledger.py",
                "check",
                "--ledger",
                "x",
                "--changes",
                "y",
                "--nope",
            ],
            cwd=os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."),
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 2)
        self.assertIn("unexpected argument", proc.stderr)

    def test_missing_required_arg_is_usage_error(self):
        proc = subprocess.run(
            [sys.executable, "-I", "scripts/change_ledger.py", "check"],
            cwd=os.path.join(os.path.dirname(os.path.abspath(__file__)), ".."),
            capture_output=True,
            text=True,
        )
        self.assertEqual(proc.returncode, 2)


if __name__ == "__main__":
    unittest.main(verbosity=2)
