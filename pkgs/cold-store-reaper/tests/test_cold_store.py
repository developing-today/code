#!/usr/bin/env python3
"""Tests for cold_store — no network, no real credentials.

Run:  python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
  or: python3 pkgs/cold-store-reaper/tests/test_cold_store.py
"""

import hashlib
import json
import os
import re
import shutil
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.parse
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
from pathlib import Path
from unittest import mock

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
import cold_store
from cold_store import ColdStore


# ---------------------------------------------------------------- fake Drive
class FakeDrive:
    """In-memory state for the fake Google endpoints."""

    def __init__(self):
        self.folders = []  # {id, name, parents}
        self.files = {}  # id -> bytes
        self.file_meta = {}  # id -> {name, parents, createdTime}
        self.sessions = {}  # sid -> {meta, chunks: {start: bytes}, total}
        self.counter = 0
        self.clock = 0

    def next_id(self, prefix):
        self.counter += 1
        return f"{prefix}-{self.counter}"

    def stamp(self):
        """Monotonic fake createdTime so newest-first ordering is testable."""
        self.clock += 1
        return f"2026-01-01T00:{self.clock // 60:02d}:{self.clock % 60:02d}.000Z"


def make_handler(fake: FakeDrive):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):  # silence
            pass

        def _send(self, code, body=b"", ctype="application/json", headers=None):
            if isinstance(body, (dict, list)):
                body = json.dumps(body).encode()
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            for k, v in (headers or {}).items():
                self.send_header(k, v)
            self.end_headers()
            if body:
                self.wfile.write(body)

        def _body(self):
            length = int(self.headers.get("Content-Length") or 0)
            return self.rfile.read(length) if length else b""

        # ---------------------------------------------------------- token
        def do_POST(self):
            parsed = urllib.parse.urlparse(self.path)
            if parsed.path == "/token":
                self._body()
                payload = {
                    "access_token": "fake-access",
                    "expires_in": 3600,
                    "token_type": "Bearer",
                    "refresh_token": "fake-refresh-token",
                }
                self._send(200, payload)
                return
            if parsed.path == "/drive/v3/files":
                meta = json.loads(self._body() or b"{}")
                fid = fake.next_id("folder")
                fake.folders.append(
                    {
                        "id": fid,
                        "name": meta["name"],
                        "parents": meta.get("parents", []),
                    }
                )
                self._send(
                    200,
                    {"id": fid, "name": meta["name"], "mimeType": meta.get("mimeType")},
                )
                return
            if parsed.path == "/upload/drive/v3/files":
                meta = json.loads(self._body() or b"{}")
                sid = fake.next_id("session")
                total = int(self.headers.get("X-Upload-Content-Length") or 0)
                fake.sessions[sid] = {"meta": meta, "chunks": {}, "total": total}
                location = f"http://{self.headers['Host']}/upload-session/{sid}"
                self._send(200, b"", headers={"Location": location})
                return
            self._send(404, {"error": "not found"})

        # ---------------------------------------------------------- reads
        def do_GET(self):
            parsed = urllib.parse.urlparse(self.path)
            if parsed.path == "/drive/v3/files":
                q = urllib.parse.parse_qs(parsed.query).get("q", [""])[0]
                name_m = re.search(r"name = '([^']+)'", q)
                parent_m = re.search(r"'([^']+)' in parents", q)
                out = []
                for folder in fake.folders:
                    if name_m and folder["name"] != name_m.group(1):
                        continue
                    if parent_m and parent_m.group(1) not in folder["parents"]:
                        continue
                    out.append({"id": folder["id"], "name": folder["name"]})
                if parent_m and not name_m:
                    # parent-only listing (list-backups): include real files
                    for fid, meta in fake.file_meta.items():
                        if parent_m.group(1) in meta.get("parents", []):
                            out.append(
                                {
                                    "id": fid,
                                    "name": meta["name"],
                                    "size": str(len(fake.files[fid])),
                                    "createdTime": meta.get("createdTime", ""),
                                }
                            )
                self._send(200, {"files": out})
                return
            m = re.fullmatch(r"/drive/v3/files/([^/]+)", parsed.path)
            if m:
                fid = m.group(1)
                qs = urllib.parse.parse_qs(parsed.query)
                if qs.get("alt") == ["media"]:
                    data = fake.files[fid]
                    range_hdr = self.headers.get("Range", "")
                    rm = re.fullmatch(r"bytes=(\d+)-(\d+)", range_hdr)
                    start, end = int(rm.group(1)), int(rm.group(2))
                    chunk = data[start : end + 1]
                    self._send(
                        206,
                        chunk,
                        ctype="application/octet-stream",
                        headers={"Content-Range": f"bytes {start}-{end}/{len(data)}"},
                    )
                    return
                data = fake.files[fid]
                self._send(
                    200,
                    {
                        "id": fid,
                        "md5Checksum": hashlib.md5(data).hexdigest(),
                        "size": str(len(data)),
                    },
                )
                return
            self._send(404, {"error": "not found"})

        # --------------------------------------------------- resumable PUT
        def do_PUT(self):
            parsed = urllib.parse.urlparse(self.path)
            m = re.fullmatch(r"/upload-session/(.+)", parsed.path)
            if not m:
                self._send(404, {"error": "not found"})
                return
            session = fake.sessions[m.group(1)]
            body = self._body()
            cr = self.headers.get("Content-Range", "")
            rm = re.fullmatch(r"bytes (\d+)-(\d+)/(\d+)", cr)
            start, end, total = int(rm.group(1)), int(rm.group(2)), int(rm.group(3))
            session["chunks"][start] = body
            received = 0
            while received in session["chunks"]:
                received += len(session["chunks"][received])
            if received >= total:
                data = b"".join(session["chunks"][k] for k in sorted(session["chunks"]))
                fid = fake.next_id("file")
                fake.files[fid] = data
                fake.file_meta[fid] = {
                    **session["meta"],
                    "createdTime": fake.stamp(),
                }
                self._send(200, {"id": fid, "name": session["meta"]["name"]})
            else:
                self._send(308, b"", headers={"Range": f"bytes=0-{received - 1}"})

    return Handler


