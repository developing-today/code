"""Contract tests for program-tending (issue #1352, slices 1+2+3 of 3).

Slice 1 contracts: the scaffold sections report verified no-actions,
the runner never fails on a per-repo fetch error, and the run log
always covers all five sections with the hard-stop labels named.
Slice 2 contracts: epics/roadmaps compute real actions in dry-run
without writing anything, and live mode posts through _gh_write only.
Slice 3 contracts: projects report unreadable board state instead of
fabricating it and correct contradictions through GraphQL only;
stalled PRs get one marker-suppressed check-in (user/owned at 7 days,
never a packet; fleet-owned also emits a pr_shepherd packet); loose
ends notify parents once, count duplicate cross-links, and surface
the aging user/review list as a planned/live tracking-issue action.
"""

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


class ProgramTendingScaffoldTest(unittest.TestCase):
    def test_run_log_covers_all_five_sections(self):
        with mock.patch.object(pt, "_gh_api", return_value=[]):
            log = pt.run(["developing-today/code"])
        self.assertEqual(
            [s["section"] for s in log["sections"]],
            ["epics", "roadmaps", "projects", "stalled-prs", "loose-ends"],
        )

    def test_scaffold_is_read_only(self):
        with mock.patch.object(pt, "_gh_api", return_value=[{"labels": []}]):
            log = pt.run(["developing-today/code"])
        self.assertEqual(log["actions_taken"], 0)
        self.assertTrue(all(s["status"] == "no-action" for s in log["sections"]))

    def test_hard_stops_named_in_log(self):
        with mock.patch.object(pt, "_gh_api", return_value=[]):
            log = pt.run(["developing-today/code"])
        for stop in ("held", "user/owned", "do-not-merge", "do-not-auto-edit"):
            self.assertIn(stop, log["hard_stops"])

    def test_fetch_failure_reports_never_fails(self):
        with mock.patch.object(pt, "_gh_api", side_effect=RuntimeError("offline")):
            section = pt.stub_stalled_prs(["developing-today/code"])
        self.assertEqual(section["status"], "no-action")
        self.assertIn("error", section["checked"][0])

    def test_cli_run_prints_valid_log(self):
        buf = io.StringIO()
        with mock.patch.object(pt, "_gh_api", return_value=[]), redirect_stdout(buf):
            self.assertEqual(pt.main(["--run", "--repo", "developing-today/code"]), 0)
        log = json.loads(buf.getvalue())
        self.assertEqual(log["job"], "program-tending")
        self.assertEqual(len(log["sections"]), 5)

    def test_cli_without_run_flag_exits_2(self):
        self.assertEqual(pt.main([]), 2)

    def test_workflow_file_exists_and_is_scheduled(self):
        wf = (
            Path(__file__).resolve().parents[2]
            / ".github/workflows/program-tending.yml"
        )
        text = wf.read_text()
        self.assertIn("cron:", text)
        self.assertIn("workflow_dispatch", text)
        self.assertIn("scripts/program_tending.py", text)

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


class FleetReposTest(unittest.TestCase):
    def test_default_fleet_repos_are_the_canonical_four(self):
        self.assertEqual(
            pt.FLEET_REPOS,
            (
                "developing-today/code",
                "developing-today-forks/opencode",
                "developing-today-forks/openchamber",
                "dezren39/darwin-nix",
            ),
        )

    def test_stub_stalled_prs_queries_every_default_repo(self):
        queried = []

        def fake_api(path):
            queried.append(path)
            return []

        with mock.patch.object(pt, "_gh_api", side_effect=fake_api):
            section = pt.stub_stalled_prs(list(pt.FLEET_REPOS))
        for repo in pt.FLEET_REPOS:
            self.assertIn(f"/repos/{repo}/pulls?state=open&per_page=100", queried)
        self.assertEqual(
            [entry["repo"] for entry in section["checked"]], list(pt.FLEET_REPOS)
        )

    def test_default_run_log_names_darwin_nix_not_the_nonexistent_repo(self):
        buf = io.StringIO()
        with mock.patch.object(pt, "_gh_api", return_value=[]), redirect_stdout(buf):
            self.assertEqual(pt.main(["--run"]), 0)
        out = buf.getvalue()
        self.assertIn("dezren39/darwin-nix", out)
        self.assertNotIn("developing-today/nix", out)


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
                return b"[]"

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


