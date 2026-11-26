#!/usr/bin/env python3
"""Tests for the standing queue coverage reconciliation (#7016).

Everything runs offline against fixtures: `--fixture` replaces both the
GitHub enumeration and the broker `/coverage` call, so no live GitHub
access, no broker, and no queue mutation is needed. The enqueue path is
exercised with a stub `fleet` binary on PATH.

Covers:
- observe mode imports nothing and reports the uncovered set;
- import mode enqueues exactly the uncovered issues, one packet each;
- a terminal completed dedupe hit retries once with the documented
  `:dispatch:<hash>` suffix so the issue becomes live coverage;
- a re-run after a successful import imports nothing (idempotence is
  the queue's dedupe key, but the script must not double-submit within
  a run or across runs given the same coverage answer);
- a fetch failure is an ERROR with exit 3, never a silent "0 uncovered";
- the triage packet is schema-shaped and carries the issue dedupe basis;
- an open PR covers its issue with a merge-watch packet, not triage;
- a recorded verdict with a resume condition covers an effort/l or
  effort/m issue (#12233 F2b), and an issue with no verdict does not.
"""

import importlib.util
import json
import os
import stat
import subprocess
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


def ready_pr(
    number: int,
    created_at: str,
    draft: bool = False,
    body: str = "",
    branch: str | None = None,
) -> dict:
    """A ready-PR enumeration row as `gh pr list` shapes it (see
    check_coverage fetch_open_prs): number, title, body, head ref,
    draft flag and created_at. The shepherd fallback is fail-closed —
    a row missing draft/created_at proof is never filed on."""
    return {
        "number": number,
        "title": f"PR {number}",
        "body": body,
        "head": {"ref": branch or f"queue/111-{number}-test"},
        "draft": draft,
        "created_at": created_at,
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

    def test_terminal_status_classifier_matches_queue_terminal_states(self):
        """Done, failed, and cancelled rows trigger retry; live states do not."""
        for status in ("done", "failed", "cancelled"):
            with self.subTest(status=status):
                self.assertTrue(
                    check_coverage._is_terminal_dedupe_response({"status": status})
                )
        for status in ("queued", "claimed", "parked", "blocked"):
            with self.subTest(status=status):
                self.assertFalse(
                    check_coverage._is_terminal_dedupe_response({"status": status})
                )

    def test_terminal_dedupe_hit_retries_with_dispatch_suffix(self):
        """A completed key is not live coverage; retry once with a valid suffix."""
        import unittest.mock

        packet = check_coverage.triage_packet(
            REPO, {"number": 42, "title": "still open"}, "2026-11-26T11:00:00Z"
        )
        original_key = packet["dedupe_key"]
        calls: list[dict] = []

        def run(cmd, **_kwargs):
            submitted = json.loads(Path(cmd[3]).read_text())
            calls.append(submitted)
            if len(calls) == 1:
                stdout = (
                    "dedupe hit: existing task 7 (queue repo-health-code-pilot, status done)\n"
                    + json.dumps(
                        {"id": 7, "status": "done", "dedupe_key": original_key}
                    )
                )
            else:
                stdout = (
                    "enqueued new task 8 (queue repo-health-code-pilot, status queued)\n"
                    + json.dumps({"id": 8, "status": "queued"})
                )
            return subprocess.CompletedProcess(cmd, 0, stdout=stdout, stderr="")

        with unittest.mock.patch.object(
            check_coverage.subprocess, "run", side_effect=run
        ):
            ok, detail = check_coverage.enqueue_packet(
                packet, QUEUE, "fleet", "http://broker", "/token"
            )

        self.assertTrue(ok, detail)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0]["dedupe_key"], original_key)
        retry_key = calls[1]["dedupe_key"]
        self.assertTrue(retry_key.startswith(f"{original_key}:dispatch:"))
        suffix = retry_key.rsplit(":dispatch:", 1)[1]
        self.assertEqual(len(suffix), 8)
        self.assertTrue(all(ch in "0123456789abcdef" for ch in suffix))
        self.assertEqual(calls[1]["payload"], calls[0]["payload"])

    def test_terminal_pr_dedupe_hit_retries_with_dispatch_suffix(self):
        """Ready-PR coverage also suffixes a completed shepherd key."""
        import unittest.mock

        packet = check_coverage.shepherd_packet(
            REPO,
            {
                "number": 900,
                "title": "ready PR",
                "created_at": "2026-11-20T00:00:00Z",
            },
            "2026-11-26T11:00:00Z",
        )
        original_key = packet["dedupe_key"]
        calls: list[dict] = []

        def run(cmd, **_kwargs):
            submitted = json.loads(Path(cmd[3]).read_text())
            calls.append(submitted)
            if len(calls) == 1:
                stdout = (
                    "not filed: dedupe key already belongs to completed task 7\n"
                    + json.dumps(
                        {"dedupe_match": {"rule": "dedupe_key_terminal"}}
                    )
                )
            else:
                stdout = (
                    "enqueued new task 8 (queue repo-health-code-pilot, status queued)\n"
                    + json.dumps({"id": 8, "status": "queued"})
                )
            return subprocess.CompletedProcess(cmd, 0, stdout=stdout, stderr="")

        with unittest.mock.patch.object(
            check_coverage.subprocess, "run", side_effect=run
        ):
            ok, detail = check_coverage.enqueue_packet(
                packet, QUEUE, "fleet", "http://broker", "/token"
            )

        self.assertTrue(ok, detail)
        self.assertEqual(len(calls), 2)
        self.assertEqual(calls[0]["dedupe_key"], original_key)
        self.assertTrue(calls[1]["dedupe_key"].startswith(f"{original_key}:dispatch:"))


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
        self.assertEqual(packet["payload"]["prs"][0]["number"], 900)
        self.assertEqual(packet["dedupe_key"], f"repo-health:v1:pr:{REPO}:pr:900")
        self.assertEqual(packet["payload"]["max_attempts"], 1000)

    def test_each_open_pr_for_one_issue_gets_its_own_merge_watch(self):
        pulls = [
            {
                "number": 900,
                "title": "first slice (#3)",
                "body": "",
                "head": {"ref": "queue/111-3-first"},
            },
            {
                "number": 901,
                "title": "second slice (#3)",
                "body": "",
                "head": {"ref": "queue/112-3-second"},
            },
        ]
        code, _ = self.run_cli(fixture([], [3], pulls), "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        self.assertEqual(len(calls), 2)
        self.assertEqual(
            {call["packet"]["payload"]["pr"]["number"] for call in calls},
            {900, 901},
        )
        self.assertEqual(
            {call["packet"]["dedupe_key"] for call in calls},
            {
                f"repo-health:v1:pr:{REPO}:pr:900",
                f"repo-health:v1:pr:{REPO}:pr:901",
            },
        )

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


class VerdictCoverageTest(unittest.TestCase):
    """#12233 F2b: a triaged large issue is covered by its recorded
    verdict and not re-enqueued; an issue with no verdict is still
    uncovered. The broker diff cannot see verdicts, so the fixture
    carries them on the issue objects (labels / verdict /
    resume_condition), exactly as the live enrichment merges them."""

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

    @staticmethod
    def vfixture(
        entries: dict, uncovered: list[int], resume_targets: dict | None = None
    ) -> dict:
        issues = [
            {"number": n, "title": f"issue {n}", **extra}
            for n, extra in entries.items()
        ]
        covered = [n for n in entries if n not in uncovered]
        fx = {
            "issues": {REPO: issues},
            "pulls": {REPO: []},
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
        if resume_targets is not None:
            fx["resume_targets"] = resume_targets
        return fx

    def test_verdict_covered_issue_is_not_reenqueued(self):
        # The F2b done-test: a triaged large issue is covered.
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                }
            },
            [3],
        )
        code, out = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Total uncovered: 0", out)
        self.assertIn("Verdict covered (#12233 F2b): 1", out)
        self.assertIn(f"{REPO}#3", out)

    def test_issue_with_no_verdict_is_still_uncovered(self):
        # The other half of the F2b done-test.
        fx = self.vfixture({4: {"labels": ["effort/l"]}}, [4])
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["packet"]["payload"]["kind"], "triage")
        self.assertEqual(calls[0]["packet"]["payload"]["issue"]["number"], 4)

    def test_verdict_without_resume_condition_is_still_uncovered(self):
        fx = self.vfixture(
            {5: {"labels": ["effort/m"], "verdict": "still_blocked"}}, [5]
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(len(self.fleet.calls()), 1)

    def test_small_issue_verdict_does_not_cover(self):
        fx = self.vfixture(
            {
                6: {
                    "labels": ["effort/s"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                }
            },
            [6],
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(len(self.fleet.calls()), 1)

    def test_placeholder_verdict_does_not_cover(self):
        fx = self.vfixture(
            {
                7: {
                    "labels": ["effort/l"],
                    "verdict": "untriaged",
                    "resume_condition": "dependency_closed:539",
                }
            },
            [7],
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(len(self.fleet.calls()), 1)

    def test_fulfilled_dependency_releases_coverage(self):
        # Thread 1 (P1): #539 closed, so the verdict's wait is over —
        # the parent is uncovered again and gets a triage packet.
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                }
            },
            [3],
            resume_targets={"dependency_closed:539": "closed"},
        )
        code, out = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        calls = self.fleet.calls()
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["packet"]["payload"]["issue"]["number"], 3)
        self.assertIn("Total uncovered: 1", out)
        self.assertIn("resume condition fulfilled", out)

    def test_open_dependency_keeps_coverage(self):
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                }
            },
            [3],
            resume_targets={"dependency_closed:539": "open"},
        )
        code, out = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Total uncovered: 0", out)

    def test_merged_slice_releases_coverage(self):
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "slice_landed:12543",
                }
            },
            [3],
            resume_targets={"slice_landed:12543": "merged"},
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(len(self.fleet.calls()), 1)

    def test_closed_unmerged_slice_keeps_coverage(self):
        # A slice PR closed without merging did not land the slice.
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "slice_landed:12543",
                }
            },
            [3],
            resume_targets={"slice_landed:12543": "closed"},
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])

    def test_unknown_resume_state_keeps_coverage(self):
        # No resume_targets entry: fulfilment is unknown, never
        # assumed — and the fixture run touches no network.
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                }
            },
            [3],
            resume_targets={},
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])

    def test_stale_verdict_is_uncovered_again(self):
        # Thread 2 (P1): a label change after the verdict ends it.
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                    "verdict_at": "2026-11-08T12:00:00Z",
                    "timeline_changes": [
                        {"kind": "label", "at": "2026-11-08T12:30:00Z"}
                    ],
                }
            },
            [3],
        )
        code, out = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(len(self.fleet.calls()), 1)
        self.assertIn("verdict stale", out)

    def test_change_before_the_verdict_keeps_coverage(self):
        fx = self.vfixture(
            {
                3: {
                    "labels": ["effort/l"],
                    "verdict": "still_blocked",
                    "resume_condition": "dependency_closed:539",
                    "verdict_at": "2026-11-08T12:00:00Z",
                    "timeline_changes": [
                        {"kind": "commit", "at": "2026-11-08T11:00:00Z"}
                    ],
                }
            },
            [3],
        )
        code, _ = self.run_cli(fx, "--import")
        self.assertEqual(code, 0)
        self.assertEqual(self.fleet.calls(), [])

    def test_verdict_from_comments_records_verdict_time(self):
        note = (
            "<!-- agent-note:v1 state=triaged task=11111 "
            "resume=dependency_closed:539 verdict=still_blocked -->\nTriaged."
        )
        comments = [
            {
                "body": note,
                "created_at": "2026-11-08T12:00:00Z",
                "user": {"type": "User"},
            }
        ]
        self.assertEqual(
            check_coverage.verdict_from_comments(comments),
            {
                "verdict": "still_blocked",
                "resume_condition": "dependency_closed:539",
                "verdict_at": "2026-11-08T12:00:00Z",
            },
        )

    def test_fetch_issue_verdict_applies_timeline_freshness(self):
        # Thread 2 (P1) at the fetch level: the timeline is read and
        # a change after the verdict voids it.
        import subprocess as sp
        import unittest.mock

        note = (
            "<!-- agent-note:v1 state=triaged task=11111 "
            "resume=dependency_closed:539 verdict=still_blocked -->\nTriaged."
        )

        def fake_run(cmd, **kw):
            if "timeline" in " ".join(cmd):
                return sp.CompletedProcess(
                    cmd,
                    0,
                    stdout=json.dumps(
                        {
                            "event": "labeled",
                            "created_at": "2026-11-08T12:30:00Z",
                            "commit_id": None,
                        }
                    )
                    + "\n",
                    stderr="",
                )
            return sp.CompletedProcess(
                cmd,
                0,
                stdout=json.dumps(
                    {
                        "body": note,
                        "created_at": "2026-11-08T12:00:00Z",
                        "user": {"type": "User"},
                    }
                )
                + "\n",
                stderr="",
            )

        with unittest.mock.patch.object(
            check_coverage.subprocess, "run", side_effect=fake_run
        ):
            stale = check_coverage.fetch_issue_verdict(REPO, 3)
        self.assertEqual(stale["verdict"], "still_blocked")
        self.assertFalse(check_coverage.verdict_fresh(stale))

    def test_fetch_resume_state_reads_unquoted_dependency_state(self):
        # Codex P1 on #12565: `--jq .state` emits the raw state word
        # (`closed` / `open`), not JSON — decoding it as JSON raised
        # on every live lookup and left closed dependencies unknown.
        import subprocess as sp
        import unittest.mock

        seen = []

        def fake_run(cmd, **kw):
            seen.append(cmd)
            if "/issues/" in " ".join(cmd):
                return sp.CompletedProcess(cmd, 0, stdout="closed\n", stderr="")
            return sp.CompletedProcess(
                cmd,
                0,
                stdout=json.dumps({"merged": True, "state": "closed"}) + "\n",
                stderr="",
            )

        with unittest.mock.patch.object(
            check_coverage.subprocess, "run", side_effect=fake_run
        ):
            state = check_coverage.fetch_resume_state(REPO, "dependency_closed:539")
            self.assertEqual(state, "closed")
            self.assertTrue(
                check_coverage.resolve_resume(REPO, "dependency_closed:539")
            )
            self.assertEqual(
                check_coverage.fetch_resume_state(REPO, "slice_landed:12543"),
                "merged",
            )
        dep_cmd = [c for c in seen if "/issues/" in " ".join(c)][0]
        self.assertEqual(dep_cmd[-2:], ["--jq", ".state"])

    def test_fetch_resume_state_open_dependency_is_not_fulfilled(self):
        import subprocess as sp
        import unittest.mock

        def fake_run(cmd, **kw):
            return sp.CompletedProcess(cmd, 0, stdout="open\n", stderr="")

        with unittest.mock.patch.object(
            check_coverage.subprocess, "run", side_effect=fake_run
        ):
            self.assertEqual(
                check_coverage.fetch_resume_state(REPO, "dependency_closed:539"),
                "open",
            )
            self.assertFalse(
                check_coverage.resolve_resume(REPO, "dependency_closed:539")
            )

    def test_unread_verdict_issue_is_skipped_from_import(self):
        # Thread 3 (P2): the comments fetch fails, so the issue's
        # verdict is unknown — no triage packet, exit 3.
        import unittest.mock

        token = self.tmp / "token"
        token.write_text("test-token\n")
        coverage = {
            "queue": QUEUE,
            "repo": REPO,
            "open_issues": 1,
            "covered": 0,
            "uncovered": 1,
            "covered_issues": [],
            "uncovered_issues": [
                {
                    "number": 3,
                    "title": "issue 3",
                    "url": f"https://github.com/{REPO}/issues/3",
                }
            ],
        }
        with (
            unittest.mock.patch.object(
                check_coverage,
                "fetch_open_issues",
                return_value=[
                    {"number": 3, "title": "issue 3", "labels": ["effort/l"]}
                ],
            ),
            unittest.mock.patch.object(
                check_coverage, "fetch_open_prs", return_value=[]
            ),
            unittest.mock.patch.object(
                check_coverage, "broker_coverage", return_value=coverage
            ),
            unittest.mock.patch.object(
                check_coverage,
                "fetch_issue_verdict",
                side_effect=check_coverage.FetchError(
                    "github comments failed for developing-today/code#3: boom"
                ),
            ),
        ):
            import io
            from contextlib import redirect_stdout, redirect_stderr

            buf = io.StringIO()
            with redirect_stdout(buf), redirect_stderr(buf):
                code = check_coverage.main(
                    [
                        "--import",
                        "--state-dir",
                        str(self.state),
                        "--fleet",
                        str(self.fleet.bin),
                        "--repo",
                        REPO,
                        "--token-file",
                        str(token),
                        "--now",
                        "2026-11-08T13:00:00Z",
                    ]
                )
            out = buf.getvalue()
        self.assertEqual(code, 3)
        self.assertEqual(self.fleet.calls(), [])
        self.assertIn("Total uncovered: 0", out)
        self.assertIn("Verdict unread", out)
        self.assertIn("github comments failed", out)

    def test_parse_resume_condition_forms(self):
        parse = check_coverage.parse_resume_condition
        self.assertEqual(parse("dependency_closed:539"), "dependency_closed:539")
        self.assertEqual(parse("slice landed #12543"), "slice_landed:12543")
        self.assertEqual(
            parse({"kind": "dependency_closed", "target": "#539"}),
            "dependency_closed:539",
        )
        for bad in ("next week", "2027-01-01", "the owner", "slice_landed", None, ""):
            with self.subTest(bad=bad):
                self.assertIsNone(parse(bad))

    def test_verdict_from_comments_reads_the_newest_note(self):
        note = (
            "<!-- agent-note:v1 state=triaged task=11111 "
            "resume=dependency_closed:539 verdict=still_blocked -->\nTriaged."
        )
        comments = [{"body": note, "user": {"type": "User"}}]
        self.assertEqual(
            check_coverage.verdict_from_comments(comments),
            {"verdict": "still_blocked", "resume_condition": "dependency_closed:539"},
        )
        # A human comment after the verdict ends its coverage.
        comments.append({"body": "take another look", "user": {"type": "User"}})
        self.assertEqual(check_coverage.verdict_from_comments(comments), {})
        # A bot comment does not.
        comments[1] = {"body": "CI summary", "user": {"type": "Bot"}}
        self.assertEqual(
            check_coverage.verdict_from_comments(comments)["verdict"],
            "still_blocked",
        )

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
        # Exactly one packet: the merge-watch, keyed by the canonical PR key.
        self.assertEqual(len(calls), 1)
        self.assertEqual(
            calls[0]["packet"]["dedupe_key"],
            f"repo-health:v1:pr:{REPO}:pr:954",
        )


if __name__ == "__main__":
    unittest.main()