class ColdStoreTestBase(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.home = Path(self.tmp.name) / "cold-store-home"
        self.fake = FakeDrive()
        self.server = ThreadingHTTPServer(("127.0.0.1", 0), make_handler(self.fake))
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        base = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.store = ColdStore(
            self.home,
            token_url=base + "/token",
            drive_base=base + "/drive/v3",
            upload_base=base,
        )
        self.store.init()
        creds = {
            "client_id": "fake-client-id",
            "client_secret": "fake-secret",
            "refresh_token": "fake-refresh-token",
        }
        self.store._write_private_json(self.store.credentials_path, creds)

    def tearDown(self):
        self.server.shutdown()
        self.server.server_close()
        self.tmp.cleanup()

    def write_src(self, name, data: bytes) -> Path:
        p = Path(self.tmp.name) / name
        p.write_bytes(data)
        return p


class TestAuth(ColdStoreTestBase):
    def test_auth_url_shape(self):
        url = self.store.auth_url()
        qs = urllib.parse.parse_qs(urllib.parse.urlparse(url).query)
        self.assertEqual(qs["scope"], ["https://www.googleapis.com/auth/drive.file"])
        self.assertEqual(qs["access_type"], ["offline"])
        self.assertEqual(qs["prompt"], ["consent"])
        self.assertEqual(qs["redirect_uri"], ["http://localhost"])

    def test_exchange_stores_refresh_token_privately(self):
        result = self.store.auth_exchange("fake-code")
        self.assertEqual(result["stored"], "refresh_token")
        creds = json.loads(self.store.credentials_path.read_text())
        self.assertEqual(creds["refresh_token"], "fake-refresh-token")
        mode = stat.S_IMODE(self.store.credentials_path.stat().st_mode)
        self.assertEqual(mode, 0o600)
        self.assertNotIn("fake-refresh-token", json.dumps(result))


class TestPack(ColdStoreTestBase):
    def test_pack_rows_offsets_and_frame_roundtrip(self):
        a = self.write_src("a.txt", b"hello cold store\n" * 40)
        b = self.write_src("b.bin", bytes(range(256)) * 8)
        result = self.store.pack([str(a), str(b)])
        self.assertTrue(result["bundle_id"].startswith("b_"))
        bundle = Path(result["path"])
        self.assertTrue(bundle.is_file())
        self.assertEqual(bundle.stat().st_size, result["bytes"])

        conn = self.store.db()
        members = conn.execute(
            "SELECT * FROM members WHERE bundle_id=? ORDER BY offset",
            (result["bundle_id"],),
        ).fetchall()
        self.assertEqual(len(members), 2)
        self.assertEqual(members[0]["offset"], 0)
        self.assertEqual(members[1]["offset"], members[0]["length"])
        brow = conn.execute(
            "SELECT * FROM bundles WHERE bundle_id=?", (result["bundle_id"],)
        ).fetchone()
        self.assertEqual(brow["member_count"], 2)
        self.assertEqual(brow["state"], "sealed")
        self.assertEqual(brow["bytes"], result["bytes"])

        raw = bundle.read_bytes()
        for member, src in zip(members, (a, b)):
            frame = raw[member["offset"] : member["offset"] + member["length"]]
            data = self.store.decompress_frame(frame)
            self.assertEqual(hashlib.sha256(data).hexdigest(), member["object_hash"])
            self.assertEqual(data, src.read_bytes())
        orow = conn.execute(
            "SELECT state FROM objects WHERE hash=?", (members[0]["object_hash"],)
        ).fetchone()
        self.assertEqual(orow["state"], "staging")
        conn.close()


class TestUploadFetch(ColdStoreTestBase):
    def test_ensure_folders_idempotent(self):
        first = self.store.ensure_folders()
        second = self.store.ensure_folders()
        self.assertEqual(first, second)
        # root + 3 subfolders + 3 db-backup subfolders (supabase/turso/d1)
        self.assertEqual(len(self.fake.folders), 7)

    def test_upload_verify_and_fetch_roundtrip(self):
        # force multi-chunk resumable uploads against the fake (308 path)
        old_chunk = cold_store.UPLOAD_CHUNK_BYTES
        cold_store.UPLOAD_CHUNK_BYTES = 64
        try:
            a = self.write_src("a.txt", b"roundtrip data " * 30)
            packed = self.store.pack([str(a)])
            uploaded = self.store.upload(packed["bundle_id"])
        finally:
            cold_store.UPLOAD_CHUNK_BYTES = old_chunk
        self.assertTrue(uploaded["verified"])

        conn = self.store.db()
        brow = conn.execute(
            "SELECT * FROM bundles WHERE bundle_id=?", (packed["bundle_id"],)
        ).fetchone()
        self.assertEqual(brow["state"], "verified")
        local = Path(self.store.bundles_dir / f"{packed['bundle_id']}.csb")
        self.assertEqual(brow["drive_md5"], cold_store.md5_file(local))
        orow = conn.execute("SELECT state FROM objects").fetchone()
        self.assertEqual(orow["state"], "cold")
        ledger = conn.execute(
            "SELECT * FROM transfer_ledger WHERE direction='upload'"
        ).fetchall()
        self.assertEqual(len(ledger), 1)
        self.assertEqual(ledger[0]["bytes"], packed["bytes"])
        self.assertEqual(ledger[0]["kind"], "bundle")

        obj_hash = hashlib.sha256(a.read_bytes()).hexdigest()
        out = Path(self.tmp.name) / "restored.txt"
        fetched = self.store.fetch(obj_hash, str(out))
        self.assertEqual(out.read_bytes(), a.read_bytes())
        self.assertEqual(fetched["bundle_id"], packed["bundle_id"])
        dledger = conn.execute(
            "SELECT * FROM transfer_ledger WHERE direction='download'"
        ).fetchall()
        self.assertEqual(len(dledger), 1)
        conn.close()

    def test_fetch_unknown_object_errors(self):
        with self.assertRaises(cold_store.ColdStoreError):
            self.store.fetch("0" * 64, str(Path(self.tmp.name) / "nope"))


class TestStatus(ColdStoreTestBase):
    def test_status_arithmetic_and_rolling_window(self):
        a = self.write_src("a.txt", b"status data " * 20)
        packed = self.store.pack([str(a)])
        self.store.upload(packed["bundle_id"])
        obj_hash = hashlib.sha256(a.read_bytes()).hexdigest()
        self.store.fetch(obj_hash, str(Path(self.tmp.name) / "out.txt"))

        conn = self.store.db()
        with conn:  # an upload 3 days ago must NOT count in the rolling 24h window
            conn.execute(
                "INSERT INTO transfer_ledger (ts, direction, bytes, kind, ref)"
                " VALUES (datetime('now','-3 days'), 'upload', 999999, 'bundle', 'b_old')"
            )
        conn.close()

        d = self.store.status_data()
        self.assertEqual(d["objects_total"], 1)
        self.assertEqual(d["objects_cold"], 1)
        self.assertEqual(d["bundles_total"], 1)
        self.assertEqual(d["bundles_verified"], 1)
        self.assertEqual(d["cold_bytes"], packed["bytes"])
        self.assertEqual(d["uploads_24h_bytes"], packed["bytes"])  # old row excluded
        conn = self.store.db()
        member = conn.execute("SELECT length FROM members").fetchone()
        conn.close()
        self.assertEqual(d["downloads_24h_bytes"], member["length"])
        self.assertEqual(d["daily_upload_hard_stop_bytes"], 700_000_000_000)
        self.assertTrue(d["hard_stop_ok"])
        self.assertTrue(d["within_budget"])
        conn.close()


# ------------------------------------------------------------------ fake D1
class FakeD1:
    """Captures D1 REST query calls; can be told to fail."""

    def __init__(self):
        self.requests = []  # {path, authorization, sql, params}
        self.fail = False


def make_d1_handler(fake: FakeD1):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):  # silence
            pass

        def _send(self, code, payload):
            body = json.dumps(payload).encode()
            self.send_response(code)
            self.send_header("Content-Type", "application/json")
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            self.wfile.write(body)

        def do_POST(self):
            length = int(self.headers.get("Content-Length") or 0)
            body = self.rfile.read(length) if length else b""
            payload = json.loads(body or b"{}")
            fake.requests.append(
                {
                    "path": self.path,
                    "authorization": self.headers.get("Authorization"),
                    "sql": payload.get("sql", ""),
                    "params": payload.get("params", []),
                }
            )
            if fake.fail:
                self._send(
                    500, {"success": False, "errors": [{"message": "fake failure"}]}
                )
                return
            self._send(
                200,
                {
                    "success": True,
                    "errors": [],
                    "messages": [],
                    "result": [{"success": True, "results": []}],
                },
            )

    return Handler


