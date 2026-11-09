"""Contract tests for the program-tending runner (issue #1352, slice 2/3).

Slice 1 was the read-only scaffold; slice 2 implements the epics and
roadmaps sections for real. The contracts:

- the run log always covers all five sections with the hard-stop labels
  named, and the runner never fails on a per-repo fetch error;
- dry runs are read-only (no POST, no file edits) and record their
  proposed actions in the log;
- epics: silent 7+ days -> one status comment, all children closed ->
  one completion comment, at most one comment per epic per run, and the
  per-section action cap holds;
- roadmaps: forward-looking drift against landed work -> fix action,
  judgment calls -> meta/roadmap-drift flag, Done rows are never parsed
  (so they can never be rewritten), and the per-section cap holds;
- both sections stay inside the ~12 actions-per-run budget.
"""

import datetime as dt
import io
import json
import os
import subprocess
import sys
import unittest
from contextlib import redirect_stdout
from pathlib import Path
from unittest import mock

SCRIPT_DIR = Path(__file__).resolve().parents[1]
sys.path.insert(0, str(SCRIPT_DIR))

import program_tending as pt

NOW = dt.datetime(2026, 11, 9, 12, tzinfo=dt.timezone.utc)


def epic(number, days_ago, children=None, labels=(), title="Epic"):
    issue = {
        "number": number,
        "title": title,
        "updated_at": (NOW - dt.timedelta(days=days_ago)).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        ),
        "labels": [{"name": name} for name in labels],
    }
    if children is not None:
        issue["sub_issues_summary"] = {
            "total": children[0],
            "completed": children[1],
        }
    return issue


