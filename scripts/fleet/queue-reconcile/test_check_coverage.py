#!/usr/bin/env python3
"""Tests for the standing queue coverage reconciliation (#7016).

Everything runs offline against fixtures: `--fixture` replaces both the
GitHub enumeration and the broker `/coverage` call, so no live GitHub
access, no broker, and no queue mutation is needed. The enqueue path is
exercised with a stub `fleet` binary on PATH.

Covers:
- observe mode imports nothing and reports the uncovered set;
- import mode enqueues exactly the uncovered issues, one packet each;
- a re-run after a successful import imports nothing (idempotence is
  the queue's dedupe key, but the script must not double-submit within
  a run or across runs given the same coverage answer);
- a fetch failure is an ERROR with exit 3, never a silent "0 uncovered";
- the triage packet is schema-shaped and carries the issue dedupe basis;
- #12280: a ready PR older than 6h with no live task naming it gets
  one warden-keyed pr_shepherd packet; drafts, young PRs, PRs with a
  live task, and PRs already merge-watched this run get none.
"""

import importlib.util
import json
import os
import stat
import sys
import tempfile
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
CHECK = HERE / "check_coverage.py"

spec = importlib.util.spec_from_file_location("check_coverage", CHECK)
check_coverage = importlib.util.module_from_spec(spec)
sys.modules["check_coverage"] = check_coverage
spec.loader.exec_module(check_coverage)

REPO = "developing-today/code"
QUEUE = "repo-health-code-pilot"


def fixture(
    covered: list[int], uncovered: list[int], pulls: list[dict] | None = None
) -> dict:
    issues = [{"number": n, "title": f"issue {n}"} for n in covered + uncovered]
    return {
        "issues": {REPO: issues},
        "pulls": {REPO: pulls or []},
        "coverage": {
            REPO: {
                "queue": QUEUE,
                "repo": REPO,
                "open_issues": len(issues),
                "covered": len(covered),
                "uncovered": len(uncovered),
                "covered_issues": [{"number": n} for n in covered],
                "uncovered_issues": [
                    {
                        "number": n,
                        "title": f"issue {n}",
                        "url": f"https://github.com/{REPO}/issues/{n}",
                    }
                    for n in uncovered
                ],
            }
        },
    }


class StubFleet:
    """A `fleet` binary that records each enqueue and exits 0."""

    def __init__(self, tmp: Path):
        self.dir = tmp
        self.log = tmp / "enqueues.jsonl"
        self.bin = tmp / "fleet"
        self.bin.write_text(
            "#!/usr/bin/env python3\n"
            "import json, os, sys\n"
            f"log = {str(self.log)!r}\n"
            "args = sys.argv[1:]\n"
            "path = args[2]\n"
            "queue = args[args.index('--queue') + 1]\n"
            "packet = json.load(open(path))\n"
            "env = {\n"
            "    'REPO_QUEUE_BROKER_URL': os.environ.get('REPO_QUEUE_BROKER_URL'),\n"
            "    'REPO_QUEUE_TOKEN_FILE': os.environ.get('REPO_QUEUE_TOKEN_FILE'),\n"
            "}\n"
            "with open(log, 'a') as fh:\n"
            "    fh.write(json.dumps({'queue': queue, 'packet': packet, 'path': path, 'env': env}) + '\\n')\n"
            "print('{\"ok\": true}')\n"
        )
        self.bin.chmod(self.bin.stat().st_mode | stat.S_IEXEC)

    def calls(self) -> list[dict]:
        if not self.log.exists():
            return []
        return [
            json.loads(line)
            for line in self.log.read_text().splitlines()
            if line.strip()
        ]