class Slice2EpicsTest(unittest.TestCase):
    """Slice 2 epics: child progress, silence check-ins, completion."""

    NOW = None  # set in setUp

    def setUp(self):
        import datetime as dt

        self.now = dt.datetime(2026, 11, 11, tzinfo=dt.timezone.utc)

    def fake_api(self, epic, children, comments=None):
        def api(path):
            if "labels=epic" in path:
                return [epic]
            if "/comments" in path:
                return comments or []
            for number, child in children.items():
                if path.endswith(f"/issues/{number}"):
                    return [child]
            return []

        return api

    def test_silent_epic_gets_planned_status_comment(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-10-20T00:00:00Z",
            "labels": [],
            "body": "- [ ] #101 first child\n- [x] #102 done child",
        }
        children = {
            101: {"number": 101, "state": "open", "title": "First", "labels": []},
            102: {"number": 102, "state": "closed", "title": "Done", "labels": []},
        }
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, children)):
            section = pt.section_epics(["developing-today/code"], False, self.now, 12)
        self.assertEqual(len(section["actions"]), 1)
        self.assertEqual(section["actions"][0]["kind"], "epic-status-comment")
        self.assertFalse(section["actions"][0]["live"])
        entry = section["checked"][0]
        self.assertEqual(
            entry["children"], {"total": 2, "open": 1, "closed": 1, "unverified": 0}
        )
        self.assertEqual(entry["body_vs_reality_mismatches"], 0)
        self.assertGreaterEqual(entry["silent_days"], 7)

    def test_body_vs_reality_mismatch_counted(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-11-10T00:00:00Z",
            "labels": [],
            "body": "- [x] #101 claimed done but open",
        }
        children = {
            101: {"number": 101, "state": "open", "title": "First", "labels": []},
        }
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, children)):
            section = pt.section_epics(["developing-today/code"], False, self.now, 12)
        self.assertEqual(section["checked"][0]["body_vs_reality_mismatches"], 1)
        self.assertEqual(section["actions"], [])

    def test_all_children_closed_proposes_completion(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-11-10T00:00:00Z",
            "labels": [],
            "body": "- [x] #102 done child",
        }
        children = {
            102: {"number": 102, "state": "closed", "title": "Done", "labels": []},
        }
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, children)):
            section = pt.section_epics(["developing-today/code"], False, self.now, 12)
        self.assertEqual(len(section["actions"]), 1)
        self.assertEqual(section["actions"][0]["kind"], "epic-completion-comment")

    def test_hard_stop_epic_skipped(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-10-01T00:00:00Z",
            "labels": [{"name": "held"}],
            "body": "- [ ] #101 first child",
        }
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, {})):
            section = pt.section_epics(["developing-today/code"], False, self.now, 12)
        self.assertEqual(section["actions"], [])
        self.assertIn("hard-stop", section["checked"][0]["disposition"])

    def test_recently_tended_epic_skipped(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-10-20T00:00:00Z",
            "labels": [],
            "body": "- [ ] #101 first child",
        }
        children = {
            101: {"number": 101, "state": "open", "title": "First", "labels": []},
        }
        comments = [
            {
                "body": "check-in <!-- program-tending:epics -->",
                "created_at": "2026-11-10T00:00:00Z",
            }
        ]
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, children, comments)):
            section = pt.section_epics(["developing-today/code"], False, self.now, 12)
        self.assertEqual(section["actions"], [])
        self.assertIn("tended within 7 days", section["checked"][0]["disposition"])

    def test_completion_proposal_suppressed_when_recently_tended(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-11-10T00:00:00Z",
            "labels": [],
            "body": "- [x] #102 done child",
        }
        children = {
            102: {"number": 102, "state": "closed", "title": "Done", "labels": []},
        }
        comments = [
            {
                "body": "completion <!-- program-tending:epics -->",
                "created_at": "2026-11-10T00:00:00Z",
            }
        ]
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, children, comments)):
            section = pt.section_epics(["developing-today/code"], False, self.now, 12)
        self.assertEqual(section["actions"], [])
        self.assertIn("tended within 7 days", section["checked"][0]["disposition"])

    def test_write_failure_isolated_per_repo(self):
        epics = {
            "aaa/one": {
                "number": 100,
                "title": "Epic A",
                "updated_at": "2026-10-20T00:00:00Z",
                "labels": [],
                "body": "- [ ] #101 first child",
            },
            "bbb/two": {
                "number": 200,
                "title": "Epic B",
                "updated_at": "2026-10-20T00:00:00Z",
                "labels": [],
                "body": "- [ ] #201 first child",
            },
        }
        kids = {
            ("aaa/one", 101): {
                "number": 101,
                "state": "open",
                "title": "First",
                "labels": [],
            },
            ("bbb/two", 201): {
                "number": 201,
                "state": "open",
                "title": "First",
                "labels": [],
            },
        }

        def api(path):
            for repo, epic in epics.items():
                if path.startswith(f"/repos/{repo}/issues?"):
                    return [epic]
            if "/comments" in path:
                return []
            for (repo, number), child in kids.items():
                if path == f"/repos/{repo}/issues/{number}":
                    return [child]
            return []

        def write(path, payload):
            if path.startswith("/repos/aaa/one/"):
                raise RuntimeError("403 forbidden")
            return {"id": 999}

        with mock.patch.object(pt, "_gh_api", api):
            with mock.patch.object(pt, "_gh_write", write):
                section = pt.section_epics(["aaa/one", "bbb/two"], True, self.now, 12)
        by_repo = {a["repo"]: a for a in section["actions"]}
        self.assertIn("write failed", by_repo["aaa/one"]["error"])
        self.assertEqual(by_repo["bbb/two"]["comment_id"], 999)
        dispositions = {c["repo"]: c["disposition"] for c in section["checked"]}
        self.assertIn("comment failed", dispositions["aaa/one"])
        self.assertIn("posted", dispositions["bbb/two"])

    def test_live_epic_comment_posts(self):
        epic = {
            "number": 100,
            "title": "Epic A",
            "updated_at": "2026-10-20T00:00:00Z",
            "labels": [],
            "body": "- [ ] #101 first child",
        }
        children = {
            101: {"number": 101, "state": "open", "title": "First", "labels": []},
        }
        with mock.patch.object(pt, "_gh_api", self.fake_api(epic, children)):
            with mock.patch.object(pt, "_gh_write", return_value={"id": 555}) as write:
                section = pt.section_epics(
                    ["developing-today/code"], True, self.now, 12
                )
        self.assertTrue(section["actions"][0]["live"])
        self.assertEqual(section["actions"][0]["comment_id"], 555)
        self.assertIn("/issues/100/comments", write.call_args[0][0])


