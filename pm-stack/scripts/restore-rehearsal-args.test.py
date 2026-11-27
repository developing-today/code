#!/usr/bin/env python3
"""No-Docker argv regression: datetime.date is the Gregorian validity oracle.

Usage: python3 -I pm-stack/scripts/restore-rehearsal-args.test.py (no arguments).
Accepted dates reach the missing-backup guard; rejected argv must exit 2 without
external commands or scratch changes. A fixed seed makes generated cases replayable.
"""
import datetime
import os
from pathlib import Path
import random
import shutil
import subprocess
import sys
import tempfile
import unittest

SCRIPT = Path(__file__).with_name("restore-rehearsal.sh")
BASH = shutil.which("bash")


class ArgumentContract(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.home = Path(self.temp.name)
        self.bin = self.home / "bin"
        self.bin.mkdir()
        self.calls = self.home / "calls"
        fake = self.bin / "fake"
        fake.write_text(
            f"#!{BASH}\n"
            'printf "%s %s\\n" "${0##*/}" "$*" >>"$CALLS"\n'
            'if [[ ${0##*/} == date ]]; then\n'
            '  case "$*" in +%F) echo 2026-11-24;; +%s) echo 100;; *) exit 97;; esac\n'
            'else exit 97; fi\n'
        )
        fake.chmod(0o755)
        for name in ("date", "rm", "mkdir", "docker", "tar", "sqlite3", "curl", "cp", "sleep"):
            (self.bin / name).symlink_to(fake)
        self.scratch = self.home / "restore-rehearsal-6816"
        self.scratch.mkdir()
        (self.scratch / "sentinel").write_text("untouched")

    def check_argv(self, args, accepted, day=None, offending=None):
        self.calls.write_text("")
        result = subprocess.run(
            [BASH, str(SCRIPT), *args],
            env={"HOME": str(self.home), "PATH": str(self.bin), "CALLS": str(self.calls)},
            capture_output=True, text=True, timeout=5,
        )
        self.assertEqual(result.returncode, 1 if accepted else 2, result.stdout + result.stderr)
        if accepted:
            self.assertEqual(self.calls.read_text(), "date +%s\n" if args else "date +%F\ndate +%s\n")
            self.assertIn(f"/backups/pm-stack/{day}/pm-stack-{day}.tar.zst", result.stdout)
        else:
            self.assertEqual(self.calls.read_text(), "")
            self.assertIn("usage: restore-rehearsal.sh [YYYY-MM-DD]", result.stderr)
            self.assertIn(f"invalid argument: {offending if offending is not None else args[0]};", result.stderr)
        self.assertEqual((self.scratch / "sentinel").read_text(), "untouched")
        self.assertEqual(sorted(p.name for p in self.scratch.iterdir()), ["sentinel"])

    def test_default_and_calendar_model(self):
        self.check_argv([], True, day="2026-11-24")
        rng = random.Random(16090)
        dates = [(1, 1, 1), (9999, 12, 31), (0, 1, 1), (1900, 2, 29),
                 (2000, 2, 29), (2100, 2, 29), (2024, 2, 29), (2026, 8, 9),
                 (2026, 4, 31), (2026, 0, 1), (2026, 1, 0), (2026, 13, 1)]
        dates += [(rng.randrange(1, 10000), rng.randrange(0, 14), rng.randrange(0, 34))
                  for _ in range(100)]
        for year, month, day in dates:
            text = f"{year:04}-{month:02}-{day:02}"
            try:
                datetime.date(year, month, day)
                accepted = True
            except ValueError:
                accepted = False
            with self.subTest(date=text):
                self.check_argv([text], accepted, day=text)

    def test_unknown_malformed_and_extra_arguments(self):
        for text in ("", "--help", "--", "--date=2026-11-24", "-n", "2026-1-01",
                     "2026-11-24 ", " 2026-11-24", "2026-11-24/../../", "2026-11-24\n"):
            with self.subTest(argument=text):
                self.check_argv([text], False)
        for args in (["2026-11-24", "extra"], ["2026-11-24", "--help"],
                     ["bad", "extra"], ["", "extra"], ["2026-11-24", "", "third"]):
            with self.subTest(arguments=args):
                self.check_argv(args, False, offending=args[1])


if __name__ == "__main__":
    if len(sys.argv) != 1:
        print(f"unexpected argument: {sys.argv[1]}", file=sys.stderr)
        sys.exit(2)
    unittest.main()
