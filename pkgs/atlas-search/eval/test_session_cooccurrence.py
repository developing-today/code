"""Oracles use explicit session sets, chronological halves and hand-counted pairs."""

import json
from pathlib import Path
import unittest

from session_cooccurrence import candidates, evaluate, index_sessions, partitions


def fixture():
    path = Path(__file__).with_name("fixtures") / "session-cooccurrence.json"
    return json.loads(path.read_text(encoding="utf-8"))


class SessionCooccurrenceTests(unittest.TestCase):
    def test_hand_counted_tie_and_cold_start(self):
        report = evaluate(fixture(), 2)
        self.assertEqual(report["scored_sessions"], 2)
        self.assertEqual(report["hits"], 1)
        self.assertEqual(report["held_out_items"], 2)
        self.assertEqual(report["precision_at_k"], 0.25)
        self.assertEqual(report["recall_at_k"], 0.5)
        self.assertEqual(report["mrr_at_k"], 0.5)
        self.assertEqual(report["ndcg_at_k"], 0.5)

    def test_retired_foreign_and_cross_cutoff_cannot_change_candidates_or_metrics(self):
        data = fixture()
        original = evaluate(data, 2)
        data["eligible_sessions"] += ["crossing", "another"]
        for sid, owner, start, end in [
            ("retired", "reader", 1, 2),
            ("crossing", "reader", 2, 14),
            ("another", "different-reader", 1, 2),
        ]:
            data["sessions"].append(dict(id=sid, subject=owner, start=start, end=end,
                                         opens=["anchor", "companion", "novel"]))
        for session in data["sessions"]:
            session["opens"].insert(0, "retired-item")
        self.assertEqual(original, evaluate(data, 2))
        train, _ = partitions(data)
        self.assertEqual(candidates(index_sessions(train), ["anchor"], 3), ["bridge", "companion"])

    def test_deduplicates_before_scoring_and_splitting(self):
        data = fixture()
        data["sessions"][2]["opens"] = ["anchor", "anchor", "bridge", "anchor"]
        self.assertEqual(evaluate(data, 2), evaluate(fixture(), 2))
        index = index_sessions([["a", "x", "x", "y"], ["a", "y"], ["b", "x"]])
        self.assertEqual(candidates(index, ["a", "a"], 2), ["y", "x"])
        self.assertEqual(candidates(index, ["a", "b"], 2), ["x", "y"])

    def test_held_out_sessions_never_train_the_graph(self):
        data = fixture()
        data["sessions"] = data["sessions"][2:]
        report = evaluate(data, 2)
        self.assertEqual((report["training_sessions"], report["hits"], report["mrr_at_k"]), (0, 0, 0))

    def test_empty_scope_and_ambiguous_input(self):
        data = fixture()
        data["eligible_items"] = []
        self.assertIsNone(evaluate(data, 2)["ndcg_at_k"])
        data["sessions"].append(data["sessions"][0])
        with self.assertRaises(ValueError):
            evaluate(data, 2)
        with self.assertRaises(ValueError):
            evaluate(fixture(), 0)


if __name__ == "__main__":
    unittest.main()
