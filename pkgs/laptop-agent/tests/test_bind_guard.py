#!/usr/bin/env python3
"""Startup bind guard (#15533): the daemon must survive Tailscale not
being ready at boot.

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests -p "test_bind_guard.py"
"""

import os
import importlib.util
import tempfile
import unittest
from pathlib import Path
from unittest.mock import Mock, patch

AGENT = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"

# Scrubbing first: importing laptop_agent parses LAPTOP_AGENT_* at
# module load and exits 3 on garbage, so the importing environment
# must be clean (#15533 slice note).
_clean_env = {
    k: v
    for k, v in os.environ.items()
    if not k.startswith(("LAPTOP_AGENT_", "FLEET_CTX_"))
}
# Hermetic token fixture (#16558): the scrub above removes
# LAPTOP_AGENT_TOKEN_FILE, and laptop_agent reads its token at import
# time from the default $HOME/.config/laptop-agent/token, exiting 3
# when that file is missing — as it is on any runner with no live
# daemon, which turned this module into a unittest loader ERROR.
# Point HOME at a fixture dir carrying a token at the default path so
# the import is hermetic.
_fixture_home = Path(tempfile.mkdtemp(prefix="bind-guard-home-"))
_fixture_token = _fixture_home / ".config" / "laptop-agent" / "token"
_fixture_token.parent.mkdir(parents=True)
_fixture_token.write_text("bind-guard-test-token\n")
_clean_env["HOME"] = str(_fixture_home)
with patch.dict(os.environ, _clean_env, clear=True):
    _spec = importlib.util.spec_from_file_location("bind_guard_agent", AGENT)
    la = importlib.util.module_from_spec(_spec)
    _spec.loader.exec_module(la)


def _raise(err):
    raise err


class BindGuardTest(unittest.TestCase):
    """Oracle: only errno 99 (EADDRNOTAVAIL) retries, with a 1s doubling
    schedule capped at 15s inside a 10-minute budget; every other bind
    error raises on the first attempt."""

    def test_main_listens_after_address_appears(self):
        """Oracle: actual daemon startup retries the server constructor
        twice, preserves TLS arguments, then enters serve_forever."""
        server = Mock()
        with (
            patch.object(la, "STATE_DIR"),
            patch.object(la, "load_persisted_jobs"),
            patch.object(la, "reap_old_jobs"),
            patch.object(la.threading, "Thread"),
            patch.object(la.time, "sleep") as sleep,
            patch.object(
                la,
                "AgentServer",
                side_effect=[
                    OSError(99, "address absent"),
                    OSError(99, "address absent"),
                    server,
                ],
            ) as constructor,
        ):
            la.main()
        self.assertEqual(constructor.call_count, 3)
        constructor.assert_called_with(
            (la.BIND_HOST, la.BIND_PORT), la.Handler, tls_context=la.TLS_CONTEXT
        )
        self.assertEqual([c.args[0] for c in sleep.call_args_list], [1.0, 2.0])
        server.serve_forever.assert_called_once_with()

    def test_retryable_class_is_only_eaddrnotavail(self):
        self.assertTrue(
            la.is_retryable_bind_error(OSError(99, "Cannot assign requested address"))
        )
        for errno in (98, 13, 111, 22):  # in-use, denied, refused, invalid
            with self.subTest(errno=errno):
                self.assertFalse(la.is_retryable_bind_error(OSError(errno, "x")))

    def test_backoff_doubles_from_1s_to_a_15s_cap(self):
        self.assertEqual(
            [la.bind_retry_delay(a) for a in range(8)],
            [1.0, 2.0, 4.0, 8.0, 15.0, 15.0, 15.0, 15.0],
        )

    def test_ready_at_start_never_sleeps(self):
        """Oracle: an available address keeps the one-attempt startup path."""
        server = object()
        make = Mock(return_value=server)
        self.assertIs(
            la.bind_with_retry(make, sleep=lambda _: self.fail("slept")), server
        )
        make.assert_called_once_with()

    def test_absent_address_clips_sleep_to_budget(self):
        """Oracle: virtual attempts at 0,1,3,5 seconds, never beyond 5s."""
        clock = [0.0]
        sleeps = []
        error = OSError(99, "address absent")
        make = Mock(side_effect=error)

        def sleep(delay):
            sleeps.append(delay)
            clock[0] += delay

        with patch.object(la.time, "monotonic", side_effect=lambda: clock[0]):
            with self.assertRaises(OSError) as raised:
                la.bind_with_retry(make, budget_s=5, sleep=sleep)
        self.assertIs(raised.exception, error)
        self.assertEqual(sleeps, [1.0, 2.0, 2.0])
        self.assertEqual(make.call_count, 4)
        self.assertEqual(clock[0], 5.0)

    def test_flapping_bind_succeeds_after_two_retries(self):
        calls = []
        sleeps = []
        server = object()

        def make():
            calls.append(1)
            if len(calls) < 3:
                raise OSError(99, "Cannot assign requested address")
            return server

        got = la.bind_with_retry(make, budget_s=600.0, sleep=sleeps.append)
        self.assertIs(got, server)
        self.assertEqual(len(calls), 3)
        self.assertEqual(sleeps, [1.0, 2.0])

    def test_fatal_bind_error_fails_fast_with_one_attempt(self):
        calls = []

        def make():
            calls.append(1)
            _raise(OSError(98, "Address already in use"))

        with self.assertRaises(OSError):
            la.bind_with_retry(
                make,
                budget_s=600.0,
                sleep=lambda d: self.fail("must not sleep on fatal errors"),
            )
        self.assertEqual(len(calls), 1)

    def test_retryable_error_past_budget_raises_the_last_error(self):
        calls = []

        def make():
            calls.append(1)
            _raise(OSError(99, "Cannot assign requested address"))

        with self.assertRaises(OSError):
            la.bind_with_retry(
                make,
                budget_s=-1.0,  # already spent: one attempt, then raise
                sleep=lambda d: self.fail("must not sleep past the budget"),
            )
        self.assertEqual(len(calls), 1)


if __name__ == "__main__":
    unittest.main()
