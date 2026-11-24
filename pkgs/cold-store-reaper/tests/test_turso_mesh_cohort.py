#!/usr/bin/env python3
"""Tests for scripts/turso-mesh-cohorts.py (#13456, slice 1).

Oracles:
- Example tests: the cohort rule in the script's docstring
  (``sha256(name)[:8] % 14`` for assignment, days-since-epoch mod 14
  for the due cohort) recomputed independently here, plus the design's
  class-A set (``fleet-queue``, ``fleet-platform``, ``atlas``).
- Property-style tests (stdlib-only, fixed seed — no hypothesis in
  this repo's Python tooling, per test_repair_dump_property.py):
  a generator emits random inventories and the invariants are
  checked directly: cohort indices in range, every class-B database
  in exactly one cohort, and inventory permutation changing nothing.
  A failing case prints its seed and case index.

Usage: python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
"""

import json
import random
import subprocess
import sys
import unittest
from datetime import date
from pathlib import Path

PLANNER = Path(__file__).resolve().parent.parent / "scripts" / "turso-mesh-cohorts.py"

CLASS_A = ("fleet-queue", "fleet-platform", "atlas")
SEED = 20261123


def run_cli(*args, stdin_text=""):
    return subprocess.run(
        [sys.executable, str(PLANNER), *args],
        input=stdin_text.encode(),
        capture_output=True,
        check=False,
    )


def plan_for(inventory, day):
    result = run_cli("--date", day.isoformat(), stdin_text="\n".join(inventory))
    assert result.returncode == 0, result.stderr
    return json.loads(result.stdout.decode())


class TestCohortRule(unittest.TestCase):
    def test_epoch_date_is_cohort_zero(self):
        # Oracle: (1970-01-01 - epoch).days % 14 == 0.
        self.assertEqual(plan_for([], date(1970, 1, 1))["cohort"], 0)

    def test_date_to_cohort_is_days_since_epoch_mod_14(self):
        # Oracle: the rule recomputed independently from the date.
        for day in (date(1970, 1, 2), date(1970, 1, 15), date(2026, 11, 24)):
            expected = (day - date(1970, 1, 1)).days % 14
            self.assertEqual(plan_for([], day)["cohort"], expected)

    def test_class_a_databases_are_excluded_and_reported(self):
        # Oracle: the design's class-A set; excluded names appear in
        # excluded_class_a and in no cohort.
        inventory = list(CLASS_A) + ["general-db", "other-db"]
        got = plan_for(inventory, date(2026, 11, 24))
        self.assertEqual(got["excluded_class_a"], sorted(CLASS_A))
        self.assertEqual(got["class_b_total"], 2)
        self.assertNotIn("atlas", got["databases"])
        full = json.loads(
            run_cli(
                "--date", "2026-11-24", "--all", stdin_text="\n".join(inventory)
            ).stdout.decode()
        )
        held = [name for cohort in full["cohorts"] for name in cohort["databases"]]
        for name in CLASS_A:
            self.assertNotIn(name, held)

    def test_cohort_assignment_matches_sha256_rule(self):
        # Oracle: sha256(name)[:8] % 14, recomputed here.
        import hashlib

        inventory = ["alpha-db", "beta-db", "gamma-db", "fleet-queue"]
        got = json.loads(
            run_cli(
                "--date", "2026-11-24", "--all", stdin_text="\n".join(inventory)
            ).stdout.decode()
        )
        by_cohort = {
            entry["cohort"]: entry["databases"] for entry in got["cohorts"]
        }
        for name in ("alpha-db", "beta-db", "gamma-db"):
            expected = int.from_bytes(
                hashlib.sha256(name.encode()).digest()[:8], "big"
            ) % 14
            self.assertIn(name, by_cohort[expected])

    def test_due_cohort_databases_are_that_cohorts_members(self):
        inventory = ["alpha-db", "beta-db", "gamma-db", "delta-db"]
        day = date(2026, 11, 24)
        due = plan_for(inventory, day)
        full = json.loads(
            run_cli("--date", day.isoformat(), "--all", stdin_text="\n".join(inventory)).stdout.decode()
        )
        members = next(
            entry["databases"] for entry in full["cohorts"] if entry["cohort"] == due["cohort"]
        )
        self.assertEqual(due["databases"], members)

    def test_same_inventory_and_date_always_give_the_same_plan(self):
        inventory = ["zeta-db", "alpha-db", "fleet-platform", "beta-db"]
        first = plan_for(inventory, date(2026, 11, 24))
        second = plan_for(list(reversed(inventory)), date(2026, 11, 24))
        self.assertEqual(first, second)

    def test_inventory_order_and_duplicates_do_not_change_the_plan(self):
        inventory = ["one-db", "two-db", "three-db", "atlas"]
        base = plan_for(inventory, date(2026, 11, 24))
        shuffled = plan_for(
            ["three-db", "one-db", "two-db", "two-db", "atlas", "one-db"],
            date(2026, 11, 24),
        )
        self.assertEqual(base, shuffled)

    def test_empty_inventory_is_a_valid_empty_plan(self):
        got = plan_for([], date(2026, 11, 24))
        self.assertEqual(got["databases"], [])
        self.assertEqual(got["class_b_total"], 0)
        self.assertEqual(got["excluded_class_a"], [])

    def test_cohort_flag_selects_one_cohort(self):
        inventory = ["alpha-db", "beta-db", "gamma-db"]
        got = json.loads(
            run_cli("--date", "2026-11-24", "--cohort", "5", stdin_text="\n".join(inventory)).stdout.decode()
        )
        full = json.loads(
            run_cli("--date", "2026-11-24", "--all", stdin_text="\n".join(inventory)).stdout.decode()
        )
        members = next(
            entry["databases"] for entry in full["cohorts"] if entry["cohort"] == 5
        )
        self.assertEqual(got["cohort"], 5)
        self.assertEqual(got["databases"], members)

    def test_bad_date_and_bad_cohort_are_usage_errors(self):
        self.assertEqual(run_cli("--date", "2026-13-01").returncode, 2)
        self.assertEqual(run_cli("--date", "tomorrow").returncode, 2)
        self.assertEqual(run_cli("--cohort", "14").returncode, 2)
        self.assertEqual(run_cli("--cohort", "-1").returncode, 2)
        self.assertEqual(run_cli("--bogus-flag").returncode, 2)

    def test_inventory_file_input_matches_stdin(self):
        import tempfile

        inventory = ["file-db", "stdin-db", "atlas"]
        with tempfile.NamedTemporaryFile("w", suffix=".txt", delete=False) as handle:
            handle.write("\n".join(inventory) + "\n")
            path = handle.name
        try:
            from_file = json.loads(
                run_cli("--date", "2026-11-24", "--inventory", path).stdout.decode()
            )
        finally:
            Path(path).unlink()
        self.assertEqual(from_file, plan_for(inventory, date(2026, 11, 24)))


