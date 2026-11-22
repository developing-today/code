"""Tests for the report-only epic closure monitor adapter."""

import unittest

import epic_close_check as checks


class EpicCloseCheckTests(unittest.TestCase):
    def test_reports_each_root_and_preserves_condition_blockers(self):
        blockers = [{"condition": "qualifying_open_descendants", "refs": ["o/r#2"]}]
        records = checks.monitor_records(
            {
                "reports": [
                    {
                        "root": "o/r#1",
                        "rootState": "OPEN",
                        "descendants": 1,
                        "factsComplete": True,
                        "closable": False,
                        "blockers": blockers,
                    }
                ]
            }
        )

        self.assertEqual(
            records,
            [
                {
                    "check": "epic_closure_gate",
                    "root": "o/r#1",
                    "status": "blocked",
                    "root_state": "OPEN",
                    "descendants": 1,
                    "facts_complete": True,
                    "closable": False,
                    "blockers": blockers,
                    "closed_with_unfinished_children": False,
                    "skipped_reparent_refs": [],
                }
            ],
        )

    def test_incomplete_facts_are_unavailable_not_clear(self):
        records = checks.monitor_records(
            {
                "reports": [
                    {
                        "root": "o/r#1",
                        "rootState": "OPEN",
                        "descendants": 0,
                        "factsComplete": False,
                        "closable": False,
                        "blockers": [],
                    }
                ]
            }
        )

        self.assertEqual(records[0]["status"], "unavailable")
        self.assertFalse(records[0]["closable"])

    def test_clear_gate_is_reported_without_a_blocking_exit(self):
        records = checks.monitor_records(
            {
                "reports": [
                    {
                        "root": "o/r#1",
                        "rootState": "OPEN",
                        "descendants": 0,
                        "factsComplete": True,
                        "closable": True,
                        "blockers": [],
                    }
                ]
            }
        )

        self.assertEqual(records[0]["status"], "clear")

    def test_rejects_malformed_or_missing_reports(self):
        with self.assertRaises(ValueError):
            checks.monitor_records({})
        with self.assertRaises(ValueError):
            checks.monitor_records({"reports": [{"root": "o/r#1", "rootState": "OPEN"}]})

    def test_closed_root_with_unfinished_direct_child_reports_skipped_reparent(self):
        records = checks.monitor_records(
            {
                "reports": [
                    {
                        "root": "o/r#1",
                        "rootState": "CLOSED",
                        "descendants": 1,
                        "factsComplete": True,
                        "closable": False,
                        "blockers": [
                            {
                                "condition": "unfinished_children_without_reparent_destination",
                                "refs": ["o/r#2"],
                            }
                        ],
                    }
                ]
            }
        )

        self.assertTrue(records[0]["closed_with_unfinished_children"])
        self.assertEqual(records[0]["skipped_reparent_refs"], ["o/r#2"])

    def test_all_roots_are_validated_and_duplicates_rejected(self):
        with self.assertRaises(SystemExit):
            checks.parse_args(["--root", "o/r#1", "--root", "bad"])
        with self.assertRaises(SystemExit):
            checks.parse_args(["--root", "o/r#1", "--root", "o/r#1"])


if __name__ == "__main__":
    unittest.main()
