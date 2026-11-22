#!/usr/bin/env python3
"""Tests for scripts/dump_line_report.py (#12000)."""

import subprocess
import sys
import tempfile
import unittest
from pathlib import Path

REPORT = Path(__file__).resolve().parent.parent / "scripts" / "dump_line_report.py"


def report(text, line):
    with tempfile.NamedTemporaryFile("w", suffix=".sql", delete=False) as fh:
        fh.write(text)
    try:
        return subprocess.run(
            [sys.executable, str(REPORT), fh.name, str(line)],
            capture_output=True,
            text=True,
            check=True,
        ).stdout.strip()
    finally:
        Path(fh.name).unlink()


class TestDumpLineReport(unittest.TestCase):
    def test_conflicting_rows_are_told_apart_from_repeats(self):
        dump = (
            "INSERT INTO t VALUES(7,'a');\n"
            "INSERT INTO t VALUES(70,'a');\n"
            "INSERT INTO t VALUES(7,'b');\n"
        )
        self.assertEqual(
            report(dump, 3),
            "line 3: INSERT into t; 1 identical statement(s) (first at line 3);"
            " 2 INSERT(s) share its first value",
        )

    def test_quoted_table_and_quoted_first_value(self):
        dump = (
            'INSERT INTO "my ""t""" VALUES(\'it\'\'s, here\',1);\n'
            'INSERT INTO "my ""t""" VALUES(\'it\'\'s, here\',1);\n'
            'INSERT INTO "my ""t""" VALUES(\'it\'\'s\',2);\n'
        )
        self.assertEqual(
            report(dump, 2),
            'line 2: INSERT into "my ""t"""; 2 identical statement(s) (first at line 1);'
            " 2 INSERT(s) share its first value",
        )

    def test_non_insert_and_past_end(self):
        self.assertEqual(
            report("CREATE TABLE t (x);\n", 1),
            "line 1: not an INSERT statement (19 bytes)",
        )
        self.assertEqual(report("x;\n", 5), "line 5: past the end of the dump")


if __name__ == "__main__":
    unittest.main()