class Slice2RoadmapsTest(unittest.TestCase):
    """Slice 2 roadmaps: certain drift by PR, ambiguous drift flagged."""

    ROADMAP = (
        "# Roadmap\n"
        "## Now\n"
        "1. Landed thing (#201).\n"
        "2. Mixed thing (#202, #203).\n"
        "3. Open thing (#203).\n"
        "## Done\n"
        "- Finished (#204).\n"
        "- Wrongly done (#203).\n"
    )

    def setUp(self):
        import datetime as dt
        import tempfile

        self.now = dt.datetime(2026, 11, 11, tzinfo=dt.timezone.utc)
        self.tmp = tempfile.TemporaryDirectory()
        self.root = Path(self.tmp.name)
        (self.root / "ROADMAP.md").write_text(self.ROADMAP)

    def tearDown(self):
        self.tmp.cleanup()

    def fake_api(self, path):
        if "state=all" in path:
            return [
                {"number": 201, "state": "closed"},
                {"number": 202, "state": "closed"},
                {"number": 203, "state": "open"},
                {"number": 204, "state": "closed"},
            ]
        return []

    def test_drift_classification(self):
        with mock.patch.object(pt, "_gh_api", self.fake_api):
            certain, ambiguous = pt._roadmap_drift(self.root)
        self.assertEqual(len(certain), 1)
        self.assertIn("Landed thing", certain[0]["text"])
        reasons = sorted(e["reason"] for e in ambiguous)
        self.assertEqual(len(ambiguous), 2)
        self.assertIn("forward item mixes closed and open refs", reasons)
        self.assertIn("Done-section item references an open issue", reasons)

    def test_dry_run_plans_without_touching_files(self):
        with mock.patch.object(pt, "_gh_api", self.fake_api):
            section = pt.section_roadmaps(
                ["developing-today/code"], False, self.now, 12, self.root
            )
        kinds = sorted(a["kind"] for a in section["actions"])
        self.assertEqual(kinds, ["roadmap-drift-flag", "roadmap-drift-pr"])
        pr_action = next(
            a for a in section["actions"] if a["kind"] == "roadmap-drift-pr"
        )
        self.assertEqual(pr_action["items_removed"], 1)
        self.assertFalse(pr_action["live"])
        self.assertEqual((self.root / "ROADMAP.md").read_text(), self.ROADMAP)

    def test_live_flag_posts_comment_and_label(self):
        calls = []

        def fake_write(path, payload):
            calls.append(path)
            return {"id": 777}

        with mock.patch.object(pt, "_gh_api", self.fake_api):
            with mock.patch.object(pt, "_gh_write", fake_write):
                with mock.patch.object(
                    pt,
                    "_open_drift_pr",
                    return_value={"kind": "roadmap-drift-pr", "live": True},
                ):
                    section = pt.section_roadmaps(
                        ["developing-today/code"], True, self.now, 12, self.root
                    )
        flag = next(a for a in section["actions"] if a["kind"] == "roadmap-drift-flag")
        self.assertTrue(flag["live"])
        self.assertEqual(flag["comment_id"], 777)
        self.assertTrue(any("/comments" in c for c in calls))
        self.assertTrue(any("/labels" in c for c in calls))

    def test_table_row_drift_detected(self):
        planning = self.root / "planning" / "roadmaps"
        planning.mkdir(parents=True)
        (planning / "proj.md").write_text(
            "# Project roadmap\n"
            "## Now\n"
            "| Item | Issue |\n"
            "| --- | --- |\n"
            "| Table landed thing | #201 |\n"
            "| Table open thing | #203 |\n"
        )
        with mock.patch.object(pt, "_gh_api", self.fake_api):
            certain, _ambiguous = pt._roadmap_drift(self.root)
        texts = [e["text"] for e in certain]
        self.assertTrue(any("Table landed thing" in x for x in texts), texts)
        self.assertFalse(any("Table open thing" in x for x in texts), texts)
        self.assertFalse(any(x.strip().startswith("| ---") for x in texts), texts)

    def test_remove_lines(self):
        target = self.root / "ROADMAP.md"
        pt._remove_lines(target, {3})
        self.assertNotIn("Landed thing", target.read_text())
        self.assertIn("Open thing", target.read_text())


