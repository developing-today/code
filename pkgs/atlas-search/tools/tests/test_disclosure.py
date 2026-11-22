#!/usr/bin/env python3
"""Disclosure-boundary tests for the legacy Python CLI (#12623).

Oracle: the disclosure-context table and fail-closed rules in design §4,
mirrored by atlas-core/src/disclosure.rs.
"""

import importlib.util
import sqlite3
import unittest
from pathlib import Path
from types import SimpleNamespace

HERE = Path(__file__).resolve().parent
SCRIPT = HERE.parents[1] / "atlas_search.py"
spec = importlib.util.spec_from_file_location("atlas_search", SCRIPT)
atlas_search = importlib.util.module_from_spec(spec)
spec.loader.exec_module(atlas_search)


class DisclosureClassificationTests(unittest.TestCase):
    def test_context_ceiling_intersects_caller_ceiling(self):
        # Oracle: the exact context table in design §4.
        for context, ceiling in (
            ("fleet-internal", "local-only"),
            ("hosted-per-user", "personal"),
            ("public-surface", "shareable"),
            ("local-owner", "secret-bearing"),
        ):
            with self.subTest(context=context):
                self.assertEqual(
                    atlas_search.effective_sensitivity_ceiling(
                        context, "secret-bearing"
                    ),
                    ceiling,
                )
        # A context cannot widen a narrower caller grant.
        self.assertEqual(
            atlas_search.effective_sensitivity_ceiling("local-owner", "personal"),
            "personal",
        )

    def test_unknown_context_and_classification_fail_closed(self):
        # Oracle: Rust DisclosureContext::for_outbound (unknown context ->
        # public-surface). For labels this CLI is deliberately stricter
        # than atlas-core chunk_classification, which resolves an unknown
        # label to secret-bearing (servable at local-owner): unclassified
        # content is never served here, matching the search ceiling
        # filter, which already excluded NULL sensitivity before #12623.
        self.assertEqual(
            atlas_search.effective_sensitivity_ceiling("unknown", "secret-bearing"),
            "shareable",
        )
        for label in (None, "", "internal", "Shareable"):
            with self.subTest(label=label):
                self.assertFalse(atlas_search.disclosure_allows(label, "secret-bearing"))
        self.assertFalse(
            atlas_search.disclosure_allows("excluded_personal_life", "secret-bearing")
        )

    def test_context_intersection_matches_exhaustive_reference_model(self):
        # Oracle: an independent rank-based model of the design's four
        # destination ceilings, caller intersection, and fail-closed labels.
        ranks = {label: rank for rank, label in enumerate(
            ("shareable", "personal", "local-only", "secret-bearing")
        )}
        contexts = {
            "fleet-internal": "local-only",
            "hosted-per-user": "personal",
            "public-surface": "shareable",
            "local-owner": "secret-bearing",
        }
        classifications = (*ranks, None, "", "unknown", "excluded_personal_life")
        for context, context_ceiling in contexts.items():
            for caller, caller_rank in ranks.items():
                ceiling = min(context_ceiling, caller, key=ranks.__getitem__)
                with self.subTest(context=context, caller=caller):
                    self.assertEqual(
                        atlas_search.effective_sensitivity_ceiling(context, caller),
                        ceiling,
                    )
                    for classification in classifications:
                        expected = (
                            classification in ranks
                            and classification != "excluded_personal_life"
                            and ranks[classification] <= ranks[ceiling]
                        )
                        with self.subTest(classification=classification):
                            self.assertEqual(
                                atlas_search.disclosure_allows(classification, ceiling),
                                expected,
                            )


