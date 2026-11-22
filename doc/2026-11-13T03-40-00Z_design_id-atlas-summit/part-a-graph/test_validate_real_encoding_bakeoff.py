#!/usr/bin/env python3
"""Tests for the layout-independent report comparison (stdlib only).

Oracle: a Markdown table says the same thing however its cells are
padded, and something different as soon as one cell's text changes. The
repository formatter re-pads tables on main, so the report check must
pass on a re-padded table and still fail on a changed number.
"""

from __future__ import annotations

import importlib.util
import unittest
from pathlib import Path

HERE = Path(__file__).resolve().parent
SPEC = importlib.util.spec_from_file_location(
    "bakeoff", HERE / "validate_real_encoding_bakeoff.py"
)
bakeoff = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(bakeoff)

TIGHT = "| engine | load µs | bytes |\n|---|---:|---:|\n| `a` | 7982 | 312475 |\n| `b` | 199808 | 1118208 |"
PADDED = (
    "\n"
    "| engine | load µs |   bytes |\n"
    "| ------ | ------: | ------: |\n"
    "| `a`    |    7982 |  312475 |\n"
    "| `b`    |  199808 | 1118208 |\n"
)


class TableCells(unittest.TestCase):
    def test_padding_rule_width_and_blank_lines_do_not_change_the_cells(self) -> None:
        self.assertEqual(bakeoff.table_cells(TIGHT), bakeoff.table_cells(PADDED))
        self.assertEqual(
            bakeoff.table_cells(PADDED),
            [
                ["engine", "load µs", "bytes"],
                ["`a`", "7982", "312475"],
                ["`b`", "199808", "1118208"],
            ],
        )

    def test_a_changed_number_a_missing_row_or_stray_text_is_a_difference(self) -> None:
        base = bakeoff.table_cells(TIGHT)
        self.assertNotEqual(
            bakeoff.table_cells(PADDED.replace("312475", "312476")), base
        )
        self.assertNotEqual(
            bakeoff.table_cells("\n".join(PADDED.splitlines()[:-1])), base
        )
        self.assertNotEqual(bakeoff.table_cells(PADDED + "edited by hand\n"), base)
        # A reordered pair of rows is a different table too.
        lines = TIGHT.splitlines()
        self.assertNotEqual(
            bakeoff.table_cells("\n".join(lines[:2] + [lines[3], lines[2]])), base
        )

    def test_list_items_ignore_blank_lines_and_indentation_only(self) -> None:
        self.assertEqual(
            bakeoff.list_items("\n- load µs: `a`\n\n  - bytes: `a`\n"),
            ["- load µs: `a`", "- bytes: `a`"],
        )
        self.assertNotEqual(
            bakeoff.list_items("- load µs: `b`"), bakeoff.list_items("- load µs: `a`")
        )


class CommittedReport(unittest.TestCase):
    def test_the_committed_report_matches_the_committed_results(self) -> None:
        results = bakeoff.load_json(bakeoff.RESULTS)
        bakeoff.check_report(results)

    def test_a_report_with_one_changed_cell_is_refused(self) -> None:
        results = bakeoff.load_json(bakeoff.RESULTS)
        engine = bakeoff.ENGINES[0]
        rows = bakeoff.table_values(results)
        original = bakeoff.REPORT
        text = original.read_text(encoding="utf-8")
        needle = f" {rows[engine][0]} "
        self.assertIn(needle, text)
        changed = HERE / "real-encoding-bakeoff.changed-for-test.md"
        changed.write_text(
            text.replace(needle, f" {rows[engine][0] + 1} ", 1), encoding="utf-8"
        )
        try:
            bakeoff.REPORT = changed
            with self.assertRaisesRegex(bakeoff.Invalid, "table differs"):
                bakeoff.check_report(results)
        finally:
            bakeoff.REPORT = original
            changed.unlink()


if __name__ == "__main__":
    unittest.main()