class GhApiSingleObjectTest(unittest.TestCase):
    """_gh_api returns a one-item list for single-object endpoints."""

    def test_dict_response_wrapped(self):
        class FakeResp:
            headers = {}

            def __enter__(self):
                return self

            def __exit__(self, *exc):
                return False

            def read(self):
                return b'{"number": 7, "state": "open"}'

        with mock.patch.object(pt.urllib.request, "urlopen", return_value=FakeResp()):
            self.assertEqual(
                pt._gh_api("/repos/x/y/issues/7"),
                [{"number": 7, "state": "open"}],
            )


class RefsInTest(unittest.TestCase):
    """Reference extraction (Codex regression, PR #8377)."""

    def test_markdown_link_label_is_not_a_local_ref(self):
        refs = pt._refs_in(
            "[#42](https://github.com/other/repo/issues/42) and #43",
            "developing-today/code",
        )
        self.assertEqual(refs, [("other/repo", 42), ("developing-today/code", 43)])

    def test_bare_and_url_refs_still_found(self):
        refs = pt._refs_in(
            "see #7 and https://github.com/a/b/pull/9",
            "developing-today/code",
        )
        self.assertEqual(refs, [("a/b", 9), ("developing-today/code", 7)])


class DriftPrGitIdentityTest(unittest.TestCase):
    """The drift-PR commit carries an explicit git identity (PR #8377)."""

    def test_commit_configures_identity(self):
        import tempfile

        calls = []

        def fake_run_cmd(argv, cwd):
            calls.append(argv)

        proc = mock.MagicMock()
        proc.stdout = "https://example.invalid/pr/1\n"
        with tempfile.TemporaryDirectory() as tmp:
            root = Path(tmp)
            (root / "ROADMAP.md").write_text("# Roadmap\nold line\n")
            certain = [
                {
                    "file": "ROADMAP.md",
                    "line": 2,
                    "text": "old line",
                    "refs": ["developing-today/code#1:closed"],
                }
            ]
            with mock.patch.object(pt, "_run_cmd", fake_run_cmd):
                with mock.patch.object(pt.subprocess, "run", return_value=proc):
                    pt._open_drift_pr(root, certain, "2026-11-11")
        commit = next(a for a in calls if "commit" in a)
        self.assertIn("user.name=github-actions[bot]", commit)
        self.assertTrue(any(x.startswith("user.email=") for x in commit), commit)


