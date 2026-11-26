#!/usr/bin/env python3
"""Tests for reports_index — oracle: issue #15020 acceptance criteria.

Run:  python3 -m unittest discover -s pkgs/reports-index/tests -v
"""

import os
import sys
import unittest

sys.path.insert(0, os.path.join(os.path.dirname(__file__), ".."))
from reports_index import (  # noqa: E402
    FRESHNESS_BUDGET_SECONDS,
    ReportEntry,
    ReportsIndex,
    index_snapshot_json,
    render_index_html,
)

NOW = "2026-11-25T12:00:00Z"


def seed():
    return [
        ReportEntry(
            id="rpt-quota-fleet",
            title="Fleet quota wall forecast",
            kind="quota",
            project="fleet",
            created_at="2026-11-20T00:00:00Z",
            updated_at="2026-11-25T11:00:00Z",
            status="live",
            period_start="2026-11-25T00:00:00Z",
            tags=["quota", "forecast"],
            summary="Pool exhaustion forecast",
            body_text="OpenAI pool projected to wall at 14:00.",
            links={"issues": ["#9180"], "pages": ["/spaces/quota"]},
            live=True,
        ),
        ReportEntry(
            id="rpt-incident-auth",
            title="Claude authentication outage",
            kind="incident",
            project="fleet",
            created_at="2026-11-22T00:00:00Z",
            updated_at="2026-11-24T09:00:00Z",
            status="final",
            period_start="2026-11-22T00:00:00Z",
            tags=["incident", "auth"],
            summary="Auth outage incident report",
            body_text="Token refresh failed across lanes.",
            links={"issues": ["#15019"], "incidents": ["inc-2026-11-22"]},
        ),
        ReportEntry(
            id="rpt-rollup-daily",
            title="Daily rollup 2026-11-24",
            kind="rollup",
            project="atlas",
            created_at="2026-11-25T06:00:00Z",
            updated_at="2026-11-25T06:30:00Z",
            status="live",
            period_start="2026-11-24T00:00:00Z",
            tags=["rollup"],
            summary="What we did yesterday",
            body_text="Merged twelve pull requests.",
            links={"periods": ["2026-11-24"]},
        ),
    ]


class SearchTests(unittest.TestCase):
    def setUp(self):
        self.index = ReportsIndex(seed())

    def test_search_by_title(self):
        r = self.index.search(query="quota wall")
        self.assertEqual([e.id for e in r.items], ["rpt-quota-fleet"])

    def test_search_by_linked_issue(self):
        r = self.index.search(query="#9180")
        self.assertEqual([e.id for e in r.items], ["rpt-quota-fleet"])
        r = self.index.search(query="inc-2026-11-22")
        self.assertEqual([e.id for e in r.items], ["rpt-incident-auth"])

    def test_search_by_body_text_and_tag(self):
        r = self.index.search(query="token refresh")
        self.assertEqual([e.id for e in r.items], ["rpt-incident-auth"])
        r = self.index.search(query="forecast")
        self.assertEqual([e.id for e in r.items], ["rpt-quota-fleet"])

    def test_filters(self):
        self.assertEqual(self.index.search(kind="rollup").total, 1)
        self.assertEqual(self.index.search(project="atlas").total, 1)
        self.assertEqual(self.index.search(status="final").total, 1)
        self.assertEqual(self.index.search(tag="incident").total, 1)
        r = self.index.search(
            date_from="2026-11-24T12:00:00Z", date_to="2026-11-26T00:00:00Z"
        )
        self.assertEqual([e.id for e in r.items], ["rpt-quota-fleet"])

    def test_deterministic_sort_and_pagination(self):
        a = self.index.search(page=1, page_size=2)
        b = self.index.search(page=2, page_size=2)
        self.assertEqual(
            [e.id for e in a.items], ["rpt-quota-fleet", "rpt-rollup-daily"]
        )
        self.assertEqual([e.id for e in b.items], ["rpt-incident-auth"])
        self.assertEqual((a.total, a.pages), (3, 2))
        # Repeat calls are identical (stable sort, ID tiebreak).
        self.assertEqual(
            [e.id for e in self.index.search().items],
            [e.id for e in self.index.search().items],
        )


class RefreshTests(unittest.TestCase):
    def test_event_idempotent_and_versioned(self):
        index = ReportsIndex(seed())
        entry = dict(
            id="rpt-new",
            title="New report",
            kind="status",
            project="fleet",
            created_at=NOW,
            updated_at=NOW,
            version=1,
        )
        ev = {"event_id": "ev-1", "type": "created", "entry": entry}
        self.assertTrue(index.apply_event(ev, NOW))
        self.assertFalse(index.apply_event(ev, NOW))  # replay is a no-op
        self.assertEqual(len(index), 4)
        stale = {
            "event_id": "ev-2",
            "type": "updated",
            "entry": {**entry, "title": "Old", "version": 1},
        }
        self.assertFalse(index.apply_event(stale, NOW))
        self.assertEqual(index.get("rpt-new").title, "New report")

    def test_refresh_failure_keeps_last_good_and_surfaces_stale(self):
        index = ReportsIndex()
        self.assertTrue(index.refresh(lambda: seed(), NOW))

        def boom():
            raise RuntimeError("source down")

        later = "2026-11-25T12:20:00Z"
        self.assertFalse(index.refresh(boom, later))
        self.assertEqual(len(index), 3)  # last-good retained
        r = index.search(now=later)
        self.assertEqual(r.state, "stale")
        self.assertEqual(r.total, 3)

    def test_freshness_budget(self):
        self.assertEqual(FRESHNESS_BUDGET_SECONDS, 900)
        index = ReportsIndex()
        index.refresh(lambda: seed(), NOW)
        self.assertEqual(index.freshness_state("2026-11-25T12:14:00Z"), "fresh")
        self.assertEqual(index.freshness_state("2026-11-25T12:16:00Z"), "stale")
        live = index.get("rpt-quota-fleet")
        self.assertEqual(live.freshness(NOW, index.last_refresh_at), "fresh")
        self.assertEqual(
            index.get("rpt-incident-auth").freshness(NOW, index.last_refresh_at),
            "static",
        )


class HandoffAndRenderTests(unittest.TestCase):
    def test_c7_c8_metadata_links_no_second_store(self):
        entry = seed()[0]
        links = entry.metadata_links()
        self.assertEqual(links["canonical"], "/reports/rpt-quota-fleet")
        self.assertEqual(links["timeline"], "/reports/rpt-quota-fleet#timeline")
        self.assertEqual(links["atlas_ingest"]["url"], "/reports/rpt-quota-fleet")
        snap = index_snapshot_json(ReportsIndex(seed()), NOW)
        self.assertIn("/reports/rpt-quota-fleet#timeline", snap)

    def test_render_phone_first_and_empty_state_keeps_filters(self):
        index = ReportsIndex(seed())
        page = render_index_html(index.search(), NOW, index.last_refresh_at)
        self.assertIn('name="viewport"', page)
        self.assertIn("Fleet quota wall forecast", page)
        empty = render_index_html(
            index.search(query="no-such-thing", kind="quota"), NOW
        )
        self.assertIn("No reports match", empty)
        self.assertIn('value="quota"', empty)  # filter preserved
        self.assertIn("fresh", empty)  # freshness explained


if __name__ == "__main__":
    unittest.main()