class TestBackups(ColdStoreTestBase):
    def test_ensure_folders_creates_backup_subfolders_once(self):
        first = self.store.ensure_folders()
        for db in cold_store.BACKUP_DBS:
            self.assertIn(f"db-backups/{db}", first)
        count = len(self.fake.folders)
        second = self.store.ensure_folders()
        self.assertEqual(first, second)
        self.assertEqual(len(self.fake.folders), count)

    def test_put_backup_naming_routing_ledger(self):
        dump = self.write_src("dump.sql", b"-- fake supabase dump\n" * 10)
        result = self.store.put_backup("supabase", str(dump))
        self.assertRegex(result["name"], r"^supabase-\d{8}T\d{6}Z-dump\.sql$")
        folders = self.store.ensure_folders()
        meta = self.fake.file_meta[result["drive_file_id"]]
        self.assertEqual(meta["parents"], [folders["db-backups/supabase"]])
        self.assertEqual(meta["name"], result["name"])
        self.assertEqual(self.fake.files[result["drive_file_id"]], dump.read_bytes())
        conn = self.store.db()
        rows = conn.execute(
            "SELECT * FROM transfer_ledger WHERE kind='backup-upload'"
        ).fetchall()
        conn.close()
        self.assertEqual(len(rows), 1)
        self.assertEqual(rows[0]["direction"], "upload")
        self.assertEqual(rows[0]["bytes"], dump.stat().st_size)
        self.assertEqual(rows[0]["ref"], result["name"])

    def test_put_backup_rejects_unknown_db(self):
        dump = self.write_src("dump.sql", b"x")
        with self.assertRaises(cold_store.ColdStoreError):
            self.store.put_backup("oracle", str(dump))

    def test_list_backups_newest_first_and_filtered(self):
        a = self.write_src("a.sql", b"a")
        b = self.write_src("b.sql", b"b")
        self.store.put_backup("supabase", str(a))
        self.store.put_backup("turso", str(b))
        all_rows = self.store.list_backups()
        self.assertEqual(len(all_rows), 2)
        self.assertEqual(all_rows[0]["db"], "turso")  # later createdTime first
        self.assertEqual(all_rows[1]["db"], "supabase")
        only = self.store.list_backups("supabase")
        self.assertEqual(len(only), 1)
        self.assertEqual(only[0]["db"], "supabase")


