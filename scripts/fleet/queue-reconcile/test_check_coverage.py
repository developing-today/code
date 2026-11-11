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
- the triage packet is schema-shaped and carries the issue dedupe basis.
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


def fixture(covered: list[int], uncovered: list[int], pulls: list[dict] | None = None) -> dict:
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
                    {"number": n, "title": f"issue {n}", "url": f"https://github.com/{REPO}/issues/{n}"}
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
        return [json.loads(line) for line in self.log.read_text().splitlines() if line.strip()]


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
                    "--fixture", str(fx_path),
                    "--state-dir", str(self.state),
                    "--fleet", str(self.fleet.bin),
                    "--repo", REPO,
                    "--now", "2026-11-08T13:00:00Z",
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
        self.assertIn("https://github.com/developing-today/code/issues/7", packet["context_refs"])

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
            "--broker-url", "http://broker.example:4801",
            "--token-file", str(token),
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
                    "--fixture", str(fx_path),
                    "--state-dir", str(self.state),
                    "--fleet", str(self.fleet.bin),
                    "--repo", REPO,
                    "--import",
                    "--now", "2026-11-08T13:00:00Z",
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
            "developing-today-forks/opencode", {"number": 7, "title": "b"}, "2026-11-08T13:00:00Z"
        )
        self.assertTrue(
            check_coverage.enqueue_packet(a, QUEUE, str(self.fleet.bin), "http://x", "/t")[0]
        )
        self.assertTrue(
            check_coverage.enqueue_packet(b, QUEUE, str(self.fleet.bin), "http://x", "/t")[0]
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
            set(cov([{"number": 1, "title": "x", "body": "", "head": {"ref": "queue/9-42-a"}}])),
            {42},
        )
        # closing keyword alone
        self.assertEqual(
            set(cov([{"number": 1, "title": "x", "body": "Fixes #43", "head": {"ref": "feat/y"}}])),
            {43},
        )
        # trailing title reference alone
        self.assertEqual(
            set(cov([{"number": 1, "title": "fix: x (#44)", "body": "", "head": {"ref": "feat/y"}}])),
            {44},
        )
        # a bare prose mention is NOT coverage
        self.assertEqual(
            cov([{"number": 1, "title": "x", "body": "discussed in #45 today", "head": {"ref": "feat/y"}}]),
            {},
        )


if __name__ == "__main__":
    unittest.main()
