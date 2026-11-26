#!/usr/bin/env python3
"""test_closure_watchdog.py — decision + watermark tests for the
closure watchdog (code#10105).

Oracle: the watchdog reopens an automation close exactly when rule (a)
(open native sub-issues) or rule (b) (closing PR without a closing
keyword) fires, and never when the closing PR carries a closing
keyword, the closer is human, or a human closed inside the guard
window before the bot's echo close. The historical dry-run fixtures
are the #9106 timeline (bot close 2026-11-13T03:04:00Z, PR #9184
title-reference only, 6 open sub-issues — both rules) and the #9078
pattern (closed with 9 open components — rule (a)).

The gh boundary is replaced with an in-memory fake — no network.

Run:  python3 pm-stack/tests/test_closure_watchdog.py
"""

import importlib.util
import sys
import unittest
import urllib.parse
from pathlib import Path

SCRIPT_PATH = Path(__file__).resolve().parent.parent / "closure-watchdog.py"
spec = importlib.util.spec_from_file_location("closure_watchdog", SCRIPT_PATH)
wd = importlib.util.module_from_spec(spec)
spec.loader.exec_module(wd)

BOT = "developing-today[bot]"
HUMAN = "dezren39"


def close_event(actor, at):
    return {"event": "closed", "actor": {"login": actor}, "created_at": at}


def xref_event(number, title, body, merged_at):
    return {
        "event": "cross-referenced",
        "created_at": merged_at,
        "source": {
            "issue": {
                "number": number,
                "title": title,
                "body": body,
                "pull_request": {"merged_at": merged_at},
            }
        },
    }


class FakeGh:
    """In-memory stand-in for GhClient.

    issues: {repo: [issue numbers recently closed]}
    timelines / sub_issues: {(repo, number): ...}
    Records every mutating call in self.calls.
    """

    def __init__(self, issues, timelines, sub_issues):
        self._issues = issues
        self._timelines = timelines
        self._subs = sub_issues
        self.calls = []

    def recently_closed_issues(self, repo, since_iso):
        return [{"number": n} for n in self._issues.get(repo, [])]

    def timeline(self, repo, number):
        return self._timelines.get((repo, number), [])

    def open_sub_issue_count(self, repo, number):
        return self._subs.get((repo, number), 0)

    def reopen(self, repo, number):
        self.calls.append(("reopen", repo, number))

    def remove_label(self, repo, number, label):
        self.calls.append(("remove_label", repo, number, label))

    def comment(self, repo, number, body):
        self.calls.append(("comment", repo, number, body))


REPO = "developing-today/code"


class DecideCloseTests(unittest.TestCase):
    def test_closing_keyword_variants(self):
        for text in ("Closes #5", "fixes #5", "Resolved #5", "close #5"):
            self.assertTrue(wd.has_closing_keyword(text, 5), text)
        for text in ("Refs #5", "see #5", "Closes #6", "", None):
            self.assertFalse(wd.has_closing_keyword(text, 5), text)

    def test_legit_closes_keyword_close_untouched(self):
        rules, _ = wd.decide_close(
            actor=BOT,
            human_close_before=False,
            open_sub_issues=0,
            closing_pr={
                "number": 9,
                "title": "x",
                "body": "Closes #5",
                "merged_at": "2026-11-13T03:03:53Z",
            },
            issue_number=5,
        )
        self.assertEqual(rules, [])

    def test_human_close_untouched(self):
        rules, _ = wd.decide_close(
            actor=HUMAN,
            human_close_before=False,
            open_sub_issues=3,
            closing_pr=None,
            issue_number=5,
        )
        self.assertEqual(rules, [])

    def test_bot_close_no_pr_no_open_subs_untouched(self):
        rules, _ = wd.decide_close(
            actor=BOT,
            human_close_before=False,
            open_sub_issues=0,
            closing_pr=None,
            issue_number=5,
        )
        self.assertEqual(rules, [])


