"""Contract tests for flow_report.py.

Oracle: hand-counted fixtures. Each fixture is small enough to count by
eye, and every expected number below is that count, not a value copied
from the tool's own output. The mainline test builds a real git history
(two merged branches and one direct commit) so the first-parent,
diff-against-first-parent reading is exercised against git itself.

Positive controls: every section asserts a non-zero count first, so a
fixture that silently matched nothing fails instead of passing 0 == 0.
"""

from __future__ import annotations

import datetime as dt
import io
import json
import subprocess
import sys
import tempfile
import unittest
from contextlib import redirect_stderr, redirect_stdout
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

import flow_report as fr

REPO = "developing-today/code"
NOW = dt.datetime(2026, 11, 21, 8, 0, tzinfo=dt.timezone.utc)


def issue(number, title, state, labels, created, closed=None, comments=0):
    return {
        "n": number,
        "t": title,
        "s": state,
        "sr": "completed" if state == "closed" else None,
        "pr": False,
        "merged": None,
        "c": created,
        "x": closed,
        "u": closed or created,
        "l": labels,
        "a": "dezren39",
        "cm": comments,
    }


def pull(number, title, created, merged=None, closed=None):
    return {
        "n": number,
        "t": title,
        "s": "closed" if (merged or closed) else "open",
        "sr": None,
        "pr": True,
        "merged": merged,
        "c": created,
        "x": merged or closed,
        "u": merged or closed or created,
        "l": [],
        "a": "dezren39",
        "cm": 0,
    }


FIXTURE = [
    # effort/l: 3 issues, 1 closed -> 33.3% closed, 2 open.
    issue(
        1,
        "id: big feature",
        "open",
        ["effort/l", "app/id", "status/blocked", "user/escalated"],
        "2026-11-02T00:00:00Z",
        comments=25,
    ),
    issue(
        2,
        "id: another big feature",
        "open",
        ["effort/l", "epic/id-program", "kind/design"],
        "2026-11-03T00:00:00Z",
        comments=3,
    ),
    issue(
        3,
        "fleet: big thing",
        "closed",
        ["effort/l", "source/human"],
        "2026-11-03T00:00:00Z",
        "2026-11-10T00:00:00Z",
    ),
    # effort/xs: 2 issues, both closed -> 100%.
    issue(
        4,
        "fix(id): small",
        "closed",
        ["effort/xs", "app/id", "source/creator"],
        "2026-11-09T00:00:00Z",
        "2026-11-09T05:00:00Z",
    ),
    issue(
        5,
        "fix(mcpx): small",
        "closed",
        ["effort/xs", "source/creator"],
        "2026-11-09T00:00:00Z",
        "2026-11-10T05:00:00Z",
    ),
    # main-broken: two for id, one for mcpx, one in the other title shape.
    issue(
        6,
        "main-broken: developing-today/code id @abc1234 — basic: test",
        "closed",
        ["main-broken"],
        "2026-11-10T00:00:00Z",
        "2026-11-10T02:00:00Z",
    ),
    issue(
        7,
        "main-broken: developing-today/code id nightly @abc1235 — full",
        "closed",
        ["main-broken"],
        "2026-11-11T00:00:00Z",
        "2026-11-11T02:00:00Z",
    ),
    issue(
        8,
        "main-broken: developing-today/code mcpx @abc1236 — basic: lint",
        "closed",
        ["main-broken"],
        "2026-11-11T00:00:00Z",
        "2026-11-11T02:00:00Z",
    ),
    issue(
        9,
        "main CI failure: scripts-test (run 1)",
        "closed",
        [],
        "2026-11-12T00:00:00Z",
        "2026-11-12T02:00:00Z",
    ),
    # One copy of this repo's own issue, filed back into this repo.
    issue(
        10,
        f"[{REPO}#2] id: another big feature",
        "closed",
        ["source/universe"],
        "2026-11-17T00:00:00Z",
        "2026-11-17T03:00:00Z",
    ),
    # A parked open issue that is not an id issue.
    issue(
        11, "pm-stack: parked thing", "open", ["status/parked"], "2026-11-12T00:00:00Z"
    ),
    # PRs: one merged in 5 minutes, one in 100 minutes, one closed unmerged.
    pull(20, "fix(id): quick", "2026-11-09T00:00:00Z", merged="2026-11-09T00:05:00Z"),
    pull(21, "feat(id): slow", "2026-11-09T00:00:00Z", merged="2026-11-09T01:40:00Z"),
    pull(22, "chore: dropped", "2026-11-09T00:00:00Z", closed="2026-11-09T02:00:00Z"),
]


