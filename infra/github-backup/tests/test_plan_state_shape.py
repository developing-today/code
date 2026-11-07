#!/usr/bin/env python3
"""Regression tests for plan/state wrong-shape crashes (#6637).

A plan.json / state.json that is valid JSON but the wrong shape used
to crash fetch_mirrors (and its sibling consumers manifest.py and
clone_all.py) with raw TypeError / KeyError / AttributeError
tracebacks, wedging the whole run for every repo until a human
repaired the file. The fix validates shape at load in ghb_common:
bad entries are reported by name and skipped; a wrong-shape top
level is a single named problem; valid behaviour is unchanged.

Oracle: the in-tree contract from clone_all.py's clone-status.jsonl
reader and #6606 — bad entries are skipped with a note, never fatal
to the whole run.

Run: python3 infra/github-backup/tests/test_plan_state_shape.py
"""

import contextlib
import io
import json
import os
import sys
import tempfile
import unittest
from types import SimpleNamespace
from unittest.mock import patch

BIN = os.path.abspath(
    os.path.join(os.path.dirname(os.path.abspath(__file__)), "..", "bin")
)
sys.path.insert(0, BIN)

import ghb_common
import fetch_mirrors
import manifest as manifest_mod


def write_json(root, name, value):
    with open(os.path.join(root, name), "w") as f:
        json.dump(value, f)


class ValidatePlanTests(unittest.TestCase):
    def test_valid_plan_passes_through(self):
        plan = [{"repo": "a/b", "size_kb": 1}, {"repo": "c/d"}]
        entries, problems = ghb_common.validate_plan(plan)
        self.assertEqual(entries, plan)
        self.assertEqual(problems, [])

    def test_plan_entry_int_is_named_and_skipped(self):
        entries, problems = ghb_common.validate_plan([{"repo": "a/b"}, 123])
        self.assertEqual([e["repo"] for e in entries], ["a/b"])
        self.assertEqual(len(problems), 1)
        self.assertIn("entry 1", problems[0])

    def test_plan_entry_missing_repo_is_named_and_skipped(self):
        entries, problems = ghb_common.validate_plan([{"name": "x"}])
        self.assertEqual(entries, [])
        self.assertIn("entry 0", problems[0])

    def test_plan_entry_string_repo_value_required(self):
        entries, problems = ghb_common.validate_plan([{"repo": 42}, "a/b"])
        self.assertEqual(entries, [])
        self.assertEqual(len(problems), 2)

    def test_plan_top_level_object_is_single_named_problem(self):
        entries, problems = ghb_common.validate_plan({"repo": "a/b"})
        self.assertEqual(entries, [])
        self.assertEqual(len(problems), 1)
        self.assertIn("top level", problems[0])


class ValidateStateTests(unittest.TestCase):
    def test_state_entry_string_is_named_and_dropped(self):
        clean, problems = ghb_common.validate_state(
            {"a/b": {"last_fetch": 1}, "c/d": "oops"}
        )
        self.assertEqual(clean, {"a/b": {"last_fetch": 1}})
        self.assertEqual(len(problems), 1)
        self.assertIn("c/d", problems[0])

    def test_state_top_level_list_is_single_named_problem(self):
        clean, problems = ghb_common.validate_state([1, 2])
        self.assertEqual(clean, {})
        self.assertIn("top level", problems[0])

    def test_drain_bookkeeping_entries_are_preserved(self):
        # drain_queue.py keeps its coalescing state in the same file:
        # drain_last::<repo> is a numeric timestamp, drain_fail::<repo>
        # an object. Validation must not drop them (Codex review, PR
        # #6696) — fetch_mirrors.main() writes the state back.
        clean, problems = ghb_common.validate_state(
            {
                "a/b": {"last_fetch": 1},
                "drain_last::a/b": 1760000000.5,
                "drain_fail::a/b": {"count": 2, "first": 1, "last": 2},
            }
        )
        self.assertEqual(problems, [])
        self.assertEqual(clean["drain_last::a/b"], 1760000000.5)
        self.assertEqual(clean["drain_fail::a/b"]["count"], 2)

    def test_drain_last_with_wrong_shape_is_named_and_dropped(self):
        clean, problems = ghb_common.validate_state(
            {"drain_last::a/b": "soon", "drain_last::c/d": True}
        )
        self.assertEqual(clean, {})
        self.assertEqual(len(problems), 2)
        self.assertIn("drain_last::a/b", problems[0])

    def test_non_list_fail_days_is_reset_with_problem(self):
        clean, problems = ghb_common.validate_state(
            {"a/b": {"fail_days": "2026-01-01"}}
        )
        self.assertEqual(clean["a/b"]["fail_days"], [])
        self.assertIn("fail_days", problems[0])


