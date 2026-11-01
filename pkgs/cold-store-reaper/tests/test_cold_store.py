#!/usr/bin/env python3
"""Tests for cold_store — no network, no real credentials.

Run:  python3 -m unittest discover -s pkgs/cold-store-reaper/tests -v
  or: python3 pkgs/cold-store-reaper/tests/test_cold_store.py
"""

import contextlib
import gzip
import hashlib
import io
import json
import os
import re
import shutil
import sqlite3
import stat
import subprocess
import sys
import tempfile
import threading
import unittest
import urllib.parse
from datetime import date, datetime, timedelta, timezone
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
        self.file_meta = {}  # id -> {name, parents, createdTime, trashed}
        self.sessions = {}  # sid -> {meta, chunks: {start: bytes}, total}
        self.trashed_calls = []  # [{id, body}] from files.update PATCHes
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
                if parent_m:
                    # file listing under a parent (list-backups with no
                    # name filter, get-backup with one); trashed files
                    # are invisible, exactly as `trashed = false` asks.
                    for fid, meta in fake.file_meta.items():
                        if meta.get("trashed"):
                            continue
                        if parent_m.group(1) not in meta.get("parents", []):
                            continue
                        if name_m and meta["name"] != name_m.group(1):
                            continue
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
                    if rm is None:
                        # full download (get-backup): no Range header
                        self._send(200, data, ctype="application/octet-stream")
                        return
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

        # ------------------------------------------------- files.update
        def do_PATCH(self):
            parsed = urllib.parse.urlparse(self.path)
            m = re.fullmatch(r"/drive/v3/files/([^/]+)", parsed.path)
            if not m:
                self._send(404, {"error": "not found"})
                return
            body = json.loads(self._body() or b"{}")
            fid = m.group(1)
            fake.trashed_calls.append({"id": fid, "body": body})
            if fid in fake.file_meta and body.get("trashed") is True:
                fake.file_meta[fid]["trashed"] = True
            self._send(200, {"id": fid, "trashed": bool(body.get("trashed"))})

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

    def seed_drive_file(
        self,
        name,
        parent_id,
        data=b"x",
        trashed=False,
        created="2026-01-01T00:00:00.000Z",
    ):
        """Place a file directly in the fake Drive (no upload round-trip)."""
        fid = self.fake.next_id("file")
        self.fake.files[fid] = data
        self.fake.file_meta[fid] = {
            "name": name,
            "parents": [parent_id],
            "createdTime": created,
            "trashed": trashed,
        }
        return fid


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

    def test_pack_duplicate_content_stores_one_frame(self):
        # #963: duplicate content in one pack call must be stored once —
        # one frame, and bundles accounting equal to the members table.
        data = b"hello cold store\n" * 10
        a = self.write_src("a.txt", data)
        b = self.write_src("b.txt", data)
        result = self.store.pack([str(a), str(b)])
        self.assertEqual(result["members"], 1)

        conn = self.store.db()
        members = conn.execute(
            "SELECT * FROM members WHERE bundle_id=?", (result["bundle_id"],)
        ).fetchall()
        self.assertEqual(len(members), 1)
        brow = conn.execute(
            "SELECT * FROM bundles WHERE bundle_id=?", (result["bundle_id"],)
        ).fetchone()
        self.assertEqual(brow["member_count"], len(members))
        self.assertEqual(brow["live_bytes"], len(data))
        # the bundle holds exactly the one member frame, no orphan bytes
        self.assertEqual(brow["bytes"], members[0]["length"])
        self.assertEqual(result["bytes"], members[0]["length"])
        raw = Path(result["path"]).read_bytes()
        frame = raw[members[0]["offset"] : members[0]["offset"] + members[0]["length"]]
        self.assertEqual(self.store.decompress_frame(frame), data)
        conn.close()

        # packing the same path twice behaves the same
        again = self.store.pack([str(a), str(a)])
        self.assertEqual(again["bundle_id"], result["bundle_id"])
        self.assertEqual(again["members"], 1)


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


class TestUploadLimits(ColdStoreTestBase):
    """#938: upload / put-backup enforce the daily hard stop and the
    project budget BEFORE any bytes move, using status's accounting."""

    def set_meta(self, key, value):
        conn = self.store.db()
        with conn:
            conn.execute("UPDATE meta SET value=? WHERE key=?", (str(value), key))
        conn.close()

    def test_upload_refused_at_daily_stop_no_drive_writes(self):
        src = self.write_src("a.txt", b"limit data " * 20)
        packed = self.store.pack([str(src)])
        # rolling-24h uploads (0) + this bundle would cross the stop
        self.set_meta("daily_upload_hard_stop_bytes", packed["bytes"] - 1)
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.upload(packed["bundle_id"])
        self.assertIn("daily upload hard stop", str(ctx.exception))
        # refused before uploading: no session, no file, no folder, and
        # the bundle was never marked uploading
        self.assertEqual(self.fake.sessions, {})
        self.assertEqual(self.fake.files, {})
        self.assertEqual(self.fake.folders, [])
        conn = self.store.db()
        brow = conn.execute(
            "SELECT state FROM bundles WHERE bundle_id=?", (packed["bundle_id"],)
        ).fetchone()
        self.assertEqual(brow["state"], "sealed")
        conn.close()

    def test_upload_refused_when_ledger_alone_crosses_stop(self):
        src = self.write_src("a.txt", b"limit data " * 20)
        packed = self.store.pack([str(src)])
        self.set_meta("daily_upload_hard_stop_bytes", packed["bytes"] + 10)
        conn = self.store.db()
        with conn:  # 20 B already uploaded inside the rolling window
            self.store.ledger_add(conn, "upload", 20, "bundle", "b_earlier")
        conn.close()
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.upload(packed["bundle_id"])
        self.assertIn("daily upload hard stop", str(ctx.exception))
        self.assertEqual(self.fake.sessions, {})

    def test_put_backup_refused_at_project_budget(self):
        self.set_meta("project_budget_bytes", 10)
        src = self.write_src("dump.sql", b"x" * 100)
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.put_backup("d1", str(src))
        self.assertIn("project budget", str(ctx.exception))
        # refused before the folder lookup could create anything
        self.assertEqual(self.fake.sessions, {})
        self.assertEqual(self.fake.files, {})
        self.assertEqual(self.fake.folders, [])

    def test_put_backup_cumulative_budget_refusal(self):
        # #1028: individually-fitting backups must be refused once
        # their CUMULATIVE total crosses the project budget — prior
        # backups count, via the transfer ledger's backup-upload rows.
        self.set_meta("project_budget_bytes", 150)
        first = self.write_src("a.sql", b"x" * 100)
        second = self.write_src("b.sql", b"y" * 100)
        result = self.store.put_backup("d1", str(first))
        self.assertTrue(result["drive_file_id"])
        files_after_first = dict(self.fake.files)
        sessions_after_first = dict(self.fake.sessions)
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.put_backup("d1", str(second))
        self.assertIn("project budget", str(ctx.exception))
        # refused before any Drive write
        self.assertEqual(self.fake.files, files_after_first)
        self.assertEqual(self.fake.sessions, sessions_after_first)
        # status shares the accounting: 100 B of backups counted
        d = self.store.status_data()
        self.assertEqual(d["backup_bytes"], 100)
        self.assertEqual(d["cold_bytes"], 0)
        self.assertTrue(d["within_budget"])

    def test_backup_prune_ledger_row_decrements_backup_bytes(self):
        # #1028: a backup-prune trash row (prune_backups' ledger
        # record) decrements the counted backup bytes, so pruned
        # backups stop consuming the project budget.
        self.set_meta("project_budget_bytes", 150)
        first = self.write_src("a.sql", b"x" * 100)
        self.store.put_backup("d1", str(first))
        self.assertEqual(self.store.status_data()["backup_bytes"], 100)
        conn = self.store.db()
        with conn:
            self.store.ledger_add(conn, "trash", 100, "backup-prune", "gone.sql")
        conn.close()
        self.assertEqual(self.store.status_data()["backup_bytes"], 0)
        second = self.write_src("b.sql", b"y" * 100)
        result = self.store.put_backup("d1", str(second))
        self.assertTrue(result["drive_file_id"])

    def test_just_under_limits_succeeds(self):
        src = self.write_src("a.txt", b"limit data " * 20)
        packed = self.store.pack([str(src)])
        self.set_meta("daily_upload_hard_stop_bytes", packed["bytes"] + 1000)
        self.set_meta("project_budget_bytes", packed["bytes"] + 1000)
        uploaded = self.store.upload(packed["bundle_id"])
        self.assertTrue(uploaded["verified"])
        # put-backup also fits: ledger/cold totals grow by the bundle,
        # so widen both limits by the backup size plus headroom
        backup = self.write_src("dump.sql", b"y" * 100)
        self.set_meta("daily_upload_hard_stop_bytes", packed["bytes"] + 100 + 1000)
        self.set_meta("project_budget_bytes", packed["bytes"] + 100 + 1000)
        result = self.store.put_backup("d1", str(backup))
        self.assertTrue(result["drive_file_id"])
        self.assertEqual(result["bytes"], 100)


class TestStateHygiene(ColdStoreTestBase):
    """#940: the access token is never persisted; state.json is 0600."""

    def test_access_token_never_persisted_and_state_is_0600(self):
        token = self.store.access_token()
        self.assertEqual(token, "fake-access")
        # in-memory reuse — the second call needs no new grant
        self.assertEqual(self.store.access_token(), "fake-access")
        self.store.ensure_folders()  # an auth-touching op that saves state
        raw = self.store.state_path.read_bytes()
        self.assertNotIn(b"fake-access", raw)
        state = json.loads(raw)
        self.assertNotIn("access_token", state)
        self.assertIn("folders", state)
        mode = stat.S_IMODE(self.store.state_path.stat().st_mode)
        self.assertEqual(mode, 0o600)

    def test_legacy_token_in_state_is_purged_on_save(self):
        state = self.store.load_state()
        state["access_token"] = "fake-access"  # as a pre-#940 file held it
        self.store.save_state(state)
        raw = self.store.state_path.read_bytes()
        self.assertNotIn(b"fake-access", raw)
        self.assertNotIn("access_token", json.loads(raw))
        mode = stat.S_IMODE(self.store.state_path.stat().st_mode)
        self.assertEqual(mode, 0o600)


