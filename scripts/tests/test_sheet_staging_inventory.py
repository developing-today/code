#!/usr/bin/env python3
"""Inventory manifest contract for #5692 sheet staging + curated rows (#6581 Wave 1).

Oracle: the §4 table inventory in
doc/2026-11-24T00-50-11Z_design_sheet-staging-turso/2026-11-24T00-50-11Z_design_sheet-staging-turso.md
(reference corpus) plus live repo file existence. The manifest must list
exactly the 19 in-scope source tables (12 staging + 7 curated), each mapped
to its Turso `schema_table` name, with producer/reader evidence or an explicit
`unknown` marker (Slice 1 acceptance: unknown remote consumers are marked
unknown, not "none").

Fails on origin/main (no manifest); passes after the Slice 1 manifest lands.
"""

import re
import unittest
from pathlib import Path

ROOT = Path(__file__).resolve().parents[2]
MANIFEST = (
    ROOT
    / "doc"
    / "2026-11-24T00-50-11Z_design_sheet-staging-turso"
    / "inventory-19-tables.md"
)

EXPECTED_STAGING = [
    "ainventory.sheet_rows",
    "ainventory.ingest_batches",
    "automation.sheet_rows",
    "automation.ingest_batches",
    "engineering.sheet_rows",
    "engineering.ingest_batches",
    "finance.sheet_rows",
    "finance.ingest_batches",
    "home.sheet_rows",
    "home.ingest_batches",
    "research.sheet_rows",
    "research.ingest_batches",
]

EXPECTED_CURATED = [
    "automation.events",
    "engineering.events",
    "finance.events",
    "home.events",
    "research.events",
    "engineering.records",
    "ideas.ideas",
]


def expected_turso(source: str) -> str:
    schema, table = source.split(".")
    return f"{schema}_{table}"


class TestSheetStagingInventory(unittest.TestCase):
    def test_manifest_exists(self):
        self.assertTrue(
            MANIFEST.is_file(), f"missing Slice 1 manifest: {MANIFEST}"
        )

    def test_exactly_19_in_scope_tables(self):
        text = MANIFEST.read_text()
        pipe_rows = [
            line
            for line in text.splitlines()
            if line.startswith("|")
            and any(
                key in line
                for key in (
                    ".sheet_rows",
                    ".ingest_batches",
                    ".events",
                    ".records",
                    "ideas.ideas",
                )
            )
        ]
        found = sorted(
            {
                m
                for line in pipe_rows
                for m in re.findall(
                    r"(ainventory|automation|engineering|finance|home|research|ideas)\."
                    r"(sheet_rows|ingest_batches|events|records|ideas)",
                    line,
                )
                for m in [f"{m[0]}.{m[1]}"]
            }
        )
        expected = sorted(EXPECTED_STAGING + EXPECTED_CURATED)
        self.assertEqual(found, expected, f"manifest must list exactly 19 tables")

    def test_turso_names_follow_schema_table_convention(self):
        text = MANIFEST.read_text()
        for source in EXPECTED_STAGING + EXPECTED_CURATED:
            self.assertIn(
                expected_turso(source),
                text,
                f"manifest must map {source} to {expected_turso(source)}",
            )

    def test_each_row_has_producer_reader_disposition(self):
        text = MANIFEST.read_text()
        for source in EXPECTED_STAGING + EXPECTED_CURATED:
            line = next(
                (ln for ln in text.splitlines() if source in ln and ln.startswith("|")),
                None,
            )
            self.assertIsNotNone(line, f"no manifest row for {source}")
            cols = [c.strip() for c in line.strip().strip("|").split("|")]
            self.assertGreaterEqual(
                len(cols), 5, f"row for {source} needs 5+ columns, got: {line}"
            )
            # producer, reader, disposition columns must be non-empty;
            # unreachable remote coverage must say "unknown", never blank.
            for col in cols[2:5]:
                self.assertTrue(col, f"row for {source} has a blank column: {line}")

    def test_cited_repo_files_exist(self):
        text = MANIFEST.read_text()
        cited = set(re.findall(r"`((?:pkgs|scripts|pm-stack)/[^`]+?)`", text))
        missing = [c for c in cited if not (ROOT / c.split("#")[0].split(":")[0]).exists()]
        self.assertEqual(missing, [], f"cited repo files must exist: {missing}")


if __name__ == "__main__":
    unittest.main()