class CoverageReconcileTest(unittest.TestCase):
    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.fleet = StubFleet(self.tmp)
        self.state = self.tmp / "state"

    def run_cli(self, fx: dict, *extra: str) -> tuple[int, str]:
        fx_path = self.tmp / "fixture.json"
        fx_path.write_text(json.dumps(fx))
        import io
        from contextlib import redirect_stdout, redirect_stderr

        buf = io.StringIO()
        with redirect_stdout(buf), redirect_stderr(buf):
            code = check_coverage.main(
                [
                    "--fixture",
                    str(fx_path),
                    "--state-dir",
                    str(self.state),
                    "--fleet",
                    str(self.fleet.bin),
                    "--repo",
                    REPO,
                    "--now",
                    "2026-11-08T13:00:00Z",
                    *extra,
                ]
            )
        return code, buf.getvalue()

    def test_observe_mode_imports_nothing(self):
        code, out = self.run_cli(fixture([1, 2], [3, 4]))
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Total uncovered: 2", out)
        self.assertIn(f"{REPO}#3", out)

    def test_import_mode_enqueues_exactly_the_uncovered(self):
        code, _ = self.run_cli(fixture([1, 2], [3, 4]), "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        self.assertEqual(len(calls), 2)
        numbers = sorted(c["packet"]["payload"]["issue"]["number"] for c in calls)
        self.assertEqual(numbers, [3, 4])
        for c in calls:
            self.assertEqual(c["queue"], QUEUE)
            self.assertEqual(c["packet"]["payload"]["kind"], "triage")
            self.assertEqual(c["packet"]["payload"]["repo"], REPO)
            self.assertEqual(c["packet"]["payload"]["schema"], "repo-task/v1")

    def test_full_coverage_imports_nothing(self):
        code, out = self.run_cli(fixture([1, 2, 3], []), "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Total uncovered: 0", out)

    def test_fetch_failure_is_an_error_not_zero_coverage(self):
        fx = fixture([], [])
        del fx["coverage"][REPO]
        code, out = self.run_cli(fx, "--import")
        self.assertEqual(code, 3)
        self.assertIn("ERROR", out)
        self.assertEqual(self.fleet.calls(), [])

    def test_packet_carries_dedupe_basis_and_acceptance(self):
        self.run_cli(fixture([], [7]), "--import")
        packet = self.fleet.calls()[0]["packet"]["payload"]
        self.assertIn("7016", packet["dedupe"]["basis"])
        self.assertEqual(packet["dedupe"]["checked_at"], "2026-11-08T13:00:00Z")
        self.assertTrue(packet["acceptance"])
        self.assertIn(
            "https://github.com/developing-today/code/issues/7", packet["context_refs"]
        )

    def test_report_is_written_to_state_dir(self):
        self.run_cli(fixture([1], [2]))
        self.assertTrue((self.state / "latest.md").exists())
        reports = list(self.state.glob("report-*.md"))
        self.assertEqual(len(reports), 1)

    def test_import_child_gets_the_selected_broker_and_token(self):
        token = self.tmp / "token"
        token.write_text("test-token\n")
        code, _ = self.run_cli(
            fixture([], [3]),
            "--import",
            "--broker-url",
            "http://broker.example:4801",
            "--token-file",
            str(token),
        )
        self.assertEqual(code, 0)
        env = self.fleet.calls()[0]["env"]
        self.assertEqual(env["REPO_QUEUE_BROKER_URL"], "http://broker.example:4801")
        self.assertEqual(env["REPO_QUEUE_TOKEN_FILE"], str(token))

    def test_enqueue_launch_failure_is_an_error_not_a_crash(self):
        import unittest.mock

        fx_path = self.tmp / "fixture.json"
        fx_path.write_text(json.dumps(fixture([], [3, 4])))
        with unittest.mock.patch.object(
            check_coverage.subprocess,
            "run",
            side_effect=OSError("fleet missing"),
        ):
            code = check_coverage.main(
                [
                    "--fixture",
                    str(fx_path),
                    "--state-dir",
                    str(self.state),
                    "--fleet",
                    str(self.fleet.bin),
                    "--repo",
                    REPO,
                    "--import",
                    "--now",
                    "2026-11-08T13:00:00Z",
                ]
            )
        self.assertEqual(code, 3)
        report = (self.state / "latest.md").read_text()
        self.assertIn("ERROR", report)
        self.assertIn("fleet missing", report)

    def test_packet_paths_are_unique_per_enqueue(self):
        a = check_coverage.triage_packet(
            "developing-today/code", {"number": 7, "title": "a"}, "2026-11-08T13:00:00Z"
        )
        b = check_coverage.triage_packet(
            "developing-today-forks/opencode",
            {"number": 7, "title": "b"},
            "2026-11-08T13:00:00Z",
        )
        self.assertTrue(
            check_coverage.enqueue_packet(
                a, QUEUE, str(self.fleet.bin), "http://x", "/t"
            )[0]
        )
        self.assertTrue(
            check_coverage.enqueue_packet(
                b, QUEUE, str(self.fleet.bin), "http://x", "/t"
            )[0]
        )
        paths = [c["path"] for c in self.fleet.calls()]
        self.assertEqual(len(paths), 2)
        self.assertNotEqual(paths[0], paths[1])
        self.assertFalse(Path(paths[0]).exists())
        self.assertFalse(Path(paths[1]).exists())


class OpenPrCoverageTest(unittest.TestCase):
    """#4951: an issue with an open fix PR is in-flight, not ownerless."""

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.fleet = StubFleet(self.tmp)
        self.state = self.tmp / "state"
        self.base = CoverageReconcileTest("test_observe_mode_imports_nothing")
        self.base.tmp = self.tmp
        self.base.fleet = self.fleet
        self.base.state = self.state

    def run_cli(self, fx: dict, *extra: str):
        return self.base.run_cli(fx, *extra)

    def test_open_pr_issue_yields_zero_triage_packets(self):
        pulls = [
            {
                "number": 900,
                "title": "fix: thing (#3)",
                "body": "Closes #3",
                "head": {"ref": "queue/111-3-fix-thing"},
            }
        ]
        code, out = self.run_cli(fixture([], [3], pulls), "--import")
        self.assertEqual(code, 0)
        kinds = [c["packet"]["payload"]["kind"] for c in self.fleet.calls()]
        self.assertNotIn("triage", kinds)
        self.assertIn("Total uncovered: 0", out)
        self.assertIn("PR #900", out)

    def test_open_pr_issue_gets_one_merge_watch_packet(self):
        pulls = [
            {
                "number": 900,
                "title": "fix: thing (#3)",
                "body": "Closes #3",
                "head": {"ref": "queue/111-3-fix-thing"},
            }
        ]
        code, _ = self.run_cli(fixture([], [3], pulls), "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        self.assertEqual(len(calls), 1)
        packet = calls[0]["packet"]
        self.assertEqual(packet["payload"]["kind"], "pr_shepherd")
        self.assertEqual(packet["payload"]["pr"]["number"], 900)
        self.assertTrue(packet["dedupe_key"].endswith(":pr:900"))

    def test_observe_mode_reports_pr_coverage_and_enqueues_nothing(self):
        pulls = [
            {
                "number": 901,
                "title": "fix: other",
                "body": "",
                "head": {"ref": "queue/112-4-fix-other"},
            }
        ]
        code, out = self.run_cli(fixture([], [4], pulls))
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Total uncovered: 0", out)
        self.assertIn("Open-PR covered (#4951): 1", out)

    def test_pr_signal_mapping(self):
        cov = check_coverage.build_pr_coverage
        # branch namespace alone
        self.assertEqual(
            set(
                cov(
                    [
                        {
                            "number": 1,
                            "title": "x",
                            "body": "",
                            "head": {"ref": "queue/9-42-a"},
                        }
                    ]
                )
            ),
            {42},
        )
        # closing keyword alone
        self.assertEqual(
            set(
                cov(
                    [
                        {
                            "number": 1,
                            "title": "x",
                            "body": "Fixes #43",
                            "head": {"ref": "feat/y"},
                        }
                    ]
                )
            ),
            {43},
        )
        # trailing title reference alone
        self.assertEqual(
            set(
                cov(
                    [
                        {
                            "number": 1,
                            "title": "fix: x (#44)",
                            "body": "",
                            "head": {"ref": "feat/y"},
                        }
                    ]
                )
            ),
            {44},
        )
        # a bare prose mention is NOT coverage
        self.assertEqual(
            cov(
                [
                    {
                        "number": 1,
                        "title": "x",
                        "body": "discussed in #45 today",
                        "head": {"ref": "feat/y"},
                    }
                ]
            ),
            {},
        )


def ready_pr(number: int, created_at: str, draft: bool = False, **kw) -> dict:
    pr = {
        "number": number,
        "title": kw.get("title", f"pr {number}"),
        "body": kw.get("body", ""),
        "head": {"ref": kw.get("branch", f"feat/pr-{number}")},
        "draft": draft,
        "created_at": created_at,
    }
    return pr


class ReadyPrShepherdTest(unittest.TestCase):
    """#12280: a ready PR older than 6h with no live task naming it
    gets a warden-keyed pr_shepherd packet from the reconciliation —
    the fallback for when the stale-PR warden has not run. Oracle:
    the issue's done-when (queue store vs `gh pr list`). The CLI runs
    at --now 2026-11-08T13:00:00Z (see CoverageReconcileTest.run_cli).
    """

    def setUp(self):
        self.tmp = Path(tempfile.mkdtemp())
        self.fleet = StubFleet(self.tmp)
        self.state = self.tmp / "state"
        self.base = CoverageReconcileTest("test_observe_mode_imports_nothing")
        self.base.tmp = self.tmp
        self.base.fleet = self.fleet
        self.base.state = self.state

    def run_cli(self, fx: dict, *extra: str):
        return self.base.run_cli(fx, *extra)

    def test_old_ready_uncovered_pr_gets_one_warden_keyed_shepherd(self):
        pulls = [ready_pr(950, "2026-11-07T00:00:00Z")]  # 37h old at --now
        code, _ = self.run_cli(fixture([], [], pulls), "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        self.assertEqual(len(calls), 1)
        packet = calls[0]["packet"]
        self.assertEqual(calls[0]["queue"], QUEUE)
        self.assertEqual(packet["payload"]["kind"], "pr_shepherd")
        # prs elements are PR references (objects with a positive
        # integer `number`) — the queue validator refuses bare
        # numbers (#5549), which the first cut of this packet used.
        self.assertEqual(
            packet["payload"]["prs"],
            [
                {
                    "number": 950,
                    "url": "https://github.com/developing-today/code/pull/950",
                    "title": "pr 950",
                }
            ],
        )
        self.assertEqual(packet["payload"]["pr"]["number"], 950)
        # The warden's own dedupe key: the two filers share one live task.
        self.assertEqual(
            packet["dedupe_key"], "repo-health:v1:pr:developing-today/code:pr:950"
        )
        # The done rule rides in the packet: merged or closed with a reason.
        self.assertIn("merged or closed with a reason", packet["body"])

    def test_draft_and_young_prs_get_nothing(self):
        pulls = [
            ready_pr(951, "2026-11-01T00:00:00Z", draft=True),  # old but draft
            ready_pr(952, "2026-11-08T10:00:00Z"),  # ready but 3h old
        ]
        code, _ = self.run_cli(fixture([], [], pulls), "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])

    def test_pr_with_a_live_task_naming_it_gets_nothing(self):
        pulls = [ready_pr(953, "2026-11-07T00:00:00Z")]
        fx = fixture([], [], pulls)
        # The broker's PR diff: PR 953 is named by a live task, so it
        # is absent from uncovered_pulls.
        fx["coverage"][REPO]["uncovered_pulls"] = []
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])

    def test_merge_watch_pr_is_not_double_filed_as_shepherd(self):
        pulls = [
            ready_pr(
                954,
                "2026-11-07T00:00:00Z",
                body="Closes #3",
                branch="queue/111-3-fix-thing",
            )
        ]
        fx = fixture([], [3], pulls)
        fx["coverage"][REPO]["uncovered_pulls"] = [{"number": 954}]
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        # Exactly one packet: the merge-watch, keyed merge-watch.
        self.assertEqual(len(calls), 1)
        self.assertTrue(
            calls[0]["packet"]["dedupe_key"].startswith("repo-health:v1:merge-watch:")
        )

    def test_observe_mode_reports_candidates_and_enqueues_nothing(self):
        pulls = [ready_pr(955, "2026-11-07T00:00:00Z")]
        code, out = self.run_cli(fixture([], [], pulls))
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Ready-PR shepherd candidates (#12280): 1", out)
        self.assertIn(f"{REPO}#955", out)

    def test_candidate_selection_fails_closed_on_unproven_fields(self):
        sel = check_coverage.ready_shepherd_candidates
        now = "2026-11-08T13:00:00Z"
        old = "2026-11-07T00:00:00Z"
        # No draft field at all: readiness unproven, never filed on.
        pr = {"number": 1, "created_at": old}
        self.assertEqual(sel([pr], {1}, set(), now), [])
        # No created_at: age unproven, never filed on.
        pr = {"number": 1, "draft": False}
        self.assertEqual(sel([pr], {1}, set(), now), [])
        # Exactly 6h old qualifies; 5h59m does not.
        pr = {"number": 1, "draft": False, "created_at": "2026-11-08T07:00:00Z"}
        self.assertEqual(len(sel([pr], {1}, set(), now)), 1)
        pr = {"number": 1, "draft": False, "created_at": "2026-11-08T07:01:00Z"}
        self.assertEqual(sel([pr], {1}, set(), now), [])


if __name__ == "__main__":
    unittest.main()
