#!/usr/bin/env python3
"""Tests for the daily drift census report path (#14134 U1 slice 2).

Oracle: the slice packet's done check — a scheduled invocation runs
`python3 -I scripts/drift-census.py --format json`; the registry row
agrees with the schedule on cadence, timezone, and command; an
end-to-end fixture shows a drifted report reaches #14134 while a
converged result stays silent — plus the design of record
(doc/2026-11-24T06-25-30Z_design_auto-update-system/ section 4):
silence means converged, and the census never fixes.

Run: python3 -m unittest scripts.test_drift_census_report
(or execute this file directly).
"""

import contextlib
import importlib.util
import io
import json
import pathlib
import shutil
import subprocess
import tempfile
import unittest

HERE = pathlib.Path(__file__).resolve().parent
ROOT = HERE.parent


def load_module(name, path):
    spec = importlib.util.spec_from_file_location(name, path)
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


census = load_module("drift_census", HERE / "drift-census.py")
report_mod = load_module(
    "drift_census_report", HERE / "fleet" / "drift-census-report.py"
)

NOW = "2026-11-26T12:00:00+00:00"
WINDOW = 24 * 60 * 60


def fixture_root(tmp: str) -> pathlib.Path:
    """A hermetic repo root: the census, the bootstrap pins, one commit.

    origin/main resolves to the fixture commit, so the laptop pin is
    known exactly and a matching observed revision is converged.
    """
    root = pathlib.Path(tmp)
    (root / "scripts" / "muse").mkdir(parents=True)
    shutil.copy(HERE / "drift-census.py", root / "scripts" / "drift-census.py")
    shutil.copy(
        ROOT / "scripts" / "muse" / "bootstrap.sh",
        root / "scripts" / "muse" / "bootstrap.sh",
    )
    subprocess.run(["git", "init", "-q", str(root)], check=True)
    subprocess.run(["git", "-C", str(root), "add", "scripts"], check=True)
    subprocess.run(
        [
            "git",
            "-C",
            str(root),
            "-c",
            "user.email=test@example.invalid",
            "-c",
            "user.name=test",
            "commit",
            "-qm",
            "fixture root",
        ],
        check=True,
    )
    subprocess.run(
        ["git", "-C", str(root), "update-ref", "refs/remotes/origin/main", "HEAD"],
        check=True,
    )
    return root


class FakeRunner:
    """Records every subprocess the wrapper issues.

    The fleet-shell laptop probe is answered from the configured
    revision; the census invocation is delegated to the real census so
    the composition is end-to-end; a gh call is recorded, not sent.
    """

    def __init__(self, laptop_revision):
        self.calls: list[list[str]] = []
        self.laptop_revision = laptop_revision
        self.posts: list[list[str]] = []

    def __call__(self, argv):
        argv = [str(a) for a in argv]
        self.calls.append(argv)
        if "nixos-version" in argv:
            if self.laptop_revision is None:
                return None
            return json.dumps({"configurationRevision": self.laptop_revision})
        if any("drift-census.py" in a for a in argv):
            proc = subprocess.run(argv, capture_output=True, text=True)
            return proc.stdout if proc.returncode == 0 else None
        if argv[0] == "gh":
            self.posts.append(argv)
            return "https://example.invalid/comment/1"
        return None


class EndToEndTests(unittest.TestCase):
    def setUp(self):
        self._tmp = tempfile.TemporaryDirectory()
        self.root = fixture_root(self._tmp.name)
        self.pin_rev = census.laptop_pin(self.root)
        self.assertNotEqual(self.pin_rev, "unreadable")
        self.pins = census.sandbox_pins(self.root)
        self.assertIn("tailscale", self.pins)

    def tearDown(self):
        self._tmp.cleanup()

    def run_report(self, running, laptop_revision):
        runner = FakeRunner(laptop_revision)
        code = report_mod.execute(
            self.root,
            14134,
            "developing-today/code",
            {"sandbox": {"running": running}},
            "gh",
            NOW,
            WINDOW,
            runner,
        )
        return code, runner

    def test_drifted_fixture_report_reaches_the_issue(self):
        running = dict(self.pins)
        running["tailscale"] = "0.0.1"
        code, runner = self.run_report(running, self.pin_rev)
        self.assertEqual(code, 0)
        self.assertEqual(len(runner.posts), 1)
        post = runner.posts[0]
        self.assertEqual(post[1:3], ["issue", "comment"])
        self.assertIn("14134", post)
        body = post[post.index("--body") + 1]
        self.assertIn("tailscale", body)
        self.assertIn("drifted", body)
        self.assertIn("Drift census — daily report", body)

    def test_converged_fixture_stays_silent(self):
        code, runner = self.run_report(dict(self.pins), self.pin_rev)
        self.assertEqual(code, 0)
        self.assertEqual(runner.posts, [])

    def test_only_read_probes_and_the_one_post_are_issued(self):
        running = dict(self.pins)
        running["tailscale"] = "0.0.1"
        _, runner = self.run_report(running, self.pin_rev)
        for argv in runner.calls:
            is_probe = "nixos-version" in argv
            is_census = any("drift-census.py" in a for a in argv)
            is_post = argv[0] == "gh" and argv[1:3] == ["issue", "comment"]
            self.assertTrue(is_probe or is_census or is_post, argv)


