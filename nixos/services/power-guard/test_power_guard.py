#!/usr/bin/env python3
"""Tests for power-guard timing-override validation (#6477).

Oracle: the acceptance criteria of #6477 plus the daemon's defining
invariant from #1687 — hybrid-sleep may only fire after a *sustained*
critical state, so a timing override is valid only as a finite,
strictly positive number of seconds no larger than the largest
interval time.sleep can represent. Invalid overrides are decided by
the parser contract (named startup error, non-zero exit, before
polling, marker file from the stubbed sleep command never written);
valid behaviour is decided by the fake-sysfs probe scenarios A-E from
the issue, run against the real daemon subprocess through its
documented POWER_GUARD_SYSFS / POWER_GUARD_NO_INHIBIT /
POWER_GUARD_SLEEP_CMD hooks.
"""

import importlib.util
import math
import os
import random
import subprocess
import sys
import tempfile
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
DAEMON = HERE / "power-guard.py"

TIMING_VARS = ("POWER_GUARD_POLL", "POWER_GUARD_CONFIRM", "POWER_GUARD_EDGE_CONFIRM")
BAD_VALUES = ["abc", "", "nan", "NaN", "inf", "-inf", "0", "0.0", "-1", "-5", "1e10", "1e100"]


def load_daemon():
    """Import power-guard.py with a clean, valid timing environment."""
    env_backup = {k: os.environ.get(k) for k in TIMING_VARS}
    for k in TIMING_VARS:
        os.environ.pop(k, None)
    try:
        spec = importlib.util.spec_from_file_location("power_guard", DAEMON)
        mod = importlib.util.module_from_spec(spec)
        spec.loader.exec_module(mod)
        return mod
    finally:
        for k, v in env_backup.items():
            if v is None:
                os.environ.pop(k, None)
            else:
                os.environ[k] = v


pg = load_daemon()


class ParseTimingTests(unittest.TestCase):
    def test_valid_values(self):
        for raw, want in [("15", 15.0), ("0.2", 0.2), ("300", 300.0), ("1e2", 100.0)]:
            with self.subTest(raw=raw):
                self.assertEqual(pg.parse_timing("POWER_GUARD_POLL", raw), want)

    def test_defaults_are_valid_and_unchanged(self):
        self.assertEqual(pg.POLL, 15.0)
        self.assertEqual(pg.CONFIRM, 300.0)
        self.assertEqual(pg.EDGE_CONFIRM, 60.0)

    def test_bad_values_rejected_naming_variable_and_value(self):
        for var in TIMING_VARS:
            for raw in BAD_VALUES:
                with self.subTest(var=var, raw=raw):
                    with self.assertRaises(ValueError) as ctx:
                        pg.parse_timing(var, raw)
                    self.assertIn(var, str(ctx.exception))
                    self.assertIn(repr(raw), str(ctx.exception))


class ParseTimingPropertyTests(unittest.TestCase):
    """Property-based coverage of the parser invariant (Codex #6485).

    Oracle: the acceptance predicate itself, evaluated
    independently of parse_timing's control flow — a generated input
    is acceptable iff float() parses it to a finite value > 0 and
    no larger than MAX_TIMING_SECONDS (the time.sleep bound).
    Generation is seeded, so failures reproduce exactly.
    """

    def oracle_accepts(self, raw):
        try:
            value = float(raw)
        except (TypeError, ValueError):
            return False
        return math.isfinite(value) and 0 < value <= pg.MAX_TIMING_SECONDS

    @staticmethod
    def generated_inputs(rng):
        cases = []
        for _ in range(200):  # plausible valid timings, many formats
            v = rng.choice(
                [
                    rng.uniform(0.001, 100000),
                    rng.randint(1, 100000),
                    rng.uniform(0.001, 1),
                ]
            )
            cases.append(rng.choice([repr(v), f"{v:.6f}", f"{v:e}", f" {v} "]))
        for _ in range(200):  # invalid: zero/negative/non-finite/junk
            cases.append(
                rng.choice(
                    [
                        repr(-rng.uniform(0, 100000)),
                        "0",
                        "-0.0",
                        rng.choice(["nan", "NaN", "inf", "-inf", "infinity"]),
                        "".join(rng.choices("abcxyz_=!@#", k=rng.randint(1, 8))),
                        f"{rng.uniform(2, 9)}e{rng.randint(10, 100)}",
                        f"{rng.uniform(1, 9)}x{rng.randint(0, 9)}",
                        "",
                    ]
                )
            )
        return cases

    def test_generated_inputs_match_finite_positive_predicate(self):
        rng = random.Random(6477)
        for raw in self.generated_inputs(rng):
            with self.subTest(raw=raw):
                if self.oracle_accepts(raw):
                    self.assertEqual(
                        pg.parse_timing("POWER_GUARD_CONFIRM", raw), float(raw)
                    )
                else:
                    with self.assertRaises(ValueError) as ctx:
                        pg.parse_timing("POWER_GUARD_CONFIRM", raw)
                    self.assertIn("POWER_GUARD_CONFIRM", str(ctx.exception))
                    self.assertIn(repr(raw), str(ctx.exception))