class FetchMirrorsShapeTests(unittest.TestCase):
    def test_iter_plan_repos_skips_poison_entries(self):
        with tempfile.TemporaryDirectory() as root:
            write_json(
                root,
                "plan.json",
                [{"repo": "a/b"}, 123, {"name": "x"}, {"repo": "c/d"}],
            )
            with contextlib.redirect_stderr(io.StringIO()) as err:
                repos = fetch_mirrors.iter_plan_repos(root)
            self.assertEqual(repos, ["a/b", "c/d"])
            self.assertIn("entry 1", err.getvalue())
            self.assertIn("entry 2", err.getvalue())

    def test_record_failure_day_rejects_non_list_days(self):
        for bad in ["2026-01-01", 123, {"2026-01-01": True}]:
            with self.assertRaises(ValueError) as cm:
                fetch_mirrors.record_failure_day(bad, "2026-01-02")
            self.assertIn("fail_days", str(cm.exception))

    def test_record_failure_day_list_behaviour_unchanged(self):
        days = ["2026-01-01"]
        fetch_mirrors.record_failure_day(days, "2026-01-02")
        self.assertEqual(days, ["2026-01-01", "2026-01-02"])

    def run_main(self, root, plan, state):
        write_json(root, "plan.json", plan)
        write_json(root, "state.json", state)
        with (
            patch.object(fetch_mirrors, "load_config", return_value={}),
            patch.object(fetch_mirrors, "backup_root", return_value=root),
            patch.object(fetch_mirrors, "mirror_path", return_value="/mirror"),
            patch.object(fetch_mirrors, "valid_mirror", return_value=False),
            patch.object(fetch_mirrors.manifest_mod, "build"),
            contextlib.redirect_stdout(io.StringIO()),
            contextlib.redirect_stderr(io.StringIO()) as err,
        ):
            code = fetch_mirrors.main(["fetch_mirrors.py"])
        return code, err.getvalue()

    def test_main_survives_state_top_level_list(self):
        with tempfile.TemporaryDirectory() as root:
            code, err = self.run_main(root, [{"repo": "a/b"}], [1, 2])
            self.assertEqual(code, 0)
            self.assertIn("state.json", err)
            with open(os.path.join(root, "state.json")) as f:
                self.assertEqual(json.load(f), {})

    def test_main_survives_string_state_entry_and_poison_plan(self):
        with tempfile.TemporaryDirectory() as root:
            code, err = self.run_main(
                root,
                [{"repo": "a/b"}, 123],
                {"a/b": "oops"},
            )
            self.assertEqual(code, 0)
            self.assertIn("a/b", err)  # poison state entry named
            self.assertIn("entry 1", err)  # poison plan entry named

    def test_fetch_repo_sanitizes_poison_state_entry(self):
        state = {"a/b": {"fail_days": "2026-01-01"}}
        with (
            patch.object(fetch_mirrors, "mirror_path", return_value="/mirror"),
            patch.object(fetch_mirrors, "valid_mirror", return_value=True),
            patch.object(
                fetch_mirrors.subprocess,
                "run",
                return_value=SimpleNamespace(
                    returncode=1, stdout="", stderr="boom"
                ),
            ),
            contextlib.redirect_stderr(io.StringIO()) as err,
        ):
            result = fetch_mirrors.fetch_repo("/root", "a/b", state)
        self.assertEqual(result, "fail")
        self.assertIsInstance(state["a/b"]["fail_days"], list)
        self.assertEqual(len(state["a/b"]["fail_days"]), 1)  # today recorded
        self.assertIn("fail_days", err.getvalue())


class ManifestShapeTests(unittest.TestCase):
    def test_build_regenerates_with_poison_entries_skipped(self):
        with tempfile.TemporaryDirectory() as root:
            write_json(
                root,
                "plan.json",
                [{"repo": "a/b", "size_kb": 3}, 123, {"name": "x"}],
            )
            write_json(root, "state.json", {"a/b": "oops", "c/d": 7})
            with (
                patch.object(manifest_mod, "load_config", return_value={}),
                contextlib.redirect_stderr(io.StringIO()) as err,
            ):
                manifest_mod.build(root)
            with open(os.path.join(root, "MANIFEST.json")) as f:
                manifest = json.load(f)
            self.assertEqual(list(manifest["mirrors"]), ["a/b"])
            self.assertIn("entry 1", err.getvalue())
            self.assertIn("c/d", err.getvalue())


if __name__ == "__main__":
    unittest.main()