class ProcessRepoTests(unittest.TestCase):
    def test_9106_fixture_both_rules_fire(self):
        # PR #9184 merged 2026-11-13T03:03:53Z, title reference only;
        # bot closed #9106 at 03:04:00Z with 6 open sub-issues.
        gh = FakeGh(
            issues={REPO: [9106]},
            timelines={
                (REPO, 9106): [
                    xref_event(
                        9184,
                        "design note (#9106)",
                        "Refs #9106",
                        "2026-11-13T03:03:53Z",
                    ),
                    close_event(BOT, "2026-11-13T03:04:00Z"),
                ]
            },
            sub_issues={(REPO, 9106): 6},
        )
        actions, watermark = wd.process_repo(gh, REPO, None)
        self.assertEqual(len(actions), 1)
        self.assertEqual(
            sorted(actions[0]["rules"]), ["open-sub-issues", "refs-only-merge"]
        )
        self.assertEqual(actions[0]["closing_pr"], 9184)
        kinds = [c[0] for c in gh.calls]
        self.assertEqual(kinds, ["reopen", "remove_label", "comment"])
        self.assertIn("#9184", gh.calls[2][3])
        self.assertEqual(watermark, "2026-11-13T03:04:00Z")

    def test_9078_fixture_open_sub_issues_rule_fires(self):
        gh = FakeGh(
            issues={REPO: [9078]},
            timelines={
                (REPO, 9078): [
                    xref_event(9088, "slice", "Refs #9078", "2026-11-12T23:07:10Z"),
                    close_event(BOT, "2026-11-12T23:07:19Z"),
                ]
            },
            sub_issues={(REPO, 9078): 9},
        )
        actions, _ = wd.process_repo(gh, REPO, None)
        self.assertEqual(len(actions), 1)
        self.assertIn("open-sub-issues", actions[0]["rules"])

    def test_legit_closes_keyword_close_not_reopened(self):
        gh = FakeGh(
            issues={REPO: [10107]},
            timelines={
                (REPO, 10107): [
                    xref_event(
                        10190, "fixture", "Closes #10107", "2026-11-14T17:04:11Z"
                    ),
                    close_event(BOT, "2026-11-14T17:04:17Z"),
                ]
            },
            sub_issues={(REPO, 10107): 0},
        )
        actions, _ = wd.process_repo(gh, REPO, None)
        self.assertEqual(actions, [])
        self.assertEqual(gh.calls, [])

    def test_human_close_inside_guard_window_not_reopened(self):
        gh = FakeGh(
            issues={REPO: [42]},
            timelines={
                (REPO, 42): [
                    close_event(HUMAN, "2026-11-14T10:00:00Z"),
                    close_event(BOT, "2026-11-14T10:01:00Z"),
                ]
            },
            sub_issues={(REPO, 42): 2},
        )
        actions, _ = wd.process_repo(gh, REPO, None)
        self.assertEqual(actions, [])
        self.assertEqual(gh.calls, [])

    def test_watermark_processes_each_close_once(self):
        gh = FakeGh(
            issues={REPO: [9106]},
            timelines={
                (REPO, 9106): [
                    xref_event(
                        9184,
                        "design note (#9106)",
                        "Refs #9106",
                        "2026-11-13T03:03:53Z",
                    ),
                    close_event(BOT, "2026-11-13T03:04:00Z"),
                ]
            },
            sub_issues={(REPO, 9106): 6},
        )
        _, watermark = wd.process_repo(gh, REPO, None)
        calls_after_first = list(gh.calls)
        actions2, watermark2 = wd.process_repo(gh, REPO, watermark)
        self.assertEqual(actions2, [])
        self.assertEqual(gh.calls, calls_after_first)
        self.assertEqual(watermark2, watermark)

    def test_dry_run_reports_without_acting(self):
        gh = FakeGh(
            issues={REPO: [9106]},
            timelines={
                (REPO, 9106): [
                    close_event(BOT, "2026-11-13T03:04:00Z"),
                ]
            },
            sub_issues={(REPO, 9106): 1},
        )
        actions, _ = wd.process_repo(gh, REPO, None, dry_run=True)
        self.assertEqual(len(actions), 1)
        self.assertTrue(actions[0]["dry_run"])
        self.assertEqual(gh.calls, [])


class RecentlyClosedIssuesUrlTests(unittest.TestCase):
    def test_since_query_is_percent_encoded(self):
        """Regression (2026-11-25, #10105): the since timestamp's
        "+00:00" offset must reach GitHub percent-encoded. A raw "+"
        in the query string decodes as a space, the issues list comes
        back empty, and the watchdog silently processes nothing --
        the fake-boundary tests cannot see this, so capture the real
        GhClient path here."""
        captured = {}

        class CapturingGh(wd.GhClient):
            def api(self, path):
                captured["path"] = path
                return []

        CapturingGh().recently_closed_issues(
            "developing-today/code", "2026-11-25T10:25:59+00:00"
        )
        query = captured["path"].split("?", 1)[1]
        params = urllib.parse.parse_qs(query)
        self.assertEqual(params["since"], ["2026-11-25T10:25:59+00:00"])
        self.assertNotIn("+", query)


if __name__ == "__main__":
    unittest.main(verbosity=2)