class IssueSectionTest(unittest.TestCase):
    def setUp(self):
        self.report = fr.issue_section(FIXTURE, REPO, NOW)

    def test_totals_count_issues_and_prs_separately(self):
        self.assertEqual(self.report["issues"], 11)
        self.assertEqual(self.report["issues_open"], 3)
        self.assertEqual(self.report["prs"], 3)
        self.assertEqual(self.report["prs_merged"], 2)
        self.assertEqual(self.report["prs_closed_unmerged"], 1)

    def test_effort_closure_rate_is_closed_over_labelled(self):
        large = self.report["effort"]["effort/l"]
        self.assertEqual((large["issues"], large["open"]), (3, 2))
        self.assertEqual(large["closed_pct"], 33.3)
        self.assertEqual(self.report["effort"]["effort/xs"]["closed_pct"], 100.0)
        # An effort label nobody used reports no rate, never 0% or 100%.
        self.assertIsNone(self.report["effort"]["effort/m"]["closed_pct"])

    def test_stalled_counts_blocked_parked_and_waiting_open_issues(self):
        # Open: #1 (blocked), #2 (none), #11 (parked) -> 2 of 3.
        self.assertEqual(self.report["open_stalled"], 2)
        self.assertEqual(self.report["open_stalled_pct"], 66.7)
        self.assertEqual(self.report["open_user_escalated"], 1)
        self.assertEqual(self.report["open_design"], 1)

    def test_id_issues_are_found_by_label_not_by_title(self):
        # #1 (app/id) and #2 (epic/id-program) are open id issues; #11 is not.
        self.assertEqual(self.report["id_open"], 2)
        self.assertEqual(self.report["id_open_stalled"], 1)

    def test_main_broken_names_the_component_from_the_title(self):
        self.assertEqual(self.report["main_broken"], 4)
        self.assertEqual(
            self.report["main_broken_components"],
            {"id": 2, "mcpx": 1, "(other title shape)": 1},
        )

    def test_id_main_broken_in_the_last_week_uses_the_report_time(self):
        # At NOW (21 November) no id breakage is under 7 days old: #6 and
        # #7 were filed on the 10th and 11th.
        self.assertEqual(self.report["main_broken_id_last_7_days"], 0)
        # Seen from the 12th, both are inside the window; the mcpx one
        # filed on the 11th is not an id breakage and is not counted.
        earlier = fr.issue_section(
            FIXTURE, REPO, dt.datetime(2026, 11, 12, tzinfo=dt.timezone.utc)
        )
        self.assertEqual(earlier["main_broken_id_last_7_days"], 2)

    def test_self_mirror_counts_only_copies_of_this_repo(self):
        self.assertEqual(self.report["self_mirror_issues"], 1)
        other = fr.issue_section(FIXTURE, "someone/else", NOW)
        self.assertEqual(other["self_mirror_issues"], 0)

    def test_origin_separates_human_fleet_and_unlabelled(self):
        self.assertEqual(
            self.report["issue_origin"],
            {
                "no source label": 7,
                "fleet-generated (any other source/*)": 3,
                "source/human": 1,
            },
        )

    def test_pr_merge_speed(self):
        # 5 and 100 minutes: median 52.5 rounds to 52; one of two under 10.
        self.assertEqual(self.report["pr_minutes_to_merge_median"], 52)
        self.assertEqual(self.report["pr_merged_within_10_min_pct"], 50.0)

    def test_weekly_flow_buckets_by_iso_week(self):
        week = self.report["weekly"]["2026-W46"]  # 9-15 November 2026
        # Opened that week: #4 #5 #6 #7 #8 #9 #11 = 7. Closed that week:
        # #3 #4 #5 #6 #7 #8 #9 = 7. Both PR merges landed that week.
        self.assertEqual(
            week, {"issues_opened": 7, "issues_closed": 7, "prs_merged": 2}
        )


class AreaTest(unittest.TestCase):
    def test_first_matching_prefix_wins(self):
        self.assertEqual(fr.area_of("pkgs/id/crates/id-core/src/lib.rs"), "id")
        self.assertEqual(fr.area_of("pkgs/identity/src/main.rs"), "other")
        self.assertEqual(
            fr.area_of("pm-stack/aggregator/src/render.ts"), "fleet-tooling"
        )
        self.assertEqual(fr.area_of("doc/agents/owner-preferences.md"), "fleet-tooling")
        self.assertEqual(fr.area_of("doc/2026-11-01_design_x/x.md"), "docs-planning")
        self.assertEqual(fr.area_of(".github/workflows/id-test.yml"), "ci")
        self.assertEqual(fr.area_of("justfile"), "other")


def git(cwd, *args):
    subprocess.run(
        ["git", "-c", "user.name=t", "-c", "user.email=t@example.invalid", *args],
        cwd=cwd,
        check=True,
        capture_output=True,
    )


