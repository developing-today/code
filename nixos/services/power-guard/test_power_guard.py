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
import threading
import time
import unittest
import unittest.mock
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


class FakeProc:
    """Minimal Popen stand-in for Inhibitor unit tests (#6534)."""

    def __init__(self, poll_result=None):
        self._poll_result = poll_result
        self.terminated = False
        self.killed = False
        self.waited = 0
        self.wait_return = poll_result

    def poll(self):
        return self._poll_result

    def wait(self, timeout=None):
        self.waited += 1
        return self.wait_return

    def terminate(self):
        self.terminated = True
        # Simulate a clean exit after terminate for the live path.
        if self._poll_result is None:
            self._poll_result = -15
            self.wait_return = -15

    def kill(self):
        self.killed = True


class InhibitorDeadChildTests(unittest.TestCase):
    """Oracle: issue #6534 acceptance — dead child detected/handled,
    live child undisturbed, NO_INHIBIT stub unchanged, release reaps."""

    def setUp(self):
        self.logs = []
        self._orig_log = pg.log
        pg.log = self.logs.append
        self.addCleanup(setattr, pg, "log", self._orig_log)

    def test_dead_child_is_detected_and_reengaged_with_exit_logged(self):
        # Repro from the issue: stub systemd-inhibit exits 1 at once.
        dead = FakeProc(poll_result=1)
        live = FakeProc(poll_result=None)
        inh = pg.Inhibitor()
        inh.proc = dead
        # held must reflect poll(), not handle non-None.
        self.assertFalse(inh.held)
        with unittest.mock.patch.object(
            pg.subprocess, "Popen", return_value=live
        ) as popen:
            inh.engage()
            popen.assert_called_once()
        self.assertIs(inh.proc, live)
        self.assertTrue(inh.held)
        self.assertGreaterEqual(dead.waited, 1, "dead child was not reaped")
        self.assertTrue(
            any("exit=1" in m for m in self.logs),
            f"exit status not logged loudly: {self.logs}",
        )

    def test_live_child_is_not_disturbed(self):
        live = FakeProc(poll_result=None)
        inh = pg.Inhibitor()
        inh.proc = live
        self.assertTrue(inh.held)
        with unittest.mock.patch.object(pg.subprocess, "Popen") as popen:
            inh.engage()
            popen.assert_not_called()
        self.assertIs(inh.proc, live)
        self.assertEqual(live.waited, 0)

    def test_stub_path_unchanged(self):
        orig = pg.NO_INHIBIT
        pg.NO_INHIBIT = True
        try:
            inh = pg.Inhibitor()
            inh.engage()
            self.assertEqual(inh.proc, "stub")
            self.assertTrue(inh.held)
            inh.engage()
            self.assertEqual(inh.proc, "stub")
            inh.release()
            self.assertIsNone(inh.proc)
            self.assertFalse(inh.held)
        finally:
            pg.NO_INHIBIT = orig

    def test_release_reaps_live_child(self):
        live = FakeProc(poll_result=None)
        inh = pg.Inhibitor()
        inh.proc = live
        inh.release()
        self.assertIsNone(inh.proc)
        self.assertFalse(inh.held)
        self.assertTrue(live.terminated)
        self.assertGreaterEqual(live.waited, 1)

    def test_release_reaps_already_dead_child(self):
        dead = FakeProc(poll_result=3)
        inh = pg.Inhibitor()
        inh.proc = dead
        inh.release()
        self.assertIsNone(inh.proc)
        self.assertFalse(dead.terminated, "terminate() must not run on a dead child")
        self.assertGreaterEqual(dead.waited, 1)
        self.assertTrue(
            any("exit=3" in m for m in self.logs),
            f"dead-child exit not logged on release: {self.logs}",
        )


class FakeStalledProc:
    """Child that ignores terminate and kill (#8183 regression).

    wait() raises TimeoutExpired on the caller's grace wait, then
    blocks inside the background reap until the test unblocks it,
    then raises TimeoutExpired again (the kill did not work either).
    """

    def __init__(self):
        self.terminated = False
        self.killed = False
        self.wait_calls = 0
        self.unblock = threading.Event()

    def poll(self):
        return None

    def terminate(self):
        self.terminated = True

    def kill(self):
        self.killed = True

    def wait(self, timeout=None):
        self.wait_calls += 1
        if self.wait_calls > 1:
            self.unblock.wait(timeout=10)
        raise subprocess.TimeoutExpired(cmd="fake-inhibit", timeout=timeout)


class InhibitorNonBlockingReapTests(unittest.TestCase):
    """Oracle: issue #8183 acceptance -- a stalled reap must not block
    the sampling loop for the full double-wait, the timeout/kill
    safeguards are preserved, and the lock is not believed released
    while the child is still running."""

    def setUp(self):
        self.logs = []
        self._orig_log = pg.log
        pg.log = self.logs.append
        self.addCleanup(setattr, pg, "log", self._orig_log)

    def test_stalled_child_reap_moves_off_the_calling_loop(self):
        proc = FakeStalledProc()
        inh = pg.Inhibitor()
        inh.proc = proc
        start = time.monotonic()
        settled = inh.release()
        elapsed = time.monotonic() - start
        self.assertFalse(settled, "stalled reap reported settled synchronously")
        self.assertLess(
            elapsed, 2.0, "release() blocked on the stalled child (old path: ~10s)"
        )
        self.assertTrue(proc.terminated)
        self.assertIsNone(inh.proc)
        self.assertFalse(inh.reap_settled(), "sleep could fire before release settles")
        self.assertTrue(inh.held, "lock believed released while child still runs")
        # Sampling continues while the reap is pending; once the
        # background escalation finishes, the release settles, the
        # kill safeguard has run, and the unconfirmed exit is loud.
        proc.unblock.set()
        deadline = time.time() + 5
        while not inh.reap_settled() and time.time() < deadline:
            inh.poll_reap()
            time.sleep(0.01)
        self.assertTrue(inh.reap_settled())
        self.assertTrue(proc.killed, "kill escalation was not preserved")
        self.assertGreaterEqual(proc.wait_calls, 2)
        inh.poll_reap()
        self.assertFalse(inh.held)
        self.assertTrue(
            any("UNCONFIRMED" in m for m in self.logs),
            f"unconfirmed release not logged loudly: {self.logs}",
        )


if __name__ == "__main__":
    unittest.main()