class Slice3ProjectsTest(unittest.TestCase):
    """Slice 3 projects: board-vs-reality, reported never fabricated."""

    def setUp(self):
        import datetime as dt

        self.now = dt.datetime(2026, 11, 17, tzinfo=dt.timezone.utc)

    def data(self, board_status, content):
        return {
            "repository": {
                "projectsV2": {
                    "nodes": [
                        {
                            "id": "PVT_1",
                            "title": "Board",
                            "fields": {
                                "nodes": [
                                    {
                                        "id": "F1",
                                        "name": "Status",
                                        "options": [
                                            {"id": "O_DONE", "name": "Done"},
                                            {"id": "O_PROG", "name": "In Progress"},
                                            {"id": "O_TODO", "name": "Todo"},
                                        ],
                                    }
                                ]
                            },
                            "items": {
                                "nodes": [
                                    {
                                        "id": "I1",
                                        "fieldValueByName": {"name": board_status},
                                        "content": content,
                                    }
                                ]
                            },
                        }
                    ]
                }
            }
        }

    def test_no_token_reports_unverified(self):
        with mock.patch.dict(os.environ, {}, clear=True):
            section = pt.section_projects(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["status"], "no-action")
        self.assertIn("unverified", section["checked"][0]["board_state"])

    def test_done_board_open_content_is_contradiction(self):
        data = self.data("Done", {"number": 7, "state": "open", "title": "X"})
        with mock.patch.dict(os.environ, {"GH_TOKEN": "t"}, clear=True):
            with mock.patch.object(pt, "_gh_graphql", return_value=data):
                section = pt.section_projects(
                    ["developing-today/code"], False, self.now, 12
                )
        self.assertEqual(len(section["actions"]), 1)
        self.assertEqual(section["actions"][0]["kind"], "project-board-correction")
        self.assertFalse(section["actions"][0]["live"])
        self.assertEqual(len(section["checked"][0]["contradictions"]), 1)

    def test_todo_board_closed_content_is_contradiction(self):
        data = self.data("Todo", {"number": 8, "state": "closed", "title": "Y"})
        with mock.patch.dict(os.environ, {"GH_TOKEN": "t"}, clear=True):
            with mock.patch.object(pt, "_gh_graphql", return_value=data):
                section = pt.section_projects(
                    ["developing-today/code"], False, self.now, 12
                )
        self.assertEqual(len(section["actions"]), 1)

    def test_consistent_board_no_action(self):
        data = self.data("In Progress", {"number": 7, "state": "open", "title": "X"})
        with mock.patch.dict(os.environ, {"GH_TOKEN": "t"}, clear=True):
            with mock.patch.object(pt, "_gh_graphql", return_value=data):
                section = pt.section_projects(
                    ["developing-today/code"], False, self.now, 12
                )
        self.assertEqual(section["actions"], [])

    def test_graphql_failure_reports_unverified_never_fails(self):
        with mock.patch.dict(os.environ, {"GH_TOKEN": "t"}, clear=True):
            with mock.patch.object(pt, "_gh_graphql", side_effect=RuntimeError("down")):
                section = pt.section_projects(
                    ["developing-today/code"], False, self.now, 12
                )
        self.assertEqual(section["checked"][0]["board_state"], "unverified")
        self.assertIn("error", section["checked"][0])

    def test_live_correction_uses_status_option(self):
        data = self.data("Todo", {"number": 8, "state": "closed", "title": "Y"})
        calls = []

        def fake_graphql(query, variables=None):
            calls.append((query, variables))
            return data if len(calls) == 1 else {}

        with mock.patch.dict(os.environ, {"GH_TOKEN": "t"}, clear=True):
            with mock.patch.object(pt, "_gh_graphql", side_effect=fake_graphql):
                section = pt.section_projects(
                    ["developing-today/code"], True, self.now, 12
                )
        action = section["actions"][0]
        self.assertTrue(action["live"])
        self.assertEqual(action["corrected_to"], "Done")
        self.assertEqual(calls[1][1]["option"], "O_DONE")


