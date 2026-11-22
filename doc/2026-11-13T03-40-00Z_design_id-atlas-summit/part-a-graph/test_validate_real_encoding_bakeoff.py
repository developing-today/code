#!/usr/bin/env python3
"""Tests for the layout-independent report comparison (stdlib only).

Oracle: a Markdown table says the same thing however its cells are
padded, and something different as soon as one cell's text changes. The
repository formatter re-pads tables on main, so the report check must
pass on a re-padded table and still fail on a changed number.
"""

from __future__ import annotations

import importlib.util
import random
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


CELL_ALPHABET = "abcxyzABC0123456789µ`_. /"
SEED = 10219
CASES = 2000


def random_matrix(rng: random.Random) -> list[list[str]]:
    """A header row and zero to five body rows, one to six cells each."""
    columns = rng.randint(1, 6)

    def cell() -> str:
        return "".join(
            rng.choice(CELL_ALPHABET) for _ in range(rng.randint(0, 9))
        ).strip()

    return [[cell() for _ in range(columns)] for _ in range(rng.randint(1, 6))]


def random_layout(rng: random.Random, matrix: list[list[str]]) -> str:
    """The matrix as a Markdown table under a random layout.

    Everything the formatter may vary is drawn at random: the padding on
    each side of every cell, the width and alignment colons of the rule
    row, and blank lines and indentation around the table.
    """

    def row(cells: list[str]) -> str:
        return (
            "|"
            + "|".join(
                " " * rng.randint(0, 4) + c + " " * rng.randint(0, 4) for c in cells
            )
            + "|"
        )

    def rule_cell() -> str:
        return rng.choice(["", ":"]) + "-" * rng.randint(1, 12) + rng.choice(["", ":"])

    lines = [row(matrix[0]), row([rule_cell() for _ in matrix[0]])] + [
        row(r) for r in matrix[1:]
    ]
    lines = [" " * rng.randint(0, 3) + line for line in lines]
    return "\n" * rng.randint(0, 3) + "\n".join(lines) + "\n" * rng.randint(0, 3)


class LayoutProperty(unittest.TestCase):
    """Property tests with a seeded generator (stdlib only).

    Oracle: the generating model. A table is rendered from a cell matrix,
    so the matrix is the expected answer for every layout of it.
    """

    def test_any_layout_of_a_table_parses_back_to_its_cells(self) -> None:
        rng = random.Random(SEED)
        for case in range(CASES):
            matrix = random_matrix(rng)
            rendered = random_layout(rng, matrix)
            self.assertEqual(
                bakeoff.table_cells(rendered),
                matrix,
                f"seed {SEED} case {case}: {rendered!r}",
            )

    def test_two_layouts_of_one_table_always_compare_equal(self) -> None:
        rng = random.Random(SEED + 1)
        for case in range(CASES):
            matrix = random_matrix(rng)
            first, second = random_layout(rng, matrix), random_layout(rng, matrix)
            self.assertEqual(
                bakeoff.table_cells(first),
                bakeoff.table_cells(second),
                f"seed {SEED + 1} case {case}",
            )

    def test_changing_any_one_cell_is_always_a_difference(self) -> None:
        rng = random.Random(SEED + 2)
        for case in range(CASES):
            matrix = random_matrix(rng)
            changed = [list(r) for r in matrix]
            r = rng.randrange(len(changed))
            c = rng.randrange(len(changed[r]))
            changed[r][c] = changed[r][c] + "7"
            self.assertNotEqual(
                bakeoff.table_cells(random_layout(rng, changed)),
                bakeoff.table_cells(random_layout(rng, matrix)),
                f"seed {SEED + 2} case {case}",
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

    def test_a_report_with_one_changed_leader_is_refused(self) -> None:
        results = bakeoff.load_json(bakeoff.RESULTS)
        original = bakeoff.REPORT
        text = original.read_text(encoding="utf-8")
        leaders = bakeoff.block(text, "leaders")
        first = bakeoff.list_items(leaders)[0]
        name = first.split("`")[1]
        other = next(e for e in bakeoff.ENGINES if e != name)
        changed = HERE / "real-encoding-bakeoff.changed-for-test.md"
        changed.write_text(
            text.replace(first, first.replace(name, other), 1), encoding="utf-8"
        )
        try:
            bakeoff.REPORT = changed
            with self.assertRaisesRegex(bakeoff.Invalid, "leaders differ"):
                bakeoff.check_report(results)
        finally:
            bakeoff.REPORT = original
            changed.unlink()


if __name__ == "__main__":
    unittest.main()
