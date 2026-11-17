"""Synthetic SQLite interleavings against the production quarantine write SQL.
Oracle: a stale selected row cannot alter a newer hold/packet or a lease.
No credentials, network, or live database; every test uses :memory:.
"""
import json
import pathlib
import re
import sqlite3
import unittest

SRC = pathlib.Path(__file__).resolve().parents[1] / 'src'
RUST = (SRC / 'turso_queue.rs').read_text()

def rust_string(s):
    return re.sub(r'\\\n\s*', '', s).replace('\\"', '"')

COLS = rust_string(re.search(r'const TASK_COLS: &str = "(.*?)";', RUST, re.S).group(1))
SNAPSHOT = 'json_array(' + COLS + ')'
SQL_PATH = SRC / 'quarantine_transition.sql'

def production_sql():
    return SQL_PATH.read_text().replace('{snapshot}', SNAPSHOT)

class QuarantineAtomic(unittest.TestCase):
    def setUp(self):
        self.db = sqlite3.connect(':memory:')
        columns = [x.strip() for x in COLS.split(',')]
        self.db.execute('CREATE TABLE tasks (' + ','.join(c + (' INTEGER PRIMARY KEY' if c == 'id' else ' TEXT') for c in columns) + ')')
        self.db.execute('CREATE TABLE leases(task_id INTEGER PRIMARY KEY, worker_id TEXT, lease_until TEXT)')
        self.db.execute('CREATE TABLE events(id INTEGER PRIMARY KEY, task_id INTEGER, kind TEXT, detail TEXT)')
        self.db.execute("INSERT INTO tasks(id,status,payload,progress,result,last_error,updated_at) VALUES(1,'blocked','{}','{}','{}','quarantined by claim path: old','same-second')")
        self.db.execute("INSERT INTO events VALUES(1,1,'blocked','{\"via\":\"claim_payload_quarantine\"}')")
        self.snapshot = self.db.execute('SELECT ' + SNAPSHOT + ' FROM tasks WHERE id=1').fetchone()[0]

    def transition(self, refresh=False):
        sql = production_sql()
        args = ('blocked' if refresh else 'queued', 'quarantined by claim path: new' if refresh else None, 'later', 1, self.snapshot, 1)
        return self.db.execute(sql, args).rowcount

    def test_unchanged_snapshot_releases(self):
        self.assertEqual(self.transition(), 1)
        self.assertEqual(self.db.execute('SELECT status,last_error FROM tasks').fetchone(), ('queued', None))

    def test_replacement_manual_hold_same_timestamp(self):
        self.db.execute("UPDATE tasks SET progress='{\"blocker\":\"human decision\"}' WHERE id=1")
        self.assertEqual(self.transition(), 0)

    def test_new_dependency_blocks_release_and_refresh(self):
        self.db.execute("UPDATE tasks SET blocked_deps='[{\"kind\":\"task\",\"task_id\":2}]' WHERE id=1")
        self.assertEqual(self.transition(), 0)
        self.assertEqual(self.transition(refresh=True), 0)

    def test_changed_packet_invalidates_verdict(self):
        self.db.execute("UPDATE tasks SET payload='{\"changed\":true}' WHERE id=1")
        self.assertEqual(self.transition(), 0)

    def test_new_block_event_invalidates_identical_row(self):
        self.db.execute("INSERT INTO events VALUES(2,1,'blocked','{\"blocker\":\"new hold\"}')")
        self.assertEqual(self.transition(), 0)

    def test_conflicting_lease_is_preserved(self):
        self.db.execute("INSERT INTO leases VALUES(1,'other-worker','later')")
        self.assertEqual(self.transition(), 0)
        self.assertEqual(self.db.execute('SELECT worker_id FROM leases').fetchone()[0], 'other-worker')

    def test_post_release_claim_survives_stale_retry(self):
        self.assertEqual(self.transition(), 1)
        self.db.execute("UPDATE tasks SET status='claimed',worker_id='new-owner',lease_until='future'")
        self.db.execute("INSERT INTO leases VALUES(1,'new-owner','future')")
        self.assertEqual(self.transition(), 0)
        self.assertEqual(self.db.execute('SELECT worker_id FROM leases').fetchone()[0], 'new-owner')

    def test_unchanged_snapshot_refreshes_reason(self):
        self.assertEqual(self.transition(refresh=True), 1)
        self.assertEqual(self.db.execute('SELECT status,last_error FROM tasks').fetchone(),
                         ('blocked', 'quarantined by claim path: new'))

    def test_unknown_provenance_fails_closed(self):
        for detail in ['{}', '{"via":"manual"}', 'malformed']:
            with self.subTest(detail=detail):
                self.db.execute('UPDATE events SET detail=?', (detail,))
                self.assertEqual(self.transition(), 0)
                self.assertEqual(self.transition(refresh=True), 0)

    def test_missing_provenance_fails_closed(self):
        self.db.execute('DELETE FROM events')
        self.assertEqual(self.transition(), 0)

    def test_refresh_provenance_is_supported(self):
        self.db.execute('UPDATE events SET detail=?', ('{"via":"quarantine_recheck"}',))
        self.assertEqual(self.transition(), 1)

    def test_claim_owner_on_blocked_row_is_not_cleared(self):
        self.db.execute("UPDATE tasks SET worker_id='owner',lease_until='future'")
        self.snapshot = self.db.execute('SELECT ' + SNAPSHOT + ' FROM tasks').fetchone()[0]
        self.assertEqual(self.transition(), 0)
        self.assertEqual(self.db.execute('SELECT worker_id FROM tasks').fetchone()[0], 'owner')

    def test_refresh_cannot_replace_new_reason(self):
        self.db.execute("UPDATE tasks SET last_error='manual replacement' WHERE id=1")
        self.assertEqual(self.transition(refresh=True), 0)

if __name__ == '__main__':
    unittest.main()
