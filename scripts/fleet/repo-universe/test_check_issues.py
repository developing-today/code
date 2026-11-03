#!/usr/bin/env python3
"""Tests for the repo-universe issue checker (#1651).

All tests run offline against fixtures: the fetch callable is
injected, so no live GitHub access (and no GitHub mutation of any
kind) is needed. The hatch-personal lifecycle test replays, from
fixtures, the exact lifecycle a live probe would drive against the
hatch-personal test issue — created open, edited, then closed — so
the delta semantics are pinned without requiring a live mutation in
routine tests.

Covers the packet's named tests:
- initial baseline (counts only, history not listed as new);
- unchanged second run (empty delta, no state churn beyond last_run);
- new / changed / closed (and reopened) transitions;
- pull-request filtering;
- failed-fetch reporting (explicit ERROR, prior state preserved).
"""

import importlib.util
import json
import sys
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE / "check_issues.py"

spec = importlib.util.spec_from_file_location("check_issues", CHECK)
check_issues = importlib.util.module_from_spec(spec)
sys.modules["check_issues"] = check_issues
spec.loader.exec_module(check_issues)

INVENTORY = """\
version: 1
overrides:
  dezren39/hatch-personal:
    watch: issues
repos:
  - name: dezren39/hatch-personal
    url: https://github.com/dezren39/hatch-personal
    class: owned
    watch: issues
    sources: [github, census]
  - name: someone/upstream
    url: https://github.com/someone/upstream
    class: upstream-dependency
    watch: inventory-only
    sources: [census]
  - name: developing-today/code
    url: https://github.com/developing-today/code
    class: owned
    watch: issues
    sources: [github, census]
"""

NOW1 = "2026-11-01T00:00:00Z"
NOW2 = "2026-11-01T01:00:00Z"
NOW3 = "2026-11-01T02:00:00Z"


def issue(number, title="t", state="open", updated="2026-10-31T00:00:00Z",
          repo="dezren39/hatch-personal", pr=False):
    raw = {"number": number, "state": state, "title": title,
           "updated_at": updated,
           "html_url": f"https://github.com/{repo}/issues/{number}"}
    if pr:
        raw["pull_request"] = {"url": "https://api.github.com/x"}
    return raw


def fetch_from(mapping):
    def fetch(repo):
        entry = mapping[repo]
        if isinstance(entry, Exception):
            raise entry
        return [check_issues.normalize_issue(i) for i in entry
                if check_issues.normalize_issue(i) is not None]
    return fetch


class InventoryTests(unittest.TestCase):
    def test_only_watch_issues_repos_are_checked(self):
        self.assertEqual(check_issues.parse_inventory(INVENTORY),
                         ["dezren39/hatch-personal", "developing-today/code"])


class BaselineTests(unittest.TestCase):
    def test_first_run_baselines_with_counts_only(self):
        fixture = {
            "dezren39/hatch-personal": [issue(1), issue(2, state="closed")],
            "developing-today/code": [issue(9)],
        }
        state, lines, had_error = check_issues.run_check(
            INVENTORY, {"version": 1, "repos": {}}, fetch_from(fixture), NOW1)
        self.assertFalse(had_error)
        report = "\n".join(lines)
        self.assertIn("baseline: 2 issues recorded (1 open, 1 closed)", report)
        self.assertNotIn("- new:", report)
        self.assertEqual(
            state["repos"]["dezren39/hatch-personal"]["issues"]["1"]["title"], "t")

    def test_second_identical_run_is_empty_and_churn_free(self):
        fixture = {
            "dezren39/hatch-personal": [issue(1)],
            "developing-today/code": [issue(9)],
        }
        state1, _, _ = check_issues.run_check(
            INVENTORY, {"version": 1, "repos": {}}, fetch_from(fixture), NOW1)
        state2, lines, _ = check_issues.run_check(
            INVENTORY, state1, fetch_from(fixture), NOW2)
        self.assertIn("issues changed: 0", "\n".join(lines))
        self.assertEqual(state1["repos"], state2["repos"])
        self.assertEqual(state2["last_run"], NOW2)