class TestCohortInvariants(unittest.TestCase):
    """Property-style invariants over generated inventories (#13456)."""

    def test_generated_inventories(self):
        rng = random.Random(SEED)
        alphabet = "abcdefghij-"
        for case in range(200):
            size = rng.randrange(0, 60)
            inventory = [
                "".join(rng.choice(alphabet) for _ in range(rng.randrange(1, 12)))
                for _ in range(size)
            ]
            # Sprinkle class-A names in: they must never land in a cohort.
            if case % 3 == 0:
                inventory += list(CLASS_A)
            day = date(2026, 11, 24)
            try:
                got = plan_for(inventory, day)
                full = json.loads(
                    run_cli(
                        "--date", day.isoformat(), "--all",
                        stdin_text="\n".join(inventory),
                    ).stdout.decode()
                )
            except AssertionError:
                print("seed=%d case=%d inventory=%r" % (SEED, case, inventory))
                raise
            class_b = sorted(set(inventory) - set(CLASS_A))
            held = [name for entry in full["cohorts"] for name in entry["databases"]]
            # In range: 14 cohorts, indices 0..13.
            self.assertEqual(len(full["cohorts"]), 14)
            self.assertEqual([entry["cohort"] for entry in full["cohorts"]], list(range(14)))
            # Unique: every class-B database in exactly one cohort.
            self.assertEqual(sorted(held), class_b)
            self.assertEqual(len(held), len(set(held)))
            # Order-independent: a permutation changes nothing.
            shuffled = inventory[:]
            rng.shuffle(shuffled)
            again = plan_for(shuffled, day)
            self.assertEqual(got, again, "seed=%d case=%d" % (SEED, case))
            # Due cohort: the plan's databases are that cohort's members.
            members = next(
                entry["databases"]
                for entry in full["cohorts"]
                if entry["cohort"] == got["cohort"]
            )
            self.assertEqual(got["databases"], members)
            self.assertEqual(got["class_b_total"], len(class_b))


if __name__ == "__main__":
    unittest.main()
