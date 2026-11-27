#!/usr/bin/env python3
"""Cutover readiness gate for code#16089 (oc-code-06).

Oracle: the public verbs/flags documented by pkgs/fleet/laptop-agent.py.
A domain appearing in capabilities is NOT sufficient evidence for cutover.

Contract: no arguments runs hermetic unittest cases; --probe FLEET checks
that binary's advertised contract without contacting the laptop. Unknown
arguments fail before subprocess or filesystem activity. A successful probe
is necessary, not sufficient: argv/env/exit and live transport parity still
need verification before the installed Python client can be retired.
"""

import argparse
import json
import subprocess
import unittest
from unittest.mock import patch


# Keep legacy optional features: dropping these silently breaks existing callers.
REQUIRED = (
    "health", "jobs", "kill", "exec", "exec-batch", "run", "poll", "fetch",
    "--cwd", "--timeout", "--shell", "--wait", "--chunk",
)


def check_help(text):
    """A conservative advertised-surface gate, never a transport/parity claim."""
    import re

    words = set(re.findall(r"--[a-z-]+|[a-z]+(?:-[a-z]+)*", text))
    return [word for word in REQUIRED if word not in words]


def probe(binary):
    # Leaf help is local parsing only, unlike health/exec. Never mutate wrappers.
    result = subprocess.run(
        [binary, "laptop-agent", "exec", "--help"],
        capture_output=True, text=True, timeout=15, check=False,
    )
    if result.returncode:
        raise RuntimeError(f"help exited {result.returncode}: {result.stderr.strip()}")
    return check_help(result.stdout)


class CutoverReadinessTests(unittest.TestCase):
    """Oracle: exact legacy contract tokens, not substring or domain presence."""

    def test_complete_contract_is_necessary_gate(self):
        self.assertEqual(check_help(" ".join(REQUIRED)), [])

    def test_deployed_rust_contract_is_not_ready(self):
        # fleet 74eee892 help observed 2026-11-27; these two APIs are absent
        # in crates/fleet-laptop-agent/src/cli.rs too. No active-path cutover.
        current = " ".join(x for x in REQUIRED if x not in ("exec-batch", "--chunk"))
        self.assertEqual(check_help(current), ["exec-batch", "--chunk"])

    def test_each_required_token_is_independently_checked(self):
        for omitted in REQUIRED:
            with self.subTest(omitted=omitted):
                self.assertEqual(
                    check_help(" ".join(x for x in REQUIRED if x != omitted)),
                    [omitted],
                )

    def test_similar_names_do_not_pass(self):
        self.assertIn("exec", check_help("exec-batch execute"))
        self.assertIn("--chunk", check_help("--chunks --chunk-size"))

    def test_probe_only_invokes_local_help(self):
        with patch("subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 0, " ".join(REQUIRED), "")
            self.assertEqual(probe("/path with spaces/fleet"), [])
            run.assert_called_once_with(
                ["/path with spaces/fleet", "laptop-agent", "exec", "--help"],
                capture_output=True, text=True, timeout=15, check=False,
            )

    def test_failed_help_cannot_pass(self):
        with patch("subprocess.run") as run:
            run.return_value = subprocess.CompletedProcess([], 69, " ".join(REQUIRED), "unavailable")
            with self.assertRaisesRegex(RuntimeError, "help exited 69"):
                probe("fleet")


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--probe", metavar="FLEET", help="check deployed help; no network calls")
    args = parser.parse_args()
    if args.probe:
        try:
            missing = probe(args.probe)
        except (OSError, RuntimeError, subprocess.TimeoutExpired) as error:
            print(json.dumps({"ready": False, "error": str(error)}))
            return 1
        print(json.dumps({"ready": not missing, "missing": missing,
                          "scope": "advertised CLI only; not live parity"}))
        return int(bool(missing))
    result = unittest.TextTestRunner(verbosity=2).run(
        unittest.defaultTestLoader.loadTestsFromTestCase(CutoverReadinessTests)
    )
    return int(not result.wasSuccessful())


if __name__ == "__main__":
    raise SystemExit(main())
