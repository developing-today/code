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
import random
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

    def test_wrong_shape_reference_is_named_and_neutralized(self):
        # Issue #6834: a list/dict reference is unhashable in
        # clone_all's scheduler and manifest's family grouping; an
        # int reference crashes manifest's mixed-type anchor sort.
        for bad in (["a/b"], {"repo": "a/b"}, 123, True):
            entries, problems = ghb_common.validate_plan(
                [{"repo": "a/b", "reference": bad}]
            )
            self.assertEqual(len(entries), 1)
            self.assertIsNone(entries[0]["reference"])
            self.assertEqual(len(problems), 1)
            self.assertIn("entry 0", problems[0])
            self.assertIn("'reference'", problems[0])

    def test_wrong_shape_branch_and_parent_are_neutralized(self):
        # A non-string branch crashes manifest head_sha for a present
        # mirror; parent is copied into MANIFEST.json verbatim.
        entries, problems = ghb_common.validate_plan(
            [{"repo": "a/b", "branch": 7, "parent": ["x/y"]},
             {"repo": "c/d", "branch": ["main"], "parent": {"x": 1}}]
        )
        self.assertEqual(len(entries), 2)
        for entry in entries:
            self.assertIsNone(entry["branch"])
            self.assertIsNone(entry["parent"])
        self.assertEqual(len(problems), 4)
        self.assertIn("'branch'", problems[0])
        self.assertIn("'parent'", problems[1])

    def test_string_and_null_fields_pass_unchanged(self):
        plan = [
            {"repo": "a/b", "reference": "c/d", "branch": "main",
             "parent": "x/y"},
            {"repo": "c/d", "reference": None, "branch": None,
             "parent": None},
            {"repo": "e/f"},
        ]
        entries, problems = ghb_common.validate_plan(plan)
        self.assertEqual(entries, plan)
        self.assertEqual(problems, [])

    def test_validate_plan_does_not_mutate_input(self):
        plan = [{"repo": "a/b", "reference": ["c/d"]}]
        ghb_common.validate_plan(plan)
        self.assertEqual(plan[0]["reference"], ["c/d"])

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

    def test_wrong_shape_last_fetch_is_named_and_dropped(self):
        # Oracle (#6821): fetch_mirrors.main does arithmetic on
        # last_fetch, so any non-number shape must surface as a named
        # problem at validation and never reach the fetch loop.
        for bad in ["2026-11-01T00:00:00Z", [1762000000], {"ts": 1}, True]:
            clean, problems = ghb_common.validate_state(
                {"a/b": {"last_fetch": bad, "fail_days": []}}
            )
            self.assertEqual(len(problems), 1, bad)
            self.assertIn("last_fetch", problems[0])
            self.assertIn("a/b", problems[0])
            self.assertNotIn("last_fetch", clean["a/b"])
            self.assertEqual(clean["a/b"]["fail_days"], [])

    def test_drain_fail_entry_inner_fields_are_preserved(self):
        # drain_fail::<repo> is a drainer bookkeeping object, preserved
        # verbatim under its own shape (#6637) — its inner fields are
        # not per-repo fetch state and are not last_fetch-validated.
        entry = {"count": 2, "last_fetch": {}}
        clean, problems = ghb_common.validate_state({"drain_fail::a/b": entry})
        self.assertEqual(problems, [])
        self.assertEqual(clean["drain_fail::a/b"], entry)

    def test_numeric_last_fetch_passes_unchanged(self):
        for good in [1762000000, 1762000000.5, 0]:
            clean, problems = ghb_common.validate_state(
                {"a/b": {"last_fetch": good}}
            )
            self.assertEqual(problems, [])
            self.assertEqual(clean["a/b"]["last_fetch"], good)

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
                saved = json.load(f)
            # The wrong-shape top level is still dropped as a single
            # named problem (the #6637 contract); the only entry in
            # the saved state is the #3270 missing-day record for
            # a/b — valid_mirror is patched False in run_main, so
            # the repo is missing and its unavailable day is now
            # recorded instead of leaving state untouched.
            self.assertEqual(list(saved), ["a/b"])
            self.assertEqual(len(saved["a/b"]["fail_days"]), 1)

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
    """Issue #6653: every wrong shape the manifest probe reproduced,
    driven against the real manifest.build() — pre-fix each raised a
    raw TypeError/KeyError/AttributeError and no MANIFEST.json or
    FAMILIES.md was written at all."""

    def run_build(self, root, plan, state):
        write_json(root, "plan.json", plan)
        write_json(root, "state.json", state)
        with (
            patch.object(manifest_mod, "load_config", return_value={}),
            contextlib.redirect_stderr(io.StringIO()) as err,
        ):
            manifest_mod.build(root)
        with open(os.path.join(root, "MANIFEST.json")) as f:
            manifest = json.load(f)
        self.assertTrue(os.path.exists(os.path.join(root, "FAMILIES.md")))
        return manifest, err.getvalue()

    def test_build_with_each_poison_plan_entry_shape(self):
        # plan entries: 123, "o/r", [], and a dict with no "repo" —
        # the valid entry must still regenerate the manifest.
        with tempfile.TemporaryDirectory() as root:
            manifest, err = self.run_build(
                root,
                [{"repo": "a/b", "size_kb": 1}, 123, "o/r", [], {"size_kb": 1}],
                {},
            )
            self.assertEqual(list(manifest["mirrors"]), ["a/b"])
            self.assertEqual(manifest["totals"]["missing"], 1)
            for idx in ("entry 1", "entry 2", "entry 3", "entry 4"):
                self.assertIn(idx, err)

    def test_build_with_plan_top_level_dict(self):
        with tempfile.TemporaryDirectory() as root:
            manifest, err = self.run_build(root, {"repo": "o/r"}, {})
            self.assertEqual(manifest["mirrors"], {})
            self.assertIn("plan.json", err)
            self.assertIn("top level", err)

    def test_build_with_poison_state_entries(self):
        # state values 123 and "t" crashed `(state.get(repo) or
        # {}).get("last_fetch")` pre-fix; both are dropped by name
        # and the manifest still regenerates for the valid plan.
        with tempfile.TemporaryDirectory() as root:
            manifest, err = self.run_build(
                root,
                [{"repo": "a/b"}, {"repo": "c/d"}],
                {"a/b": 123, "c/d": "t"},
            )
            self.assertEqual(sorted(manifest["mirrors"]), ["a/b", "c/d"])
            self.assertIsNone(manifest["mirrors"]["a/b"]["last_fetch"])
            self.assertIn("a/b", err)
            self.assertIn("c/d", err)

    def test_build_with_wrong_shape_reference_branch_parent(self):
        # Issue #6834: pre-fix the int reference wrote a bogus
        # "## anchor: 123" family (or crashed the mixed-type sort
        # next to a string anchor) and the list reference raised an
        # unhashable TypeError in families.setdefault.
        with tempfile.TemporaryDirectory() as root:
            manifest, err = self.run_build(
                root,
                [
                    {"repo": "a/b", "reference": 123},
                    {"repo": "c/d", "reference": "a/b"},
                    {"repo": "e/f", "reference": ["a/b"]},
                    {"repo": "g/h", "branch": ["main"], "parent": {"x": 1}},
                ],
                {},
            )
            self.assertEqual(
                sorted(manifest["mirrors"]), ["a/b", "c/d", "e/f", "g/h"]
            )
            self.assertIsNone(manifest["mirrors"]["a/b"]["reference"])
            self.assertIsNone(manifest["mirrors"]["e/f"]["reference"])
            self.assertIsNone(manifest["mirrors"]["g/h"]["default_branch"])
            self.assertIsNone(manifest["mirrors"]["g/h"]["parent"])
            with open(os.path.join(root, "FAMILIES.md")) as f:
                families = f.read()
            self.assertIn("## anchor: a/b", families)
            self.assertNotIn("123", families)
            for field in ("'reference'", "'branch'", "'parent'"):
                self.assertIn(field, err)

    def test_build_with_wrong_shape_branch_and_present_mirror(self):
        # head_sha concatenates "refs/heads/" + branch — only reached
        # for present mirrors, so patch valid_mirror on.
        with tempfile.TemporaryDirectory() as root:
            write_json(
                root, "plan.json", [{"repo": "a/b", "branch": ["main"]}]
            )
            write_json(root, "state.json", {})
            with (
                patch.object(manifest_mod, "load_config", return_value={}),
                patch.object(manifest_mod, "valid_mirror", return_value=True),
                patch.object(manifest_mod, "disk_kb", return_value=0),
                contextlib.redirect_stderr(io.StringIO()) as err,
            ):
                manifest_mod.build(root)
            with open(os.path.join(root, "MANIFEST.json")) as f:
                manifest = json.load(f)
            self.assertIsNone(manifest["mirrors"]["a/b"]["head"])
            self.assertIsNone(manifest["mirrors"]["a/b"]["default_branch"])
            self.assertIn("'branch'", err.getvalue())

    def test_build_with_state_top_level_list(self):
        with tempfile.TemporaryDirectory() as root:
            manifest, err = self.run_build(root, [{"repo": "a/b"}], [])
            self.assertEqual(list(manifest["mirrors"]), ["a/b"])
            self.assertIn("state.json", err)

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


