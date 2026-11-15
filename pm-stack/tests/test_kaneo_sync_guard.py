#!/usr/bin/env python3
"""test_kaneo_sync_guard.py — heal-persistence tests for kaneo-sync-guard (#10528).

Oracle: after a heal, the config PERSISTED in the integration row must
equal the healed map the guard computes in memory — EXPECTED applied,
every non-expected transition targeting a FORBIDDEN target removed,
all other transitions preserved — including when the stored config has
no statusTransitions object at all. A heal whose write does not stick
must surface as GUARD ERROR (exit 2), never as HEALED.

The guard's psql() is replaced with an in-memory fake that emulates
the one SQL shape the heal uses (jsonb_set at the {statusTransitions}
path, which creates the key when missing) — no Docker/Postgres needed.

Run:  python3 pm-stack/tests/test_kaneo_sync_guard.py
"""

import contextlib
import copy
import importlib.util
import io
import json
import re
import sys
import unittest
from pathlib import Path

GUARD_PATH = Path(__file__).resolve().parent.parent / "kaneo-sync-guard.py"
spec = importlib.util.spec_from_file_location("kaneo_sync_guard", GUARD_PATH)
guard = importlib.util.module_from_spec(spec)
spec.loader.exec_module(guard)

EXPECTED = dict(guard.EXPECTED)


class FakePsql:
    """In-memory stand-in for the guard's psql() against a fake store.

    store: {integration_id: config dict}. Counts UPDATEs so a test can
    prove a conforming config is never written.
    """

    def __init__(self, store, apply_updates=True):
        self.store = copy.deepcopy(store)
        self.apply_updates = apply_updates
        self.updates = 0

    def __call__(self, sql):
        if sql.startswith("SELECT id ||"):
            return "\n".join(
                f"{integ_id}\t{json.dumps(cfg)}" for integ_id, cfg in self.store.items()
            )
        if sql.startswith("UPDATE integration"):
            self.updates += 1
            if not self.apply_updates:
                return ""
            integ_id = re.search(r"WHERE id='([^']+)'", sql).group(1)
            raw = re.search(
                r"'\{statusTransitions\}', '(.*)'::jsonb", sql, re.DOTALL
            ).group(1)
            healed = json.loads(raw.replace("''", "'"))
            self.store[integ_id]["statusTransitions"] = healed
            return ""
        if sql.startswith("SELECT config FROM"):
            integ_id = re.search(r"id='([^']+)'", sql).group(1)
            return json.dumps(self.store[integ_id])
        raise AssertionError(f"unexpected SQL: {sql!r}")


def run_guard(store, apply_updates=True):
    fake = FakePsql(store, apply_updates=apply_updates)
    original = guard.psql
    guard.psql = fake
    out = io.StringIO()
    try:
        with contextlib.redirect_stdout(out):
            rc = guard.main()
    finally:
        guard.psql = original
    return rc, out.getvalue(), fake


BASE = {"repositoryOwner": "developing-today", "repositoryName": "code"}


class HealPersistenceTests(unittest.TestCase):
    def test_missing_status_transitions_is_created_complete(self):
        rc, out, fake = run_guard({"int1": dict(BASE)})
        self.assertEqual(rc, 1)  # drifted and healed
        self.assertIn("HEALED", out)
        self.assertEqual(fake.store["int1"]["statusTransitions"], EXPECTED)

    def test_extra_forbidden_transition_is_removed_and_rest_preserved(self):
        st = {**EXPECTED, "onIssueReopen": "done", "onLabelAdd": "backlog"}
        rc, out, fake = run_guard({"int1": {**BASE, "statusTransitions": st}})
        self.assertEqual(rc, 1)
        self.assertIn("HEALED", out)
        self.assertEqual(
            fake.store["int1"]["statusTransitions"],
            {**EXPECTED, "onLabelAdd": "backlog"},
        )

    def test_drifted_expected_value_is_repaired(self):
        st = {**EXPECTED, "onPRMerge": "done"}
        rc, out, fake = run_guard({"int1": {**BASE, "statusTransitions": st}})
        self.assertEqual(rc, 1)
        self.assertEqual(fake.store["int1"]["statusTransitions"], EXPECTED)

    def test_conforming_config_is_never_written(self):
        store = {"int1": {**BASE, "statusTransitions": dict(EXPECTED)}}
        rc, out, fake = run_guard(store)
        self.assertEqual(rc, 0)
        self.assertEqual(fake.updates, 0)
        self.assertEqual(fake.store, store)

    def test_unpersisted_heal_is_an_error_not_a_healed_line(self):
        # The pre-#10528 defect class: the write leaves a forbidden
        # transition in place. Verification must catch it.
        st = {**EXPECTED, "onIssueReopen": "done"}
        rc, out, fake = run_guard(
            {"int1": {**BASE, "statusTransitions": st}}, apply_updates=False
        )
        self.assertEqual(rc, 2)
        self.assertIn("GUARD ERROR", out)
        self.assertNotIn("HEALED", out)


if __name__ == "__main__":
    unittest.main(verbosity=2)