def make_fake_sysfs(root: Path, capacity=3, status="Discharging", ac_online=0):
    ac = root / "ACAD"
    ac.mkdir(parents=True)
    (ac / "type").write_text("Mains\n")
    (ac / "online").write_text(f"{ac_online}\n")
    bat = root / "BAT0"
    bat.mkdir()
    (bat / "type").write_text("Battery\n")
    (bat / "capacity").write_text(f"{capacity}\n")
    (bat / "status").write_text(f"{status}\n")


class DaemonSubprocessTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        base = Path(self.tmp.name)
        self.sysfs = base / "sysfs"
        make_fake_sysfs(self.sysfs)
        self.marker = base / "slept"

    def daemon_env(self, **timing):
        env = dict(os.environ)
        for k in TIMING_VARS:
            env.pop(k, None)
        env.update(
            POWER_GUARD_SYSFS=str(self.sysfs),
            POWER_GUARD_NO_INHIBIT="1",
            POWER_GUARD_SLEEP_CMD=f"touch {self.marker}",
        )
        env.update(timing)
        return env

    def run_daemon(self, env, seconds):
        proc = subprocess.Popen(
            [sys.executable, str(DAEMON)],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            out, err = proc.communicate(timeout=seconds)
            return proc.returncode, out, err, True
        except subprocess.TimeoutExpired:
            proc.terminate()
            out, err = proc.communicate(timeout=10)
            return proc.returncode, out, err, False

    def test_invalid_overrides_exit_nonzero_named_no_traceback_no_fire(self):
        cases = [
            {"POWER_GUARD_CONFIRM": "0"},
            {"POWER_GUARD_CONFIRM": "-5"},
            {"POWER_GUARD_CONFIRM": "abc"},
            {"POWER_GUARD_EDGE_CONFIRM": "0"},
            {"POWER_GUARD_EDGE_CONFIRM": "nan"},
            {"POWER_GUARD_POLL": "abc"},
            {"POWER_GUARD_POLL": "-1"},
            {"POWER_GUARD_POLL": "inf"},
            {"POWER_GUARD_POLL": "1e100"},
        ]
        for timing in cases:
            var = next(iter(timing))
            with self.subTest(**timing):
                rc, out, err, exited = self.run_daemon(self.daemon_env(**timing), 10)
                self.assertTrue(exited, "daemon kept polling on an invalid override")
                self.assertNotEqual(rc, 0)
                self.assertIn(var, err)
                self.assertIn(repr(timing[var]), err)
                self.assertNotIn("Traceback", err)
                self.assertFalse(self.marker.exists(), "sleep command fired")

    def test_valid_confirm_300_does_not_fire_on_first_samples(self):
        # Issue probe B (control): sustained requirement intact.
        env = self.daemon_env(POWER_GUARD_POLL="0.05", POWER_GUARD_CONFIRM="300")
        rc, out, err, exited = self.run_daemon(env, 1.0)
        self.assertFalse(exited, "daemon should still be polling")
        self.assertIn("CRITICAL state entered", out)
        self.assertFalse(self.marker.exists())

    def test_defaults_do_not_fire_and_report_documented_timings(self):
        rc, out, err, exited = self.run_daemon(self.daemon_env(), 1.0)
        self.assertFalse(exited, "daemon should still be polling")
        self.assertIn("poll=15.0s", out)
        self.assertIn("confirm=300.0s", out)
        self.assertIn("edge_confirm=60.0s", out)
        self.assertFalse(self.marker.exists())

    def test_valid_small_confirm_still_fires_after_sustained_critical(self):
        # Valid overrides behave as before: a genuinely sustained
        # critical state (CONFIRM=0.3s of continuous 3% samples) fires.
        env = self.daemon_env(POWER_GUARD_POLL="0.05", POWER_GUARD_CONFIRM="0.3")
        deadline = time.time() + 10
        proc = subprocess.Popen(
            [sys.executable, str(DAEMON)],
            env=env,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            text=True,
        )
        try:
            while time.time() < deadline and not self.marker.exists():
                if proc.poll() is not None:
                    break
                time.sleep(0.05)
        finally:
            proc.terminate()
            proc.communicate(timeout=10)
        self.assertTrue(self.marker.exists(), "sustained critical state never fired")


if __name__ == "__main__":
    unittest.main()