class ShapePropertyTests(unittest.TestCase):
    """Property tests for the validators (testing doctrine: parsers
    and validators carry invariants, and a generator hammers them).

    Stdlib-only: a seeded `random.Random` generates arbitrary
    JSON-shaped values. Oracles are independent restatements of the
    shape contract, not the implementation:

    - plan oracle: an entry is valid iff it is a dict whose "repo"
      is a string; the valid entries, in order, are exactly the
      accepted ones, and every invalid entry yields one problem.
    - state oracle: a top-level value is valid iff per-repo values
      are dicts (with fail_days absent or a list), drain_last::
      values are non-bool numbers, and drain_fail:: values are
      dicts; valid inputs survive validation unchanged, validation
      is idempotent, and a poison entry never changes the verdict
      on its siblings.
    """

    SEED = 6637
    CASES = 2000

    def gen_value(self, rng, depth=0):
        kinds = ["dict", "list", "str", "int", "float", "bool", "none"]
        if depth >= 3:
            kinds = ["str", "int", "bool", "none"]
        kind = rng.choice(kinds)
        if kind == "dict":
            keys = rng.sample(
                ["repo", "size_kb", "fail_days", "last_fetch", "x"],
                rng.randint(0, 3),
            )
            return {k: self.gen_value(rng, depth + 1) for k in keys}
        if kind == "list":
            return [self.gen_value(rng, depth + 1) for _ in range(rng.randint(0, 3))]
        if kind == "str":
            return rng.choice(["a/b", "c/d", "true", "", "2026-01-01"])
        if kind == "int":
            return rng.randint(-5, 10**6)
        if kind == "float":
            return rng.uniform(-5, 10**6)
        if kind == "bool":
            return rng.choice([True, False])
        return None

    def gen_plan(self, rng):
        if rng.random() < 0.1:
            return self.gen_value(rng)
        plan = []
        for _ in range(rng.randint(0, 6)):
            r = rng.random()
            if r < 0.45:
                plan.append({"repo": rng.choice(["a/b", "c/d", "e/f"])})
            elif r < 0.6:
                entry = {"repo": rng.choice(["a/b", "c/d"])}
                entry["size_kb"] = rng.randint(0, 100)
                plan.append(entry)
            else:
                plan.append(self.gen_value(rng))
        return plan

    @staticmethod
    def plan_oracle(plan):
        if not isinstance(plan, list):
            return [], 1
        valid = [
            e
            for e in plan
            if isinstance(e, dict) and isinstance(e.get("repo"), str)
        ]
        return valid, len(plan) - len(valid)

    def test_plan_invariants_over_generated_inputs(self):
        rng = random.Random(self.SEED)
        for _ in range(self.CASES):
            plan = self.gen_plan(rng)
            # Round-trip through JSON: the validators only ever see
            # json.load output.
            plan = json.loads(json.dumps(plan))
            entries, problems = ghb_common.validate_plan(plan)
            want_entries, want_problems = self.plan_oracle(plan)
            self.assertEqual(entries, want_entries)
            self.assertEqual(len(problems), want_problems)
            for e in entries:
                self.assertIsInstance(e["repo"], str)

    @staticmethod
    def state_oracle(state):
        if not isinstance(state, dict):
            return {}, 1
        clean, problems = {}, 0
        for key, value in state.items():
            if key.startswith("drain_last::"):
                if isinstance(value, (int, float)) and not isinstance(value, bool):
                    clean[key] = value
                else:
                    problems += 1
                continue
            if not isinstance(value, dict):
                problems += 1
                continue
            entry = dict(value)
            days = entry.get("fail_days")
            if days is not None and not isinstance(days, list):
                entry["fail_days"] = []
                problems += 1
            # #6821: a wrong-shape per-repo last_fetch is dropped
            # under a named problem; drain bookkeeping entries are
            # preserved under their own shapes.
            last_fetch = entry.get("last_fetch")
            if (
                not key.startswith("drain_")
                and last_fetch is not None
                and (
                    not isinstance(last_fetch, (int, float))
                    or isinstance(last_fetch, bool)
                )
            ):
                del entry["last_fetch"]
                problems += 1
            clean[key] = entry
        return clean, problems

    def gen_state(self, rng):
        if rng.random() < 0.1:
            return self.gen_value(rng)
        state = {}
        for _ in range(rng.randint(0, 5)):
            r = rng.random()
            if r < 0.4:
                state[rng.choice(["a/b", "c/d"])] = {
                    "last_fetch": rng.randint(0, 10**6),
                    "fail_days": ["2026-01-01"],
                }
            elif r < 0.55:
                state[f"drain_last::{rng.choice(['a/b', 'c/d'])}"] = rng.uniform(
                    0, 10**6
                )
            elif r < 0.65:
                state[f"drain_fail::{rng.choice(['a/b', 'c/d'])}"] = {
                    "count": rng.randint(0, 5),
                    "first": 1,
                    "last": 2,
                }
            else:
                key = rng.choice(
                    ["a/b", "c/d", "drain_last::a/b", "drain_fail::c/d", "zz"]
                )
                state[key] = self.gen_value(rng)
        return state

    def test_state_invariants_over_generated_inputs(self):
        rng = random.Random(self.SEED + 1)
        for _ in range(self.CASES):
            state = json.loads(json.dumps(self.gen_state(rng)))
            clean, problems = ghb_common.validate_state(state)
            want_clean, want_problems = self.state_oracle(state)
            self.assertEqual(clean, want_clean)
            self.assertEqual(len(problems), want_problems)
            # Idempotence: a validated state validates clean.
            again, again_problems = ghb_common.validate_state(clean)
            self.assertEqual(again, clean)
            self.assertEqual(again_problems, [])

    def test_poison_entry_never_changes_sibling_verdicts(self):
        rng = random.Random(self.SEED + 2)
        poisons = [123, "oops", [1], None, True, {"name": "x"}, {"repo": 7}]
        for _ in range(500):
            plan = self.gen_plan(rng)
            if not isinstance(plan, list):
                continue
            plan = json.loads(json.dumps(plan))
            base_entries, base_problems = ghb_common.validate_plan(plan)
            poisoned = plan + [rng.choice(poisons)]
            entries, problems = ghb_common.validate_plan(poisoned)
            self.assertEqual(entries, base_entries)
            self.assertEqual(len(problems), len(base_problems) + 1)


if __name__ == "__main__":
    unittest.main()