# ------------------------------------------------------------------ fake D1
class FakeD1:
    """Captures D1 REST query calls; can be told to fail.

    If `db` (a sqlite3 connection) is set, SELECT statements are executed
    against it for real and their rows returned — enough of D1 for the
    backup-d1 export path. Non-SELECT statements are only recorded.
    """

    def __init__(self):
        self.requests = []  # {path, authorization, sql, params}
        self.fail = False
        self.db = None


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
            sql = payload.get("sql", "")
            if fake.db is not None and sql.lstrip().upper().startswith("SELECT"):
                cur = fake.db.execute(sql)
                cols = [d[0] for d in cur.description]
                rows = [dict(zip(cols, r)) for r in cur.fetchall()]
                self._send(
                    200,
                    {
                        "success": True,
                        "errors": [],
                        "messages": [],
                        "result": [{"success": True, "results": rows}],
                    },
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


# -------------------------------------------------------------------- fake S3
class FakeS3:
    """In-memory state for the fake R2 S3 endpoint.

    Listings are served in pages of `page_size` objects no matter what
    the client asks for, so pagination is always exercised. A key in
    `truncate_once` gets one response whose body stops halfway while
    its Content-Length promises the whole object — the truncated-read
    case the real client must retry. A key in `truncate_always` gets
    that response every time — the systematic large-object truncation
    that only the curl fallback survives.
    """

    BUCKET = "supabase-backups"

    def __init__(self):
        self.objects = {}  # key -> {"body": bytes, "last_modified": str}
        self.deleted = []  # keys, in DELETE order
        self.requests = []  # {"method", "path", "query", "headers"}
        self.truncate_once = set()
        self.truncate_always = set()
        self._truncated = set()
        self.page_size = 2
        # Transport-failure injection for LIST requests: a queue of
        # modes consumed one per listing call — "truncate" (headers
        # promise the full body, connection closes halfway through
        # it) or "drop" (connection closes with no response at all).
        # list_attempts counts every listing call, failed or not.
        self.list_failures = []
        self.list_attempts = 0
        # Broken-pagination injection for LIST responses: None serves
        # well-behaved advancing tokens; "missing" truncates with no
        # NextContinuationToken, "empty" with an empty one, "repeat"
        # with the token the current page was requested with (no
        # progress). A correct client aborts; a broken one loops.
        self.list_token_mode = None

    def put(self, key, body: bytes, last_modified: str):
        self.objects[key] = {"body": body, "last_modified": last_modified}

    def get_requests(self, key):
        return [
            r
            for r in self.requests
            if r["method"] == "GET" and r["path"].endswith("/" + key)
        ]


def make_s3_handler(fake: FakeS3):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):  # silence
            pass

        def _record(self):
            parsed = urllib.parse.urlparse(self.path)
            fake.requests.append(
                {
                    "method": self.command,
                    "path": parsed.path,
                    "query": urllib.parse.parse_qs(parsed.query),
                    "headers": {k.lower(): v for k, v in self.headers.items()},
                }
            )
            return parsed

        def _send(self, code, body=b"", ctype="application/xml"):
            self.send_response(code)
            self.send_header("Content-Type", ctype)
            self.send_header("Content-Length", str(len(body)))
            self.end_headers()
            if body:
                self.wfile.write(body)

        def _error(self, code, message):
            body = (
                '<?xml version="1.0" encoding="UTF-8"?>'
                f"<Error><Code>NoSuchKey</Code><Message>{message}</Message></Error>"
            ).encode()
            self._send(code, body)

        def do_GET(self):
            parsed = self._record()
            qs = urllib.parse.parse_qs(parsed.query)
            rest = parsed.path.lstrip("/").split("/", 1)
            if len(rest) == 1:  # bucket root: ListObjectsV2
                fake.list_attempts += 1
                failure = fake.list_failures.pop(0) if fake.list_failures else None
                if failure == "drop":
                    # No response at all: the client sees the peer
                    # vanish (RemoteDisconnected) and must retry.
                    self.close_connection = True
                    return
                start = int(qs.get("continuation-token", ["0"])[0])
                keys = sorted(fake.objects)
                page = keys[start : start + fake.page_size]
                truncated = start + fake.page_size < len(keys)
                contents = "".join(
                    f"<Contents><Key>{k}</Key>"
                    f"<LastModified>{fake.objects[k]['last_modified']}</LastModified>"
                    f"<Size>{len(fake.objects[k]['body'])}</Size></Contents>"
                    for k in page
                )
                if not truncated:
                    token = ""
                elif fake.list_token_mode == "missing":
                    token = ""
                elif fake.list_token_mode == "empty":
                    token = "<NextContinuationToken></NextContinuationToken>"
                elif fake.list_token_mode == "repeat":
                    token = f"<NextContinuationToken>{start}</NextContinuationToken>"
                else:
                    token = (
                        f"<NextContinuationToken>{start + fake.page_size}"
                        "</NextContinuationToken>"
                    )
                body = (
                    '<?xml version="1.0" encoding="UTF-8"?>'
                    '<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
                    f"<Name>{FakeS3.BUCKET}</Name>"
                    f"<IsTruncated>{str(truncated).lower()}</IsTruncated>"
                    f"{contents}{token}</ListBucketResult>"
                ).encode()
                if failure == "truncate":
                    # Headers promise the full body; the connection
                    # closes halfway — the client's read is truncated
                    # (IncompleteRead) and it must retry the listing.
                    self.send_response(200)
                    self.send_header("Content-Type", "application/xml")
                    self.send_header("Content-Length", str(len(body)))
                    self.end_headers()
                    self.wfile.write(body[: len(body) // 2])
                    self.close_connection = True
                    return
                self._send(200, body)
                return
            key = urllib.parse.unquote(rest[1])
            obj = fake.objects.get(key)
            if obj is None:
                self._error(404, "The specified key does not exist.")
                return
            body = obj["body"]
            if key in fake.truncate_always or (
                key in fake.truncate_once and key not in fake._truncated
            ):
                fake._truncated.add(key)
                self.send_response(200)
                self.send_header("Content-Type", "application/octet-stream")
                self.send_header("Content-Length", str(len(body)))
                self.end_headers()
                self.wfile.write(body[: len(body) // 2])
                self.close_connection = True
                return
            self._send(200, body, ctype="application/octet-stream")

        def do_HEAD(self):
            parsed = self._record()
            rest = parsed.path.lstrip("/").split("/", 1)
            key = urllib.parse.unquote(rest[1]) if len(rest) > 1 else ""
            obj = fake.objects.get(key)
            if obj is None:
                self.send_response(404)
                self.send_header("Content-Length", "0")
                self.end_headers()
                return
            self.send_response(200)
            self.send_header("Content-Type", "application/octet-stream")
            self.send_header("Content-Length", str(len(obj["body"])))
            self.end_headers()

        def do_DELETE(self):
            parsed = self._record()
            rest = parsed.path.lstrip("/").split("/", 1)
            key = urllib.parse.unquote(rest[1])
            if key not in fake.objects:
                self._error(404, "The specified key does not exist.")
                return
            del fake.objects[key]
            fake.deleted.append(key)
            self._send(204)

    return Handler


class S3TestBase(ColdStoreTestBase):
    """ColdStoreTestBase plus a fake S3 endpoint wired into the store,
    with dummy R2 credentials in the environment (values are fake; the
    real ones are operator-supplied from Bitwarden, never stored)."""

    R2_ENV = {
        "R2_ACCESS_KEY_ID": "test-access-key",
        "R2_SECRET_ACCESS_KEY": "test-secret-key",
    }

    def setUp(self):
        super().setUp()
        self.s3 = FakeS3()
        self.s3_server = ThreadingHTTPServer(("127.0.0.1", 0), make_s3_handler(self.s3))
        self.s3_thread = threading.Thread(
            target=self.s3_server.serve_forever, daemon=True
        )
        self.s3_thread.start()
        self.s3_base_url = f"http://127.0.0.1:{self.s3_server.server_address[1]}"
        self.store.s3_base = self.s3_base_url
        # Transport retries must not slow the suite down.
        self.store.s3_backoff_seconds = 0
        self._r2_env = mock.patch.dict(os.environ, self.R2_ENV)
        self._r2_env.start()

    def tearDown(self):
        self._r2_env.stop()
        self.s3_server.shutdown()
        self.s3_server.server_close()
        super().tearDown()

    @staticmethod
    def key_for(obj_hash):
        return f"pg/muse-dev/2026/10/{obj_hash}.api.jsonl.zst"

    def make_cold(self, contents: dict) -> dict:
        """Pack {name: bytes} into one bundle and upload it against the
        fake Drive: every object ends up cold in a verified bundle.
        Returns {name: sha256}."""
        files = [str(self.write_src(name, data)) for name, data in contents.items()]
        packed = self.store.pack(files)
        self.store.upload(packed["bundle_id"])
        return {
            name: hashlib.sha256(data).hexdigest() for name, data in contents.items()
        }


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


class TestPruneBackups(ColdStoreTestBase):
    """Retention scenario, hand-computed (all dates 2026):

    35 consecutive days of d1 backups, one per day at 02:00, ending
    Mar 31, plus same-day dupes on Mar 31 at 01:00 and 03:00 (37 files).
    Feb 25..Mar 1 all fall in ISO week 9; months present are Feb + Mar.

    - daily   (newest 30 distinct days): Mar 2..Mar 31 newest-per-day
    - weekly  (newest 12 distinct ISO weeks; only 6 present): newest of
      W9..W14 = Mar 1, Mar 8, Mar 15, Mar 22, Mar 29, Mar 31 03:00
    - monthly (newest 12 distinct months; only 2 present): Feb 28,
      Mar 31 03:00
    Union keep = 32. Prune = Feb 25, Feb 26, Feb 27, Mar 31 01:00 and
    Mar 31 02:00 (the day's newest, 03:00, is the keeper).
    """

    EXPECTED_PRUNE = {
        "d1-20260225T020000Z-export.sql",
        "d1-20260226T020000Z-export.sql",
        "d1-20260227T020000Z-export.sql",
        "d1-20260331T010000Z-export.sql",
        "d1-20260331T020000Z-export.sql",
    }

    def setUp(self):
        super().setUp()
        folders = self.store.ensure_folders()
        self.d1_folder = folders["db-backups/d1"]

    def _seed_series(self):
        end = date(2026, 3, 31)
        ids = {}
        for i in range(35):
            d = end - timedelta(days=i)
            name = f"d1-{d:%Y%m%d}T020000Z-export.sql"
            ids[name] = self.seed_drive_file(name, self.d1_folder, data=b"dump")
        for hh in ("010000", "030000"):
            name = f"d1-20260331T{hh}Z-export.sql"
            ids[name] = self.seed_drive_file(name, self.d1_folder, data=b"dump")
        return ids

    def test_prune_dry_run_plan(self):
        ids = self._seed_series()
        self.seed_drive_file("d1-random.sql", self.d1_folder, data=b"?")
        self.seed_drive_file("notes.txt", self.d1_folder, data=b"?")
        trashed_id = self.seed_drive_file(
            "d1-20200101T020000Z-export.sql", self.d1_folder, data=b"?", trashed=True
        )
        plans = self.store.prune_backups("d1")
        self.assertEqual(len(plans), 1)
        plan = plans[0]
        self.assertFalse(plan["applied"])
        keep = {f["name"]: f for f in plan["keep"]}
        prune = {f["name"] for f in plan["prune"]}
        self.assertEqual(prune, self.EXPECTED_PRUNE)
        self.assertEqual(len(keep), 32)
        self.assertEqual(keep["d1-20260301T020000Z-export.sql"]["reasons"], ["weekly"])
        self.assertEqual(keep["d1-20260228T020000Z-export.sql"]["reasons"], ["monthly"])
        self.assertEqual(
            keep["d1-20260331T030000Z-export.sql"]["reasons"],
            ["daily", "monthly", "weekly"],
        )
        self.assertEqual(keep["d1-20260302T020000Z-export.sql"]["reasons"], ["daily"])
        skipped = {f["name"] for f in plan["unparseable"]}
        self.assertEqual(skipped, {"d1-random.sql", "notes.txt"})
        # the already-trashed file is invisible to the plan entirely
        all_ids = {f["id"] for f in plan["keep"] + plan["prune"] + plan["unparseable"]}
        self.assertNotIn(trashed_id, all_ids)
        self.assertEqual(len(all_ids), 39)
        # dry-run touches nothing: no PATCH, no ledger rows
        self.assertEqual(self.fake.trashed_calls, [])
        conn = self.store.db()
        n = conn.execute(
            "SELECT count(*) n FROM transfer_ledger WHERE kind='backup-prune'"
        ).fetchone()["n"]
        conn.close()
        self.assertEqual(n, 0)
        self.assertEqual(set(ids) - set(keep) - prune, set())

    def test_prune_apply_trashes_and_ledgers(self):
        ids = self._seed_series()
        plans = self.store.prune_backups("d1", apply=True)
        plan = plans[0]
        self.assertTrue(plan["applied"])
        prune_ids = {ids[name] for name in self.EXPECTED_PRUNE}
        self.assertEqual({c["id"] for c in self.fake.trashed_calls}, prune_ids)
        for call in self.fake.trashed_calls:
            self.assertEqual(call["body"], {"trashed": True})
        for fid in prune_ids:
            self.assertTrue(self.fake.file_meta[fid]["trashed"])
        conn = self.store.db()
        rows = conn.execute(
            "SELECT * FROM transfer_ledger WHERE kind='backup-prune'"
        ).fetchall()
        conn.close()
        self.assertEqual(len(rows), len(self.EXPECTED_PRUNE))
        self.assertEqual({r["ref"] for r in rows}, self.EXPECTED_PRUNE)
        for r in rows:
            self.assertEqual(r["direction"], "trash")
        # after applying, the folder's visible set is exactly the keepers
        self.assertEqual(len(self.store.list_backups("d1")), 32)

    def test_prune_long_history_oldest_pruned(self):
        end = date(2026, 3, 31)
        for i in range(400):
            d = end - timedelta(days=i)
            self.seed_drive_file(
                f"d1-{d:%Y%m%d}T020000Z-export.sql", self.d1_folder, data=b"x"
            )
        plan = self.store.prune_backups("d1")[0]
        keep = {f["name"] for f in plan["keep"]}
        prune = {f["name"] for f in plan["prune"]}
        self.assertEqual(len(keep) + len(prune), 400)
        oldest = f"d1-{end - timedelta(days=399):%Y%m%d}T020000Z-export.sql"
        self.assertIn(oldest, prune)
        for i in range(30):  # newest-per-day of the last 30 days survives
            d = end - timedelta(days=i)
            self.assertIn(f"d1-{d:%Y%m%d}T020000Z-export.sql", keep)
        # the union can never exceed 30 daily + 12 weekly + 12 monthly
        self.assertLessEqual(len(keep), 30 + 12 + 12)

    def test_prune_all_dbs_and_unknown_db(self):
        plans = self.store.prune_backups()
        self.assertEqual([p["db"] for p in plans], list(cold_store.BACKUP_DBS))
        with self.assertRaises(cold_store.ColdStoreError):
            self.store.prune_backups("oracle")


class TestGetBackup(ColdStoreTestBase):
    def test_get_backup_roundtrip(self):
        data = b"-- turso dump\n" + bytes(range(256)) * 4
        src = self.write_src("libsql.sql", data)
        put = self.store.put_backup("turso", str(src))
        out = Path(self.tmp.name) / "restored.sql"
        result = self.store.get_backup("turso", put["name"], str(out))
        self.assertEqual(out.read_bytes(), data)
        self.assertEqual(result["bytes"], len(data))
        self.assertEqual(result["sha256"], hashlib.sha256(data).hexdigest())
        self.assertEqual(result["drive_file_id"], put["drive_file_id"])

    def test_get_backup_missing_and_unknown_db(self):
        with self.assertRaises(cold_store.ColdStoreError):
            self.store.get_backup(
                "d1", "d1-20260101T000000Z-nope.sql", str(Path(self.tmp.name) / "x")
            )
        with self.assertRaises(cold_store.ColdStoreError):
            self.store.get_backup("oracle", "x", str(Path(self.tmp.name) / "x"))


class TestBackupD1(ColdStoreTestBase):
    def setUp(self):
        super().setUp()
        self.d1 = FakeD1()
        # check_same_thread=False: the HTTP handler thread runs the queries.
        self.d1.db = sqlite3.connect(":memory:", check_same_thread=False)
        self.d1.db.executescript(
            """
            CREATE TABLE objects (hash TEXT PRIMARY KEY, size INTEGER, note TEXT);
            CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);
            INSERT INTO objects VALUES ('aaa', 10, 'plain');
            INSERT INTO objects VALUES ('bbb', 20, 'o''brien');
            INSERT INTO objects VALUES ('ccc', 30, NULL);
            INSERT INTO meta VALUES ('schema_version', '1');
            INSERT INTO meta VALUES ('project_budget_bytes', '3298534883328');
            """
        )
        self.d1_server = ThreadingHTTPServer(("127.0.0.1", 0), make_d1_handler(self.d1))
        self.d1_thread = threading.Thread(
            target=self.d1_server.serve_forever, daemon=True
        )
        self.d1_thread.start()
        self.store.d1_base = f"http://127.0.0.1:{self.d1_server.server_address[1]}"

    def tearDown(self):
        self.d1_server.shutdown()
        self.d1_server.server_close()
        super().tearDown()

    def test_backup_d1_export_loads_back_identically(self):
        with mock.patch.dict(os.environ, {"CLOUDFLARE_API_TOKEN": "fake-cf-token"}):
            result = self.store.backup_d1()
        self.assertRegex(
            result["name"], r"^d1-\d{8}T\d{6}Z-cold-store-manifest-export\.sql$"
        )
        self.assertEqual(result["tables"], ["meta", "objects"])
        self.assertEqual(result["rows"], 5)
        folders = self.store.ensure_folders()
        meta = self.fake.file_meta[result["drive_file_id"]]
        self.assertEqual(meta["parents"], [folders["db-backups/d1"]])
        sql_text = self.fake.files[result["drive_file_id"]].decode()
        self.assertTrue(
            sql_text.startswith("-- cold-store-manifest D1 export (via REST query API)")
        )
        self.assertIn("-- tables: meta, objects", sql_text)
        self.assertIn("'o''brien'", sql_text)  # quote escaping survived
        fresh = sqlite3.connect(":memory:")
        fresh.executescript(
            "CREATE TABLE objects (hash TEXT PRIMARY KEY, size INTEGER, note TEXT);"
            "CREATE TABLE meta (key TEXT PRIMARY KEY, value TEXT);"
        )
        fresh.executescript(sql_text)
        self.assertEqual(
            fresh.execute(
                "SELECT hash, size, note FROM objects ORDER BY hash"
            ).fetchall(),
            [("aaa", 10, "plain"), ("bbb", 20, "o'brien"), ("ccc", 30, None)],
        )
        self.assertEqual(
            fresh.execute("SELECT key, value FROM meta ORDER BY key").fetchall(),
            [("project_budget_bytes", "3298534883328"), ("schema_version", "1")],
        )

    def test_backup_d1_requires_token(self):
        env = {k: v for k, v in os.environ.items() if k != "CLOUDFLARE_API_TOKEN"}
        with mock.patch.dict(os.environ, env, clear=True):
            with self.assertRaises(cold_store.ColdStoreError):
                self.store.backup_d1()


class TestS3Client(S3TestBase):
    def test_sigv4_header_shape(self):
        self.s3.put("pg/muse-dev/manifest.json", b"{}", "2026-10-01T00:00:00.000Z")
        self.store.s3_list_objects("supabase-backups")
        req = self.s3.requests[0]
        auth = req["headers"]["authorization"]
        self.assertIsNotNone(
            re.fullmatch(
                r"AWS4-HMAC-SHA256 Credential=test-access-key/\d{8}"
                r"/auto/s3/aws4_request, SignedHeaders=host;x-amz-content-sha256"
                r";x-amz-date, Signature=[0-9a-f]{64}",
                auth,
            ),
            auth,
        )
        # GET signs the empty-payload hash
        self.assertEqual(
            req["headers"]["x-amz-content-sha256"], hashlib.sha256(b"").hexdigest()
        )
        self.assertRegex(req["headers"]["x-amz-date"], r"^\d{8}T\d{6}Z$")
        self.assertEqual(req["query"]["list-type"], ["2"])

    def test_list_pagination(self):
        for i in range(5):
            self.s3.put(f"pg/muse-dev/k{i}.bin", b"x", "2026-10-01T00:00:00.000Z")
        objects = self.store.s3_list_objects("supabase-backups")
        self.assertEqual([o["key"] for o in objects], sorted(self.s3.objects))
        list_calls = [r for r in self.s3.requests if r["path"] == "/supabase-backups"]
        self.assertEqual(len(list_calls), 3)  # fake pages are 2, 2, 1
        self.assertNotIn("continuation-token", list_calls[0]["query"])
        self.assertEqual(list_calls[1]["query"]["continuation-token"], ["2"])
        self.assertEqual(list_calls[2]["query"]["continuation-token"], ["4"])
        for o in objects:
            self.assertEqual(o["size"], 1)
            self.assertIsNotNone(o["last_modified"])

    def _list_calls(self):
        return [r for r in self.s3.requests if r["path"] == "/supabase-backups"]

    def _put_five_for_pagination(self):
        for i in range(5):
            self.s3.put(f"pg/muse-dev/k{i}.bin", b"x", "2026-10-01T00:00:00.000Z")

    def test_list_pagination_missing_token_aborts(self):
        self._put_five_for_pagination()
        self.s3.list_token_mode = "missing"
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.s3_list_objects("supabase-backups")
        self.assertIn("supabase-backups", str(ctx.exception))
        self.assertIn("missing or empty", str(ctx.exception))
        self.assertEqual(len(self._list_calls()), 1)  # aborts on page 1

    def test_list_pagination_empty_token_aborts(self):
        self._put_five_for_pagination()
        self.s3.list_token_mode = "empty"
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.s3_list_objects("supabase-backups")
        self.assertIn("supabase-backups", str(ctx.exception))
        self.assertEqual(len(self._list_calls()), 1)

    def test_list_pagination_repeated_token_aborts(self):
        self._put_five_for_pagination()
        self.s3.list_token_mode = "repeat"
        with self.assertRaises(cold_store.ColdStoreError) as ctx:
            self.store.s3_list_objects("supabase-backups")
        self.assertIn("supabase-backups", str(ctx.exception))
        self.assertIn("non-advancing", str(ctx.exception))
        # Page 1 hands back token "0"; the re-request returns it again
        # and the guard fires — two calls, never a third.
        self.assertEqual(len(self._list_calls()), 2)

    def test_missing_credentials_error(self):
        env = {
            k: v
            for k, v in os.environ.items()
            if k not in ("R2_ACCESS_KEY_ID", "R2_SECRET_ACCESS_KEY")
        }
        with mock.patch.dict(os.environ, env, clear=True):
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store.s3_list_objects("supabase-backups")
        self.assertIn("R2_ACCESS_KEY_ID", str(ctx.exception))


class TestS3TransportRetry(S3TestBase):
    """Transport-level retries in the S3 layer: through the sandbox
    egress proxy, R2 responses are intermittently truncated mid-body
    or dropped outright, and before the fix those escaped
    _s3_request as raw http.client tracebacks that killed
    backup-supabase / evict-scan at the LIST step. Backoff is zeroed
    in S3TestBase.setUp, so these run instantly."""

    def _put_five(self):
        for i in range(5):
            self.s3.put(f"pg/muse-dev/k{i}.bin", b"x", "2026-10-01T00:00:00.000Z")

    def test_list_truncated_mid_body_retries_and_returns_all(self):
        self._put_five()
        self.s3.list_failures = ["truncate"]
        objects = self.store.s3_list_objects("supabase-backups")
        self.assertEqual([o["key"] for o in objects], sorted(self.s3.objects))
        # 1 truncated attempt + the 3 good pages (fake pages are 2,2,1)
        self.assertEqual(self.s3.list_attempts, 4)

    def test_list_remote_disconnect_retries(self):
        self.s3.put("pg/muse-dev/a.bin", b"x", "2026-10-01T00:00:00.000Z")
        self.s3.put("pg/muse-dev/b.bin", b"x", "2026-10-01T00:00:00.000Z")
        self.s3.list_failures = ["drop"]
        objects = self.store.s3_list_objects("supabase-backups")
        self.assertEqual([o["key"] for o in objects], sorted(self.s3.objects))
        self.assertEqual(self.s3.list_attempts, 2)  # dropped + retried page

    def test_list_permanent_truncation_raises_named_error_after_4(self):
        self.s3.put("pg/muse-dev/a.bin", b"x", "2026-10-01T00:00:00.000Z")
        self.s3.list_failures = ["truncate"] * 10

        # urllib exhausts its 4 attempts, then the GET curl fallback
        # (TestS3RequestCurlFallback) is attempted and also fails —
        # the exhaustion error must name both.
        def failing_status(url, access, secret):
            raise cold_store.ColdStoreError("curl fallback failed (exit 18): simulated")

        with mock.patch.object(self.store, "_curl_fetch_status", failing_status):
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store.s3_list_objects("supabase-backups")
        message = str(ctx.exception)
        self.assertIn("S3 GET supabase-backups failed after 4 attempts", message)
        self.assertIn("IncompleteRead", message)
        self.assertIn("curl fallback also failed", message)
        self.assertEqual(self.s3.list_attempts, 4)  # exactly 4, no more

    def test_delete_transport_failure_retries(self):
        # DELETE is idempotent, so it gets the transport retry too:
        # the first attempt dies at the transport level, the retry
        # goes through and the delete lands exactly once.
        import http.client

        self.s3.put("pg/muse-dev/a.bin", b"x", "2026-10-01T00:00:00.000Z")
        real_http = cold_store.http_request
        state = {"n": 0}

        def flaky_http(method, url, headers=None, data=None, timeout=120):
            state["n"] += 1
            if state["n"] == 1:
                raise http.client.RemoteDisconnected("simulated drop")
            return real_http(method, url, headers=headers, data=data, timeout=timeout)

        with mock.patch.object(cold_store, "http_request", flaky_http):
            self.store.s3_delete_object("supabase-backups", "pg/muse-dev/a.bin")
        self.assertEqual(state["n"], 2)
        self.assertEqual(self.s3.deleted, ["pg/muse-dev/a.bin"])


class TestBackupSupabase(S3TestBase):
    def test_files_newest_non_manifest_with_truncation_retry(self):
        older = b"older backup " * 40
        newest = b"newest backup payload " * 60
        old_key = self.key_for(hashlib.sha256(older).hexdigest())
        new_key = self.key_for(hashlib.sha256(newest).hexdigest())
        self.s3.put(old_key, older, "2026-10-20T02:00:00.000Z")
        self.s3.put(new_key, newest, "2026-10-21T02:00:00.000Z")
        # the bucket manifest is newer still — and is never a dump filing
        self.s3.put("pg/muse-dev/manifest.json", b"{}", "2026-10-22T02:00:00.000Z")
        # the first GET of the newest object arrives truncated (the
        # live proxy failure mode); the client must retry the whole GET
        self.s3.truncate_once.add(new_key)
        result = self.store.backup_supabase()
        self.assertEqual(result["source_key"], new_key)
        self.assertRegex(
            result["name"],
            r"^supabase-\d{8}T\d{6}Z-" + re.escape(new_key.rsplit("/", 1)[-1]) + r"$",
        )
        self.assertEqual(result["sha256"], hashlib.sha256(newest).hexdigest())
        folders = self.store.ensure_folders()
        meta = self.fake.file_meta[result["drive_file_id"]]
        self.assertEqual(meta["parents"], [folders["db-backups/supabase"]])
        # byte-count verification did its job: the filed copy is the
        # whole object, not the truncated first response
        self.assertEqual(self.fake.files[result["drive_file_id"]], newest)
        self.assertGreaterEqual(len(self.s3.get_requests(new_key)), 2)

    def test_no_backup_objects_errors(self):
        self.s3.put("pg/muse-dev/manifest.json", b"{}", "2026-10-22T02:00:00.000Z")
        with self.assertRaises(cold_store.ColdStoreError):
            self.store.backup_supabase()


class TestS3CurlFallback(S3TestBase):
    """The curl fallback in s3_get_object: multi-megabyte R2
    responses truncate SYSTEMATICALLY through the sandbox egress
    proxy (live: every urllib attempt cut at exactly 409,600 bytes),
    which no amount of urllib retrying survives — so once urllib is
    exhausted, one curl attempt (stubbed here at the _curl_fetch
    transport seam; no network, no real curl) delivers the object,
    byte-count verified like the urllib path."""

    BODY = b"large backup payload " * 500

    def _persistent_truncation(self, body=None):
        """The newest bucket object, truncated on every urllib GET."""
        body = self.BODY if body is None else body
        key = self.key_for(hashlib.sha256(body).hexdigest())
        self.s3.put(key, body, "2026-10-21T02:00:00.000Z")
        # the bucket manifest is newer still — and is never a dump filing
        self.s3.put("pg/muse-dev/manifest.json", b"{}", "2026-10-22T02:00:00.000Z")
        self.s3.truncate_always.add(key)
        return key

    def _stub_curl(self, body):
        """Patch the store's curl transport; return the call log. Each
        entry records how many urllib GETs of `key` had happened when
        curl was invoked, proving the fallback fires only after
        urllib exhaustion, never alongside it."""
        calls = []

        def fake_fetch(url, access, secret):
            calls.append(
                {
                    "url": url,
                    "access": access,
                    "secret": secret,
                    "get_requests": {
                        k: len(self.s3.get_requests(k)) for k in self.s3.objects
                    },
                }
            )
            return body

        return mock.patch.object(self.store, "_curl_fetch", fake_fetch), calls

    def test_fallback_after_urllib_exhaustion_files_backup(self):
        key = self._persistent_truncation()
        patcher, calls = self._stub_curl(self.BODY)
        with patcher:
            result = self.store.backup_supabase()
        # exactly one curl attempt, and only after urllib was fully
        # exhausted: S3_GET_ATTEMPTS outer loops, each burning
        # S3_TRANSPORT_ATTEMPTS transport attempts on the object GET
        self.assertEqual(len(calls), 1)
        self.assertEqual(
            calls[0]["get_requests"][key],
            cold_store.S3_GET_ATTEMPTS * cold_store.S3_TRANSPORT_ATTEMPTS,
        )
        self.assertTrue(calls[0]["url"].endswith("/" + key))
        self.assertEqual(calls[0]["access"], "test-access-key")
        # the fallback body was byte-verified and filed whole
        self.assertEqual(result["source_key"], key)
        self.assertEqual(result["sha256"], hashlib.sha256(self.BODY).hexdigest())
        self.assertEqual(self.fake.files[result["drive_file_id"]], self.BODY)

    def test_short_curl_body_named_error_nothing_filed(self):
        key = self._persistent_truncation()
        files_before = dict(self.fake.files)
        patcher, calls = self._stub_curl(self.BODY[:100])
        with patcher:
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store.backup_supabase()
        self.assertEqual(len(calls), 1)
        message = str(ctx.exception)
        self.assertIn("supabase-backups", message)
        self.assertIn(key, message)
        self.assertIn("curl fallback delivered 100 of", message)
        self.assertNotIn("test-secret-key", message)
        self.assertEqual(self.fake.files, files_before)

    def test_curl_never_invoked_on_urllib_happy_path(self):
        body = b"small backup " * 20
        key = self.key_for(hashlib.sha256(body).hexdigest())
        self.s3.put(key, body, "2026-10-21T02:00:00.000Z")
        patcher, calls = self._stub_curl(body)
        with patcher:
            result = self.store.backup_supabase()
        self.assertEqual(calls, [])
        self.assertEqual(result["sha256"], hashlib.sha256(body).hexdigest())

    def test_curl_fetch_keeps_credentials_out_of_argv(self):
        """The spawned curl argv must never carry the credentials
        (issue #983): _curl_fetch feeds curl its config on stdin
        (--config -) instead of passing --user in argv, where the
        local process list would see the secret for up to the 900s
        fallback timeout. This test is the guard for cold_store.py in
        the spirit of push-fleet-feed.guard.sh (#948/#970)."""
        seen = {}

        def fake_run(argv, **kwargs):
            seen["argv"] = argv
            seen["kwargs"] = kwargs
            return subprocess.CompletedProcess(argv, 0, stdout=b"payload", stderr=b"")

        with mock.patch.object(cold_store.subprocess, "run", fake_run):
            body = self.store._curl_fetch(
                "https://store.example/supabase-backups/some/key",
                "test-access-key",
                "test-secret-key",
            )
        self.assertEqual(body, b"payload")
        argv = seen["argv"]
        self.assertNotIn("--user", argv)
        for element in argv:
            self.assertNotIn("test-access-key", element)
            self.assertNotIn("test-secret-key", element)
        # the credential arrives on stdin as a curl config instead
        config = seen["kwargs"]["input"].decode()
        self.assertIn('user = "test-access-key:test-secret-key"', config)
        self.assertEqual(argv[argv.index("--config") + 1], "-")

    def test_fallback_sizes_via_head_when_caller_gives_none(self):
        key = self._persistent_truncation()
        patcher, calls = self._stub_curl(self.BODY)
        with patcher:
            data = self.store.s3_get_object("supabase-backups", key)
        self.assertEqual(data, self.BODY)
        self.assertEqual(len(calls), 1)
        head_requests = [r for r in self.s3.requests if r["method"] == "HEAD"]
        self.assertEqual(len(head_requests), 1)
        self.assertTrue(head_requests[0]["path"].endswith("/" + key))


class TestS3RequestCurlFallback(S3TestBase):
    """The curl fallback inside _s3_request for GET-class requests:
    on a hostile-proxy night (live 2026-10-28) EVERY urllib attempt
    at the bucket LIST truncated at a different offset, so the
    transport retry loop exhausted and backup-supabase / evict-scan
    died before their first object — the object-download fallback in
    TestS3CurlFallback never got a chance to run. A GET whose urllib
    attempts all fail at the transport level now gets exactly one
    curl attempt (stubbed here at the _curl_fetch_status transport
    seam; no network, no real curl) whose response flows through the
    same status handling as a urllib response."""

    @staticmethod
    def _list_xml(entries):
        """A ListObjectsV2 page in the fake's shape. entries are
        (key, size, last_modified) triples."""
        contents = "".join(
            f"<Contents><Key>{k}</Key><LastModified>{lm}</LastModified>"
            f"<Size>{size}</Size></Contents>"
            for k, size, lm in entries
        )
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
            "<Name>supabase-backups</Name><IsTruncated>false</IsTruncated>"
            f"{contents}</ListBucketResult>"
        ).encode()

    def _stub_curl_status(self, result):
        """Patch the store's curl-status transport; return the call
        log. Each entry snapshots the fake's urllib list_attempts
        count at call time, proving the fallback fires only after
        urllib exhaustion, never alongside it."""
        calls = []

        def fake_status(url, access, secret):
            calls.append(
                {
                    "url": url,
                    "access": access,
                    "secret": secret,
                    "list_attempts": self.s3.list_attempts,
                }
            )
            if isinstance(result, Exception):
                raise result
            return result

        return mock.patch.object(self.store, "_curl_fetch_status", fake_status), calls

    def test_list_truncated_on_all_attempts_falls_back_to_curl(self):
        self.s3.put("pg/muse-dev/a.bin", b"x", "2026-10-01T00:00:00.000Z")
        self.s3.put("pg/muse-dev/b.bin", b"yy", "2026-10-02T00:00:00.000Z")
        self.s3.list_failures = ["truncate"] * cold_store.S3_TRANSPORT_ATTEMPTS
        xml = self._list_xml(
            [
                ("pg/muse-dev/a.bin", 1, "2026-10-01T00:00:00.000Z"),
                ("pg/muse-dev/b.bin", 2, "2026-10-02T00:00:00.000Z"),
            ]
        )
        patcher, calls = self._stub_curl_status((200, xml))
        with patcher:
            objects = self.store.s3_list_objects("supabase-backups")
        self.assertEqual(
            [o["key"] for o in objects], ["pg/muse-dev/a.bin", "pg/muse-dev/b.bin"]
        )
        # exactly one curl attempt, only after all 4 urllib attempts
        self.assertEqual(len(calls), 1)
        self.assertEqual(calls[0]["list_attempts"], cold_store.S3_TRANSPORT_ATTEMPTS)
        self.assertIn("list-type=2", calls[0]["url"])
        self.assertEqual(calls[0]["access"], "test-access-key")
        self.assertEqual(self.s3.list_attempts, cold_store.S3_TRANSPORT_ATTEMPTS)

    def test_backup_supabase_proceeds_when_list_falls_back(self):
        body = b"newest backup payload " * 60
        key = self.key_for(hashlib.sha256(body).hexdigest())
        self.s3.put(key, body, "2026-10-21T02:00:00.000Z")
        self.s3.put("pg/muse-dev/manifest.json", b"{}", "2026-10-22T02:00:00.000Z")
        self.s3.list_failures = ["truncate"] * cold_store.S3_TRANSPORT_ATTEMPTS
        xml = self._list_xml(
            [
                (key, len(body), "2026-10-21T02:00:00.000Z"),
                ("pg/muse-dev/manifest.json", 2, "2026-10-22T02:00:00.000Z"),
            ]
        )
        patcher, calls = self._stub_curl_status((200, xml))
        with patcher:
            result = self.store.backup_supabase()
        self.assertEqual(len(calls), 1)
        self.assertEqual(result["source_key"], key)
        self.assertEqual(result["sha256"], hashlib.sha256(body).hexdigest())
        self.assertEqual(self.fake.files[result["drive_file_id"]], body)

    def test_curl_http_error_surfaces_as_s3_error_not_transport(self):
        self.s3.put("pg/muse-dev/a.bin", b"x", "2026-10-01T00:00:00.000Z")
        self.s3.list_failures = ["truncate"] * cold_store.S3_TRANSPORT_ATTEMPTS
        error_xml = (
            '<?xml version="1.0" encoding="UTF-8"?>'
            "<Error><Code>AccessDenied</Code>"
            "<Message>Access Denied</Message></Error>"
        ).encode()
        patcher, calls = self._stub_curl_status((403, error_xml))
        with patcher:
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store.s3_list_objects("supabase-backups")
        self.assertEqual(len(calls), 1)
        message = str(ctx.exception)
        # the same error shape _s3_check gives a urllib 403
        self.assertIn("S3 list of supabase-backups failed (HTTP 403)", message)
        self.assertIn("Access Denied", message)
        self.assertNotIn("transport error", message)
        self.assertNotIn("failed after 4 attempts", message)

    def test_curl_fetch_status_splits_status_trailer(self):
        """The real _curl_fetch_status (subprocess.run stubbed): the
        --write-out trailer splits status from body, an HTTP error
        status is a normal result (no --fail), and credentials ride
        stdin config, never argv."""
        seen = {}

        def fake_run(argv, **kwargs):
            seen["argv"] = argv
            seen["kwargs"] = kwargs
            trailer = cold_store.ColdStore._CURL_STATUS_TRAILER.encode()
            return subprocess.CompletedProcess(
                argv, 0, stdout=b"<xml>page</xml>" + trailer + b"403>>>", stderr=b""
            )

        with mock.patch.object(cold_store.subprocess, "run", fake_run):
            status, body = self.store._curl_fetch_status(
                "https://store.example/supabase-backups?list-type=2",
                "test-access-key",
                "test-secret-key",
            )
        self.assertEqual(status, 403)
        self.assertEqual(body, b"<xml>page</xml>")
        argv = seen["argv"]
        self.assertNotIn("--fail", argv)
        self.assertNotIn("--user", argv)
        for element in argv:
            self.assertNotIn("test-secret-key", element)
        self.assertIn(
            'user = "test-access-key:test-secret-key"',
            seen["kwargs"]["input"].decode(),
        )

    def test_curl_fetch_status_transport_failure_raises_scrubbed(self):
        def fake_run(argv, **kwargs):
            return subprocess.CompletedProcess(
                argv,
                18,
                stdout=b"",
                stderr=b"curl: (18) transfer closed, keyed by test-secret-key",
            )

        with mock.patch.object(cold_store.subprocess, "run", fake_run):
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store._curl_fetch_status(
                    "https://store.example/supabase-backups?list-type=2",
                    "test-access-key",
                    "test-secret-key",
                )
        message = str(ctx.exception)
        self.assertIn("curl fallback failed (exit 18)", message)
        self.assertNotIn("test-secret-key", message)

    def test_delete_exhaustion_never_invokes_curl(self):
        # DELETE is not GET-class: it keeps the plain exhaustion
        # error and the curl fallback must not fire for it.
        import http.client

        self.s3.put("pg/muse-dev/a.bin", b"x", "2026-10-01T00:00:00.000Z")
        state = {"n": 0}

        def always_drop(method, url, headers=None, data=None, timeout=120):
            state["n"] += 1
            raise http.client.RemoteDisconnected("simulated drop")

        patcher, calls = self._stub_curl_status((204, b""))
        with patcher, mock.patch.object(cold_store, "http_request", always_drop):
            with self.assertRaises(cold_store.ColdStoreError) as ctx:
                self.store.s3_delete_object("supabase-backups", "pg/muse-dev/a.bin")
        self.assertEqual(state["n"], cold_store.S3_TRANSPORT_ATTEMPTS)
        self.assertEqual(calls, [])
        message = str(ctx.exception)
        self.assertIn("S3 DELETE supabase-backups", message)
        self.assertIn("failed after 4 attempts", message)
        self.assertNotIn("curl fallback", message)


class EvictTestBase(S3TestBase):
    """One bucket holding every eviction verdict at once.

    LastModified order (newest first): young (now-1d), oldtimer
    (2026-06), evict2, evict1, unknown, not-in-manifest, unverified —
    with keep_newest=2 the protected pair is {young, oldtimer}; age is
    checked before position, so young reports too-young and oldtimer
    (old enough on its own) is the isolated kept-newest case.
    """

    def setUp(self):
        super().setUp()
        self.h = self.make_cold(
            {
                "young.bin": b"young object " * 30,
                "oldtimer.bin": b"oldtimer object " * 30,
                "evict1.bin": b"evict me one " * 30,
                "evict2.bin": b"evict me two " * 30,
            }
        )
        self.keys = {
            "young": self.key_for(self.h["young.bin"]),
            "oldtimer": self.key_for(self.h["oldtimer.bin"]),
            "evict1": self.key_for(self.h["evict1.bin"]),
            "evict2": self.key_for(self.h["evict2.bin"]),
            "unverified": self.key_for("d" * 64),
            "not-in-manifest": self.key_for("e" * 64),
            "unknown": "pg/muse-dev/2026/01/random-file.bin",
            "manifest": "pg/muse-dev/manifest.json",
        }
        # a cold object whose only bundle was never verified
        conn = self.store.db()
        with conn:
            conn.execute(
                "INSERT INTO objects (hash, size, state) VALUES (?, ?, 'cold')",
                ("d" * 64, 123),
            )
            conn.execute(
                "INSERT INTO bundles (bundle_id, sha256, bytes, state)"
                " VALUES ('b_unverified00', ?, 123, 'sealed')",
                ("f" * 64,),
            )
            conn.execute(
                "INSERT INTO members (bundle_id, object_hash, offset, length)"
                " VALUES ('b_unverified00', ?, 0, 10)",
                ("d" * 64,),
            )
        conn.close()
        young_lm = (datetime.now(timezone.utc) - timedelta(days=1)).strftime(
            "%Y-%m-%dT%H:%M:%SZ"
        )
        self.s3.put(self.keys["manifest"], b"{}", "2026-10-25T00:00:00.000Z")
        self.s3.put(self.keys["young"], b"y", young_lm)
        self.s3.put(self.keys["oldtimer"], b"o", "2026-06-01T00:00:00.000Z")
        self.s3.put(self.keys["evict1"], b"1", "2026-01-05T00:00:00.000Z")
        self.s3.put(self.keys["evict2"], b"2", "2026-01-06T00:00:00.000Z")
        self.s3.put(self.keys["unverified"], b"u", "2026-01-02T00:00:00.000Z")
        self.s3.put(self.keys["not-in-manifest"], b"n", "2026-01-03T00:00:00.000Z")
        self.s3.put(self.keys["unknown"], b"?", "2026-01-04T00:00:00.000Z")

    def scan_by_key(self, **kwargs):
        kwargs.setdefault("min_age_days", 14)
        kwargs.setdefault("keep_newest", 2)
        return {e["key"]: e for e in self.store.evict_scan(**kwargs)}


class TestEvictScan(EvictTestBase):
    def test_verdict_matrix(self):
        scan = self.scan_by_key()
        self.assertEqual(len(scan), 7)  # manifest.json is excluded
        self.assertNotIn(self.keys["manifest"], scan)
        self.assertEqual(scan[self.keys["young"]]["verdict"], "too-young")
        self.assertEqual(scan[self.keys["oldtimer"]]["verdict"], "kept-newest")
        self.assertEqual(scan[self.keys["evict1"]]["verdict"], "evictable")
        self.assertEqual(scan[self.keys["evict2"]]["verdict"], "evictable")
        self.assertEqual(scan[self.keys["unverified"]]["verdict"], "bundle-unverified")
        self.assertEqual(
            scan[self.keys["not-in-manifest"]]["verdict"], "not-in-manifest"
        )
        self.assertEqual(scan[self.keys["unknown"]]["verdict"], "hash-unknown")
        # a scan is a dry run: nothing deleted, nothing mutated
        self.assertEqual(self.s3.deleted, [])
        self.assertIn(self.keys["evict1"], self.s3.objects)


class TestOffsetLessLastModified(EvictTestBase):
    """#2110: an offset-less LastModified has no zone, so it is
    unparseable -> None (the aware-or-None contract of
    _parse_s3_list_page). It must never reach a comparison or the
    age subtraction as a naive datetime, and it must never be
    reinterpreted as UTC: eviction deletes objects, and None reads
    as too-young, which is non-evictable by construction."""

    def _listed(self):
        return {
            o["key"]: o for o in self.store.s3_list_objects("supabase-backups")
        }

    def test_offset_less_stamp_parses_to_none(self):
        body = (
            '<?xml version="1.0" encoding="UTF-8"?>'
            '<ListBucketResult xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
            "<Name>supabase-backups</Name><IsTruncated>false</IsTruncated>"
            "<Contents><Key>a</Key>"
            "<LastModified>2026-10-01T00:00:00.000</LastModified>"
            "<Size>1</Size></Contents>"
            "<Contents><Key>b</Key>"
            "<LastModified>2026-10-01T00:00:00.000Z</LastModified>"
            "<Size>1</Size></Contents>"
            "<Contents><Key>c</Key>"
            "<LastModified>2026-10-01T05:00:00+05:00</LastModified>"
            "<Size>1</Size></Contents>"
            "<Contents><Key>d</Key><Size>1</Size></Contents>"
            "</ListBucketResult>"
        ).encode()
        objects, truncated, _token = cold_store._parse_s3_list_page(body)
        self.assertFalse(truncated)
        by_key = {o["key"]: o["last_modified"] for o in objects}
        self.assertIsNone(by_key["a"])  # offset-less -> None
        self.assertIsNone(by_key["d"])  # absent -> None, as before
        # Z and explicit-offset stamps are unchanged
        self.assertEqual(by_key["b"], datetime(2026, 10, 1, tzinfo=timezone.utc))
        self.assertEqual(by_key["c"].utcoffset(), timedelta(hours=5))

    def test_mixed_listing_scan_offset_less_is_too_young(self):
        # one offset-less stamp among aware neighbours: the sort and
        # the age check both used to die with TypeError
        self.s3.put(self.keys["evict1"], b"1", "2026-01-05T00:00:00.000")
        scan = self.scan_by_key()
        self.assertIsNone(scan[self.keys["evict1"]]["last_modified"])
        self.assertEqual(scan[self.keys["evict1"]]["verdict"], "too-young")
        self.assertEqual(scan[self.keys["evict2"]]["verdict"], "evictable")
        self.assertEqual(self.s3.deleted, [])

    def test_uniformly_offset_less_listing_scan_completes(self):
        # every stamp offset-less: the sort survives naive stamps,
        # but the age subtraction does not -- the trap a sort-only
        # test would miss (triage case 5)
        for key in self.keys.values():
            self.s3.put(key, b"x", "2026-01-01T00:00:00.000")
        scan = self.scan_by_key()
        for name in ("young", "oldtimer", "evict1", "evict2"):
            self.assertEqual(scan[self.keys[name]]["verdict"], "too-young")
        self.assertEqual(
            scan[self.keys["unverified"]]["verdict"], "bundle-unverified"
        )
        self.assertEqual(self.s3.deleted, [])

    def test_aware_stamps_unchanged_in_listing(self):
        listed = self._listed()
        self.assertEqual(
            listed[self.keys["oldtimer"]]["last_modified"],
            datetime(2026, 6, 1, tzinfo=timezone.utc),
        )



class TestS3ListPageValidation(unittest.TestCase):
    """#2733: _parse_s3_list_page must fail closed with ColdStoreError.

    A malformed or shape-drifted body must never escape as a raw
    ParseError/ValueError, and a wrong-root document must never be
    read as an empty bucket (backup + eviction plan from this list).
    """

    def _page(self, contents="", root="ListBucketResult", extra=""):
        return (
            '<?xml version="1.0" encoding="UTF-8"?>'
            f'<{root} xmlns="http://s3.amazonaws.com/doc/2006-03-01/">'
            f"<Name>b</Name>{extra}{contents}</{root}>"
        ).encode()

    def _entry(self, size_marker):
        return (
            "<Contents><Key>a.bin</Key>"
            "<LastModified>2026-10-01T00:00:00.000Z</LastModified>"
            f"{size_marker}</Contents>"
        )

    def test_non_xml_body_raises_cold_store_error(self):
        with self.assertRaises(cold_store.ColdStoreError):
            cold_store._parse_s3_list_page(b'{"error": "nope"}')

    def test_wrong_root_raises_cold_store_error(self):
        body = b"<html><body>proxy error</body></html>"
        with self.assertRaises(cold_store.ColdStoreError):
            cold_store._parse_s3_list_page(body)

    def test_non_numeric_size_raises(self):
        for bad in ("abc", "1.5"):
            with self.subTest(size=bad):
                with self.assertRaises(cold_store.ColdStoreError):
                    cold_store._parse_s3_list_page(
                        self._page(self._entry(f"<Size>{bad}</Size>"))
                    )

    def test_negative_size_raises(self):
        with self.assertRaises(cold_store.ColdStoreError):
            cold_store._parse_s3_list_page(
                self._page(self._entry("<Size>-5</Size>"))
            )

    def test_missing_size_keeps_default_zero(self):
        objects, truncated, token = cold_store._parse_s3_list_page(
            self._page(self._entry(""))
        )
        self.assertEqual(objects[0]["size"], 0)
        self.assertFalse(truncated)
        self.assertIsNone(token)

    def test_valid_page_pagination_and_stamp_unchanged(self):
        body = self._page(
            self._entry("<Size>3</Size>"),
            extra=("<IsTruncated>true</IsTruncated>"
                   "<NextContinuationToken>tok2</NextContinuationToken>"),
        )
        objects, truncated, token = cold_store._parse_s3_list_page(body)
        self.assertEqual(len(objects), 1)
        self.assertEqual(objects[0]["key"], "a.bin")
        self.assertEqual(objects[0]["size"], 3)
        self.assertEqual(
            objects[0]["last_modified"],
            datetime(2026, 10, 1, tzinfo=timezone.utc),
        )
        self.assertTrue(truncated)
        self.assertEqual(token, "tok2")


class TestOffsetLessBackupPick(S3TestBase):
    """backup_supabase picks the newest by LastModified with max():
    a mixed listing used to crash the pick outright (#2110)."""

    def test_newest_pick_ignores_offset_less_stamp(self):
        aware_body = b"aware backup payload " * 10
        aware_key = self.key_for(hashlib.sha256(aware_body).hexdigest())
        naive_body = b"naive backup payload " * 10
        naive_key = self.key_for(hashlib.sha256(naive_body).hexdigest())
        self.s3.put(aware_key, aware_body, "2026-10-21T02:00:00.000Z")
        # a later wall-clock reading, but its zone is unknown, so it
        # is not evidence of newness and must not win the pick
        self.s3.put(naive_key, naive_body, "2026-10-23T02:00:00.000")
        self.s3.put(
            "pg/muse-dev/manifest.json", b"{}", "2026-10-22T02:00:00.000Z"
        )
        result = self.store.backup_supabase()
        self.assertEqual(result["source_key"], aware_key)
        self.assertEqual(
            result["sha256"], hashlib.sha256(aware_body).hexdigest()
        )


class TestEvictApply(EvictTestBase):
    def test_apply_deletes_only_evictable_and_records(self):
        scan = self.store.evict_scan(min_age_days=14, keep_newest=2)
        outcomes = self.store.evict_apply("supabase-backups", scan)
        self.assertEqual(
            {o["key"] for o in outcomes if o["outcome"] == "evicted"},
            {self.keys["evict1"], self.keys["evict2"]},
        )
        self.assertEqual(
            sorted(self.s3.deleted),
            sorted([self.keys["evict1"], self.keys["evict2"]]),
        )
        for name in ("young", "oldtimer", "unverified", "not-in-manifest", "unknown"):
            self.assertIn(self.keys[name], self.s3.objects)
        conn = self.store.db()
        states = {
            r["hash"]: r["state"]
            for r in conn.execute("SELECT hash, state FROM objects")
        }
        rows = conn.execute(
            "SELECT * FROM transfer_ledger WHERE kind='evict'"
        ).fetchall()
        conn.close()
        self.assertEqual(states[self.h["evict1.bin"]], "evicted")
        self.assertEqual(states[self.h["evict2.bin"]], "evicted")
        self.assertEqual(states[self.h["young.bin"]], "cold")
        self.assertEqual(len(rows), 2)
        self.assertEqual(
            {r["ref"] for r in rows}, {self.keys["evict1"], self.keys["evict2"]}
        )
        for r in rows:
            self.assertEqual(r["direction"], "trash")

    def test_apply_reverifies_and_skips_changed_object(self):
        scan = self.store.evict_scan(min_age_days=14, keep_newest=2)
        # between scan and apply the object stops being cold — the
        # deletion-time re-verification must refuse to delete it
        conn = self.store.db()
        with conn:
            conn.execute(
                "UPDATE objects SET state='hot' WHERE hash=?",
                (self.h["evict1.bin"],),
            )
        conn.close()
        outcomes = {
            o["key"]: o for o in self.store.evict_apply("supabase-backups", scan)
        }
        self.assertEqual(outcomes[self.keys["evict1"]]["outcome"], "skipped")
        self.assertEqual(outcomes[self.keys["evict2"]]["outcome"], "evicted")
        self.assertEqual(self.s3.deleted, [self.keys["evict2"]])
        self.assertIn(self.keys["evict1"], self.s3.objects)

    def test_cli_refuses_without_apply(self):
        env = dict(self.R2_ENV)
        env["COLD_STORE_S3_BASE"] = self.s3_base_url
        out, err = io.StringIO(), io.StringIO()
        with mock.patch.dict(os.environ, env):
            with contextlib.redirect_stdout(out), contextlib.redirect_stderr(err):
                rc = cold_store.main(
                    [
                        "--home",
                        str(self.home),
                        "evict",
                        "--bucket",
                        "supabase-backups",
                        "--min-age-days",
                        "14",
                        "--keep-newest",
                        "2",
                    ]
                )
        self.assertEqual(rc, 1)
        self.assertIn("--apply", err.getvalue())
        self.assertIn("evictable", out.getvalue())  # the scan still printed
        self.assertEqual(self.s3.deleted, [])


class TestObjectsEvictedMigration(ColdStoreTestBase):
    def test_pre_v4_manifest_gains_evicted_state(self):
        # Rebuild a manifest exactly as a pre-v4 home has it (objects
        # CHECK without 'evicted'), with a cold object in a verified
        # bundle, then open it via a store: the migration must admit
        # 'evicted' and preserve the member row that references the
        # rebuilt table.
        legacy_home = Path(self.tmp.name) / "legacy-home-v4"
        legacy_home.mkdir()
        legacy_schema = cold_store.SCHEMA_PATH.read_text().replace(",'evicted')", ")")
        self.assertIn(
            "CHECK (state IN ('hot','staging','cold','rehydrating','gone'))",
            legacy_schema,
        )
        conn = sqlite3.connect(legacy_home / "manifest.sqlite")
        conn.executescript(legacy_schema)
        conn.execute(
            "INSERT INTO objects (hash, size, state) VALUES (?, 10, 'cold')",
            ("a" * 64,),
        )
        conn.execute(
            "INSERT INTO bundles (bundle_id, sha256, bytes, state)"
            " VALUES ('b_legacy', ?, 10, 'verified')",
            ("b" * 64,),
        )
        conn.execute(
            "INSERT INTO members (bundle_id, object_hash, offset, length)"
            " VALUES ('b_legacy', ?, 0, 10)",
            ("a" * 64,),
        )
        conn.commit()
        conn.close()
        base = self.store.drive_base.rsplit("/drive/v3", 1)[0]
        legacy = ColdStore(
            legacy_home,
            token_url=base + "/token",
            drive_base=self.store.drive_base,
            upload_base=base,
        )
        conn = legacy.db()
        with conn:
            conn.execute("UPDATE objects SET state='evicted' WHERE hash=?", ("a" * 64,))
        row = conn.execute(
            "SELECT state FROM objects WHERE hash=?", ("a" * 64,)
        ).fetchone()
        members = conn.execute("SELECT count(*) n FROM members").fetchone()
        conn.close()
        self.assertEqual(row["state"], "evicted")
        self.assertEqual(members["n"], 1)


class TestDrillCompare(ColdStoreTestBase):
    """restore-drill.sh's backup-vs-live check (scripts/drill_compare.py).

    The invariant is subset + content-equal, NOT row-count equality: a
    backup is point-in-time and the manifest is append-mostly — the
    nightly run appends its own transfer_ledger rows (its backup-upload
    row, prune trash rows) after the export was taken — so live rows
    appended since the backup are expected, never a failure. A backup
    row missing from live, or present with different content, fails.
    """

    DRILL_COMPARE = (
        Path(__file__).resolve().parent.parent / "scripts" / "drill_compare.py"
    )
    HASH_A = "a" * 64
    HASH_B = "b" * 64

    def setUp(self):
        super().setUp()
        self.d1 = FakeD1()
        # check_same_thread=False: the HTTP handler thread runs the queries.
        self.live = sqlite3.connect(":memory:", check_same_thread=False)
        self.live.executescript(cold_store.SCHEMA_PATH.read_text())
        self.live.executemany(
            "INSERT INTO objects (hash, size, content_type, source_bucket,"
            " source_key, first_seen, state) VALUES (?, ?, ?, ?, ?, ?, 'cold')",
            [
                (
                    self.HASH_A,
                    100,
                    "text/plain",
                    "supabase-backups",
                    "k/a",
                    "2026-10-01T00:00:00",
                ),
                (
                    self.HASH_B,
                    200,
                    None,
                    "supabase-backups",
                    "k/b",
                    "2026-10-02T00:00:00",
                ),
            ],
        )
        self.live.execute(
            "INSERT INTO bundles (bundle_id, drive_file_id, sha256, drive_md5,"
            " bytes, member_count, live_bytes, sealed_at, uploaded_at, state)"
            " VALUES ('b_test', 'drive-1', ?, 'md5x', 300, 2, 300,"
            " '2026-10-03T00:00:00', '2026-10-03T01:00:00', 'verified')",
            ("c" * 64,),
        )
        self.live.executemany(
            "INSERT INTO members VALUES ('b_test', ?, ?, ?)",
            [(self.HASH_A, 0, 50), (self.HASH_B, 50, 60)],
        )
        self.live.executemany(
            "INSERT INTO transfer_ledger (id, ts, direction, bytes, kind, ref)"
            " VALUES (?, ?, 'upload', ?, 'bundle', 'b_test')",
            [(i, f"2026-10-{i:02d}T02:00:00", 100 * i) for i in range(1, 10)],
        )
        self.live.commit()
        self.d1.db = self.live
        self.d1_server = ThreadingHTTPServer(("127.0.0.1", 0), make_d1_handler(self.d1))
        self.d1_thread = threading.Thread(
            target=self.d1_server.serve_forever, daemon=True
        )
        self.d1_thread.start()

    def tearDown(self):
        self.d1_server.shutdown()
        self.d1_server.server_close()
        super().tearDown()

    def _render_backup_sql(self, mutate=None) -> Path:
        """Snapshot live into INSERT-only export SQL (the backup-d1
        file format), optionally mutating the snapshot first."""
        bak = sqlite3.connect(":memory:")
        self.live.backup(bak)
        if mutate:
            mutate(bak)
            bak.commit()
        lines = []
        for table in cold_store.MANIFEST_TABLES:
            cols = [r[1] for r in bak.execute(f"PRAGMA table_info({table})")]
            for row in bak.execute(f'SELECT * FROM "{table}" ORDER BY rowid'):
                vals = ", ".join(cold_store.ColdStore._sql_literal(v) for v in row)
                lines.append(
                    f'INSERT INTO "{table}" ({", ".join(cols)}) VALUES ({vals});'
                )
        bak.close()
        path = Path(self.tmp.name) / "drill-backup.sql"
        path.write_text("\n".join(lines) + "\n")
        return path

    def _run_drill(self, backup_sql: Path):
        env = dict(os.environ)
        env["CLOUDFLARE_API_TOKEN"] = "fake-cf-token"
        env["COLD_STORE_D1_BASE"] = (
            f"http://127.0.0.1:{self.d1_server.server_address[1]}"
        )
        return subprocess.run(
            [
                sys.executable,
                str(self.DRILL_COMPARE),
                str(backup_sql),
                str(Path(self.home) / "manifest.sqlite"),
            ],
            env=env,
            capture_output=True,
            text=True,
            timeout=60,
        )

    def test_backup_subset_of_live_passes(self):
        # The first live drill's scenario: the backup holds ledger rows
        # 1-5; the nightly run then appended its own rows 6-9 to live.
        # The backup rows are an exact prefix of live — must PASS.

        def drop_appended(conn):
            conn.execute("DELETE FROM transfer_ledger WHERE id > 5")

        proc = self._run_drill(self._render_backup_sql(drop_appended))
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn(
            "transfer_ledger: backup has 5 row(s), live D1 has 9", proc.stdout
        )
        self.assertIn("4 additional row(s) appended since this backup", proc.stdout)
        self.assertNotIn("FAIL", proc.stdout)

    def test_exact_equal_passes(self):
        proc = self._run_drill(self._render_backup_sql())
        self.assertEqual(proc.returncode, 0, proc.stdout + proc.stderr)
        self.assertIn("objects: backup has 2 row(s), live D1 has 2", proc.stdout)
        self.assertNotIn("additional", proc.stdout)
        self.assertNotIn("FAIL", proc.stdout)

    def test_altered_live_row_fails(self):
        backup_sql = self._render_backup_sql()
        self.live.execute(
            "UPDATE objects SET size = 101 WHERE hash = ?", (self.HASH_A,)
        )
        self.live.commit()
        proc = self._run_drill(backup_sql)
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertIn("objects", proc.stdout)
        self.assertIn(self.HASH_A, proc.stdout)
        self.assertIn("differs", proc.stdout)

    def test_missing_live_row_fails(self):
        backup_sql = self._render_backup_sql()
        self.live.execute("DELETE FROM objects WHERE hash = ?", (self.HASH_B,))
        self.live.commit()
        proc = self._run_drill(backup_sql)
        self.assertEqual(proc.returncode, 1, proc.stdout + proc.stderr)
        self.assertIn("objects", proc.stdout)
        self.assertIn(self.HASH_B, proc.stdout)
        self.assertIn("missing", proc.stdout)


class TestLedgerTrashMigration(ColdStoreTestBase):
    def test_pre_v3_manifest_gains_trash_direction(self):
        # Rebuild a manifest exactly as a pre-v3 home has it (CHECK
        # without 'trash'), with one live row, then open it via a store:
        # the migration must preserve the row and admit 'trash' rows.
        legacy_home = Path(self.tmp.name) / "legacy-home"
        legacy_home.mkdir()
        legacy_schema = cold_store.SCHEMA_PATH.read_text().replace(
            "('upload','download','trash')", "('upload','download')"
        )
        self.assertIn("IN ('upload','download')", legacy_schema)
        conn = sqlite3.connect(legacy_home / "manifest.sqlite")
        conn.executescript(legacy_schema)
        conn.execute(
            "INSERT INTO transfer_ledger (direction, bytes, kind, ref)"
            " VALUES ('upload', 42, 'bundle', 'b_legacy')"
        )
        conn.commit()
        conn.close()
        base = self.store.drive_base.rsplit("/drive/v3", 1)[0]
        legacy = ColdStore(
            legacy_home,
            token_url=base + "/token",
            drive_base=self.store.drive_base,
            upload_base=base,
        )
        conn = legacy.db()
        rows = conn.execute("SELECT direction, ref FROM transfer_ledger").fetchall()
        self.assertEqual(
            [(r["direction"], r["ref"]) for r in rows], [("upload", "b_legacy")]
        )
        with conn:
            legacy.ledger_add(conn, "trash", 7, "backup-prune", "d1-old.sql")
        n = conn.execute(
            "SELECT count(*) n FROM transfer_ledger WHERE direction='trash'"
        ).fetchone()["n"]
        conn.close()
        self.assertEqual(n, 1)


# ---------------------------------------------------------------- fake Turso
class FakeTurso:
    """State for the fake Turso Platform API + libsql dump endpoints."""

    PLATFORM_TOKEN = "test-platform-token"
    ORG = "testorg"
    DUMP_SQL = (
        "PRAGMA foreign_keys=OFF;\n"
        "BEGIN TRANSACTION;\n"
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);\n"
        "INSERT INTO notes VALUES(1,'hello');\n"
        "COMMIT;\n"
    )
    # A dump cut mid-stream: complete statements, but the transaction
    # never commits — it would rebuild into an integrity-clean shell
    # with every row rolled back.
    TRUNCATED_SQL = (
        "PRAGMA foreign_keys=OFF;\n"
        "BEGIN TRANSACTION;\n"
        "CREATE TABLE notes (id INTEGER PRIMARY KEY, body TEXT);\n"
        "INSERT INTO notes VALUES(1,'hello');\n"
    )
    # Well-formed and committed, but carries no user tables at all.
    EMPTY_SQL = "PRAGMA foreign_keys=OFF;\nBEGIN TRANSACTION;\nCOMMIT;\n"
    # The atlas shape (#1391): an FTS5 external-content table whose
    # CREATE carries single quotes. libsql registers it inside a
    # writable_schema block with the embedded SQL verbatim — quotes
    # NOT doubled — so the literal terminates early and stock parsers
    # fail with: near "chunks": syntax error. The shadow tables and
    # the FTS index rows (real bytes from a live FTS5 index holding
    # the seeded row) follow as ordinary statements, as in a real
    # dump.
    FTS_SQL = (
        "PRAGMA foreign_keys=OFF;\n"
        "BEGIN TRANSACTION;\n"
        "CREATE TABLE chunks (id INTEGER PRIMARY KEY, text TEXT);\n"
        "INSERT INTO chunks VALUES(1,'hello fts world');\n"
        "PRAGMA writable_schema=ON;\n"
        "INSERT INTO sqlite_schema(type,name,tbl_name,rootpage,sql)"
        " VALUES('table','chunks_fts','chunks_fts',0,"
        "'CREATE VIRTUAL TABLE chunks_fts\n"
        "      USING fts5(text, content='chunks', content_rowid='id')');\n"
        "PRAGMA writable_schema=OFF;\n"
        "CREATE TABLE 'chunks_fts_data'(id INTEGER PRIMARY KEY, block BLOB);\n"
        "CREATE TABLE 'chunks_fts_idx'(segid, term, pgno,"
        " PRIMARY KEY(segid, term)) WITHOUT ROWID;\n"
        "CREATE TABLE 'chunks_fts_docsize'(id INTEGER PRIMARY KEY, sz BLOB);\n"
        "CREATE TABLE 'chunks_fts_config'(k PRIMARY KEY, v) WITHOUT ROWID;\n"
        "INSERT INTO chunks_fts_data VALUES(1, X'0103');\n"
        "INSERT INTO chunks_fts_data VALUES(10, X'000000000101010001010101');\n"
        "INSERT INTO chunks_fts_data VALUES(137438953473,"
        " X'000000200430667473010203010568656C6C6F0102020105776F726C6401020404080A');\n"
        "INSERT INTO chunks_fts_idx VALUES(1, X'', 2);\n"
        "INSERT INTO chunks_fts_docsize VALUES(1, X'03');\n"
        "INSERT INTO chunks_fts_config VALUES('version', 4);\n"
        "COMMIT;\n"
    )
    # The same dump with the schema literal correctly escaped ('' in
    # place of each embedded quote): the repair filter must pass it
    # through byte-identical, so the registered SQL stays exactly as
    # authored.
    ESCAPED_SQL = FTS_SQL.replace("content='chunks'", "content=''chunks''").replace(
        "content_rowid='id'", "content_rowid=''id''"
    )

    def __init__(self):
        self.failing_dumps = set()
        self.truncated_dumps = set()
        self.empty_dumps = set()
        self.fts_dumps = set()
        self.escaped_dumps = set()
        self.dump_accept_encodings = {}
        self.databases = [
            {"Name": "fake-one", "Hostname": "fake-one.example.invalid"},
            {"Name": "fake-two", "Hostname": "fake-two.example.invalid"},
        ]


def make_turso_handler(fake: FakeTurso):
    class Handler(BaseHTTPRequestHandler):
        protocol_version = "HTTP/1.1"

        def log_message(self, *a):  # silence
            pass

        def _send(self, code, body, content_type="application/json"):
            data = body.encode() if isinstance(body, str) else body
            self.send_response(code)
            self.send_header("Content-Type", content_type)
            self.send_header("Content-Length", str(len(data)))
            self.end_headers()
            self.wfile.write(data)

        def do_GET(self):
            parts = [p for p in self.path.split("/") if p]
            if parts == ["v1", "organizations", FakeTurso.ORG, "databases"]:
                if (
                    self.headers.get("Authorization")
                    != f"Bearer {FakeTurso.PLATFORM_TOKEN}"
                ):
                    return self._send(401, {"error": "bad platform token"})
                return self._send(200, json.dumps({"databases": fake.databases}))
            if len(parts) == 2 and parts[1] == "dump":
                name = parts[0]
                fake.dump_accept_encodings[name] = self.headers.get("Accept-Encoding")
                if self.headers.get("Authorization") != f"Bearer fake-jwt-{name}":
                    return self._send(401, {"error": "bad db token"})
                if name in fake.failing_dumps:
                    return self._send(500, "dump exploded", "text/plain")
                if name in fake.truncated_dumps:
                    return self._send(200, FakeTurso.TRUNCATED_SQL, "text/plain")
                if name in fake.empty_dumps:
                    return self._send(200, FakeTurso.EMPTY_SQL, "text/plain")
                if name in fake.fts_dumps:
                    return self._send(200, FakeTurso.FTS_SQL, "text/plain")
                if name in fake.escaped_dumps:
                    return self._send(200, FakeTurso.ESCAPED_SQL, "text/plain")
                return self._send(200, FakeTurso.DUMP_SQL, "text/plain")
            return self._send(404, {"error": "not found"})

        def do_POST(self):
            length = int(self.headers.get("Content-Length") or 0)
            if length:
                self.rfile.read(length)
            parts = [p for p in self.path.split("/") if p]
            if (
                len(parts) == 7
                and parts[:4] == ["v1", "organizations", FakeTurso.ORG, "databases"]
                and parts[5:] == ["auth", "tokens"]
            ):
                if (
                    self.headers.get("Authorization")
                    != f"Bearer {FakeTurso.PLATFORM_TOKEN}"
                ):
                    return self._send(401, {"error": "bad platform token"})
                return self._send(200, json.dumps({"jwt": f"fake-jwt-{parts[4]}"}))
            return self._send(404, {"error": "not found"})

    return Handler


class TestTursoExportScript(unittest.TestCase):
    """scripts/export-turso.sh against the fake Turso HTTP API and a stub
    cold-store executable: databases are discovered, tokens minted, dumps
    downloaded and rebuilt, and each export filed via put-backup."""

    def setUp(self):
        self.fake = FakeTurso()
        self.server = ThreadingHTTPServer(
            ("127.0.0.1", 0), make_turso_handler(self.fake)
        )
        self.thread = threading.Thread(target=self.server.serve_forever, daemon=True)
        self.thread.start()
        self.addCleanup(self.server.server_close)
        self.addCleanup(self.server.shutdown)
        self.base = f"http://127.0.0.1:{self.server.server_address[1]}"
        self.tmp = Path(tempfile.mkdtemp(prefix="cold-store-turso-test-"))
        self.addCleanup(shutil.rmtree, self.tmp, True)
        self.calls_log = self.tmp / "cold-store-calls.log"
        self.captured = self.tmp / "captured"
        self.captured.mkdir()
        self.stub = self.tmp / "cold-store-stub"
        self.stub.write_text(
            "#!/usr/bin/env bash\n"
            f'echo "$@" >>"{self.calls_log}"\n'
            f'cp "$3" "{self.captured}/"\n'
        )
        self.stub.chmod(0o755)

    def run_export(self, with_token=True, extra_env=None):
        script = Path(__file__).resolve().parent.parent / "scripts" / "export-turso.sh"
        env = dict(os.environ)
        env.pop("TURSO_PLATFORM_API_TOKEN", None)
        if with_token:
            env["TURSO_PLATFORM_API_TOKEN"] = FakeTurso.PLATFORM_TOKEN
        env.update(
            {
                "TURSO_ORG": FakeTurso.ORG,
                "TURSO_API_BASE": self.base,
                "TURSO_DUMP_BASE": self.base,
                "COLD_STORE_BIN": str(self.stub),
                "NO_PROXY": "127.0.0.1,localhost",
                "no_proxy": "127.0.0.1,localhost",
            }
        )
        if extra_env:
            env.update(extra_env)
        return subprocess.run(
            ["bash", str(script)],
            env=env,
            capture_output=True,
            text=True,
            timeout=120,
            check=False,
        )

    def test_files_every_database(self):
        proc = self.run_export()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("turso export: all 2 database(s) filed", proc.stdout)
        calls = self.calls_log.read_text().splitlines()
        self.assertEqual(len(calls), 2)
        for name in ("fake-one", "fake-two"):
            self.assertTrue(
                any(
                    c.startswith("put-backup turso ") and f"{name}.db.gz" in c
                    for c in calls
                ),
                calls,
            )
            raw = gzip.decompress((self.captured / f"{name}.db.gz").read_bytes())
            check = self.tmp / f"check-{name}.db"
            check.write_bytes(raw)
            con = sqlite3.connect(str(check))
            try:
                self.assertEqual(
                    con.execute("PRAGMA integrity_check").fetchone()[0], "ok"
                )
                self.assertEqual(
                    con.execute("SELECT body FROM notes WHERE id=1").fetchone()[0],
                    "hello",
                )
            finally:
                con.close()
        # No token material (platform or per-database) is ever printed.
        self.assertNotIn(FakeTurso.PLATFORM_TOKEN, proc.stdout + proc.stderr)
        self.assertNotIn("fake-jwt", proc.stdout + proc.stderr)

    def test_missing_token_skips_cleanly(self):
        proc = self.run_export(with_token=False)
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn(
            "turso export: skipped (TURSO_PLATFORM_API_TOKEN not set)", proc.stdout
        )
        self.assertFalse(self.calls_log.exists())

    def test_one_dump_failure_still_files_the_other(self):
        self.fake.failing_dumps = {"fake-two"}
        proc = self.run_export()
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("fake-two", proc.stderr)
        self.assertTrue((self.captured / "fake-one.db.gz").is_file())
        self.assertFalse((self.captured / "fake-two.db.gz").exists())

    def test_dump_download_offers_compression(self):
        # --compressed on the client: the dump endpoint must see an
        # Accept-Encoding offering gzip (uncompressed GB-class dumps
        # get cut mid-stream by the sandbox proxy).
        proc = self.run_export()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertEqual(set(self.fake.dump_accept_encodings), {"fake-one", "fake-two"})
        for name, encoding in self.fake.dump_accept_encodings.items():
            self.assertIn("gzip", encoding or "", name)

    def test_truncated_dump_is_rejected_and_other_still_files(self):
        # A dump whose tail lacks COMMIT; would rebuild into an
        # integrity-clean but empty database — it must be refused
        # before rebuilding, by name, without starving the other DB.
        self.fake.truncated_dumps = {"fake-two"}
        proc = self.run_export()
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("fake-two", proc.stderr)
        self.assertIn("truncated", proc.stderr)
        self.assertTrue((self.captured / "fake-one.db.gz").is_file())
        self.assertFalse((self.captured / "fake-two.db.gz").exists())

    def test_committed_but_tableless_dump_is_rejected(self):
        self.fake.empty_dumps = {"fake-two"}
        proc = self.run_export()
        self.assertNotEqual(proc.returncode, 0)
        self.assertIn("fake-two", proc.stderr)
        self.assertIn("no user tables", proc.stderr)
        self.assertTrue((self.captured / "fake-one.db.gz").is_file())
        self.assertFalse((self.captured / "fake-two.db.gz").exists())

    def check_fts_export(self, name):
        """The filed export of the FTS fixture: virtual table registered
        with its SQL exactly as authored, MATCH serving the seeded row."""
        raw = gzip.decompress((self.captured / f"{name}.db.gz").read_bytes())
        check = self.tmp / f"check-{name}.db"
        check.write_bytes(raw)
        con = sqlite3.connect(str(check))
        try:
            self.assertEqual(con.execute("PRAGMA integrity_check").fetchone()[0], "ok")
            self.assertEqual(
                con.execute(
                    "SELECT sql FROM sqlite_master WHERE name='chunks_fts'"
                ).fetchone()[0],
                "CREATE VIRTUAL TABLE chunks_fts\n"
                "      USING fts5(text, content='chunks', content_rowid='id')",
            )
            self.assertEqual(
                con.execute(
                    "SELECT rowid FROM chunks_fts WHERE chunks_fts MATCH 'hello'"
                ).fetchall(),
                [(1,)],
            )
        finally:
            con.close()

    def test_unescaped_virtual_table_dump_is_repaired_and_files(self):
        # #1391: the mis-escaped writable_schema INSERT would kill the
        # sqlite3 -bail rebuild with a syntax error; the repair filter
        # must rewrite it so the database files, FTS index intact.
        self.fake.fts_dumps = {"fake-two"}
        proc = self.run_export()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("turso export: all 2 database(s) filed", proc.stdout)
        self.check_fts_export("fake-two")

    def test_escaped_virtual_table_dump_files_unchanged(self):
        # A correctly-escaped schema INSERT needs no repair; if the
        # filter touched it (e.g. doubled the quotes again) the
        # registered SQL would differ from what was authored.
        self.fake.escaped_dumps = {"fake-two"}
        proc = self.run_export()
        self.assertEqual(proc.returncode, 0, proc.stderr)
        self.assertIn("turso export: all 2 database(s) filed", proc.stdout)
        self.check_fts_export("fake-two")

    def test_rebuild_streams_through_sqlite3_cli(self):
        # SQLITE3_BIN is the rebuild seam: a recording shim that
        # delegates to the real sqlite3 must be what builds and
        # verifies the export (never an in-memory python read).
        real_sqlite3 = shutil.which("sqlite3")
        if not real_sqlite3:
            self.skipTest("sqlite3 CLI unavailable")
        shim_log = self.tmp / "sqlite3-shim.log"
        shim = self.tmp / "sqlite3-shim"
        shim.write_text(
            "#!/usr/bin/env bash\n"
            f'echo "$@" >>"{shim_log}"\n'
            f'exec "{real_sqlite3}" "$@"\n'
        )
        shim.chmod(0o755)
        proc = self.run_export(extra_env={"SQLITE3_BIN": str(shim)})
        self.assertEqual(proc.returncode, 0, proc.stderr)
        calls = shim_log.read_text()
        self.assertIn("-bail", calls)
        self.assertIn("PRAGMA integrity_check;", calls)
        raw = gzip.decompress((self.captured / "fake-one.db.gz").read_bytes())
        check = self.tmp / "check-shim.db"
        check.write_bytes(raw)
        con = sqlite3.connect(str(check))
        try:
            self.assertEqual(con.execute("PRAGMA integrity_check").fetchone()[0], "ok")
            self.assertEqual(
                con.execute("SELECT body FROM notes WHERE id=1").fetchone()[0],
                "hello",
            )
        finally:
            con.close()


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