class ProgramTendingScaffoldTest(unittest.TestCase):
    def test_run_log_covers_all_five_sections(self):
        with (
            mock.patch.object(pt, "_gh_api", return_value=[]),
            mock.patch.object(pt, "_gh_api_one", return_value={}),
        ):
            log = pt.run(["developing-today/code"])
        self.assertEqual(
            [s["section"] for s in log["sections"]],
            ["epics", "roadmaps", "projects", "stalled-prs", "loose-ends"],
        )

    def test_dry_run_is_read_only(self):
        posted = []
        with (
            mock.patch.object(pt, "_gh_api", return_value=[{"labels": []}]),
            mock.patch.object(pt, "_gh_api_one", return_value={}),
            mock.patch.object(
                pt, "_gh_post", side_effect=lambda *a: posted.append(a)
            ),
        ):
            log = pt.run(["developing-today/code"])
        self.assertEqual(log["actions_taken"], 0)
        self.assertTrue(all(s["status"] == "no-action" for s in log["sections"]))
        self.assertEqual(posted, [])

    def test_hard_stops_named_in_log(self):
        with (
            mock.patch.object(pt, "_gh_api", return_value=[]),
            mock.patch.object(pt, "_gh_api_one", return_value={}),
        ):
            log = pt.run(["developing-today/code"])
        for stop in ("held", "user/owned", "do-not-merge", "do-not-auto-edit"):
            self.assertIn(stop, log["hard_stops"])

    def test_fetch_failure_reports_never_fails(self):
        with mock.patch.object(pt, "_gh_api", side_effect=RuntimeError("offline")):
            section = pt.stub_stalled_prs(["developing-today/code"])
        self.assertEqual(section["status"], "no-action")
        self.assertIn("error", section["checked"][0])

    def test_epic_listing_failure_reports_never_fails(self):
        with mock.patch.object(pt, "_gh_api", side_effect=RuntimeError("offline")):
            section = pt.tend_epics(["developing-today/code"], now=NOW)
        self.assertEqual(section["status"], "no-action")
        self.assertEqual(section["actions"], [])
        self.assertEqual(section["fetch_errors"], 1)
        self.assertIn("unverified", section["detail"])

    def test_cli_run_prints_valid_log(self):
        buf = io.StringIO()
        with (
            mock.patch.object(pt, "_gh_api", return_value=[]),
            mock.patch.object(pt, "_gh_api_one", return_value={}),
            redirect_stdout(buf),
        ):
            self.assertEqual(
                pt.main(["--run", "--repo", "developing-today/code"]), 0
            )
        log = json.loads(buf.getvalue())
        self.assertEqual(log["job"], "program-tending")
        self.assertEqual(len(log["sections"]), 5)

    def test_cli_without_run_flag_exits_2(self):
        self.assertEqual(pt.main([]), 2)

    def test_cli_apply_without_token_exits_2(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            self.assertEqual(pt.main(["--apply"]), 2)

    def test_workflow_file_exists_and_is_scheduled(self):
        wf = (
            Path(__file__).resolve().parents[2]
            / ".github/workflows/program-tending.yml"
        )
        text = wf.read_text()
        self.assertIn("cron:", text)
        self.assertIn("workflow_dispatch", text)
        self.assertIn("scripts/program_tending.py", text)

    def test_workflow_grants_drift_writes(self):
        # Slice 2 needs these to post epic comments, apply the drift
        # label and open roadmap-drift PRs; the schedule stays slice 1's.
        wf = (
            Path(__file__).resolve().parents[2]
            / ".github/workflows/program-tending.yml"
        )
        text = wf.read_text()
        self.assertIn("0 13 * * *", text)
        self.assertIn("issues: write", text)
        self.assertIn("pull-requests: write", text)

    def test_script_runs_isolated(self):
        bootstrap = "\n".join(
            [
                "import runpy",
                "import sys",
                "from unittest import mock",
                "response = mock.MagicMock()",
                "response.__enter__.return_value = response",
                "response.read.return_value = b'[]'",
                "response.headers.get.return_value = ''",
                "sys.argv = ['program_tending.py', '--run', '--repo', 'developing-today/code']",
                "with mock.patch('urllib.request.urlopen', return_value=response):",
                f"    runpy.run_path({str(SCRIPT_DIR / 'program_tending.py')!r}, run_name='__main__')",
            ]
        )
        proc = subprocess.run(
            [sys.executable, "-I", "-c", bootstrap],
            capture_output=True,
            text=True,
            timeout=120,
            cwd=str(SCRIPT_DIR.parent),
        )
        self.assertEqual(proc.returncode, 0, proc.stderr[-2000:])
        log = json.loads(proc.stdout)
        self.assertEqual(len(log["sections"]), 5)


class EpicsSectionTest(unittest.TestCase):
    def tend(self, issues, **kwargs):
        kwargs.setdefault("now", NOW)
        kwargs.setdefault("dry_run", True)
        return pt.tend_epics(
            ["developing-today/code"],
            get=lambda path: issues,
            **kwargs,
        )

    def test_silent_epic_gets_one_status_comment(self):
        section = self.tend([epic(10, days_ago=9, children=(4, 1))])
        self.assertEqual(section["status"], "action")
        self.assertEqual(len(section["actions"]), 1)
        action = section["actions"][0]
        self.assertEqual(action["kind"], "epic-status")
        self.assertEqual(action["issue"], 10)
        self.assertTrue(action["dry_run"])
        # The comment names last movement, next step and blockers.
        self.assertIn("silent for 9d", action["comment"])
        self.assertIn("Next step", action["comment"])
        self.assertIn("Blockers", action["comment"])

    def test_finished_epic_gets_completion_comment(self):
        section = self.tend([epic(11, days_ago=1, children=(3, 3))])
        self.assertEqual(len(section["actions"]), 1)
        action = section["actions"][0]
        self.assertEqual(action["kind"], "epic-completion")
        self.assertIn("Proposing closure", action["comment"])

    def test_recent_epic_is_no_action(self):
        section = self.tend([epic(12, days_ago=2, children=(4, 1))])
        self.assertEqual(section["status"], "no-action")
        self.assertEqual(section["actions"], [])

    def test_hard_stop_epic_is_skipped(self):
        section = self.tend(
            [epic(13, days_ago=30, children=(4, 0), labels=("held", "epic"))]
        )
        self.assertEqual(section["actions"], [])

    def test_pull_requests_never_count_as_epics(self):
        pr = epic(14, days_ago=30)
        pr["pull_request"] = {"url": "x"}
        section = self.tend([pr])
        self.assertEqual(section["actions"], [])

    def test_no_epic_gets_two_comments_per_run(self):
        dupes = [epic(15, days_ago=9, children=(2, 0)), epic(15, days_ago=9)]
        section = self.tend(dupes)
        self.assertEqual(len(section["actions"]), 1)

    def test_section_action_cap(self):
        issues = [epic(100 + n, days_ago=30, children=(2, 0)) for n in range(9)]
        section = self.tend(issues)
        self.assertEqual(len(section["actions"]), pt.MAX_ACTIONS_PER_SECTION)
        self.assertEqual(
            section["dropped_due_to_cap"], 9 - pt.MAX_ACTIONS_PER_SECTION
        )

    def test_completions_come_before_silence_pings(self):
        issues = [
            epic(20, days_ago=60, children=(2, 0)),
            epic(21, days_ago=1, children=(2, 2)),
        ]
        section = self.tend(issues)
        self.assertEqual(section["actions"][0]["kind"], "epic-completion")

    def test_apply_posts_exactly_one_comment_per_epic(self):
        posted = []

        def fake_post(path, payload):
            posted.append((path, payload))
            return {"ok": True, "html_url": "https://example/x"}

        section = self.tend(
            [epic(30, days_ago=9, children=(2, 0))],
            dry_run=False,
            post=fake_post,
        )
        self.assertEqual(len(posted), 1)
        path, payload = posted[0]
        self.assertIn("/repos/developing-today/code/issues/30/comments", path)
        self.assertIn("Next step", payload["body"])
        self.assertTrue(section["actions"][0]["posted"])

    def test_already_noted_epics_are_not_repinged(self):
        def fake_get(path):
            if "comments" in path:
                return [
                    {
                        "body": "Program tending (#1352): silent note",
                        "created_at": NOW.strftime("%Y-%m-%dT%H:%M:%SZ"),
                    }
                ]
            return [epic(31, days_ago=9, children=(2, 0))]

        section = pt.tend_epics(["developing-today/code"], now=NOW, get=fake_get)
        self.assertEqual(section["actions"], [])
        self.assertEqual(section["already_noted"], 1)

    def test_tracking_issue_is_never_pinged(self):
        tracker = epic(pt.TRACKING_NUMBER, days_ago=60, children=(2, 0))
        section = pt.tend_epics(
            ["developing-today/code"],
            now=NOW,
            get=lambda path: [tracker],
        )
        self.assertEqual(section["actions"], [])
        decisions = {c["issue"]: c["decision"] for c in section["checked"]}
        self.assertEqual(decisions[pt.TRACKING_NUMBER], "tracking-issue")


class RoadmapsSectionTest(unittest.TestCase):
    TABLE = """\
# Roadmap: test

## Now

| Title   | Status      | Intent | Links |
| ------- | ----------- | ------ | ----- |
| Landed  | in-progress | did it | #42   |
| Dropped | proposed    | nope   | #43   |
| Mixed   | planned     | maybe  | #42, #43 |
| Open    | in-progress | doing  | #44   |
| Lonely  | proposed    | hmm    | none yet |
| Moot    | in-progress | x      | `some/path` |

## Done

| Title  | Status  | Intent | Links |
| ------ | ------- | ------ | ----- |
| Old    | shipped | done   | #42   |
"""

    def states(self, mapping):
        return lambda path: {
            "state": mapping[int(path.rsplit("/", 1)[-1])][0],
            "state_reason": mapping[int(path.rsplit("/", 1)[-1])][1],
        }

    def test_done_rows_are_never_parsed(self):
        rows = pt.parse_roadmap_rows(self.TABLE)
        self.assertNotIn("Old", [r["title"] for r in rows])
        self.assertEqual(len(rows), 6)

    def test_overview_done_is_never_parsed(self):
        text = "## Now\n\n1. thing (#42).\n\n## Done\n\n1. old (#42).\n"
        items = pt.parse_overview_lists(text)
        self.assertEqual(len(items), 1)
        self.assertEqual(items[0]["section"], "Now")

    def test_concrete_drift_becomes_pr_action(self):
        import tempfile

        get_one = self.states({42: ("closed", "completed"), 43: ("closed", "not_planned"), 44: ("open", "")})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(self.TABLE)
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        kinds = {(a["file"], a["kind"], a.get("to_status")) for a in section["actions"]}
        self.assertIn(("planning/roadmaps/t.md", "roadmap-pr", "shipped"), kinds)
        self.assertIn(("planning/roadmaps/t.md", "roadmap-pr", "dropped"), kinds)
        # Disagreeing close reasons need a human call, not a guessed PR;
        # tracker-bound ambiguity is batched into one aggregate flag.
        flags = [a for a in section["actions"] if a["kind"] == "roadmap-flag"]
        self.assertEqual(len(flags), 1)
        self.assertEqual(flags[0]["label"], pt.ROADMAP_DRIFT_LABEL)
        mixed = [d for d in flags[0]["details"] if d["row"] == "Mixed"]
        self.assertEqual(len(mixed), 1)
        self.assertIn("disagreeing", mixed[0]["uncertainty"])
        self.assertIn("disagreeing", mixed[0]["uncertainty"])

    def test_agreeing_multi_link_closures_become_one_fix(self):
        import tempfile

        get_one = lambda path: {"state": "closed", "state_reason": "completed"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(
                "## Now\n\n| Title | Status | Intent | Links |\n"
                "| --- | --- | --- | --- |\n"
                "| Pair | in-progress | did both | #42, #43 |\n"
                "\n## Done\n\nnone\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(len(section["actions"]), 1)
        action = section["actions"][0]
        self.assertEqual(action["kind"], "roadmap-pr")
        self.assertEqual(action["to_status"], "shipped")
        self.assertEqual(action["issues"], [42, 43])

    def test_foreign_refs_are_never_read_as_local_issues(self):
        rows = pt.parse_roadmap_rows(
            "## Now\n\n| Title | Status | Intent | Links |\n"
            "| --- | --- | --- | --- |\n"
            "| X | proposed | y | nix#203, nix#200 |\n"
        )
        self.assertEqual(rows[0]["issues"], [])
        items = pt.parse_overview_lists("## Now\n\n1. thing (nix#203).\n")
        self.assertEqual(items, [])

    def test_linkless_and_foreign_rows_are_unverifiable_not_drift(self):
        import tempfile

        seen = []

        def get_one(path):
            seen.append(path)
            return {"state": "closed", "state_reason": "completed"}

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(
                "## Now\n\n| Title | Status | Intent | Links |\n"
                "| --- | --- | --- | --- |\n"
                "| Foreign | proposed | y | nix#203 |\n"
                "| Lonely | proposed | hmm | none yet |\n"
                "\n## Done\n\nnone\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(section["actions"], [])
        self.assertEqual(seen, [])
        checked = next(
            c for c in section["checked"] if c.get("file", "").endswith("t.md")
        )
        self.assertEqual(checked["unverifiable"], 1)

    def test_needs_owner_rows_are_flagged_never_fixed(self):
        import tempfile

        get_one = lambda path: {"state": "closed", "state_reason": "completed"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            rel = "planning/roadmaps/t.md"
            (root / rel).write_text(
                "## Now\n\n| Title | Status | Intent | Links |\n"
                "| --- | --- | --- | --- |\n"
                "| Kept | in-progress | broader than one issue "
                "<!-- program-tending: needs-owner --> | #42 |\n"
                "\n## Done\n\nnone\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
            self.assertEqual((root / rel).read_text().count("in-progress"), 1)
        self.assertEqual(len(section["actions"]), 1)
        self.assertEqual(section["actions"][0]["kind"], "roadmap-flag")
        self.assertNotEqual(section["actions"][0]["kind"], "roadmap-pr")

    def test_tracker_flags_are_batched_into_one_comment(self):
        import tempfile

        get_one = lambda path: {"state": "closed", "state_reason": "completed"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text(
                "# Roadmap\n\n## Now\n\n1. a (#42).\n2. b (#43).\n3. c (#44).\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        flags = [a for a in section["actions"] if a["kind"] == "roadmap-flag"]
        self.assertEqual(len(flags), 1)
        self.assertEqual(len(flags[0]["details"]), 3)

    def test_already_flagged_targets_are_not_reflagged(self):
        import tempfile

        def get_one(path):
            if path.endswith("/issues/1352"):
                return {
                    "state": "open",
                    "state_reason": "",
                    "labels": [{"name": "meta/roadmap-drift"}],
                }
            return {"state": "closed", "state_reason": "completed"}

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text(
                "# Roadmap\n\n## Now, across the repo\n\n1. thing (#42).\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(section["actions"], [])
        self.assertEqual(section["checked"][0].get("already_flagged"), 1)
        import tempfile

        get_one = self.states({42: ("closed", "completed"), 44: ("open", "")})
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(
                "## Now\n\n| Title | Status | Intent | Links |\n"
                "| --- | --- | --- | --- |\n"
                "| Open | in-progress | doing | #44 |\n"
                "| Lonely | proposed | hmm | none yet |\n"
                "\n## Done\n\nnone\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(section["status"], "no-action")
        self.assertEqual(section["actions"], [])

    def test_unknown_close_reason_becomes_flag(self):
        import tempfile

        get_one = lambda path: {"state": "closed", "state_reason": ""}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(
                "## Now\n\n| Title | Status | Intent | Links |\n"
                "| --- | --- | --- | --- |\n"
                "| Landed | in-progress | did it | #42 |\n"
                "\n## Done\n\nnone\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(len(section["actions"]), 1)
        self.assertEqual(section["actions"][0]["kind"], "roadmap-flag")

    def test_rate_limited_run_never_reports_clean(self):
        # Every lookup failing must read as unverified, never as
        # "no drift": a 403 storm is not a clean bill of health.
        import tempfile

        def always_fail(path):
            raise RuntimeError("rate limited")

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text(
                "# Roadmap\n\n## Now\n\n1. thing (#42).\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=always_fail
            )
        self.assertEqual(section["status"], "no-action")
        self.assertEqual(section["actions"], [])
        self.assertIn("unverified", section["detail"])
        self.assertGreater(section["lookup_failures"], 0)

    def test_overview_drift_becomes_flag_not_pr(self):
        import tempfile

        get_one = lambda path: {"state": "closed", "state_reason": "completed"}
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text(
                "# Roadmap\n\n## Now, across the repo\n\n1. thing (#42).\n\n## Done\n\nnone\n"
            )
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        flags = [a for a in section["actions"] if a["kind"] == "roadmap-flag"]
        prs = [a for a in section["actions"] if a["kind"] == "roadmap-pr"]
        self.assertEqual(len(flags), 1)
        self.assertEqual(prs, [])
        self.assertTrue(
            any("promotes" in d["uncertainty"] for d in flags[0]["details"])
        )

    def test_apply_flips_status_cell(self):
        import tempfile

        table = (
            "## Now\n\n| Title | Status | Intent | Links |\n"
            "| --- | --- | --- | --- |\n"
            "| Landed | in-progress | did it | #42 |\n"
        )
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            rel = "planning/roadmaps/t.md"
            (root / rel).write_text(table)
            changed = pt.apply_roadmap_fix(root, rel, 5, "shipped")
            self.assertTrue(changed)
            after = (root / rel).read_text()
        self.assertIn("shipped", after)
        self.assertNotIn("in-progress", after)

    def test_apply_mode_cap_gates_file_writes(self):
        # No unlogged mutations: rows past the cap stay forward-looking
        # on disk and are counted as dropped, never silently applied.
        import tempfile

        rows = "".join(f"| R{n} | in-progress | x | #4{n} |\n" for n in range(10))
        table = (
            "## Now\n\n| Title | Status | Intent | Links |\n"
            "| --- | --- | --- | --- |\n" + rows
        )

        def get_one(path):
            return {"state": "closed", "state_reason": "completed"}

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            rel = "planning/roadmaps/t.md"
            (root / rel).write_text(table)
            section = pt.tend_roadmaps(
                ["developing-today/code"],
                root=root,
                dry_run=False,
                get_one=get_one,
                post=lambda *a: {"ok": True},
            )
            after = (root / rel).read_text()
        self.assertEqual(len(section["actions"]), pt.MAX_ACTIONS_PER_SECTION)
        self.assertTrue(all(a.get("applied") for a in section["actions"]))
        self.assertEqual(
            section["dropped_due_to_cap"], 10 - pt.MAX_ACTIONS_PER_SECTION
        )
        self.assertEqual(after.count("shipped"), pt.MAX_ACTIONS_PER_SECTION)
        self.assertEqual(after.count("in-progress"), 10 - pt.MAX_ACTIONS_PER_SECTION)

    def test_dropped_flags_are_never_posted(self):
        import tempfile

        rows = "".join(f"| R{n} | in-progress | x | #4{n} |\n" for n in range(6))
        rows += "| U1 | proposed | y | #50 |\n| U2 | proposed | y | #51 |\n"
        table = (
            "## Now\n\n| Title | Status | Intent | Links |\n"
            "| --- | --- | --- | --- |\n" + rows
        )

        def get_one(path):
            n = int(path.rsplit("/", 1)[-1])
            if n >= 50:
                return {"state": "closed", "state_reason": ""}
            return {"state": "closed", "state_reason": "completed"}

        posted = []

        def fake_post(path, payload):
            posted.append(path)
            return {"ok": True}

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(table)
            section = pt.tend_roadmaps(
                ["developing-today/code"],
                root=root,
                dry_run=False,
                get_one=get_one,
                post=fake_post,
            )
        self.assertEqual(len(section["actions"]), pt.MAX_ACTIONS_PER_SECTION)
        self.assertEqual(posted, [])
        self.assertEqual(section["dropped_due_to_cap"], 1)

    def test_roadmaps_section_action_cap(self):
        import tempfile

        rows = "".join(
            f"| R{n} | in-progress | x | #4{n} |\n" for n in range(10)
        )
        table = (
            "## Now\n\n| Title | Status | Intent | Links |\n"
            "| --- | --- | --- | --- |\n" + rows
        )

        def get_one(path):
            return {"state": "closed", "state_reason": "completed"}

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(table)
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(len(section["actions"]), pt.MAX_ACTIONS_PER_SECTION)
        self.assertIn("dropped_due_to_cap", section)
        table = (
            "## Now\n\n| Title | Status | Intent | Links |\n"
            "| --- | --- | --- | --- |\n" + rows
        )

        def get_one(path):
            return {"state": "closed", "state_reason": "completed"}

        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "planning" / "roadmaps").mkdir(parents=True)
            (root / "ROADMAP.md").write_text("# Roadmap\n")
            (root / "planning" / "roadmaps" / "t.md").write_text(table)
            section = pt.tend_roadmaps(
                ["developing-today/code"], root=root, get_one=get_one
            )
        self.assertEqual(len(section["actions"]), pt.MAX_ACTIONS_PER_SECTION)
        self.assertIn("dropped_due_to_cap", section)


class RunBudgetTest(unittest.TestCase):
    def test_run_stays_within_action_budget(self):
        issues = [epic(200 + n, days_ago=30, children=(2, 0)) for n in range(9)]
        with (
            mock.patch.object(pt, "_gh_api", return_value=issues),
            mock.patch.object(pt, "_gh_api_one", return_value={}),
        ):
            log = pt.run(["developing-today/code"])
        total = sum(len(s.get("actions", [])) for s in log["sections"])
        self.assertLessEqual(total, pt.MAX_ACTIONS_PER_RUN)
        self.assertEqual(total, log["actions_taken"])

    def test_run_cap_trims_oldest_sections_last(self):
        sections = [
            {"section": "epics", "status": "action", "actions": [{"n": n} for n in range(10)]},
            {"section": "roadmaps", "status": "action", "actions": [{"n": n} for n in range(10)]},
        ]
        dropped = pt.enforce_run_cap(sections)
        self.assertEqual(dropped, 20 - pt.MAX_ACTIONS_PER_RUN)
        self.assertEqual(len(sections[0]["actions"]), 10)
        self.assertEqual(len(sections[1]["actions"]), pt.MAX_ACTIONS_PER_RUN - 10)


class GhApiAuthHeaderTest(unittest.TestCase):
    """#6860: _gh_api sends the workflow token when one is in the
    environment, and sends no Authorization header when none is."""

    def capture_request(self):
        captured = {}

        class FakeResp:
            headers = {}

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

            def read(self):
                return b'[]'

        def fake_urlopen(req, timeout=None):
            captured["req"] = req
            return FakeResp()

        return captured, fake_urlopen

    def test_authorization_sent_from_gh_token(self):
        captured, fake_urlopen = self.capture_request()
        with mock.patch.dict(os.environ, {"GH_TOKEN": "test-token-6860"}, clear=True):
            with mock.patch.object(pt.urllib.request, "urlopen", fake_urlopen):
                self.assertEqual(pt._gh_api("/repos/x/y/issues"), [])
        self.assertEqual(
            captured["req"].get_header("Authorization"), "Bearer test-token-6860"
        )

    def test_authorization_falls_back_to_github_token(self):
        captured, fake_urlopen = self.capture_request()
        with mock.patch.dict(os.environ, {"GITHUB_TOKEN": "fallback-6860"}, clear=True):
            with mock.patch.object(pt.urllib.request, "urlopen", fake_urlopen):
                pt._gh_api("/repos/x/y/issues")
        self.assertEqual(
            captured["req"].get_header("Authorization"), "Bearer fallback-6860"
        )

    def test_no_authorization_without_token(self):
        captured, fake_urlopen = self.capture_request()
        with mock.patch.dict(os.environ, {}, clear=True):
            with mock.patch.object(pt.urllib.request, "urlopen", fake_urlopen):
                pt._gh_api("/repos/x/y/issues")
        self.assertIsNone(captured["req"].get_header("Authorization"))



if __name__ == "__main__":
    unittest.main()
