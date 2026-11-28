#!/usr/bin/env python3
"""Tests for doc/backups/embedding-store-inventory.md — #13824 inventory leg.

The inventory names every embedding/vector store in the tree, its home
(Turso DB / local files / hosted), and its mesh coverage path (rotation
member, restic file leg, existing leg, or exclusion with reason), per the
backup-mesh design (doc/2026-11-23T16-34-18Z_design_backup_mesh/note.md
§§2, 6) and #13461 / #13824 acceptance.

Oracles:
- Vector tables: the `vec_<model>_<dims>_f32` table names the code
  actually uses (atlas-core Rust sources, the data-stores book chapter,
  and the schema dir). A table the code names but the inventory does not
  is a drift failure — the test, not a human, catches a new vector table.
- Store classes: the pinned list below, each with the file that proves
  the store exists. A store class missing from the inventory fails.
- Coverage: every row of the inventory table carries a coverage
  disposition (a mesh leg, an existing leg, an exclusion with reason,
  or an explicit out-of-scope note), and every row marked uncovered
  names its follow-on issue number.

Run:  python3 -m pytest pkgs/cold-store-reaper/tests/test_embedding_store_inventory.py
"""

import re
import unittest
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent.parent.parent
INVENTORY = REPO / "doc" / "backups" / "embedding-store-inventory.md"

# Vector-table names are `vec_<model>_<dims>_f32` (the stored table name
# in every SQLite-shaped store); plain `vec_*` identifiers are Rust
# locals, not tables, and are not matched.
VECTOR_TABLE_RE = re.compile(r"vec_[a-z0-9_]+_f32\b")
ISSUE_RE = re.compile(r"#\d+")

# Store classes the inventory must name, with the evidence that proves
# each store exists. The string is a substring the inventory must
# contain (a store name, env var, or file path), not the full row.
STORE_CLASSES = [
    # (inventory substring, evidence)
    ("atlas", "pkgs/atlas-search/book/reference/data-stores.md: canonical Turso DB"),
    (
        "vec_snowflake_m_v2_768_f32",
        "pkgs/atlas-search/book/reference/data-stores.md: production vector table",
    ),
    (
        "vec_bge_small_384_f32",
        "pkgs/atlas-search/book/reference/data-stores.md: dormant legacy vector table",
    ),
    (
        "atlas-core-",
        "pkgs/atlas-search/crates/atlas-core/src/shards/mod.rs: scan-shard name pattern",
    ),
    (
        "atlas-core-",
        "pkgs/atlas-search/book/reference/data-stores.md: per-tier content stores",
    ),
    (
        "query_embedding_cache",
        "pkgs/atlas-search/crates/atlas-core/src/embed_cache.rs: query-embedding cache table",
    ),
    (
        "ATLAS_EMBED_CACHE_TURSO_URL",
        "pkgs/atlas-search/crates/atlas-core/src/embed_cache.rs: cache DB env var",
    ),
    (
        "ATLAS_VEC_DB",
        "pkgs/atlas-search/crates/atlas-core/src/local.rs: local side-store vector DB",
    ),
    (
        "memory-lancedb",
        "pkgs/atlas-search/docs/memory-integration-design-8567.md: OpenClaw LanceDB store",
    ),
    (
        "QueryCache",
        "pkgs/atlas-search/book/reference/data-stores.md: Worker Durable Object cache",
    ),
    (
        "atlas-hot-cache",
        "pkgs/atlas-search/book/reference/data-stores.md: D1 hot cache",
    ),
    (
        "staging",
        "pkgs/atlas-search/book/reference/data-stores.md: staging DBs never store vectors",
    ),
]

# Coverage dispositions: every inventory-table row must carry one.
COVERAGE_WORDS = (
    "rotation",
    "restic",
    "existing leg",
    "exclusion",
    "out of scope",
    "covered",
)


def discover_vector_tables():
    """Vector-table names the code actually uses."""
    found = set()
    roots = [
        REPO / "pkgs" / "atlas-search" / "crates",
        REPO / "pkgs" / "atlas-search" / "book" / "reference" / "data-stores.md",
        REPO / "pkgs" / "atlas-search" / "schema",
    ]
    for root in roots:
        if root.is_file():
            files = [root]
        else:
            files = [p for p in root.rglob("*") if p.suffix in {".rs", ".md", ".sql"}]
        for path in files:
            found.update(VECTOR_TABLE_RE.findall(path.read_text(errors="replace")))
    return found


def inventory_table_rows(text):
    """Rows of the first markdown table whose header names a Coverage column."""
    lines = text.splitlines()
    rows = []
    in_table = False
    for line in lines:
        if (
            line.startswith("|")
            and "coverage" in line.lower()
            and "store" in line.lower()
        ):
            in_table = True
            continue
        if in_table:
            if line.startswith("|") and "---" not in line:
                rows.append(line)
            elif not line.startswith("|"):
                break
    return rows


class InventoryTests(unittest.TestCase):
    def test_inventory_doc_exists(self):
        self.assertTrue(INVENTORY.is_file(), f"inventory doc missing: {INVENTORY}")

    def test_every_vector_table_in_code_is_named(self):
        tables = discover_vector_tables()
        self.assertGreater(len(tables), 0, "discovery found no vector tables")
        text = INVENTORY.read_text()
        for table in sorted(tables):
            self.assertIn(table, text, f"vector table {table} not in inventory")

    def test_every_store_class_is_named(self):
        text = INVENTORY.read_text()
        for needle, evidence in STORE_CLASSES:
            with self.subTest(store=needle):
                self.assertIn(
                    needle,
                    text,
                    f"store {needle!r} not in inventory (evidence: {evidence})",
                )

    def test_every_row_has_a_coverage_path(self):
        rows = inventory_table_rows(INVENTORY.read_text())
        self.assertGreater(len(rows), 0, "no inventory table rows found")
        for row in rows:
            lowered = row.lower()
            self.assertTrue(
                any(word in lowered for word in COVERAGE_WORDS),
                f"inventory row has no coverage disposition: {row}",
            )

    def test_uncovered_rows_name_a_follow_on_issue(self):
        rows = inventory_table_rows(INVENTORY.read_text())
        uncovered = [r for r in rows if "uncovered" in r.lower()]
        # Any store still uncovered must name its follow-on issue; when
        # the last gap closes this list is empty and the test is vacuous.
        for row in uncovered:
            self.assertRegex(
                row, ISSUE_RE, f"uncovered row names no follow-on issue: {row}"
            )


if __name__ == "__main__":
    unittest.main()
