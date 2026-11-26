"""sccache wedge watchdog for the cargo target broker (code#13367).

Oracle: the issue's acceptance shape. A wedged sccache client -- old
but ~0 CPU, waiting on a daemon reply that never arrives -- must fail
loudly within a bounded time (loud stderr report naming the crate plus
SIGTERM so cargo reports the unit failed), while healthy clients (young
idlers waiting on a cargo jobslot, CPU-burning compiles, fast exits)
are untouched. The bypass (RUSTC_WRAPPER= empty) disables the guard.
"""

from __future__ import annotations

import importlib.util
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "cargo_target_broker", HERE.parent / "bin" / "cargo_target_broker.py"
)
broker_mod = importlib.util.module_from_spec(SPEC)
sys.modules["cargo_target_broker"] = broker_mod
SPEC.loader.exec_module(broker_mod)

PATTERN = "sccache-watchdog-test"


def spawn_fake_client(kind: str, seconds: int) -> subprocess.Popen:
    """A process whose argv[0] basename matches the test pattern.

    `exec -a` sets argv[0] on a real python3 (which, unlike this host's
    multicall sleep, does not dispatch on argv[0]), so these tests can
    never match (or signal) a genuine sccache client. `kind` is "stall"
    (sleep: ~0 CPU, the wedge shape) or "burn" (busy loop: healthy).
    """
    if kind == "burn":
        command = f"exec -a {PATTERN} python3 -c 'import time; end=time.time()+{seconds}; exec(\"while time.time()<end: pass\")'"
    else:
        command = f"exec -a {PATTERN} python3 -c 'import time; time.sleep({seconds})'"
    return subprocess.Popen(["bash", "-c", command])


class FindWedgedTest(unittest.TestCase):
    def test_old_idle_is_wedged(self) -> None:
        clients = [
            broker_mod.SccacheClient(
                pid=1, age_secs=901.0, cpu_secs=0.2, cmdline=["sccache", "rustc"]
            )
        ]
        got = broker_mod.find_wedged_sccache_clients(clients, 900.0, 5.0)
        self.assertEqual([c.pid for c in got], [1])

    def test_young_idler_is_not_wedged(self) -> None:
        # Cargo jobslot waiters are idle but young: not wedges.
        clients = [
            broker_mod.SccacheClient(
                pid=2, age_secs=30.0, cpu_secs=0.0, cmdline=["sccache", "rustc"]
            )
        ]
        self.assertEqual(
            broker_mod.find_wedged_sccache_clients(clients, 900.0, 5.0), []
        )

    def test_old_cpu_burner_is_not_wedged(self) -> None:
        # A genuinely compiling rustc accrues CPU: not a wedge.
        clients = [
            broker_mod.SccacheClient(
                pid=3, age_secs=1800.0, cpu_secs=1700.0, cmdline=["sccache", "rustc"]
            )
        ]
        self.assertEqual(
            broker_mod.find_wedged_sccache_clients(clients, 900.0, 5.0), []
        )

    def test_crate_name_parsing(self) -> None:
        client = broker_mod.SccacheClient(
            pid=4,
            age_secs=901.0,
            cpu_secs=0.0,
            cmdline=["sccache", "rustc", "--crate-name", "backon", "--edition=2021"],
        )
        self.assertEqual(client.crate_name, "backon")
        report = broker_mod.sccache_wedge_report(client, 900.0)
        self.assertIn("backon", report)
        self.assertIn("RUSTC_WRAPPER=", report)


class WatchdogLiveTest(unittest.TestCase):
    def test_wedged_client_fails_loudly_within_bound(self) -> None:
        proc = spawn_fake_client("stall", 300)
        try:
            stop = threading.Event()
            terminated: list[int] = []
            watcher = threading.Thread(
                target=lambda: terminated.extend(
                    broker_mod.watch_sccache_clients(
                        stop,
                        stall_secs=2.0,
                        max_cpu_secs=5.0,
                        poll_secs=0.5,
                        pattern=PATTERN,
                    )
                ),
                daemon=True,
            )
            watcher.start()
            deadline = time.time() + 30.0
            while proc.poll() is None and time.time() < deadline:
                time.sleep(0.2)
            stop.set()
            watcher.join(timeout=10.0)
            self.assertIsNotNone(
                proc.poll(), "wedged fake client was not terminated within 30s"
            )
            self.assertIn(proc.pid, terminated)
        finally:
            if proc.poll() is None:
                proc.kill()
            proc.wait()

    def test_healthy_fast_client_untouched(self) -> None:
        proc = spawn_fake_client("stall", 1)
        rc = proc.wait(timeout=30)
        self.assertEqual(rc, 0)
        # A client that exits before the stall bound is never a wedge.
        clients = broker_mod.iter_sccache_clients(PATTERN)
        self.assertNotIn(proc.pid, [c.pid for c in clients])

    def test_cpu_burning_client_untouched(self) -> None:
        proc = spawn_fake_client("burn", 8)
        try:
            stop = threading.Event()
            terminated: list[int] = []
            watcher = threading.Thread(
                target=lambda: terminated.extend(
                    broker_mod.watch_sccache_clients(
                        stop,
                        stall_secs=2.0,
                        max_cpu_secs=1.0,
                        poll_secs=0.5,
                        pattern=PATTERN,
                    )
                ),
                daemon=True,
            )
            watcher.start()
            rc = proc.wait(timeout=30)
            time.sleep(2.0)  # let at least one more poll run past the bound
            stop.set()
            watcher.join(timeout=10.0)
            self.assertEqual(rc, 0)
            self.assertNotIn(proc.pid, terminated)
        finally:
            if proc.poll() is None:
                proc.kill()
            proc.wait()


class EnabledTest(unittest.TestCase):
    def test_opt_out_env_disables(self) -> None:
        self.assertFalse(
            broker_mod.sccache_watchdog_enabled(
                {"CARGO_BROKER_NO_SCCACHE_WATCHDOG": "1"}
            )
        )

    def test_bypass_disables(self) -> None:
        self.assertFalse(broker_mod.sccache_watchdog_enabled({"RUSTC_WRAPPER": ""}))
        self.assertFalse(
            broker_mod.sccache_watchdog_enabled({"CARGO_BUILD_RUSTC_WRAPPER": ""})
        )

    def test_default_enabled(self) -> None:
        self.assertTrue(broker_mod.sccache_watchdog_enabled({}))

    def test_bad_numeric_env_falls_back_to_defaults(self) -> None:
        stall, cpu, poll = broker_mod.sccache_watchdog_config(
            {"CARGO_BROKER_SCCACHE_STALL_SECS": "junk"}
        )
        self.assertEqual((stall, cpu, poll), (900.0, 5.0, 30.0))


if __name__ == "__main__":
    unittest.main()
