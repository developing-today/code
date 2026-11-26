#!/usr/bin/env python3
"""Startup bind guard (#15533): the daemon must survive Tailscale not
being ready at boot.

Run with: python3 -m unittest discover -s pkgs/laptop-agent/tests -p "test_bind_guard.py"
"""

import os
import sys
import unittest
from pathlib import Path

AGENT = Path(__file__).resolve().parents[1] / "src" / "laptop_agent.py"

# Scrubbing first: importing laptop_agent parses LAPTOP_AGENT_* at
# module load and exits 3 on garbage, so the importing environment
# must be clean (#15533 slice note).
for _var in list(os.environ):
    if _var.startswith("LAPTOP_AGENT_") or _var.startswith("FLEET_CTX_"):
        del os.environ[_var]
sys.path.insert(0, str(AGENT.parent))
import laptop_agent as la  # noqa: E402


def _raise(err):
    raise err


class BindGuardTest(unittest.TestCase):
    """Oracle: only errno 99 (EADDRNOTAVAIL) retries, with a 1s doubling
    schedule capped at 15s inside a 10-minute budget; every other bind
    error raises on the first attempt."""

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