class Slice3StalledPrsTest(unittest.TestCase):
    """Slice 3 stalled PRs: 48h check-ins, holds, user/owned, packets."""

    def setUp(self):
        import datetime as dt

        self.now = dt.datetime(2026, 11, 17, tzinfo=dt.timezone.utc)

    def pr(self, number, updated, labels=(), author="someone", draft=False):
        return {
            "number": number,
            "title": "PR",
            "updated_at": updated,
            "labels": [{"name": name} for name in labels],
            "draft": draft,
            "user": {"login": author},
        }

    def api(self, prs, comments=None):
        comments = comments or {}

        def fake(path):
            if "/pulls?" in path:
                return prs
            for number, items in comments.items():
                if f"/issues/{number}/comments" in path:
                    return items
            return []

        return fake

    def test_stalled_pr_planned_checkin(self):
        prs = [self.pr(10, "2026-11-14T00:00:00Z")]
        with mock.patch.object(pt, "_gh_api", self.api(prs)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(len(section["actions"]), 1)
        action = section["actions"][0]
        self.assertEqual(action["kind"], "stalled-pr-checkin")
        self.assertEqual(action["silent_days"], 3)
        self.assertNotIn("packet", action)

    def test_fleet_owned_pr_gets_shepherd_packet(self):
        prs = [self.pr(11, "2026-11-14T00:00:00Z", author="dezren39")]
        with mock.patch.object(pt, "_gh_api", self.api(prs)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        packet = section["actions"][0]["packet"]
        self.assertEqual(packet["kind"], "pr_shepherd")
        self.assertIn("pr:11", packet["dedupe_key"])

    def test_recent_pr_no_action(self):
        prs = [self.pr(12, "2026-11-16T12:00:00Z")]
        with mock.patch.object(pt, "_gh_api", self.api(prs)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["actions"], [])

    def test_held_pr_skipped(self):
        prs = [self.pr(13, "2026-11-01T00:00:00Z", labels=("held",))]
        with mock.patch.object(pt, "_gh_api", self.api(prs)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["actions"], [])
        self.assertEqual(section["checked"][0]["skipped_hard_stop"], 1)

    def test_user_owned_waits_seven_days_and_never_packets(self):
        young = [
            self.pr(
                14, "2026-11-14T00:00:00Z", labels=("user/owned",), author="dezren39"
            )
        ]
        with mock.patch.object(pt, "_gh_api", self.api(young)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["actions"], [])
        old = [
            self.pr(
                14, "2026-11-09T00:00:00Z", labels=("user/owned",), author="dezren39"
            )
        ]
        with mock.patch.object(pt, "_gh_api", self.api(old)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["actions"][0]["kind"], "stalled-pr-user-owned-checkin")
        self.assertNotIn("packet", section["actions"][0])

    def test_already_tended_pr_skipped(self):
        prs = [self.pr(15, "2026-11-10T00:00:00Z")]
        comments = {15: [{"body": "check-in <!-- program-tending:stalled-prs -->"}]}
        with mock.patch.object(pt, "_gh_api", self.api(prs, comments)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["actions"], [])
        self.assertEqual(section["checked"][0]["already_tended"], [15])

    def test_live_checkin_posts_comment(self):
        prs = [self.pr(16, "2026-11-14T00:00:00Z")]
        with mock.patch.object(pt, "_gh_api", self.api(prs)):
            with mock.patch.object(pt, "_gh_write", return_value={"id": 42}) as write:
                section = pt.section_stalled_prs(
                    ["developing-today/code"], True, self.now, 12
                )
        self.assertTrue(section["actions"][0]["live"])
        self.assertEqual(section["actions"][0]["comment_id"], 42)
        self.assertIn("/issues/16/comments", write.call_args[0][0])

    def test_zero_budget_defers(self):
        prs = [self.pr(17, "2026-11-14T00:00:00Z")]
        with mock.patch.object(pt, "_gh_api", self.api(prs)):
            section = pt.section_stalled_prs(
                ["developing-today/code"], False, self.now, 0
            )
        self.assertEqual(section["actions"], [])
        self.assertEqual(section["checked"][0]["deferred"], 1)


class Slice3LooseEndsTest(unittest.TestCase):
    """Slice 3 loose ends: parent notify, duplicate links, user/review."""

    def setUp(self):
        import datetime as dt

        self.now = dt.datetime(2026, 11, 17, tzinfo=dt.timezone.utc)

    def api(self, closed=(), opened=(), review=(), parent=None, parent_comments=()):
        def fake(path):
            if "labels=user/review" in path:
                return list(review)
            if "state=closed" in path:
                return list(closed)
            if "/comments" in path:
                return list(parent_comments)
            if parent is not None and path.endswith(f"/issues/{parent['number']}"):
                return [parent]
            if "state=open" in path:
                return list(opened)
            return []

        return fake

    def test_parent_notified_when_child_closes(self):
        child = {"number": 60, "title": "Kid", "body": "Child of #50"}
        parent = {"number": 50, "state": "open", "title": "Parent", "labels": []}
        with mock.patch.object(pt, "_gh_api", self.api(closed=[child], parent=parent)):
            section = pt.section_loose_ends(
                ["developing-today/code"], False, self.now, 12
            )
        kinds = [a["kind"] for a in section["actions"]]
        self.assertIn("loose-end-parent-notification", kinds)
        action = section["actions"][0]
        self.assertEqual((action["issue"], action["child"]), (50, 60))

    def test_parent_notification_suppressed_when_already_posted(self):
        child = {"number": 60, "title": "Kid", "body": "Child of #50"}
        parent = {"number": 50, "state": "open", "title": "Parent", "labels": []}
        comments = [{"body": "child #60 closed <!-- program-tending:loose-ends -->"}]
        with mock.patch.object(
            pt,
            "_gh_api",
            self.api(closed=[child], parent=parent, parent_comments=comments),
        ):
            section = pt.section_loose_ends(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(section["actions"], [])

    def test_duplicate_cross_links_counted(self):
        opened = [{"number": 70, "body": "see #7 and #7 and #8", "labels": []}]
        with mock.patch.object(pt, "_gh_api", self.api(opened=opened)):
            section = pt.section_loose_ends(
                ["developing-today/code"], False, self.now, 12
            )
        self.assertEqual(
            section["checked"][0]["duplicate_cross_links"],
            [{"issue": 70, "duplicate_refs": {"developing-today/code#7": 2}}],
        )

    def test_user_review_surface_planned(self):
        review = [{"number": 80, "updated_at": "2026-10-01T00:00:00Z", "labels": []}]
        with mock.patch.object(pt, "_gh_api", self.api(review=review)):
            section = pt.section_loose_ends(
                ["developing-today/code"], False, self.now, 12
            )
        surface = [
            a
            for a in section["actions"]
            if a["kind"] == "loose-end-user-review-surface"
        ]
        self.assertEqual(len(surface), 1)
        self.assertEqual(surface[0]["items"], 1)
        self.assertEqual(section["checked"][0]["user_review_aging"][0]["age_days"], 47)

    def test_run_reports_slice_3(self):
        with mock.patch.object(pt, "_gh_api", return_value=[]):
            log = pt.run(["developing-today/code"])
        self.assertEqual(log["slice"], 3)


if __name__ == "__main__":
    unittest.main()