class FormatAndDeliverTests(unittest.TestCase):
    def synthetic_report(self):
        return {
            "total": 3,
            "counts": {"converged": 1, "pending": 0, "held": 1, "drifted": 1},
            "silent": False,
            "needs_you": ["bws", "fleet"],
            "rows": [
                {
                    "plane": "sandbox",
                    "service": "fleet",
                    "pinned": "2.0",
                    "running": "1.0",
                    "lag_s": None,
                    "state": "drifted",
                    "overrides": [],
                    "held": [],
                    "reason": "behind; lag unknown",
                },
                {
                    "plane": "sandbox",
                    "service": "bws",
                    "pinned": "2.0",
                    "running": "1.0",
                    "lag_s": 999999,
                    "state": "held",
                    "overrides": ["pin until fix"],
                    "held": [],
                    "reason": "held by override or held update",
                },
                {
                    "plane": "sandbox",
                    "service": "restic",
                    "pinned": "0.20.0",
                    "running": "0.20.0",
                    "lag_s": 0,
                    "state": "converged",
                    "overrides": [],
                    "held": [],
                    "reason": "",
                },
            ],
        }

    def test_format_names_held_and_drifted_rows_only(self):
        text = report_mod.format_report(self.synthetic_report(), "2026-11-26")
        self.assertIn("| sandbox | bws | held |", text)
        self.assertIn("| sandbox | fleet | drifted |", text)
        self.assertNotIn("restic", text)
        self.assertIn("Needs you: bws, fleet.", text)

    def test_deliver_posts_once_via_gh(self):
        runner = FakeRunner(None)
        text = report_mod.format_report(self.synthetic_report(), "2026-11-26")
        self.assertTrue(
            report_mod.deliver(text, "gh", 14134, "developing-today/code", runner)
        )
        self.assertEqual(len(runner.posts), 1)

    def test_deliver_none_never_posts(self):
        runner = FakeRunner(None)
        self.assertFalse(
            report_mod.deliver("x", "none", 14134, "developing-today/code", runner)
        )
        self.assertEqual(runner.calls, [])


class CensusFixtureTests(unittest.TestCase):
    """The census fixture observations slice 2 relies on."""

    def test_sandbox_observation_converges_and_unpinned_binary_is_out_of_scope(self):
        pins = census.sandbox_pins(ROOT)
        report = census.build(
            ["sandbox"],
            ROOT,
            {"sandbox": {"running": dict(pins)}},
            census.datetime.fromisoformat(NOW),
            WINDOW,
        )
        services = {row["service"] for row in report["rows"]}
        self.assertEqual(services, set(pins))
        self.assertNotIn("op", services)
        self.assertTrue(report["silent"])

    def test_laptop_observation_replaces_the_live_probe(self):
        with tempfile.TemporaryDirectory() as tmp:
            root = fixture_root(tmp)
            pin = census.laptop_pin(root)
            report = census.build(
                ["laptop"],
                root,
                {"laptop": {"running_revision": pin}},
                census.datetime.fromisoformat(NOW),
                WINDOW,
            )
            self.assertEqual(report["rows"][0]["state"], "converged")
            self.assertTrue(report["silent"])


class ReadOnlySourceTests(unittest.TestCase):
    """The wrapper never fixes: no system-mutating command in its source."""

    FORBIDDEN = (
        "nixos-rebuild",
        "systemctl",
        "wrangler deploy",
        "git push",
        "git reset",
        "git checkout",
        "git commit",
        "rmtree",
        "os.system",
        "shell=True",
    )

    def test_wrapper_source_has_no_mutating_command(self):
        source = (HERE / "fleet" / "drift-census-report.py").read_text()
        for pattern in self.FORBIDDEN:
            self.assertNotIn(pattern, source)


class RegistryAgreementTests(unittest.TestCase):
    """The registry row agrees with the schedule: cadence, timezone, command."""

    def test_registry_row_matches_the_schedule_constants(self):
        registry = (ROOT / "doc" / "fleet-schedule-registry.md").read_text()
        rows = [
            line
            for line in registry.splitlines()
            if f"`{report_mod.SCHEDULE_ID}`" in line
        ]
        self.assertEqual(len(rows), 1)
        row = rows[0]
        self.assertIn(report_mod.CADENCE, row)
        self.assertIn(report_mod.TIMEZONE, row)
        self.assertIn(report_mod.CENSUS_COMMAND_LINE, row)
        self.assertIn("scripts/fleet/drift-census-report.py", row)


class ArgumentContractTests(unittest.TestCase):
    def test_abbreviated_options_are_rejected(self):
        with contextlib.redirect_stderr(io.StringIO()):
            with self.assertRaises(SystemExit) as raised:
                report_mod.main(["--iss", "14134"])
        self.assertEqual(raised.exception.code, 2)


if __name__ == "__main__":
    unittest.main()