class TestSyncManifest(ColdStoreTestBase):
    def setUp(self):
        super().setUp()
        self.d1 = FakeD1()
        self.d1_server = ThreadingHTTPServer(("127.0.0.1", 0), make_d1_handler(self.d1))
        self.d1_thread = threading.Thread(
            target=self.d1_server.serve_forever, daemon=True
        )
        self.d1_thread.start()
        self.store.d1_base = f"http://127.0.0.1:{self.d1_server.server_address[1]}"
        a = self.write_src("a.txt", b"sync me " * 10)
        packed = self.store.pack([str(a)])
        self.store.upload(packed["bundle_id"])

    def tearDown(self):
        self.d1_server.shutdown()
        self.d1_server.server_close()
        super().tearDown()

    def _row_counts(self):
        conn = self.store.db()
        counts = {
            t: conn.execute(f"SELECT count(*) n FROM {t}").fetchone()["n"]
            for t in cold_store.MANIFEST_TABLES
        }
        conn.close()
        return counts

    def _env_without_token(self):
        return {k: v for k, v in os.environ.items() if k != "CLOUDFLARE_API_TOKEN"}

    def test_dry_run_counts_without_contacting_d1(self):
        with mock.patch.dict(os.environ, self._env_without_token(), clear=True):
            result = self.store.sync_manifest(dry_run=True)
        self.assertTrue(result["dry_run"])
        expected = self._row_counts()
        for table, n in expected.items():
            self.assertEqual(result["tables"][table]["rows"], n)
            self.assertEqual(result["tables"][table]["statements"], 1 if n else 0)
        self.assertEqual(result["rows"], sum(expected.values()))
        self.assertEqual(self.d1.requests, [])  # nothing was contacted

    def test_sync_posts_idempotent_upserts(self):
        with mock.patch.dict(os.environ, {"CLOUDFLARE_API_TOKEN": "fake-cf-token"}):
            result = self.store.sync_manifest()
            again = self.store.sync_manifest()  # re-sync must be safe + identical
        self.assertFalse(result["dry_run"])
        self.assertEqual(result["rows"], again["rows"])
        self.assertEqual(len(self.d1.requests), 2 * result["statements"])
        expected_path = (
            f"/accounts/{cold_store.DEFAULT_CF_ACCOUNT_ID}"
            f"/d1/database/{cold_store.DEFAULT_CF_D1_DATABASE_ID}/query"
        )
        for req in self.d1.requests:
            self.assertEqual(req["authorization"], "Bearer fake-cf-token")
            self.assertIn(expected_path, req["path"])
            self.assertTrue(req["sql"].startswith("INSERT INTO "))
            self.assertIn("ON CONFLICT", req["sql"])
        by_table = {}
        for req in self.d1.requests[: result["statements"]]:
            m = re.match(r"INSERT INTO (\w+)", req["sql"])
            by_table[m.group(1)] = req
        self.assertIn("ON CONFLICT(hash) DO UPDATE SET", by_table["objects"]["sql"])
        self.assertIn(
            "ON CONFLICT(bundle_id, object_hash) DO UPDATE SET",
            by_table["members"]["sql"],
        )
        self.assertIn("offset=excluded.offset", by_table["members"]["sql"])
        self.assertIn(
            "ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            by_table["meta"]["sql"],
        )
        col_counts = {
            "objects": 8,
            "bundles": 10,
            "members": 4,
            "transfer_ledger": 6,
            "meta": 2,
        }
        for table, req in by_table.items():
            self.assertEqual(len(req["params"]) % col_counts[table], 0)
            self.assertGreater(len(req["params"]), 0)

    def test_sync_many_objects_stays_under_d1_param_limit(self):
        # Regression: D1 allows at most 100 bound parameters per query,
        # and the old flat 50-row batch put 50 x 8 = 400 params into one
        # objects statement (already 112 params at 14 rows), which D1
        # rejected with HTTP 400. Seed enough objects that the old
        # batching would blow the limit, then assert every statement
        # stays under it and every row of every table arrives.
        conn = self.store.db()
        for i in range(30):
            digest = hashlib.sha256(f"seed-object-{i}".encode()).hexdigest()
            conn.execute(
                "INSERT INTO objects (hash, size, state) VALUES (?, ?, 'cold')",
                (digest, 1000 + i),
            )
        conn.commit()
        conn.close()
        expected = self._row_counts()
        # 13 objects x 8 columns = 104 params: the old batching already
        # breaks below this seed size.
        self.assertGreater(expected["objects"], 12)
        with mock.patch.dict(os.environ, {"CLOUDFLARE_API_TOKEN": "fake-cf-token"}):
            result = self.store.sync_manifest()
        col_counts = {
            "objects": 8,
            "bundles": 10,
            "members": 4,
            "transfer_ledger": 6,
            "meta": 2,
        }
        arrived = {t: 0 for t in cold_store.MANIFEST_TABLES}
        for req in self.d1.requests:
            # D1's hard per-query limit is 100 bound parameters.
            self.assertLessEqual(len(req["params"]), 100)
            table = re.match(r"INSERT INTO (\w+)", req["sql"]).group(1)
            self.assertEqual(len(req["params"]) % col_counts[table], 0)
            arrived[table] += len(req["params"]) // col_counts[table]
        self.assertEqual(arrived, expected)
        self.assertEqual(result["rows"], sum(expected.values()))
        # objects batch is 90 // 8 = 11 rows, so 31 rows take 3 statements.
        self.assertEqual(result["tables"]["objects"]["statements"], 3)

    def test_sync_http_error_is_clear(self):
        self.d1.fail = True
        with mock.patch.dict(os.environ, {"CLOUDFLARE_API_TOKEN": "fake-cf-token"}):
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store.sync_manifest()
        self.assertIn("HTTP 500", str(ctx.exception))
        # The D1 API's own error text rides along, not just the status.
        self.assertIn("fake failure", str(ctx.exception))

    def test_sync_requires_token(self):
        with mock.patch.dict(os.environ, self._env_without_token(), clear=True):
            with self.assertRaises(cold_store.ColdStoreError):
                self.store.sync_manifest()


class TestWorkerDecoder(unittest.TestCase):
    """Local proof for the rehydrator Worker: the exact decoder module the
    Worker bundles (worker/src/decoder.js, fzstd) decodes one member frame
    out of a real bundle produced by `cold-store pack`, sha256-checked by
    worker/test/run-local-proof.mjs. Skips when node or the worker's
    node_modules are unavailable (e.g. a bare repo checkout in CI)."""

    def test_worker_decoder_decodes_packed_frame(self):
        worker_dir = Path(__file__).resolve().parent.parent / "worker"
        node = shutil.which("node")
        if not node or not (worker_dir / "node_modules" / "fzstd").is_dir():
            self.skipTest("node or worker node_modules (fzstd) unavailable")
        proc = subprocess.run(
            [node, "test/run-local-proof.mjs"],
            cwd=worker_dir,
            capture_output=True,
            text=True,
            timeout=180,
        )
        self.assertEqual(proc.returncode, 0, proc.stderr[-500:])
        self.assertIn("PASS", proc.stdout)


if __name__ == "__main__":
    unittest.main(verbosity=2)
