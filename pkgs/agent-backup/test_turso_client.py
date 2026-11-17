#!/usr/bin/env python3
"""Regression tests for turso_client session handling (#8545).

The fake server below models the one property of the libsql HTTP
pipeline protocol this client depends on: a pipeline call without a
baton runs on a FRESH session (its own connection, hence its own
temp tables), a call carrying a baton resumes that session, and a
`close` request ends it. Each session is backed by its own in-memory
SQLite database, which reproduces temp-table visibility exactly.

Before the fix, Turso.execute chunked a batch into pipeline calls of
at most 100 statements with no baton carried between them, so a
history-enabled load_rows batch (CREATE TEMP TABLE rde_incoming +
one INSERT per row + the archive INSERT..SELECT) lost the staging
table at the first chunk boundary.
"""

import base64
import json
import os
import sqlite3
import sys
import threading
import unittest
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))
import turso_client


def _decode(arg):
    kind = arg.get("type")
    if kind == "null":
        return None
    if kind == "integer":
        return int(arg["value"])
    if kind == "float":
        return float(arg["value"])
    if kind == "blob":
        return base64.b64decode(arg["base64"])
    return arg.get("value")


class FakeTurso:
    def __init__(self):
        self.sessions = {}
        self.lock = threading.Lock()
        self.next_id = 0
        self.pipeline_calls = 0

    def handle(self, body):
        with self.lock:
            self.pipeline_calls += 1
            baton = body.get("baton")
            if baton and baton in self.sessions:
                conn = self.sessions[baton]
            else:
                self.next_id += 1
                baton = f"baton-{self.next_id}"
                conn = sqlite3.connect(":memory:", check_same_thread=False)
                self.sessions[baton] = conn
            results = []
            closed = False
            for req in body.get("requests", []):
                if req.get("type") == "close":
                    closed = True
                    results.append({"type": "ok", "response": {"type": "close"}})
                    continue
                stmt = req["stmt"]
                args = [_decode(a) for a in stmt.get("args", [])]
                try:
                    cur = conn.execute(stmt["sql"], args)
                    if cur.description:
                        cols = [{"name": d[0]} for d in cur.description]
                        rows = [
                            [{"value": v} for v in row] for row in cur.fetchall()
                        ]
                        result = {
                            "cols": cols,
                            "rows": rows,
                            "affected_row_count": 0,
                        }
                    else:
                        result = {
                            "cols": [],
                            "rows": [],
                            "affected_row_count": cur.rowcount,
                        }
                    conn.commit()
                    results.append(
                        {"type": "ok", "response": {"type": "execute", "result": result}}
                    )
                except Exception as e:  # noqa: BLE001 - mirrors pipeline errors
                    results.append(
                        {"type": "error", "error": {"message": str(e), "code": "SQLITE"}}
                    )
            if closed:
                self.sessions.pop(baton, None)
                conn.close()
                return {"baton": None, "results": results}
            return {"baton": baton, "results": results}


def _serve(fake):
    class Handler(BaseHTTPRequestHandler):
        def do_POST(self):
            length = int(self.headers.get("Content-Length", 0))
            body = json.loads(self.rfile.read(length) or b"{}")
            payload = json.dumps(fake.handle(body)).encode()
            self.send_response(200)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(payload)))
            self.end_headers()
            self.wfile.write(payload)

        def log_message(self, *args):
            pass

    server = ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    return server


class TursoSessionTest(unittest.TestCase):
    def setUp(self):
        self.fake = FakeTurso()
        self.server = _serve(self.fake)
        self._old_url = turso_client.DB_HTTP_URL
        turso_client.DB_HTTP_URL = (
            f"http://127.0.0.1:{self.server.server_port}/v2/pipeline"
        )
        # the ambient proxy must not swallow the loopback fake
        self._old_no_proxy = os.environ.get("no_proxy")
        os.environ["no_proxy"] = "127.0.0.1,localhost"
        self.client = turso_client.Turso()
        self.client._token = "test-token"

    def tearDown(self):
        turso_client.DB_HTTP_URL = self._old_url
        if self._old_no_proxy is None:
            os.environ.pop("no_proxy", None)
        else:
            os.environ["no_proxy"] = self._old_no_proxy
        self.server.shutdown()

    def test_temp_table_survives_chunk_boundary(self):
        # A history-enabled load shape: staging CREATE + 150 INSERTs
        # (crossing the 100-statement chunk boundary) + the archive
        # INSERT..SELECT reading the staged rows + the drop.
        stmts = [
            ("CREATE TABLE hist (id INTEGER, payload TEXT)", []),
            ("CREATE TEMP TABLE rde_incoming (id INTEGER, payload TEXT)", []),
            ("DELETE FROM rde_incoming", []),
        ]
        stmts += [
            ("INSERT INTO rde_incoming (id, payload) VALUES (?, ?)", [i, f"p{i}"])
            for i in range(150)
        ]
        stmts.append(
            ("INSERT INTO hist (id, payload) SELECT id, payload FROM rde_incoming", [])
        )
        stmts.append(("DROP TABLE IF EXISTS rde_incoming", []))
        stmts.append(("SELECT COUNT(*) AS n FROM hist", []))
        self.assertGreater(len(stmts), 100)
        out = self.client.execute(stmts)
        self.assertEqual(out[-1]["rows"], [{"n": 150}])
        # the batch really did take more than one pipeline call, all
        # on one session, and the session was closed at the end
        self.assertGreaterEqual(self.fake.pipeline_calls, 2)
        self.assertEqual(self.fake.sessions, {})
        self.assertIsNone(self.client._baton)

    def test_failed_batch_drops_its_session(self):
        stmts = [("CREATE TEMP TABLE rde_incoming (id INTEGER)", [])]
        stmts += [("INSERT INTO rde_incoming (id) VALUES (?)", [i]) for i in range(120)]
        stmts.append(("INSERT INTO no_such_table VALUES (1)", []))
        with self.assertRaises(RuntimeError):
            self.client.execute(stmts)
        self.assertIsNone(self.client._baton)
        self.assertEqual(self.fake.sessions, {})
        # and the next batch starts clean: the temp table is gone, so
        # recreating it works instead of colliding with the failed one
        out = self.client.execute(
            [
                ("CREATE TEMP TABLE rde_incoming (id INTEGER)", []),
                ("INSERT INTO rde_incoming (id) VALUES (1)", []),
                ("SELECT COUNT(*) AS n FROM rde_incoming", []),
            ]
        )
        self.assertEqual(out[-1]["rows"], [{"n": 1}])


if __name__ == "__main__":
    unittest.main()