class DeltaTests(unittest.TestCase):
    def test_new_changed_closed_reopened(self):
        base = {
            "dezren39/hatch-personal": [
                issue(1, title="stays"), issue(2, title="will change"),
                issue(3, title="will close"), issue(4, title="was closed",
                                                state="closed")],
            "developing-today/code": [],
        }
        state1, _, _ = check_issues.run_check(
            INVENTORY, {"version": 1, "repos": {}}, fetch_from(base), NOW1)
        nxt = {
            "dezren39/hatch-personal": [
                issue(1, title="stays"),
                issue(2, title="changed title", updated="2026-11-01T00:30:00Z"),
                issue(3, title="will close", state="closed",
                      updated="2026-11-01T00:30:00Z"),
                issue(4, title="was closed", state="open",
                      updated="2026-11-01T00:30:00Z"),
                issue(5, title="brand new")],
            "developing-today/code": [],
        }
        _, lines, _ = check_issues.run_check(
            INVENTORY, state1, fetch_from(nxt), NOW2)
        report = "\n".join(lines)
        self.assertIn("- new: #5 brand new", report)
        self.assertIn("- changed: #2 changed title", report)
        self.assertIn("- closed: #3 will close", report)
        self.assertIn("- reopened: #4 was closed", report)
        self.assertNotIn("#1", report.split("## dezren39/hatch-personal")[1]
                         .split("## developing-today/code")[0].replace("issues changed", ""))

    def test_pull_requests_are_filtered(self):
        fixture = {
            "dezren39/hatch-personal": [issue(1), issue(2, pr=True)],
            "developing-today/code": [],
        }
        state, _, _ = check_issues.run_check(
            INVENTORY, {"version": 1, "repos": {}}, fetch_from(fixture), NOW1)
        self.assertEqual(list(state["repos"]["dezren39/hatch-personal"]["issues"]),
                         ["1"])

    def test_failed_fetch_is_explicit_and_preserves_state(self):
        good = {
            "dezren39/hatch-personal": [issue(1)],
            "developing-today/code": [issue(9)],
        }
        state1, _, _ = check_issues.run_check(
            INVENTORY, {"version": 1, "repos": {}}, fetch_from(good), NOW1)
        failing = {
            "dezren39/hatch-personal": check_issues.FetchError("HTTP 503"),
            "developing-today/code": [issue(9), issue(10, title="new in code")],
        }
        state2, lines, had_error = check_issues.run_check(
            INVENTORY, state1, fetch_from(failing), NOW2)
        self.assertTrue(had_error)
        self.assertIn("- ERROR: fetch failed: HTTP 503", "\n".join(lines))
        # Prior state for the failed repo is untouched — never an
        # apparent empty repository.
        self.assertEqual(state2["repos"]["dezren39/hatch-personal"],
                         state1["repos"]["dezren39/hatch-personal"])
        # The healthy repo still advanced.
        self.assertIn("10", state2["repos"]["developing-today/code"]["issues"])


class MainTests(unittest.TestCase):
    def test_cli_fixture_round_trip_and_exit_codes(self):
        import tempfile
        with tempfile.TemporaryDirectory() as td:
            td = Path(td)
            inventory = td / "inv.yaml"
            inventory.write_text(INVENTORY)
            fixture = td / "fixture.json"
            fixture.write_text(json.dumps({
                "dezren39/hatch-personal": [issue(1)],
                "developing-today/code": []}))
            argv = ["--inventory", str(inventory), "--state-dir", str(td / "state"),
                    "--fixture", str(fixture), "--now", NOW1]
            self.assertEqual(check_issues.main(argv), 0)
            self.assertEqual(check_issues.main(argv), 0)  # identical: still 0
            fixture.write_text(json.dumps({
                "dezren39/hatch-personal": {"error": "boom"},
                "developing-today/code": []}))
            self.assertEqual(check_issues.main(argv), 1)  # failed fetch: exit 1
            reports = sorted((td / "state").glob("report-*.md"))
            self.assertTrue(reports)
            self.assertTrue(any("ERROR" in r.read_text() for r in reports))


class HatchPersonalLifecycleTests(unittest.TestCase):
    """The test-issue lifecycle, replayed from fixtures (no live writes).

    A live probe would create a test issue in dezren39/hatch-personal,
    edit its title, then close it, checking the delta after each step.
    Routine tests must not mutate GitHub, so the same three upstream
    snapshots are replayed here; the semantics pinned are identical.
    """

    def test_create_edit_close_lifecycle(self):
        repo = "dezren39/hatch-personal"
        empty = {repo: [], "developing-today/code": []}
        state, _, _ = check_issues.run_check(
            INVENTORY, {"version": 1, "repos": {}}, fetch_from(empty), NOW1)

        created = {repo: [issue(42, title="checker lifecycle probe")],
                   "developing-today/code": []}
        state, lines, _ = check_issues.run_check(
            INVENTORY, state, fetch_from(created), NOW2)
        self.assertIn("- new: #42 checker lifecycle probe", "\n".join(lines))

        edited = {repo: [issue(42, title="checker lifecycle probe (edited)",
                               updated="2026-11-01T01:30:00Z")],
                  "developing-today/code": []}
        state, lines, _ = check_issues.run_check(
            INVENTORY, state, fetch_from(edited), NOW3)
        self.assertIn("- changed: #42", "\n".join(lines))

        closed = {repo: [issue(42, title="checker lifecycle probe (edited)",
                               state="closed", updated="2026-11-01T01:45:00Z")],
                  "developing-today/code": []}
        _, lines, _ = check_issues.run_check(
            INVENTORY, state, fetch_from(closed), "2026-11-01T03:00:00Z")
        self.assertIn("- closed: #42", "\n".join(lines))


if __name__ == "__main__":
    unittest.main()