class MainlineTest(unittest.TestCase):
    def test_merges_are_read_against_their_first_parent(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            git(root, "init", "-q", "-b", "main")
            (root / "README.md").write_text("x\n")
            git(root, "add", "-A")
            git(root, "commit", "-q", "-m", "chore: root")

            # Branch 1: three id files and one doc -> an id commit.
            git(root, "checkout", "-q", "-b", "one")
            for name in ("a.rs", "b.rs", "c.rs"):
                target = root / "pkgs/id/src" / name
                target.parent.mkdir(parents=True, exist_ok=True)
                target.write_text("x\n")
            (root / "doc").mkdir()
            (root / "doc/note.md").write_text("x\n")
            git(root, "add", "-A")
            git(root, "commit", "-q", "-m", "wip")
            git(root, "checkout", "-q", "main")
            git(
                root,
                "merge",
                "-q",
                "--no-ff",
                "one",
                "-m",
                "Merge pull request #20 from o/one",
            )

            # Branch 2: fleet tooling only.
            git(root, "checkout", "-q", "-b", "two")
            (root / "pm-stack").mkdir()
            (root / "pm-stack/x.ts").write_text("x\n")
            git(root, "add", "-A")
            git(root, "commit", "-q", "-m", "wip")
            git(root, "checkout", "-q", "main")
            git(
                root,
                "merge",
                "-q",
                "--no-ff",
                "two",
                "-m",
                "Merge pull request #30 from o/two",
            )

            # A direct squash-style commit touching id, typed in its subject.
            (root / "pkgs/id/src/d.rs").write_text("x\n")
            git(root, "add", "-A")
            git(root, "commit", "-q", "-m", "feat(id): direct (#21)")

            commits = fr.read_mainline(root, "main", None)
            # Positive control: the branch-side "wip" commits are not mainline.
            self.assertEqual(len(commits), 4)
            self.assertNotIn("wip", [commit["subject"] for commit in commits])

            titles = {20: "fix(id): quick", 21: "feat(id): slow"}
            section = fr.mainline_section(commits, titles)
            self.assertEqual(section["commits"], 4)
            self.assertEqual(section["primary_area"]["id"]["commits"], 2)
            self.assertEqual(section["primary_area"]["fleet-tooling"]["commits"], 1)
            self.assertEqual(section["primary_area"]["other"]["commits"], 1)
            self.assertEqual(section["id_touching"], 2)
            # PR titles, not merge subjects, give the type.
            self.assertEqual(section["id_touching_types"], {"fix": 1, "feat": 1})
            self.assertEqual(section["id_fix_pct"], 50.0)

    def test_a_failing_git_call_is_an_error_not_an_empty_section(self):
        with tempfile.TemporaryDirectory() as tmp, self.assertRaises(fr.ToolError):
            fr.read_mainline(Path(tmp), "main", None)


class CommandLineTest(unittest.TestCase):
    def run_main(self, argv):
        out, err = io.StringIO(), io.StringIO()
        with redirect_stdout(out), redirect_stderr(err):
            try:
                code = fr.main(argv)
            except SystemExit as stop:
                code = stop.code
        return code, out.getvalue(), err.getvalue()

    def write_fixture(self, directory):
        path = Path(directory) / "issues.jsonl"
        path.write_text("".join(json.dumps(row) + "\n" for row in FIXTURE))
        return path

    def test_unknown_flag_is_a_usage_error(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self.write_fixture(tmp)
            code, out, err = self.run_main(
                ["report", "--repo", REPO, "--issues", str(path), "--bogus"]
            )
        self.assertEqual(code, 2)
        self.assertEqual(out, "")
        self.assertIn("--bogus", err)

    def test_abbreviated_flag_is_refused(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self.write_fixture(tmp)
            code, _, _ = self.run_main(["report", "--repo", REPO, "--iss", str(path)])
        self.assertEqual(code, 2)

    def test_missing_input_exits_3_without_a_scoreboard(self):
        code, out, err = self.run_main(
            ["report", "--repo", REPO, "--issues", "/nonexistent/issues.jsonl"]
        )
        self.assertEqual(code, 3)
        self.assertEqual(out, "")
        self.assertIn("cannot read", err)

    def test_empty_input_exits_3_instead_of_printing_zeros(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = Path(tmp) / "empty.jsonl"
            path.write_text("\n")
            code, out, _ = self.run_main(
                ["report", "--repo", REPO, "--issues", str(path)]
            )
        self.assertEqual(code, 3)
        self.assertEqual(out, "")

    def test_markdown_says_when_git_was_not_measured(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self.write_fixture(tmp)
            code, out, _ = self.run_main(
                [
                    "report",
                    "--repo",
                    REPO,
                    "--issues",
                    str(path),
                    "--now",
                    "2026-11-21T08:00:00Z",
                ]
            )
        self.assertEqual(code, 0)
        self.assertIn("Not measured: no `--git-dir` given.", out)
        self.assertIn("| `effort/l` | 3 | 2 | 33.3% |", out)

    def test_equals_form_matches_space_form(self):
        with tempfile.TemporaryDirectory() as tmp:
            path = self.write_fixture(tmp)
            spaced = self.run_main(
                [
                    "report",
                    "--repo",
                    REPO,
                    "--issues",
                    str(path),
                    "--now",
                    "2026-11-21T08:00:00Z",
                    "--format",
                    "json",
                ]
            )
            joined = self.run_main(
                [
                    "report",
                    f"--repo={REPO}",
                    f"--issues={path}",
                    "--now=2026-11-21T08:00:00Z",
                    "--format=json",
                ]
            )
        self.assertEqual(spaced[0], 0)
        self.assertEqual(spaced[1], joined[1])


if __name__ == "__main__":
    unittest.main()
