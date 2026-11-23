#!/usr/bin/env python3
"""table_columns (#8584 live drill finding): the D1 query API refuses
PRAGMA, so the restore drill parses column order from CREATE TABLE
text. The two schemas below are verbatim from the live queue store --
an inline PRIMARY KEY shape and a table-level PRIMARY KEY shape with
a quoted column name.

Run: python3 -I infra/fleet-queue-backup/tests/test_table_columns.py
"""

import os
import sys
import unittest

BIN = os.path.abspath(os.path.join(os.path.dirname(__file__), "..", "bin"))
sys.path.insert(0, BIN)

import fq_common

EVENTS = (
    "CREATE TABLE events (id INTEGER PRIMARY KEY AUTOINCREMENT, "
    "worker_id TEXT, task_id INTEGER, kind TEXT NOT NULL, "
    "detail TEXT NOT NULL DEFAULT '{}', created_at TEXT NOT NULL)"
)
CHECKPOINTS = (
    'CREATE TABLE checkpoints (worker_id TEXT NOT NULL, "key" TEXT NOT NULL, '
    "value TEXT NOT NULL DEFAULT '{}', updated_at TEXT NOT NULL, "
    'PRIMARY KEY (worker_id, "key"))'
)


class TestTableColumns(unittest.TestCase):
    def test_inline_pk_schema(self):
        self.assertEqual(
            fq_common.table_columns(EVENTS),
            ["id", "worker_id", "task_id", "kind", "detail", "created_at"],
        )
        self.assertEqual(fq_common._pk_columns(EVENTS), ["id"])

    def test_table_level_pk_schema_with_quoted_column(self):
        self.assertEqual(
            fq_common.table_columns(CHECKPOINTS),
            ["worker_id", "key", "value", "updated_at"],
        )
        self.assertEqual(
            fq_common._pk_columns(CHECKPOINTS), ["worker_id", "key"]
        )


if __name__ == "__main__":
    unittest.main()
