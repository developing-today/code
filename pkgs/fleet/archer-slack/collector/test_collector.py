#!/usr/bin/env python3
"""Unit tests for the #2137 collector's store mapping and dedupe.

No network, no token: exercises event_row/store_event only.
Run: ../.venv/bin/python -m unittest test_collector -v  (stdlib only --
the venv is used for a single interpreter, websockets is not imported
by these paths.)
"""

import os
import sys
import tempfile
import unittest

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import collector

WATCHED = {"C0CBYQR65LL"}


def message(ts="1729000000.000100", channel="C0CBYQR65LL", **kw):
    e = {
        "type": "message",
        "channel": channel,
        "ts": ts,
        "user": "U123",
        "text": "hello",
    }
    e.update(kw)
    return e


class StoreEventTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.NamedTemporaryFile(suffix=".db", delete=False)
        self.tmp.close()
        self.conn = collector.open_store(self.tmp.name)

    def tearDown(self):
        self.conn.close()
        os.unlink(self.tmp.name)

    def count(self):
        return self.conn.execute("SELECT COUNT(*) FROM collected_events").fetchone()[0]

    def test_message_stored(self):
        self.assertTrue(collector.store_event(self.conn, message(), WATCHED))
        self.assertEqual(self.count(), 1)

    def test_duplicate_message_deduped(self):
        e = message()
        self.assertTrue(collector.store_event(self.conn, e, WATCHED))
        self.assertFalse(collector.store_event(self.conn, e, WATCHED))
        self.assertEqual(self.count(), 1)

    def test_thread_reply_keeps_thread_ts(self):
        e = message(ts="1729000100.000200", thread_ts="1729000000.000100")
        self.assertTrue(collector.store_event(self.conn, e, WATCHED))
        row = self.conn.execute("SELECT thread_ts FROM collected_events").fetchone()
        self.assertEqual(row[0], "1729000000.000100")

    def test_unwatched_channel_ignored(self):
        self.assertFalse(
            collector.store_event(self.conn, message(channel="C9999999999"), WATCHED)
        )
        self.assertEqual(self.count(), 0)

    def test_reaction_stored_and_deduped_per_user(self):
        r = {
            "type": "reaction_added",
            "user": "U123",
            "reaction": "eyes",
            "item": {
                "type": "message",
                "channel": "C0CBYQR65LL",
                "ts": "1729000000.000100",
            },
        }
        self.assertTrue(collector.store_event(self.conn, r, WATCHED))
        self.assertFalse(collector.store_event(self.conn, r, WATCHED))
        r2 = dict(r, user="U456")
        self.assertTrue(collector.store_event(self.conn, r2, WATCHED))
        self.assertEqual(self.count(), 2)

    def test_unhandled_event_type_ignored(self):
        self.assertFalse(
            collector.store_event(
                self.conn,
                {"type": "app_mention", "channel": "C0CBYQR65LL", "ts": "1.2"},
                WATCHED,
            )
        )
        self.assertEqual(self.count(), 0)

    def test_message_without_ts_ignored(self):
        e = message()
        del e["ts"]
        self.assertFalse(collector.store_event(self.conn, e, WATCHED))


if __name__ == "__main__":
    unittest.main()
