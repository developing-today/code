#!/usr/bin/env python3
"""No-Docker argv regression: datetime.date is the Gregorian validity oracle.

Usage: python3 -I pm-stack/scripts/restore-rehearsal-args.test.py (no arguments).
Accepted argv reaches the missing-backup guard (exit 1, real
restore-point-check.py gate passes against the fresh fixture record);
rejected argv must exit 2 without external commands or scratch changes.
A fixed seed makes generated dates replayable.

The backup-record flag is part of the accepted contract: argv without
--backup-record-file is rejected, and a stale fixture record is REFUSED
(exit 3) by the real gate — pinning the restore-point wiring, not a stub.
"""

import datetime
import json
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
REAL_PYTHON = shutil.which("python3")


def write_record(path, *, result="complete", age_minutes=5, r2="ok",
                 digest=("a" * 64)):
    finished = datetime.datetime.now(datetime.timezone.utc) - datetime.timedelta(
        minutes=age_minutes
    )
    record = {
        "date": finished.date().isoformat(),
        "finishedAt": finished.isoformat(),
        "result": result,
        "archiveSha256": digest,
        "destinations": {"local": "ok", "r2": r2, "drive": "ok"},
    }
    Path(path).write_text(json.dumps(record))


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
            "if [[ ${0##*/} == date ]]; then\n"
            '  case "$*" in +%F) echo 2026-11-24;; +%s) echo 100;; *) exit 97;; esac\n'
            "else exit 97; fi\n"
        )
        fake.chmod(0o755)
        for name in (
            "date",
            "rm",
            "mkdir",
            "docker",
            "tar",
            "sqlite3",
            "curl",
            "cp",
            "sleep",
        ):
            (self.bin / name).symlink_to(fake)
        # cd and pwd are shell builtins (PATH-proof); dirname is an external
        # command the driver needs to locate its own directory, so it
        # resolves to the real binary and the fixture gate path is genuine.
        (self.bin / "dirname").symlink_to(shutil.which("dirname"))
        # The gate runs for real: python3 resolves to the real interpreter
        # so the fixture backup records are validated, not stubbed.
        (self.bin / "python3").symlink_to(REAL_PYTHON)
        self.fresh = self.home / "record-fresh.json"
        write_record(self.fresh)
        self.stale = self.home / "record-stale.json"
        write_record(self.stale, age_minutes=37 * 60)
        self.scratch = self.home / "restore-rehearsal-6816"
        self.scratch.mkdir()
        (self.scratch / "sentinel").write_text("untouched")

    def run_argv(self, args):
        self.calls.write_text("")
        return subprocess.run(
            [BASH, str(SCRIPT), *args],
            env={
                "HOME": str(self.home),
                "PATH": str(self.bin),
                "CALLS": str(self.calls),
            },
            capture_output=True,
            text=True,
            timeout=30,
        )

    def check_argv(self, args, accepted, day=None, fragment=None,
                   date_given=True):
        result = self.run_argv(
            ["--backup-record-file", str(self.fresh), *args]
        )
        if accepted:
            self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
            self.assertEqual(
                self.calls.read_text(),
                "date +%s\n" if date_given else "date +%F\ndate +%s\n",
            )
            self.assertIn(
                f"/backups/pm-stack/{day}/pm-stack-{day}.tar.zst", result.stdout
            )
        else:
            self.assertEqual(result.returncode, 2, result.stdout + result.stderr)
            self.assertEqual(self.calls.read_text(), "")
            self.assertIn(
                "usage: restore-rehearsal.sh --backup-record-file FILE",
                result.stderr,
            )
            self.assertIn(fragment, result.stderr)
        self.assertEqual((self.scratch / "sentinel").read_text(), "untouched")
        self.assertEqual(sorted(p.name for p in self.scratch.iterdir()), ["sentinel"])

    def test_default_and_calendar_model(self):
        self.check_argv([], True, day="2026-11-24", date_given=False)
        rng = random.Random(16090)
        dates = [
            (1, 1, 1),
            (9999, 12, 31),
            (0, 1, 1),
            (1900, 2, 29),
            (2000, 2, 29),
            (2100, 2, 29),
            (2024, 2, 29),
            (2026, 8, 9),
            (2026, 4, 31),
            (2026, 0, 1),
            (2026, 1, 0),
            (2026, 13, 1),
        ]
        dates += [
            (rng.randrange(1, 10000), rng.randrange(0, 14), rng.randrange(0, 34))
            for _ in range(100)
        ]
        for year, month, day in dates:
            text = f"{year:04}-{month:02}-{day:02}"
            try:
                datetime.date(year, month, day)
                accepted = True
                fragment = None
            except ValueError:
                accepted = False
                fragment = f"invalid date '{text}'"
            with self.subTest(date=text):
                self.check_argv([text], accepted, day=text, fragment=fragment)

    def test_unknown_malformed_and_extra_arguments(self):
        cases = [
            ("", "invalid date ''"),
            ("--help", "unexpected argument '--help'"),
            ("--", "unexpected argument '--'"),
            ("--date=2026-11-24", "unexpected argument '--date=2026-11-24'"),
            ("-n", "invalid date '-n'"),
            ("2026-1-01", "invalid date '2026-1-01'"),
            ("2026-11-24 ", "invalid date '2026-11-24 '"),
            (" 2026-11-24", "invalid date ' 2026-11-24'"),
            ("2026-11-24/../../", "invalid date '2026-11-24/../../'"),
            ("2026-11-24\n", "invalid date '2026-11-24\n'"),
        ]
        for text, fragment in cases:
            with self.subTest(argument=text):
                self.check_argv([text], False, fragment=fragment)
        for args, fragment in (
            (["2026-11-24", "extra"], "at most one date"),
            (["2026-11-24", "--help"], "unexpected argument '--help'"),
            (["bad", "extra"], "at most one date"),
            (["", "extra"], "at most one date"),
            (["2026-11-24", "", "third"], "at most one date"),
        ):
            with self.subTest(arguments=args):
                self.check_argv(args, False, fragment=fragment)

    def test_flag_after_date_is_accepted(self):
        result = self.run_argv(
            ["2026-11-24", "--backup-record-file", str(self.fresh)]
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)
        self.assertIn("pm-stack-2026-11-24.tar.zst", result.stdout)

    def test_equals_form_is_accepted(self):
        result = self.run_argv(
            [f"--backup-record-file={self.fresh}", "--max-backup-age-hours=36"]
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_stale_record_is_refused_before_any_work(self):
        result = self.run_argv(
            ["--backup-record-file", str(self.stale), "2026-11-24"]
        )
        self.assertEqual(result.returncode, 3, result.stdout + result.stderr)
        self.assertIn("REFUSED", result.stderr)
        self.assertEqual(self.calls.read_text(), "")
        self.assertEqual((self.scratch / "sentinel").read_text(), "untouched")
        self.assertEqual(sorted(p.name for p in self.scratch.iterdir()), ["sentinel"])

    def test_raised_max_age_honors_a_stale_record(self):
        result = self.run_argv(
            [
                "--backup-record-file",
                str(self.stale),
                "--max-backup-age-hours",
                "48",
                "2026-11-24",
            ]
        )
        self.assertEqual(result.returncode, 1, result.stdout + result.stderr)

    def test_backup_record_flag_contract(self):
        cases = [
            ([], "--backup-record-file is required"),
            (["2026-11-24"], "--backup-record-file is required"),
            (
                ["--backup-record-file", "2026-11-24"],
                "backup record file '2026-11-24' is not readable",
            ),
            (
                ["--backup-record-file"],
                "--backup-record-file requires a value",
            ),
            (
                ["--backup-record-file="],
                "--backup-record-file requires a non-empty value",
            ),
            (
                [
                    "--backup-record-file",
                    str(self.fresh),
                    "--max-backup-age-hours",
                    "0",
                ],
                "--max-backup-age-hours must be a positive integer",
            ),
            (
                [
                    "--backup-record-file",
                    str(self.fresh),
                    "--max-backup-age-hours",
                    "soon",
                ],
                "--max-backup-age-hours must be a positive integer",
            ),
            (
                ["--backup-record-file", str(self.fresh), "--max-backup-age-hours"],
                "--max-backup-age-hours requires a value",
            ),
        ]
        for args, fragment in cases:
            with self.subTest(arguments=args):
                result = self.run_argv(args)
                self.assertEqual(
                    result.returncode, 2, result.stdout + result.stderr
                )
                self.assertIn(
                    "usage: restore-rehearsal.sh --backup-record-file FILE",
                    result.stderr,
                )
                self.assertIn(fragment, result.stderr)
                self.assertEqual(self.calls.read_text(), "")
                self.assertFalse(
                    (self.home / "restore-rehearsal-6816" / "src").exists()
                )


if __name__ == "__main__":
    if len(sys.argv) != 1:
        print(f"unexpected argument: {sys.argv[1]}", file=sys.stderr)
        sys.exit(2)
    unittest.main()
