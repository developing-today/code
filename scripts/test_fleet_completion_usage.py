"""Oracle: console trends' latest finishing event + exact cohort token sums.

Permutation checks exercise the aggregation's order-invariance property.
No network, service, credentials, or non-stdlib dependencies are needed.
"""

import copy
import importlib.util
from pathlib import Path
import random
import unittest

SPEC = importlib.util.spec_from_file_location(
    "fleet_completion_usage", Path(__file__).with_name("fleet_completion_usage.py")
)
usage = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(usage)
START = "2026-11-01T00:00:00Z"
END = "2026-11-02T00:00:00Z"


def fixture():
    return {
        "tasks": [
            {"id": 1, "queue": "code", "status": "done", "progress": {"session_id": "s1"}},
            {"id": 2, "queue": "code", "status": "done", "progress": "{}"},
            {"id": 3, "queue": "other", "status": "queued"},
        ],
        "events": [
            {"task_id": 1, "kind": "completed", "created_at": START},
            {"task_id": 1, "kind": "completed", "created_at": "2026-11-01T02:00:00Z"},
            {"task_id": 2, "kind": "staleness_autocomplete", "created_at": START},
            {"task_id": 3, "kind": "completed", "created_at": START},
        ],
        "ledger": [
            {"task_id": 1, "queue": "code", "session_id": "retry"},
            {"task_id": 1, "queue": "code", "session_id": "s1"},
            {"task_id": 2, "queue": "code", "session_id": "s2"},
        ],
        "sessions": [
            {"id": "s1", "tokens": {"input": 10, "output": 900}},
            {"id": "retry", "tokens": {"input": 5}},
            {"id": "s2", "tokens": {"input": 25}},
        ],
    }


class CompletionUsageTest(unittest.TestCase):
    def result(self, data):
        return usage.report(data, START, END)["queues"][0]

    def test_exact_sum_unique_completions_and_retry_sessions(self):
        self.assertEqual(self.result(fixture()), {
            "queue": "code", "completed_tasks": 2, "measured_tasks": 2,
            "missing_task_ids": [], "known_input_tokens": 40,
            "input_tokens_per_completed_task": 20,
        })

    def test_incomplete_usage_is_not_zero_or_partial_ratio(self):
        data = fixture()
        data["sessions"].pop()
        result = self.result(data)
        self.assertEqual(result["measured_tasks"], 1)
        self.assertEqual(result["known_input_tokens"], 15)
        self.assertEqual(result["missing_task_ids"], [2])
        self.assertIsNone(result["input_tokens_per_completed_task"])

    def test_claim_session_is_not_execution_binding(self):
        data = fixture()
        data["ledger"] = []
        data["tasks"][1]["claim_session_id"] = "s2"
        self.assertEqual(self.result(data)["missing_task_ids"], [2])

    def test_latest_completion_and_half_open_window(self):
        data = fixture()
        data["events"].append({"task_id": 1, "kind": "completed", "created_at": END})
        self.assertEqual(self.result(data)["completed_tasks"], 1)
        self.assertEqual(self.result(data)["known_input_tokens"], 25)

    def test_no_completed_tasks_has_no_ratio(self):
        data = fixture()
        data["events"] = []
        self.assertEqual(usage.report(data, START, END)["queues"], [])

    def test_shared_session_rejects_double_attribution(self):
        data = fixture()
        data["ledger"].append({"task_id": 2, "queue": "code", "session_id": "s1"})
        with self.assertRaisesRegex(ValueError, "multiple tasks"):
            self.result(data)

    def test_invalid_token_counts_are_not_silently_coerced(self):
        for invalid in [-1, True, 1.5, "10"]:
            data = fixture()
            data["sessions"][0]["tokens"]["input"] = invalid
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                self.result(data)

    def test_duplicate_usage_rows_rejected(self):
        data = fixture()
        data["sessions"].append(data["sessions"][0])
        with self.assertRaisesRegex(ValueError, "duplicate session"):
            self.result(data)

    def test_permutation_invariance(self):
        expected = usage.report(fixture(), START, END)
        rng = random.Random(2140)
        for _ in range(100):
            data = copy.deepcopy(fixture())
            for rows in data.values():
                rng.shuffle(rows)
            self.assertEqual(usage.report(data, START, END), expected)


if __name__ == "__main__":
    unittest.main()