class SQLiteFixtureBackend:
    name = "local"
    sens_expr = "cs.sensitivity"
    labels_table = "session_labels"
    sens_join = "LEFT JOIN v_chunk_sens cs ON cs.id = c.id"
    vec_table = "unused"

    def __init__(self):
        self.con = sqlite3.connect(":memory:")
        self.con.row_factory = sqlite3.Row
        self.con.executescript(
            """
            CREATE TABLE chunks (id INTEGER, message_id INTEGER, chunk_idx INTEGER, text TEXT);
            CREATE TABLE messages (id INTEGER, role TEXT, text TEXT, session_id INTEGER);
            CREATE TABLE sessions (id INTEGER, title TEXT, client TEXT, created_at TEXT);
            CREATE TABLE session_labels (
              session_id INTEGER, kind TEXT, topics TEXT, projects TEXT, sensitivity TEXT
            );
            CREATE TABLE v_chunk_sens (id INTEGER, sensitivity TEXT);
            INSERT INTO sessions VALUES (7, 'title', 'client', '2026-01-01');
            INSERT INTO messages VALUES (9, 'user', 'whole raw message', 7);
            INSERT INTO session_labels VALUES (7, 'chat', '[]', '[]', 'shareable');
            INSERT INTO chunks VALUES (1, 9, 0, 'safe before');
            INSERT INTO chunks VALUES (2, 9, 1, 'safe target');
            INSERT INTO chunks VALUES (3, 9, 2, 'personal neighbor');
            INSERT INTO chunks VALUES (4, 9, 3, 'excluded neighbor');
            INSERT INTO chunks VALUES (5, 9, 4, 'unclassified neighbor');
            INSERT INTO v_chunk_sens VALUES (1, 'shareable');
            INSERT INTO v_chunk_sens VALUES (2, 'shareable');
            INSERT INTO v_chunk_sens VALUES (3, 'personal');
            INSERT INTO v_chunk_sens VALUES (4, 'excluded_personal_life');
            """
        )
        self.notes = []

    def execute(self, sql, args=()):
        return [dict(row) for row in self.con.execute(sql, args).fetchall()]


class DisclosureOutputTests(unittest.TestCase):
    def setUp(self):
        self.backend = SQLiteFixtureBackend()

    def tearDown(self):
        self.backend.con.close()

    def test_get_chunk_filters_neighbors_at_destination_context(self):
        # Oracle: design §4 — public-surface permits shareable only; excluded
        # material remains ineligible even when a neighbor is requested.
        args = SimpleNamespace(
            chunk_id=2,
            neighbors=2,
            max_sensitivity="secret-bearing",
            context="public-surface",
        )
        result = atlas_search.run_get_chunk(self.backend, args)
        self.assertEqual(
            result["context"],
            {"before": [{"chunk_id": 1, "text": "safe before"}], "after": []},
        )

    def test_missing_chunk_classification_does_not_inherit_session_label(self):
        # Oracle: raw chunk classification must be known before admission;
        # session-level shareable metadata cannot authorize an unclassified chunk.
        details = atlas_search.fetch_details(self.backend, [5])[5]
        self.assertIsNone(details["chunk_sensitivity"])
        self.assertFalse(
            atlas_search.disclosure_allows(
                details.get("chunk_sensitivity"), "shareable"
            )
        )
        args = SimpleNamespace(
            chunk_id=2,
            neighbors=3,
            max_sensitivity="secret-bearing",
            context="public-surface",
        )
        result = atlas_search.run_get_chunk(self.backend, args)
        self.assertNotIn(5, [c["chunk_id"] for c in result["context"]["after"]])

    def test_full_message_withheld_if_any_chunk_exceeds_boundary(self):
        # Oracle: full-message hydration cannot bypass the same output boundary.
        details = atlas_search.fetch_details(self.backend, [2])[2]
        self.assertIsNone(atlas_search.message_obj(self.backend, details, "shareable"))

    def test_search_filters_candidates_by_context_and_caller_intersection(self):
        args = SimpleNamespace(
            kind=None,
            topic=None,
            project=None,
            client=None,
            since=None,
            until=None,
            max_sensitivity="secret-bearing",
            context="hosted-per-user",
        )
        filters = atlas_search.Filters(args)
        self.assertEqual(filters.allowed_sens, ["shareable", "personal"])

    def test_full_message_is_returned_when_every_chunk_is_allowed(self):
        self.backend.execute(
            "UPDATE v_chunk_sens SET sensitivity='shareable'", []
        )
        self.backend.execute(
            "INSERT INTO v_chunk_sens VALUES (5, 'shareable')", []
        )
        details = atlas_search.fetch_details(self.backend, [2])[2]
        self.assertEqual(
            atlas_search.message_obj(self.backend, details, "shareable")["text"],
            "whole raw message",
        )


if __name__ == "__main__":
    unittest.main()
