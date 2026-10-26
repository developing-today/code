#!/usr/bin/env python3
"""cold-store reaper v1: archive files into Google Drive as immutable,
content-addressed bundles of concatenated per-object zstd frames.

Stdlib only. Compression/decompression is delegated to the `zstd` CLI.

Identity model (see README.md): the OAuth app holds ONLY the drive.file
scope, so it can see nothing but files/folders it creates itself. On first
run it creates its own root folder ("Cold Store Reaper") and remembers the
folder IDs in state.json. Folder IDs created by any other app are invisible
to it and are never used.

Secrets: client_id / client_secret / refresh_token live in credentials.json
(mode 600) under the cold-store home. They are loaded, never printed.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import mimetypes
import os
import sqlite3
import subprocess
import sys
import urllib.error
import urllib.parse
import urllib.request
from datetime import datetime, timedelta, timezone
from pathlib import Path

SCHEMA_PATH = Path(__file__).with_name("schema.sql")

DRIVE_SCOPE = "https://www.googleapis.com/auth/drive.file"
AUTH_URL_BASE = "https://accounts.google.com/o/oauth2/v2/auth"
DEFAULT_TOKEN_URL = "https://oauth2.googleapis.com/token"
DEFAULT_DRIVE_BASE = "https://www.googleapis.com/drive/v3"
DEFAULT_UPLOAD_BASE = "https://www.googleapis.com"  # + /upload/drive/v3/files
REDIRECT_URI = "http://localhost"

ROOT_FOLDER_NAME = "Cold Store Reaper"
SUBFOLDERS = ("bundles", "manifests", "db-backups")

# Design constant: bundles are sealed at this size in the full reaper;
# `pack` packs exactly what it is given regardless of size.
BUNDLE_TARGET_BYTES = 8 * 1024 * 1024 * 1024  # 8 GiB
UPLOAD_CHUNK_BYTES = 8 * 1024 * 1024

FOLDER_MIME = "application/vnd.google-apps.folder"


class ColdStoreError(Exception):
    pass


def utcnow_str() -> str:
    """SQLite datetime('now') compatible format (UTC, second precision)."""
    return datetime.now(timezone.utc).strftime("%Y-%m-%d %H:%M:%S")


def sha256_file(path: Path) -> str:
    h = hashlib.sha256()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


def md5_file(path: Path) -> str:
    h = hashlib.md5()
    with open(path, "rb") as f:
        for block in iter(lambda: f.read(1024 * 1024), b""):
            h.update(block)
    return h.hexdigest()


class _NoRedirect(urllib.request.HTTPRedirectHandler):
    def redirect_request(self, req, fp, code, msg, headers, newurl):
        return None


_OPENER = urllib.request.build_opener(_NoRedirect)


def http_request(method, url, headers=None, data=None, timeout=120):
    """Minimal HTTP helper. Returns (status, headers, body_bytes).

    Redirects are NOT followed: Drive resumable uploads answer chunk PUTs
    with 308 + a Range header, which is a protocol response, not a redirect.
    HTTP error statuses are returned the same way (never raised), so callers
    decide what each status means. No header or body is ever logged here:
    callers pass Authorization headers and token-endpoint bodies.
    """
    req = urllib.request.Request(url, data=data, method=method, headers=headers or {})
    try:
        with _OPENER.open(req, timeout=timeout) as resp:
            return resp.status, dict(resp.headers), resp.read()
    except urllib.error.HTTPError as e:
        return e.code, dict(e.headers or {}), e.read()
    except urllib.error.URLError as e:
        raise ColdStoreError(
            f"network error talking to {urllib.parse.urlparse(url).netloc}: {e.reason}"
        )


class ColdStore:
    def __init__(
        self, home: Path, token_url=None, drive_base=None, upload_base=None, zstd=None
    ):
        self.home = Path(home)
        self.credentials_path = self.home / "credentials.json"
        self.state_path = self.home / "state.json"
        self.manifest_path = self.home / "manifest.sqlite"
        self.bundles_dir = self.home / "bundles"
        self.token_url = token_url or os.environ.get(
            "COLD_STORE_TOKEN_URL", DEFAULT_TOKEN_URL
        )
        self.drive_base = (
            drive_base or os.environ.get("COLD_STORE_DRIVE_BASE", DEFAULT_DRIVE_BASE)
        ).rstrip("/")
        self.upload_base = (
            upload_base or os.environ.get("COLD_STORE_UPLOAD_BASE", DEFAULT_UPLOAD_BASE)
        ).rstrip("/")
        candidate = zstd or os.environ.get("COLD_STORE_ZSTD") or "/usr/local/bin/zstd"
        if not Path(candidate).exists():
            candidate = "zstd"  # fall back to PATH lookup
        self.zstd = candidate
        self._access_token = None

    # ------------------------------------------------------------------ init
    def init(self) -> dict:
        self.home.mkdir(parents=True, exist_ok=True)
        os.chmod(self.home, 0o700)
        self.bundles_dir.mkdir(exist_ok=True)
        created = []
        if not self.credentials_path.exists():
            template = {"client_id": "", "client_secret": "", "refresh_token": ""}
            self._write_private_json(self.credentials_path, template)
            created.append("credentials.json (template — fill from Bitwarden)")
        if not self.state_path.exists():
            self._write_json(self.state_path, {})
            created.append("state.json")
        had_manifest = self.manifest_path.exists()
        self.db()  # creates manifest.sqlite with the schema
        if not had_manifest:
            created.append("manifest.sqlite")
        return {"home": str(self.home), "created": created}

    def _write_private_json(self, path: Path, obj: dict) -> None:
        fd = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_TRUNC, 0o600)
        with os.fdopen(fd, "w") as f:
            json.dump(obj, f, indent=2)
            f.write("\n")
        os.chmod(path, 0o600)

    @staticmethod
    def _write_json(path: Path, obj: dict) -> None:
        path.write_text(json.dumps(obj, indent=2) + "\n")

    # ------------------------------------------------------------ credentials
    def load_credentials(self) -> dict:
        if not self.credentials_path.exists():
            raise ColdStoreError(
                f"no credentials at {self.credentials_path} — run `cold-store init`, "
                "then fill it from Bitwarden (never commit this file)"
            )
        creds = json.loads(self.credentials_path.read_text())
        for key in ("client_id", "client_secret"):
            if not creds.get(key):
                raise ColdStoreError(f"credentials.json is missing {key}")
        return creds

    def load_state(self) -> dict:
        if self.state_path.exists():
            return json.loads(self.state_path.read_text())
        return {}

    def save_state(self, state: dict) -> None:
        self._write_json(self.state_path, state)

    # ------------------------------------------------------------------- db
    def db(self) -> sqlite3.Connection:
        conn = sqlite3.connect(self.manifest_path)
        conn.row_factory = sqlite3.Row
        conn.execute("PRAGMA foreign_keys = ON")
        if not self.manifest_path.exists() or self._schema_missing(conn):
            conn.executescript(SCHEMA_PATH.read_text())
            conn.commit()
        return conn

    @staticmethod
    def _schema_missing(conn) -> bool:
        row = conn.execute(
            "SELECT count(*) AS n FROM sqlite_master WHERE type='table' AND name='objects'"
        ).fetchone()
        return row["n"] == 0

    def ledger_add(
        self, conn, direction: str, nbytes: int, kind: str, ref: str
    ) -> None:
        conn.execute(
            "INSERT INTO transfer_ledger (ts, direction, bytes, kind, ref) VALUES (?, ?, ?, ?, ?)",
            (utcnow_str(), direction, nbytes, kind, ref),
        )

    # ------------------------------------------------------------------ auth
    def auth_url(self) -> str:
        creds = self.load_credentials()
        params = {
            "client_id": creds["client_id"],
            "redirect_uri": REDIRECT_URI,
            "response_type": "code",
            "scope": DRIVE_SCOPE,
            "access_type": "offline",
            "prompt": "consent",
        }
        return AUTH_URL_BASE + "?" + urllib.parse.urlencode(params)

    def auth_exchange(self, code: str) -> dict:
        creds = self.load_credentials()
        body = urllib.parse.urlencode(
            {
                "code": code,
                "client_id": creds["client_id"],
                "client_secret": creds["client_secret"],
                "redirect_uri": REDIRECT_URI,
                "grant_type": "authorization_code",
            }
        ).encode()
        status, _headers, resp = http_request(
            "POST",
            self.token_url,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            data=body,
        )
        if status != 200:
            raise ColdStoreError(f"token exchange failed (HTTP {status})")
        payload = json.loads(resp)
        refresh = payload.get("refresh_token")
        if not refresh:
            raise ColdStoreError(
                "token response contained no refresh_token — re-run the grant "
                "from `cold-store auth-url` (prompt=consent is required)"
            )
        creds["refresh_token"] = refresh
        self._write_private_json(self.credentials_path, creds)
        return {"stored": "refresh_token", "expires_in": payload.get("expires_in")}

    def access_token(self) -> str:
        if self._access_token:
            return self._access_token
        state = self.load_state()
        cached = state.get("access_token")
        expires_at = state.get("access_token_expires_at", 0)
        if cached and expires_at > datetime.now(timezone.utc).timestamp() + 60:
            self._access_token = cached
            return cached
        creds = self.load_credentials()
        if not creds.get("refresh_token"):
            raise ColdStoreError(
                "no refresh_token in credentials.json — complete the grant: "
                "`cold-store auth-url`, then `cold-store auth-exchange --code <code>`"
            )
        body = urllib.parse.urlencode(
            {
                "client_id": creds["client_id"],
                "client_secret": creds["client_secret"],
                "refresh_token": creds["refresh_token"],
                "grant_type": "refresh_token",
            }
        ).encode()
        status, _headers, resp = http_request(
            "POST",
            self.token_url,
            headers={"Content-Type": "application/x-www-form-urlencoded"},
            data=body,
        )
        if status != 200:
            raise ColdStoreError(f"token refresh failed (HTTP {status})")
        payload = json.loads(resp)
        token = payload["access_token"]
        state["access_token"] = token
        state["access_token_expires_at"] = (
            datetime.now(timezone.utc)
            + timedelta(seconds=payload.get("expires_in", 3600))
        ).timestamp()
        self.save_state(state)
        self._access_token = token
        return token

    def _auth_headers(self, extra=None) -> dict:
        headers = {"Authorization": f"Bearer {self.access_token()}"}
        if extra:
            headers.update(extra)
        return headers

    # ----------------------------------------------------------------- drive
    def _find_folder(self, name: str, parent_id: str | None) -> str | None:
        q = f"name = '{name}' and mimeType = '{FOLDER_MIME}' and trashed = false"
        if parent_id:
            q += f" and '{parent_id}' in parents"
        url = (
            self.drive_base
            + "/files?"
            + urllib.parse.urlencode(
                {"q": q, "fields": "files(id,name)", "pageSize": 1}
            )
        )
        status, _h, resp = http_request("GET", url, headers=self._auth_headers())
        if status != 200:
            raise ColdStoreError(f"files.list failed (HTTP {status})")
        files = json.loads(resp).get("files", [])
        return files[0]["id"] if files else None

    def _create_folder(self, name: str, parent_id: str | None) -> str:
        meta = {"name": name, "mimeType": FOLDER_MIME}
        if parent_id:
            meta["parents"] = [parent_id]
        status, _h, resp = http_request(
            "POST",
            self.drive_base + "/files",
            headers=self._auth_headers({"Content-Type": "application/json"}),
            data=json.dumps(meta).encode(),
        )
        if status not in (200, 201):
            raise ColdStoreError(f"folder create failed (HTTP {status})")
        return json.loads(resp)["id"]

    def ensure_folders(self) -> dict:
        state = self.load_state()
        folders = dict(state.get("folders") or {})
        if not folders.get("root"):
            folders["root"] = self._find_folder(
                ROOT_FOLDER_NAME, None
            ) or self._create_folder(ROOT_FOLDER_NAME, None)
        for sub in SUBFOLDERS:
            if not folders.get(sub):
                folders[sub] = self._find_folder(
                    sub, folders["root"]
                ) or self._create_folder(sub, folders["root"])
        state["folders"] = folders
        self.save_state(state)
        return folders

    def _folder_id(self, key: str) -> str:
        folders = self.load_state().get("folders") or {}
        if key not in folders:
            folders = self.ensure_folders()
        return folders[key]

    # ------------------------------------------------------------------ pack
    def _run_zstd(self, args, **kwargs):
        try:
            return subprocess.run([self.zstd] + args, **kwargs)
        except FileNotFoundError:
            raise ColdStoreError(f"zstd binary not found (looked for {self.zstd!r})")

    def _compress_to_frame(self, src: Path, frame_path: Path) -> None:
        with open(frame_path, "wb") as out:
            proc = self._run_zstd(
                ["-q", "-f", "-3", "-c", str(src)],
                stdout=out,
                stderr=subprocess.PIPE,
            )
        if proc.returncode != 0:
            raise ColdStoreError(
                f"zstd failed for {src}: {proc.stderr.decode(errors='replace')[:200]}"
            )

    def decompress_frame(self, frame: bytes) -> bytes:
        proc = self._run_zstd(["-q", "-d", "-c"], input=frame, capture_output=True)
        if proc.returncode != 0:
            raise ColdStoreError("zstd decompression failed")
        return proc.stdout

    def pack(self, files: list[str]) -> dict:
        if not files:
            raise ColdStoreError("pack needs at least one file")
        self.bundles_dir.mkdir(parents=True, exist_ok=True)
        tmp_bundle = self.bundles_dir / "bundle.tmp"
        bundle_hash = hashlib.sha256()
        members = []
        offset = 0
        with open(tmp_bundle, "wb") as bundle_out:
            for name in files:
                src = Path(name)
                if not src.is_file():
                    raise ColdStoreError(f"not a file: {src}")
                obj_hash = sha256_file(src)
                frame_path = self.bundles_dir / "frame.tmp"
                self._compress_to_frame(src, frame_path)
                frame_len = frame_path.stat().st_size
                with open(frame_path, "rb") as fin:
                    for block in iter(lambda: fin.read(1024 * 1024), b""):
                        bundle_hash.update(block)
                        bundle_out.write(block)
                frame_path.unlink()
                members.append(
                    {
                        "hash": obj_hash,
                        "size": src.stat().st_size,
                        "content_type": mimetypes.guess_type(src.name)[0],
                        "offset": offset,
                        "length": frame_len,
                    }
                )
                offset += frame_len
        bundle_sha = bundle_hash.hexdigest()
        bundle_id = "b_" + bundle_sha[:16]
        final_path = self.bundles_dir / f"{bundle_id}.csb"
        if final_path.exists():
            tmp_bundle.unlink()
        else:
            tmp_bundle.rename(final_path)
        total_bytes = final_path.stat().st_size
        conn = self.db()
        with conn:
            conn.execute(
                "INSERT OR IGNORE INTO bundles (bundle_id, sha256, bytes, member_count, live_bytes, sealed_at, state)"
                " VALUES (?, ?, ?, ?, ?, ?, 'sealed')",
                (
                    bundle_id,
                    bundle_sha,
                    total_bytes,
                    len(members),
                    sum(m["size"] for m in members),
                    utcnow_str(),
                ),
            )
            for m in members:
                conn.execute(
                    "INSERT OR IGNORE INTO objects (hash, size, content_type, state) VALUES (?, ?, ?, 'staging')",
                    (m["hash"], m["size"], m["content_type"]),
                )
                conn.execute(
                    "INSERT OR IGNORE INTO members (bundle_id, object_hash, offset, length) VALUES (?, ?, ?, ?)",
                    (bundle_id, m["hash"], m["offset"], m["length"]),
                )
        conn.close()
        return {
            "bundle_id": bundle_id,
            "path": str(final_path),
            "bytes": total_bytes,
            "members": len(members),
        }

    # ---------------------------------------------------------------- upload
    def _resumable_upload(self, path: Path, name: str, parent_id: str) -> str:
        total = path.stat().st_size
        meta = {"name": name, "parents": [parent_id]}
        status, headers, _resp = http_request(
            "POST",
            self.upload_base + "/upload/drive/v3/files?uploadType=resumable",
            headers=self._auth_headers(
                {
                    "Content-Type": "application/json; charset=UTF-8",
                    "X-Upload-Content-Type": "application/octet-stream",
                    "X-Upload-Content-Length": str(total),
                }
            ),
            data=json.dumps(meta).encode(),
        )
        if status not in (200, 201):
            raise ColdStoreError(f"resumable session create failed (HTTP {status})")
        session_uri = headers.get("Location") or headers.get("location")
        if not session_uri:
            raise ColdStoreError("resumable session create returned no Location header")
        sent = 0
        file_id = None
        with open(path, "rb") as f:
            while sent < total or total == 0:
                chunk = f.read(min(UPLOAD_CHUNK_BYTES, total - sent)) if total else b""
                end = sent + len(chunk) - 1
                content_range = f"bytes {sent}-{end}/{total}" if total else "bytes */0"
                status, headers, resp = http_request(
                    "PUT",
                    session_uri,
                    headers={
                        "Content-Length": str(len(chunk)),
                        "Content-Range": content_range,
                    },
                    data=chunk,
                    timeout=600,
                )
                if status in (200, 201):
                    file_id = json.loads(resp)["id"]
                    break
                if status == 308:
                    range_hdr = headers.get("Range") or headers.get("range") or ""
                    # "bytes=0-12345" => next byte is 12346
                    try:
                        sent = int(range_hdr.rsplit("-", 1)[1]) + 1
                    except (IndexError, ValueError):
                        sent += len(chunk)
                    f.seek(sent)
                    continue
                raise ColdStoreError(f"chunk upload failed (HTTP {status})")
        if not file_id:
            raise ColdStoreError("upload finished without a file id")
        return file_id

    def upload(self, bundle_ref: str) -> dict:
        conn = self.db()
        row = conn.execute(
            "SELECT * FROM bundles WHERE bundle_id = ?", (bundle_ref,)
        ).fetchone()
        if row is None and Path(bundle_ref).is_file():
            stem = Path(bundle_ref).stem
            row = conn.execute(
                "SELECT * FROM bundles WHERE bundle_id = ?", (stem,)
            ).fetchone()
        if row is None:
            conn.close()
            raise ColdStoreError(f"unknown bundle: {bundle_ref}")
        bundle_id = row["bundle_id"]
        path = self.bundles_dir / f"{bundle_id}.csb"
        if not path.is_file():
            conn.close()
            raise ColdStoreError(f"bundle file missing: {path}")
        if row["state"] == "verified":
            conn.close()
            return {
                "bundle_id": bundle_id,
                "already": "verified",
                "drive_file_id": row["drive_file_id"],
            }
        with conn:
            conn.execute(
                "UPDATE bundles SET state='uploading' WHERE bundle_id=?", (bundle_id,)
            )
        conn.close()
        file_id = self._resumable_upload(
            path, f"{bundle_id}.csb", self._folder_id("bundles")
        )
        status, _h, resp = http_request(
            "GET",
            self.drive_base
            + f"/files/{file_id}?"
            + urllib.parse.urlencode({"fields": "id,md5Checksum,size"}),
            headers=self._auth_headers(),
        )
        if status != 200:
            raise ColdStoreError(f"files.get after upload failed (HTTP {status})")
        info = json.loads(resp)
        local_md5 = md5_file(path)
        drive_size = int(info.get("size", -1))
        if info.get("md5Checksum") != local_md5 or drive_size != row["bytes"]:
            raise ColdStoreError(
                "verification mismatch: Drive reports size/md5 that differ from "
                "the local bundle — bundle NOT marked verified"
            )
        conn = self.db()
        with conn:
            conn.execute(
                "UPDATE bundles SET state='verified', drive_file_id=?, drive_md5=?, uploaded_at=?"
                " WHERE bundle_id=?",
                (file_id, info["md5Checksum"], utcnow_str(), bundle_id),
            )
            conn.execute(
                "UPDATE objects SET state='cold' WHERE hash IN"
                " (SELECT object_hash FROM members WHERE bundle_id=?)",
                (bundle_id,),
            )
            self.ledger_add(conn, "upload", row["bytes"], "bundle", bundle_id)
        conn.close()
        return {"bundle_id": bundle_id, "drive_file_id": file_id, "verified": True}

    # ----------------------------------------------------------------- fetch
    def fetch(self, object_hash: str, out_path: str) -> dict:
        conn = self.db()
        row = conn.execute(
            "SELECT m.offset AS offset, m.length AS length, b.drive_file_id AS drive_file_id,"
            " b.bundle_id AS bundle_id FROM members m JOIN bundles b ON b.bundle_id = m.bundle_id"
            " WHERE m.object_hash = ? AND b.state = 'verified' ORDER BY b.uploaded_at LIMIT 1",
            (object_hash,),
        ).fetchone()
        conn.close()
        if row is None:
            raise ColdStoreError(
                f"object not found in any verified bundle: {object_hash}"
            )
        start, length = row["offset"], row["length"]
        url = self.drive_base + f"/files/{row['drive_file_id']}?alt=media"
        status, _h, resp = http_request(
            "GET",
            url,
            headers=self._auth_headers(
                {"Range": f"bytes={start}-{start + length - 1}"}
            ),
        )
        if status not in (200, 206):
            raise ColdStoreError(f"range download failed (HTTP {status})")
        frame = resp[:length] if status == 200 else resp
        if len(frame) != length:
            raise ColdStoreError(
                f"range download short: got {len(frame)} of {length} bytes"
            )
        data = self.decompress_frame(frame)
        if hashlib.sha256(data).hexdigest() != object_hash:
            raise ColdStoreError(
                "sha256 mismatch after decompression — output NOT written"
            )
        Path(out_path).write_bytes(data)
        conn = self.db()
        with conn:
            self.ledger_add(conn, "download", length, "rehydrate", object_hash)
        conn.close()
        return {
            "hash": object_hash,
            "out": out_path,
            "bytes": len(data),
            "bundle_id": row["bundle_id"],
        }

    # ---------------------------------------------------------------- status
    def status_data(self) -> dict:
        conn = self.db()
        objects_total = conn.execute("SELECT count(*) n FROM objects").fetchone()["n"]
        objects_cold = conn.execute(
            "SELECT count(*) n FROM objects WHERE state='cold'"
        ).fetchone()["n"]
        bundles_total = conn.execute("SELECT count(*) n FROM bundles").fetchone()["n"]
        bundles_verified = conn.execute(
            "SELECT count(*) n FROM bundles WHERE state='verified'"
        ).fetchone()["n"]
        cold_bytes = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM bundles WHERE state='verified'"
        ).fetchone()["n"]
        up_24h = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM transfer_ledger"
            " WHERE direction='upload' AND ts >= datetime('now','-1 day')"
        ).fetchone()["n"]
        down_24h = conn.execute(
            "SELECT coalesce(sum(bytes),0) n FROM transfer_ledger"
            " WHERE direction='download' AND ts >= datetime('now','-1 day')"
        ).fetchone()["n"]
        meta = {
            r["key"]: r["value"] for r in conn.execute("SELECT key, value FROM meta")
        }
        conn.close()
        hard_stop = int(meta.get("daily_upload_hard_stop_bytes", "700000000000"))
        budget = int(meta.get("project_budget_bytes", "0"))
        return {
            "objects_total": objects_total,
            "objects_cold": objects_cold,
            "bundles_total": bundles_total,
            "bundles_verified": bundles_verified,
            "cold_bytes": cold_bytes,
            "uploads_24h_bytes": up_24h,
            "downloads_24h_bytes": down_24h,
            "daily_upload_hard_stop_bytes": hard_stop,
            "hard_stop_ok": up_24h < hard_stop,
            "project_budget_bytes": budget,
            "within_budget": cold_bytes <= budget if budget else True,
        }


def home_from_env() -> Path:
    return Path(
        os.environ.get("COLD_STORE_HOME", Path.home() / ".config" / "cold-store")
    )


def build_store(args) -> ColdStore:
    home = Path(args.home) if getattr(args, "home", None) else home_from_env()
    return ColdStore(home)


def _human(n: int) -> str:
    return f"{n} bytes ({n / 1e9:.2f} GB)"


def main(argv=None) -> int:
    parser = argparse.ArgumentParser(
        prog="cold-store", description=__doc__.splitlines()[0]
    )
    parser.add_argument(
        "--home",
        help="cold-store home (default: $COLD_STORE_HOME or ~/.config/cold-store)",
    )
    sub = parser.add_subparsers(dest="cmd", required=True)
    sub.add_parser(
        "init", help="create the cold-store home, credential template and manifest DB"
    )
    sub.add_parser("auth-url", help="print the OAuth consent URL for the manual grant")
    p_exchange = sub.add_parser(
        "auth-exchange", help="exchange a grant code and store the refresh token"
    )
    p_exchange.add_argument("--code", required=True)
    sub.add_parser("ensure-folders", help="create/find the app-owned Drive folder tree")
    p_pack = sub.add_parser(
        "pack", help="pack files into a local bundle and record it in the manifest"
    )
    p_pack.add_argument("files", nargs="+")
    p_upload = sub.add_parser(
        "upload", help="resumable-upload a bundle, verify it, mark it verified"
    )
    p_upload.add_argument("bundle", help="bundle id (b_...) or path to the .csb file")
    p_fetch = sub.add_parser(
        "fetch", help="range-download, decompress and verify one object"
    )
    p_fetch.add_argument("hash")
    p_fetch.add_argument("out_path")
    sub.add_parser(
        "status",
        help="manifest counts, cold bytes, last-24h transfers, hard-stop check",
    )
    args = parser.parse_args(argv)
    store = build_store(args)
    try:
        if args.cmd == "init":
            result = store.init()
            print(f"cold-store home: {result['home']}")
            for item in result["created"]:
                print(f"  ensured: {item}")
            print(
                "fill credentials.json from Bitwarden (mode 600; never commit it), then:"
            )
            print("  cold-store auth-url   # open, grant, copy the code")
            print("  cold-store auth-exchange --code <code>")
        elif args.cmd == "auth-url":
            print(store.auth_url())
        elif args.cmd == "auth-exchange":
            result = store.auth_exchange(args.code)
            print(
                f"refresh token stored in {store.credentials_path} (expires_in={result['expires_in']})"
            )
        elif args.cmd == "ensure-folders":
            folders = store.ensure_folders()
            for key in ("root",) + SUBFOLDERS:
                print(f"{key}: {folders[key]}")
        elif args.cmd == "pack":
            result = store.pack(args.files)
            print(
                f"bundle {result['bundle_id']}: {_human(result['bytes'])}, {result['members']} member(s)"
            )
            print(f"  at {result['path']}")
        elif args.cmd == "upload":
            result = store.upload(args.bundle)
            print(
                f"bundle {result['bundle_id']}: verified in Drive as {result.get('drive_file_id')}"
            )
        elif args.cmd == "fetch":
            result = store.fetch(args.hash, args.out_path)
            print(
                f"{result['hash'][:16]}... -> {result['out']}: {_human(result['bytes'])} (bundle {result['bundle_id']})"
            )
        elif args.cmd == "status":
            d = store.status_data()
            print(f"objects: {d['objects_total']} total, {d['objects_cold']} cold")
            print(
                f"bundles: {d['bundles_total']} total, {d['bundles_verified']} verified"
            )
            print(
                f"cold bytes in Drive: {_human(d['cold_bytes'])} of budget {_human(d['project_budget_bytes'])}"
                f" ({'within' if d['within_budget'] else 'OVER'} budget)"
            )
            print(
                f"last 24h: uploads {_human(d['uploads_24h_bytes'])}, downloads {_human(d['downloads_24h_bytes'])}"
            )
            print(
                f"daily upload hard stop: {_human(d['daily_upload_hard_stop_bytes'])} — "
                f"{'OK' if d['hard_stop_ok'] else 'EXCEEDED'}"
            )
    except ColdStoreError as e:
        print(f"error: {e}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
